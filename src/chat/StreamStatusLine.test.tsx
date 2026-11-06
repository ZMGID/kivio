import { act, render, renderHook, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { BLOB_DONE_MS, StreamStatusLine } from './StreamStatusLine'
import { STATUS_QUIPS, quipPlan } from './blobQuips'
import { patchSnapshot, reset, setCoarse } from './streamingStore'
import type { ToolCallRecord } from './types'
import { subagentsApi } from '../api/subagents'
import { api } from '../api/tauri'
import { chatApi } from './api'
import { useBackgroundTaskWait } from './useBackgroundTaskWait'
import type { SubagentIrcMessage } from '../generated/chatProtocol'

vi.mock('./conversationTransitionStore', () => ({
  useConversationTransition: () => ({ loading: false }),
}))

const moodProbe = vi.fn()
vi.mock('./KivioBlob', () => ({
  KivioBlob: ({ mood }: { mood: string }) => {
    moodProbe(mood)
    return <span data-testid="blob" data-mood={mood} />
  },
}))

function tool(name: string, status: ToolCallRecord['status']): ToolCallRecord {
  return { id: name, toolCallId: name, name, status } as ToolCallRecord
}

describe('StreamStatusLine', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2026-09-04T12:00:00Z'))
    vi.spyOn(Math, 'random').mockReturnValue(0)
    reset()
    setCoarse({ streamError: '' })
    moodProbe.mockClear()
  })

  afterEach(() => {
    vi.restoreAllMocks()
    vi.useRealTimers()
    reset()
  })

  it('shows IRC routes one at a time, speeding up and keeping the latest routes during a burst', async () => {
    vi.spyOn(performance, 'now').mockImplementation(() => Date.now())
    let deliver: ((message: SubagentIrcMessage) => void) | undefined
    vi.spyOn(api, 'onChatSubagentIrc').mockImplementation(async handler => {
      deliver = handler
      return () => {}
    })
    const { rerender } = render(<StreamStatusLine active={false} conversationId="root" lang="zh" />)
    await act(async () => {})
    const first: SubagentIrcMessage = { id: 'one', conversationId: 'root', fromAgent: 'Main', toAgent: 'Scout',
      content: 'Hidden IRC text', observation: true, timestamp: 1, progress: false, broadcast: false, replyTo: null }
    const second = { ...first, id: 'two', fromAgent: 'Scout', toAgent: 'Main', timestamp: 2 }
    act(() => {
      deliver?.({ ...first, conversationId: 'other' })
      deliver?.(first)
      deliver?.({ ...first, observation: false })
      deliver?.(second)
    })
    expect(screen.getByRole('status', { name: /Main → Scout/ })).toBeVisible()
    expect(screen.queryByRole('status', { name: /Scout → Main/ })).not.toBeInTheDocument()
    expect(screen.queryByText('Hidden IRC text')).not.toBeInTheDocument()
    act(() => { vi.advanceTimersByTime(800) })
    expect(screen.queryByRole('status', { name: /Main → Scout/ })).not.toBeInTheDocument()
    expect(screen.getByRole('status', { name: /Scout → Main/ })).toBeVisible()

    act(() => { vi.advanceTimersByTime(250) })
    act(() => {
      for (let index = 1; index <= 40; index += 1) {
        deliver?.({ ...first, id: `burst-${index}`, toAgent: `Worker-${index}` })
      }
    })
    expect(screen.getByRole('status', { name: /Scout → Main/ })).toBeVisible()
    act(() => { vi.advanceTimersByTime(60) })
    expect(screen.getByRole('status', { name: /Main → Worker-29/ })).toBeVisible()
    act(() => { vi.advanceTimersByTime(300) })
    expect(screen.getByRole('status', { name: /Main → Worker-30/ })).toBeVisible()
    rerender(<StreamStatusLine active={false} conversationId="other" lang="zh" />)
    expect(screen.queryByRole('status', { name: /代理通信/ })).not.toBeInTheDocument()
  })

  it('问用户时是 wait 脸，并很快接一句「等你呢」', () => {
    patchSnapshot({
      streaming: true,
      startedAt: Date.now() - 3000,
      content: '',
      toolCalls: [tool('ask_user', 'running')],
    })
    render(<StreamStatusLine active lang="zh" />)
    expect(screen.getByTestId('blob').dataset.mood).toBe('wait')
    expect(screen.getByText(/3s · 1 running/)).toBeTruthy()
    act(() => {
      vi.advanceTimersByTime(quipPlan('wait')!.first)
    })
    expect(screen.getByText(new RegExp(STATUS_QUIPS.zh.wait[0]))).toBeTruthy()
  })

  it('后端状态一行字优先，墨团不抢话', () => {
    patchSnapshot({
      streaming: true,
      startedAt: Date.now(),
      toolCalls: [tool('ask_user', 'running')],
      statusNote: '上游重试中',
    })
    render(<StreamStatusLine active lang="zh" />)
    act(() => {
      vi.advanceTimersByTime(1500)
    })
    expect(screen.getByText(/上游重试中/)).toBeTruthy()
    expect(screen.queryByText(new RegExp(STATUS_QUIPS.zh.wait[0]))).toBeNull()
  })

  it('keeps built-in waits out of the visible live stats and clears the accessible status when they settle', () => {
    const wait: ToolCallRecord = { ...tool('native__wait', 'running'), source: 'native' }
    patchSnapshot({ streaming: true, startedAt: Date.now(), toolCalls: [wait, { ...wait, id: 'second' }] })
    render(<StreamStatusLine active lang="zh" />)
    expect(screen.getByText('0s')).toBeVisible()
    expect(screen.getByRole('status')).toHaveTextContent('等待任务结果')
    expect(screen.getByRole('status')).toHaveClass('sr-only')
    patchSnapshot({ toolCalls: [{ ...wait, status: 'completed' }] })
    act(() => { vi.advanceTimersByTime(250) })
    expect(screen.queryByText(/等待任务结果/)).not.toBeInTheDocument()
  })

  it('shows idle background waiting only for the current conversation', async () => {
    let activity: (id: string) => void = () => {}
    vi.spyOn(api, 'onChatSubagentActivity').mockImplementation(async listener => {
      activity = listener
      return () => {}
    })
    const waiting = vi.spyOn(subagentsApi, 'waiting').mockResolvedValue(true)
    function Status({ active }: { active: boolean }) {
      const stop = useBackgroundTaskWait(active ? null : 'first', 'zh')
      return <StreamStatusLine active={active} waitingInBackground={Boolean(stop)} lang="zh" />
    }
    const { rerender } = render(<Status active={false} />)
    await act(async () => {})
    expect(screen.getByRole('status')).toHaveTextContent('等待任务结果 · Esc 停止等待')
    expect(screen.getByRole('status')).toHaveClass('sr-only')
    expect(screen.getByTestId('blob').dataset.mood).toBe('wait')
    waiting.mockResolvedValue(false)
    await act(async () => activity('other'))
    expect(screen.getByText(/等待任务结果/)).toBeInTheDocument()
    await act(async () => activity('first'))
    expect(screen.queryByText(/等待任务结果/)).not.toBeInTheDocument()
    waiting.mockResolvedValue(true)
    await act(async () => activity('first'))
    rerender(<Status active />)
    expect(screen.queryByText(/等待任务结果/)).not.toBeInTheDocument()
  })

  it('ends idle waiting for the current conversation', async () => {
    const waiting = vi.spyOn(subagentsApi, 'waiting').mockResolvedValue(true)
    const stop = vi.spyOn(chatApi, 'cancelStream').mockImplementation(async () => {
      waiting.mockResolvedValue(false)
    })
    const { result } = renderHook(() => useBackgroundTaskWait('first', 'zh'))
    await act(async () => {})
    expect(result.current).toBeDefined()
    await act(async () => { await result.current?.() })
    expect(stop).toHaveBeenCalledTimes(1)
    expect(stop).toHaveBeenCalledWith('first', 'backgroundWait')
    expect(result.current).toBeUndefined()
  })

  it('shows an English retry countdown and clears it when output resumes', () => {
    patchSnapshot({
      streaming: true,
      startedAt: Date.now(),
      statusNote: 'Server overloaded · retry 2/10',
      retryStatus: { attempt: 2, maxRetries: 10, retryAtMs: Date.now() + 8000, reason: 'Server overloaded' },
    })
    render(<StreamStatusLine active lang="zh" />)
    expect(screen.getByText(/Server overloaded · retry 2\/10 · in 8s/)).toBeTruthy()
    act(() => { vi.advanceTimersByTime(3000) })
    expect(screen.getByText(/retry 2\/10 · in 5s/)).toBeTruthy()
    act(() => { vi.advanceTimersByTime(5000) })
    expect(screen.getByText(/retry 2\/10 · Retrying…/)).toBeTruthy()
    patchSnapshot({ statusNote: null, retryStatus: null, content: 'Recovered' })
    act(() => { vi.advanceTimersByTime(250) })
    expect(screen.queryByText(/retry 2\/10/)).toBeNull()
  })

  it('生成结束且没翻车：进「搞定」窗口，到点回闲置', () => {
    patchSnapshot({ streaming: true, startedAt: Date.now(), content: '答' })
    const { rerender } = render(<StreamStatusLine active lang="en" />)
    expect(screen.getByTestId('blob').dataset.mood).toBe('speak')
    reset()
    rerender(<StreamStatusLine active={false} lang="en" />)
    expect(screen.getByTestId('blob').dataset.mood).toBe('done')
    act(() => {
      vi.advanceTimersByTime(0)
    })
    expect(screen.getByText(STATUS_QUIPS.en.done[0])).toBeTruthy()
    act(() => {
      vi.advanceTimersByTime(BLOB_DONE_MS + 100)
    })
    expect(screen.getByTestId('blob').dataset.mood).toBe('idle')
    expect(screen.queryByText(STATUS_QUIPS.en.done[0])).toBeNull()
  })

  it('收工没掷中就安静回闲置，不蹦不说', () => {
    vi.spyOn(Math, 'random').mockReturnValue(0.9)
    patchSnapshot({ streaming: true, startedAt: Date.now(), content: '答' })
    const { rerender } = render(<StreamStatusLine active lang="en" />)
    reset()
    rerender(<StreamStatusLine active={false} lang="en" />)
    expect(screen.getByTestId('blob').dataset.mood).toBe('idle')
    act(() => {
      vi.advanceTimersByTime(1000)
    })
    expect(screen.queryByText(STATUS_QUIPS.en.done[0])).toBeNull()
  })

  it('翻车收尾不得意：直接出错脸', () => {
    patchSnapshot({ streaming: true, startedAt: Date.now(), content: '答' })
    const { rerender } = render(<StreamStatusLine active lang="zh" />)
    setCoarse({ streamError: 'boom' })
    reset()
    rerender(<StreamStatusLine active={false} lang="zh" />)
    expect(screen.getByTestId('blob').dataset.mood).toBe('error')
    act(() => {
      vi.advanceTimersByTime(600)
    })
    expect(screen.getByText(STATUS_QUIPS.zh.error[0])).toBeTruthy()
  })
})
