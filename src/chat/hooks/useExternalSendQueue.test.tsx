import { renderHook, act } from '@testing-library/react'
import { StrictMode, useEffect } from 'react'
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest'
import { useExternalSendQueue } from './useExternalSendQueue'
import { api } from '../../api/tauri'
import type { Conversation } from '../types'

vi.mock('../../api/tauri', () => ({
  api: {
    chatTakeExternalSends: vi.fn(),
    chatAckExternalSend: vi.fn(),
    chatReleaseExternalSends: vi.fn(),
    chatRenewExternalSends: vi.fn(),
  },
}))

const mockTake = vi.mocked(api.chatTakeExternalSends)
const mockAck = vi.mocked(api.chatAckExternalSend)
const mockRelease = vi.mocked(api.chatReleaseExternalSends)
const mockRenew = vi.mocked(api.chatRenewExternalSends)

/**
 * 回归重点：
 *   1. 单飞 —— drain 进行中再次调用只置 requested 标志，不并发取消息
 *   2. 发送被拒（正在生成）时请求留在队首、不 shift 掉，并置 requested
 *   3. 历史预置分支走 import 而非 send
 *   4. 附件映射（type/name 兜底、无 path 的被过滤）
 */
function setup() {
  const onEnterConversationView = vi.fn()
  const onImportConversation = vi.fn().mockResolvedValue(true)
  const onSendMessage = vi.fn().mockResolvedValue(true)
  const onError = vi.fn()
  const rendered = renderHook(() => useExternalSendQueue({
    onEnterConversationView, onImportConversation, onSendMessage, onError,
  }))
  return { ...rendered, onEnterConversationView, onImportConversation, onSendMessage, onError }
}

const partialConversation = {
  id: 'created-partial', revision: 1, title: 'partial', provider_id: 'p', model: 'm',
  messages: [], created_at: 1, updated_at: 1,
} as Conversation

beforeEach(() => {
  mockTake.mockReset()
  mockTake.mockResolvedValue({ success: true, requests: [] } as never)
  mockAck.mockReset()
  mockAck.mockResolvedValue({ success: true })
  mockRelease.mockReset()
  mockRelease.mockResolvedValue({ success: true })
  mockRenew.mockReset()
  mockRenew.mockResolvedValue({ success: true, renewed: 1 })
})

afterEach(() => { vi.useRealTimers() })

describe('useExternalSendQueue 基本流转', () => {
  it('无消息时不发送、不报错', async () => {
    const { result, onSendMessage, onError } = setup()
    await act(async () => { await result.current.drainExternalSends() })
    expect(onSendMessage).not.toHaveBeenCalled()
    expect(onError).not.toHaveBeenCalled()
  })

  it('取到消息后切视图并发送', async () => {
    mockTake.mockResolvedValueOnce({
      success: true,
      requests: [{ id: 'r1', content: '你好', attachments: [] }],
    } as never)
    const { result, onEnterConversationView, onSendMessage } = setup()
    await act(async () => { await result.current.drainExternalSends() })
    expect(onEnterConversationView).toHaveBeenCalled()
    expect(onSendMessage).toHaveBeenCalledWith('你好', [], expect.objectContaining({ forceNewConversation: true }))
    expect(mockAck).toHaveBeenCalledWith(expect.any(String), 'r1')
  })

  it('带 messages 的请求走 import 而非 send', async () => {
    mockTake.mockResolvedValueOnce({
      success: true,
      requests: [{
        id: 'r2',
        messages: [{ role: 'user', content: '历史' }],
        attachments: [{ id: 'a1', type: 'image', path: '/tmp/a.png' }],
      }],
    } as never)
    const { result, onImportConversation, onSendMessage } = setup()
    await act(async () => { await result.current.drainExternalSends() })
    expect(onImportConversation).toHaveBeenCalledWith(
      [{ role: 'user', content: '历史' }],
      ['/tmp/a.png'],
    )
    expect(onSendMessage).not.toHaveBeenCalled()
  })

  it('历史导入返回 false 时保留已取走的请求，后续唤醒可重试', async () => {
    mockTake
      .mockResolvedValueOnce({ success: true, requests: [{ id: 'history-retry', messages: [{ role: 'user', content: '历史' }] }] } as never)
      .mockResolvedValue({ success: true, requests: [] } as never)
    const { result, onImportConversation } = setup()
    onImportConversation.mockResolvedValueOnce(false).mockResolvedValueOnce(true)

    await act(async () => { await result.current.drainExternalSends() })
    expect(onImportConversation).toHaveBeenCalledTimes(1)
    expect(mockAck).not.toHaveBeenCalled()
    await act(async () => { await result.current.wakeAfterRun() })
    expect(onImportConversation).toHaveBeenCalledTimes(2)
    expect(mockAck).toHaveBeenCalledWith(expect.any(String), 'history-retry')
  })

  it('附件映射：无 path 的被过滤，name/type 有兜底', async () => {
    mockTake.mockResolvedValueOnce({
      success: true,
      requests: [{
        id: 'r3',
        content: '看图',
        attachments: [
          { id: '', type: 'file', path: '/tmp/doc.pdf' },
          { id: 'x', type: 'image', name: '截图', path: '/tmp/s.png' },
          { id: 'no-path', type: 'image' },
        ],
      }],
    } as never)
    const { result, onSendMessage } = setup()
    await act(async () => { await result.current.drainExternalSends() })
    const attachments = onSendMessage.mock.calls[0][1]
    expect(attachments).toHaveLength(2)
    expect(attachments[0]).toMatchObject({ id: 'external-r3-0', type: 'file', name: 'Attachment' })
    expect(attachments[1]).toMatchObject({ id: 'x', type: 'image', name: '截图' })
  })
})

describe('useExternalSendQueue 单飞与重排', () => {
  it('StrictMode 旧 cleanup 的延迟 release 不会释放新 setup 的认领', async () => {
    let finishRelease!: () => void
    mockRelease.mockImplementationOnce(() => new Promise((resolve) => {
      finishRelease = () => resolve({ success: true })
    }))
    const onEnterConversationView = vi.fn()
    const onImportConversation = vi.fn().mockResolvedValue(true)
    const onSendMessage = vi.fn().mockResolvedValue(true)
    const onError = vi.fn()
    const { result, rerender, unmount } = renderHook(() => useExternalSendQueue({
      onEnterConversationView, onImportConversation, onSendMessage, onError,
    }), { wrapper: StrictMode })

    expect(mockRelease).toHaveBeenCalledTimes(1)
    const drain = result.current.drainExternalSends
    const wake = result.current.wakeAfterRun
    rerender()
    expect(result.current.drainExternalSends).toBe(drain)
    expect(result.current.wakeAfterRun).toBe(wake)
    await act(async () => { await result.current.drainExternalSends() })
    expect(mockTake.mock.calls[0][0]).not.toBe(mockRelease.mock.calls[0][0])
    finishRelease()
    unmount()
  })

  it('StrictMode 首次 setup 的迟到 take 不进入第二次 setup 的发送', async () => {
    let finishTake!: (value: unknown) => void
    mockTake
      .mockImplementationOnce(() => new Promise((resolve) => { finishTake = resolve }) as never)
      .mockResolvedValueOnce({ success: true, requests: [{ id: 'new-owner', content: 'new' }] } as never)
      .mockResolvedValue({ success: true, requests: [] } as never)
    const onEnterConversationView = vi.fn()
    const onImportConversation = vi.fn().mockResolvedValue(true)
    const onSendMessage = vi.fn().mockResolvedValue(true)
    const onError = vi.fn()
    let started = false
    let firstDrain!: Promise<void>
    const { result, unmount } = renderHook(() => {
      const queue = useExternalSendQueue({ onEnterConversationView, onImportConversation, onSendMessage, onError })
      const { drainExternalSends } = queue
      useEffect(() => {
        if (!started) {
          started = true
          firstDrain = drainExternalSends()
        }
      }, [drainExternalSends])
      return queue
    }, { wrapper: StrictMode })

    finishTake({ success: true, requests: [{ id: 'old-owner', content: 'old' }] })
    await act(async () => { await firstDrain })
    expect(onSendMessage).not.toHaveBeenCalled()
    await act(async () => { await result.current.drainExternalSends() })
    expect(onSendMessage.mock.calls.map((call) => call[0])).toEqual(['new'])
    unmount()
  })

  it('窗口卸载后未确认的请求由新窗口重新认领，旧窗口不确认', async () => {
    mockTake
      .mockResolvedValueOnce({ success: true, requests: [{ id: 'handoff', content: 'X' }] } as never)
      .mockResolvedValueOnce({ success: true, requests: [{ id: 'handoff', content: 'X' }] } as never)
      .mockResolvedValue({ success: true, requests: [] } as never)
    let finishOld!: (accepted: boolean) => void
    const old = setup()
    old.onSendMessage.mockImplementationOnce(() => new Promise((resolve) => { finishOld = resolve }))

    const pending = old.result.current.drainExternalSends()
    await Promise.resolve()
    expect(old.onSendMessage).toHaveBeenCalledTimes(1)
    old.unmount()
    expect(mockRelease).toHaveBeenCalledTimes(1)
    finishOld(false)
    await pending
    expect(mockAck).not.toHaveBeenCalled()

    const reopened = setup()
    await act(async () => { await reopened.result.current.drainExternalSends() })
    expect(reopened.onSendMessage).toHaveBeenCalledTimes(1)
    expect(mockAck).toHaveBeenCalledWith(expect.any(String), 'handoff')
    expect(mockAck.mock.calls[0][0]).not.toBe(mockRelease.mock.calls[0][0])
    reopened.unmount()
  })

  it('发送已成功而确认暂时失败时只重试确认，不再次发送', async () => {
    mockTake
      .mockResolvedValueOnce({ success: true, requests: [{ id: 'ack-retry', content: 'X' }] } as never)
      .mockResolvedValueOnce({ success: true, requests: [{ id: 'ack-retry', content: 'X' }] } as never)
      .mockResolvedValue({ success: true, requests: [] } as never)
    mockAck.mockRejectedValueOnce(new Error('ack unavailable'))
    const { result, onSendMessage, onError, unmount } = setup()
    const error = vi.spyOn(console, 'error').mockImplementation(() => undefined)

    await act(async () => { await result.current.drainExternalSends() })
    expect(onError).toHaveBeenCalledWith('ack unavailable')
    await act(async () => { await result.current.wakeAfterRun() })
    expect(onSendMessage).toHaveBeenCalledTimes(1)
    expect(mockAck).toHaveBeenCalledTimes(2)
    error.mockRestore()
    unmount()
  })

  it('长时间发送期间续租，卸载后停止续租', async () => {
    vi.useFakeTimers()
    mockTake.mockResolvedValueOnce({ success: true, requests: [{ id: 'long-run', content: 'X' }] } as never)
    let finish!: (accepted: boolean) => void
    const { result, onSendMessage, unmount } = setup()
    onSendMessage.mockImplementationOnce(() => new Promise((resolve) => { finish = resolve }))

    const pending = result.current.drainExternalSends()
    await Promise.resolve()
    expect(onSendMessage).toHaveBeenCalledTimes(1)
    await vi.advanceTimersByTimeAsync(20_000)
    expect(mockRenew).toHaveBeenCalledWith(expect.any(String))
    const renewals = mockRenew.mock.calls.length
    unmount()
    finish(false)
    await pending
    await vi.advanceTimersByTimeAsync(30_000)
    expect(mockRenew).toHaveBeenCalledTimes(renewals)
  })

  it('旧窗口释放未到达时，新窗口持续重试租约后可取到原请求', async () => {
    vi.useFakeTimers()
    mockTake
      .mockResolvedValueOnce({ success: true, requests: [], pendingLeased: true } as never)
      .mockResolvedValueOnce({ success: true, requests: [{ id: 'leased', content: 'X' }], pendingLeased: false } as never)
      .mockResolvedValue({ success: true, requests: [], pendingLeased: false } as never)
    const { result, onSendMessage, unmount } = setup()

    await act(async () => { await result.current.drainExternalSends() })
    expect(onSendMessage).not.toHaveBeenCalled()
    await act(async () => { await vi.advanceTimersByTimeAsync(100) })
    expect(onSendMessage).toHaveBeenCalledTimes(1)
    expect(mockAck).toHaveBeenCalledWith(expect.any(String), 'leased')
    unmount()
  })

  it('卸载期间 take 才失败，不会在 finally 重新挂定时器', async () => {
    vi.useFakeTimers()
    let rejectTake!: (error: Error) => void
    mockTake.mockImplementationOnce(() => new Promise((_, reject) => { rejectTake = reject }))
    const { result, unmount, onError } = setup()
    const error = vi.spyOn(console, 'error').mockImplementation(() => undefined)

    const pending = result.current.drainExternalSends()
    unmount()
    rejectTake(new Error('late failure'))
    await pending
    await vi.advanceTimersByTimeAsync(5000)

    expect(onError).not.toHaveBeenCalled()
    expect(mockTake).toHaveBeenCalledTimes(1)
    error.mockRestore()
  })

  it('卸载期间发送才被拒绝，不会继续重试该外部请求', async () => {
    vi.useFakeTimers()
    mockTake.mockResolvedValueOnce({ success: true, requests: [{ id: 'late-send', content: 'X' }] } as never)
    let rejectSend!: (accepted: boolean) => void
    const { result, onSendMessage, unmount } = setup()
    onSendMessage.mockImplementationOnce(() => new Promise((resolve) => { rejectSend = resolve }))

    const pending = result.current.drainExternalSends()
    await Promise.resolve()
    expect(onSendMessage).toHaveBeenCalledTimes(1)
    unmount()
    rejectSend(false)
    await pending
    await vi.advanceTimersByTimeAsync(5000)

    expect(onSendMessage).toHaveBeenCalledTimes(1)
    expect(mockTake).toHaveBeenCalledTimes(1)
  })

  it('忙碌拒绝后不以 0ms 自旋，执行结束信号立即重试部分创建的会话', async () => {
    vi.useFakeTimers()
    mockTake
      .mockResolvedValueOnce({ success: true, requests: [{ id: 'partial-wake', content: 'X', attachments: [] }] } as never)
      .mockResolvedValue({ success: true, requests: [] } as never)
    const { result, onSendMessage, unmount } = setup()
    onSendMessage.mockImplementationOnce(async (_content, _attachments, options) => {
      options.onPartialConversation(partialConversation)
      return false
    })

    await act(async () => { await result.current.drainExternalSends() })
    await act(async () => { await vi.advanceTimersByTimeAsync(0) })
    expect(onSendMessage).toHaveBeenCalledTimes(1)
    onSendMessage.mockResolvedValueOnce(true)
    await act(async () => { await result.current.wakeAfterRun() })
    expect(onSendMessage).toHaveBeenCalledTimes(2)
    expect(onSendMessage.mock.calls[1][2].conversationOverride).toEqual(partialConversation)
    unmount()
  })

  it('take 失败后会以有界定时器自愈，无需页面 coarse 补偿 effect', async () => {
    vi.useFakeTimers()
    mockTake
      .mockRejectedValueOnce(new Error('temporary'))
      .mockResolvedValue({ success: true, requests: [] } as never)
    const { result, onError, unmount } = setup()
    const error = vi.spyOn(console, 'error').mockImplementation(() => undefined)

    await act(async () => { await result.current.drainExternalSends() })
    expect(onError).toHaveBeenCalledWith('temporary')
    expect(mockTake).toHaveBeenCalledTimes(1)
    await act(async () => { await vi.advanceTimersByTimeAsync(99) })
    expect(mockTake).toHaveBeenCalledTimes(1)
    await act(async () => { await vi.advanceTimersByTimeAsync(1) })
    expect(mockTake).toHaveBeenCalledTimes(2)
    error.mockRestore()
    unmount()
  })
  it('drain 进行中再次调用不并发取消息', async () => {
    let release: (() => void) | undefined
    mockTake.mockImplementationOnce(() => new Promise((resolve) => {
      release = () => resolve({ success: true, requests: [] } as never)
    }))
    const { result, unmount } = setup()
    const drain = result.current.drainExternalSends

    const first = drain()
    // 第一次仍挂在 take 上；第二次应立即返回且不再调 take
    await drain()
    expect(mockTake).toHaveBeenCalledTimes(1)

    // 放行并收尾，避免悬挂 promise 影响后续用例
    mockTake.mockResolvedValue({ success: true, requests: [] } as never)
    release?.()
    await first
    unmount()
  })

  it('发送被拒时请求留在队首，下次重试', async () => {
    mockTake
      .mockResolvedValueOnce({ success: true, requests: [{ id: 'r4', content: 'X', attachments: [] }] } as never)
      .mockResolvedValueOnce({ success: true, requests: [{ id: 'r4', content: 'X', attachments: [] }] } as never)
      .mockResolvedValue({ success: true, requests: [] } as never)
    const { result, onSendMessage } = setup()
    onSendMessage.mockResolvedValueOnce(false)
    await act(async () => { await result.current.drainExternalSends() })
    expect(onSendMessage).toHaveBeenCalledTimes(1)
    // 被拒后应置 requested，供流式结束后补一次
    expect(result.current.hasPendingDrainRequest()).toBe(true)

    // 第二次 drain：队列里仍有 r4，应重新尝试发送
    onSendMessage.mockResolvedValueOnce(true)
    await act(async () => { await result.current.drainExternalSends() })
    expect(onSendMessage).toHaveBeenCalledTimes(2)
    expect(onSendMessage.mock.calls[1][0]).toBe('X')
  })

  it('retries a partially created conversation instead of creating a duplicate', async () => {
    mockTake
      .mockResolvedValueOnce({ success: true, requests: [{ id: 'r-partial', content: 'X', attachments: [] }] } as never)
      .mockResolvedValue({ success: true, requests: [] } as never)
    const { result, onSendMessage } = setup()
    onSendMessage.mockImplementationOnce(async (_content, _attachments, options) => {
      options.onPartialConversation(partialConversation)
      return false
    })
    await act(async () => { await result.current.drainExternalSends() })
    onSendMessage.mockResolvedValueOnce(true)
    await act(async () => { await result.current.drainExternalSends() })

    expect(onSendMessage.mock.calls[1][2].conversationOverride).toEqual(partialConversation)
  })

  it('发送成功后请求出队，不重复发', async () => {
    mockTake
      .mockResolvedValueOnce({ success: true, requests: [{ id: 'r5', content: 'Y', attachments: [] }] } as never)
      .mockResolvedValue({ success: true, requests: [] } as never)
    const { result, onSendMessage } = setup()
    await act(async () => { await result.current.drainExternalSends() })
    await act(async () => { await result.current.drainExternalSends() })
    expect(onSendMessage).toHaveBeenCalledTimes(1)
  })
})

describe('useExternalSendQueue 错误处理', () => {
  it('take 失败时报错并释放单飞标志', async () => {
    mockTake.mockResolvedValueOnce({ success: false, error: '后端拒绝' } as never)
    const { result, onError } = setup()
    await act(async () => { await result.current.drainExternalSends() })
    expect(onError).toHaveBeenCalledWith('后端拒绝')

    // 单飞标志必须已释放，否则后续 drain 永久失效。
    // 注：失败时 requested 仍为 true，finally 会再排一次 drain，故只断言"能再取"。
    const before = mockTake.mock.calls.length
    mockTake.mockResolvedValue({ success: true, requests: [] } as never)
    await act(async () => { await result.current.drainExternalSends() })
    expect(mockTake.mock.calls.length).toBeGreaterThan(before)
  })

  it('take 抛异常时也释放单飞标志', async () => {
    mockTake.mockRejectedValueOnce(new Error('网络中断'))
    const { result, onError } = setup()
    await act(async () => { await result.current.drainExternalSends() })
    expect(onError).toHaveBeenCalledWith('网络中断')
    const before = mockTake.mock.calls.length
    mockTake.mockResolvedValue({ success: true, requests: [] } as never)
    await act(async () => { await result.current.drainExternalSends() })
    expect(mockTake.mock.calls.length).toBeGreaterThan(before)
  })
})
