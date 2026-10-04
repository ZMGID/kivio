import { StrictMode, memo, useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { MessageList } from './MessageList'
import {
  getCoarse,
  patchSnapshot,
  reset,
  setCoarse,
  setSnapshot,
  useStreamCoarse,
} from './streamingStore'
import { createEmptyStreamSnapshot } from './conversationRuns'
import type { ConversationStreamSnapshot } from './conversationRuns'
import type { ChatMessage, Conversation } from './types'
import { isEmptyChatPresentation } from './emptyHero'
import * as messageNavigator from './messageNavigator'
import { beginGroup, endGroup, resetGroups } from './groupStreamingStore'
import { createChatExecutionOwner } from './chatExecutionOwner'
import { createChatStreamLifecycleOwner } from './chatStreamLifecycleOwner'
import { createStreamPreviewOwner } from './streamPreviewOwner'
import type { ChatStreamPayload } from '../api/tauri'

// 真实集成：挂载真 MessageList（订阅真 streamingStore），按 Chat 各 helper 的调用方式驱动 store，
// 验证「流式更新只重渲订阅者、不波及兄弟节点」这一核心收益，以及各 helper→store 映射的渲染结果。

function snapWith(partial: Partial<ConversationStreamSnapshot>): ConversationStreamSnapshot {
  return { ...createEmptyStreamSnapshot(), ...partial }
}

// MessageList 现在用 TanStack Virtual 虚拟化：视口测量 + 可见区间计算发生在 mount 后的一个微任务，
// 故断言渲染结果前需让 React 把这次异步更新刷出来。
async function flush() {
  await act(async () => {
    await Promise.resolve()
  })
}

afterEach(() => {
  vi.restoreAllMocks()
  act(() => {
    resetGroups()
    reset()
    setCoarse({ streaming: false, streamFrozen: false, cancelling: false, streamError: '' })
  })
})

// 不订阅 store 的兄弟节点，记录自身渲染次数。
let siblingRenders = 0
const Sibling = memo(function Sibling() {
  const count = useRef(0)
  count.current += 1
  siblingRenders = count.current
  return <div data-testid="sibling">sibling</div>
})

function mountList() {
  return render(
    <>
      <MessageList messages={[]} conversationId="c1" />
      <Sibling />
    </>,
  )
}

function message(id: number): ChatMessage {
  return {
    id: `m-${id}`,
    role: id % 2 === 0 ? 'user' : 'assistant',
    content: `message ${id}`,
    timestamp: id,
  }
}

describe('MessageList ← streamingStore 集成', () => {
  it.each(['wheel', 'scrollbar', 'keyboard', 'touch'])('pins the live page after viewport %s input without mounting every later step', async (input) => {
    const snapshot = (count: number) => snapWith({
      messageId: 'reading-process', streaming: true,
      toolCalls: Array.from({ length: count }, (_, i) => ({
        id: `tool-${i}`, name: `viewport_step_${i}`, status: 'completed',
      })),
      segments: Array.from({ length: count }, (_, i) => ({
        id: `segment-${i}`, kind: 'tool', phase: 'tool_loop', order: i, tool_call_id: `tool-${i}`,
      })),
    })
    act(() => {
      setSnapshot(snapshot(60))
      setCoarse({ streaming: true, streamFrozen: false })
    })
    const { container } = mountList()
    await flush()
    const viewport = container.querySelector<HTMLElement>('.chat-scroll-viewport')!
    Object.defineProperties(viewport, {
      scrollHeight: { configurable: true, value: 3000 },
      clientHeight: { configurable: true, value: 600 },
    })
    // Deliver the initial bottom/layout scroll before the reader moves the thumb.
    act(() => {
      viewport.scrollTop = 2400
      fireEvent.scroll(viewport)
    })
    const inspected = screen.getByText('viewport_step_40')
    act(() => {
      if (input === 'wheel') fireEvent.wheel(viewport, { deltaY: -100 })
      if (input === 'scrollbar') {
        viewport.scrollTop = 1200
        fireEvent.scroll(viewport)
      }
      if (input === 'keyboard') fireEvent.keyDown(viewport, { key: 'PageUp' })
      if (input === 'touch') {
        fireEvent.touchStart(viewport, { touches: [{ clientY: 100 }] })
        fireEvent.touchMove(viewport, { touches: [{ clientY: 200 }] })
      }
      setSnapshot(snapshot(61))
    })
    expect(screen.getByText('viewport_step_40')).toBe(inspected)
    act(() => setSnapshot(snapshot(160)))
    expect(screen.getByText('viewport_step_40')).toBe(inspected)
    expect(screen.getByText('viewport_step_159')).toBeVisible()
    expect(screen.queryByText('viewport_step_100')).not.toBeInTheDocument()
    expect(screen.queryByText('viewport_step_39')).not.toBeInTheDocument()
  })

  it.each([0, 2])('preserves the transcript while a completed send waits for React history commit (prior messages: %s)', async (historyCount) => {
    const conversationId = `empty-handoff-${historyCount}`
    const history = Array.from({ length: historyCount }, (_, index) => message(index))
    const execution = createChatExecutionOwner()
    const preview = createStreamPreviewOwner()
    let commitHistory!: (messages: ChatMessage[]) => void
    function Harness() {
      const [stored, setStored] = useState(history)
      commitHistory = setStored
      useSyncExternalStore(execution.subscribe, execution.getRevision)
      const coarse = useStreamCoarse()
      const displayed = execution.overlayMessages(conversationId, stored)
      useEffect(() => { preview.reconcile(conversationId, stored) }, [stored])
      return isEmptyChatPresentation(displayed.length, coarse)
        ? <div data-testid="empty-hero">New chat</div>
        : <MessageList conversationId={conversationId} messages={displayed} />
    }
    const { container, unmount } = render(<Harness />)
    await flush()
    expect(Boolean(screen.queryByTestId('empty-hero'))).toBe(historyCount === 0)
    const content = 'Stable answer\n\n```ts\nconst answer = 42\n```'
    let lease!: NonNullable<ReturnType<typeof execution.begin>>
    let user!: ChatMessage
    act(() => {
      lease = execution.begin({ conversationId, kind: 'send', startedAt: Date.now(),
        optimistic: { content: 'Question', attachments: [] } })!
      user = execution.overlayMessages(conversationId, history).at(-1)!
      preview.activate(conversationId)
      preview.begin(conversationId)
      preview.receive({
        protocolVersion: 1, scope: 'run', seq: 1, baseRevision: 0,
        type: 'text_delta', conversationId, runId: 'run', messageId: 'answer', delta: content, segment: null,
      })
      preview.activate(conversationId)
    })
    await flush()
    const viewport = container.querySelector<HTMLDivElement>('.chat-scroll-viewport')!
    const markdown = container.querySelector('[data-message-id="answer"] .chat-markdown')
    const code = markdown?.querySelector('pre code')
    expect(code?.textContent).toContain('const answer = 42')
    const persisted = [...history, user, { id: 'answer', role: 'assistant' as const, content, timestamp: 2 }]
    const conversation: Conversation = {
      id: conversationId, revision: 1, title: 'Question', provider_id: 'provider', model: 'model',
      messages: persisted, created_at: 1, updated_at: 2,
    }
    try {
      // External-store updates can commit before the queued React history update.
      // Hold that update to assert the observable state at this real boundary.
      await act(async () => {
        await execution.finish(lease, conversation, {
          completeWithConversation: () => preview.complete(conversationId, { kind: 'persisted', committedMessages: history }),
          completeTerminal: async () => {}, abandonPreview: () => {}, settleQueue: () => {},
        })
      })
      expect(getCoarse().streamFrozen).toBe(true)
      expect(screen.queryByTestId('empty-hero')).not.toBeInTheDocument()
      expect(container.querySelector('.chat-scroll-viewport')).toBe(viewport)
      expect(container.querySelector('[data-message-id="answer"] .chat-markdown')).toBe(markdown)

      Object.defineProperties(viewport, {
        scrollHeight: { configurable: true, value: 4000 },
        clientHeight: { configurable: true, value: 600 },
      })
      viewport.scrollTop = 2000
      fireEvent.wheel(viewport, { deltaY: -40 })
      fireEvent.scroll(viewport)
      // WebKit may clamp the viewport during the flow→absolute DOM mutation,
      // before MessageList's layout effect can restore the reader.
      const setAttribute = Element.prototype.setAttribute
      let nativeClamp = false
      vi.spyOn(Element.prototype, 'setAttribute').mockImplementation(function (this: Element, name, value) {
        setAttribute.call(this, name, value)
        if (name === 'class' && value.includes('absolute')
          && this.getAttribute('data-message-id') === 'answer') {
          viewport.scrollTop = 0
          nativeClamp = true
        }
      })
      act(() => commitHistory(persisted))
      await flush()
      expect(getCoarse().streamFrozen).toBe(false)
      expect(container.querySelector('.chat-scroll-viewport')).toBe(viewport)
      expect(container.querySelector('[data-message-id="answer"] pre code')).toBe(code)
      expect(nativeClamp).toBe(true)
      expect(viewport.scrollTop).toBe(2000)

      act(() => {
        preview.drop(conversationId)
        commitHistory([])
      })
      expect(screen.getByTestId('empty-hero')).toBeInTheDocument()
    } finally {
      unmount()
      preview.dispose()
    }
  })

  it('keeps single-run deltas visible after StrictMode replays mount effects', async () => {
    const conversationId = 'strict-mode-live'
    const base = { conversationId, runId: 'run-strict', messageId: 'message-strict' }
    let start: () => void = () => { throw new Error('harness not mounted') }
    let receive: (payload: ChatStreamPayload) => void = () => { throw new Error('harness not mounted') }
    function Harness() {
      const [execution] = useState(createChatExecutionOwner)
      const [preview] = useState(createStreamPreviewOwner)
      const [lifecycle] = useState(() => createChatStreamLifecycleOwner(execution, preview))
      useEffect(() => {
        preview.attach()
        preview.activate(conversationId)
        return () => preview.dispose()
      }, [preview])
      start = () => {
        execution.begin({ conversationId, kind: 'send', startedAt: Date.now() - 3000 })
        preview.begin(conversationId, Date.now() - 3000)
      }
      receive = (payload) => { lifecycle.receive(payload) }
      return <MessageList messages={[]} conversationId={conversationId} />
    }
    const { container } = render(<StrictMode><Harness /></StrictMode>)
    act(() => {
      start()
      receive({ ...base, type: 'run_started', recovery: null } as ChatStreamPayload)
      receive({ ...base, type: 'text_delta', delta: 'visible partial' } as ChatStreamPayload)
    })
    await waitFor(() => expect(container).toHaveTextContent('visible partial'))
    await waitFor(() => expect(container.textContent).toMatch(/[1-9]\d*s ·/))
  })

  it('preserves the selected model and its content when a live group commits', async () => {
    const conversationId = 'group-continuity'
    const user: ChatMessage = { id: 'group-user', role: 'user', content: 'question', timestamp: 1, group_id: 'g1' }
    const answers: ChatMessage[] = ['first', 'second'].map((model) => ({
      id: `${model}-answer`, role: 'assistant', content: `Answer from ${model}`, model,
      provider_id: 'provider', group_id: 'g1', timestamp: 2,
    }))
    act(() => {
      beginGroup(conversationId, 'g1', answers.map((answer) => ({
        messageId: answer.id, providerId: 'provider', model: answer.model!, content: answer.content,
      })))
      setCoarse({ streaming: true, streamFrozen: false })
    })
    const { container, rerender } = render(<MessageList messages={[user]} conversationId={conversationId} />)
    await flush()
    fireEvent.click(screen.getByRole('button', { name: /^second$/ }))
    const row = container.querySelector('[data-chat-message-list-item="live-group"]')
    const markdown = row?.querySelector('.chat-markdown')
    expect(markdown?.textContent).toContain('Answer from second')
    rerender(<MessageList messages={[user, ...answers]} conversationId={conversationId} />)
    await flush()
    act(() => { endGroup(conversationId); reset() })
    await flush()
    const committed = container.querySelector('[data-chat-message-list-item="group"]')
    expect(committed).toBe(row)
    expect(committed?.querySelector('.chat-markdown')).toBe(markdown)
    expect(screen.getByText('Answer from second')).toBeInTheDocument()
    expect(screen.queryByText('Answer from first')).not.toBeInTheDocument()
  })

  it.each([false, true])('keeps the live row, markdown and expanded process through freeze and commit (recovered: %s)', async (recovered) => {
    const user: ChatMessage = { id: 'continuous-user', role: 'user', content: 'question', timestamp: 1 }
    const content = '## Answer\n\nA **stable** answer.\n\n```ts\nconst answer = 42\n```'
    const segments = [
      { id: 'reasoning-1', kind: 'reasoning' as const, phase: 'plain' as const, order: 0, text: 'Inspecting the implementation.' },
      { id: 'answer-1', kind: 'text' as const, phase: 'plain' as const, order: 1, text: content },
    ]
    const assistant: ChatMessage = { id: 'continuous-answer', role: 'assistant', content, segments, timestamp: 2 }
    act(() => {
      setSnapshot(snapWith({ messageId: assistant.id, content, segments, streaming: true }))
      setCoarse({ streaming: true, streamFrozen: false })
    })
    const { container, rerender } = render(<MessageList messages={recovered ? [user, assistant] : [user]} conversationId="continuous-c1" />)
    await flush()
    const row = container.querySelector(`[data-message-id="${assistant.id}"]`)
    const markdown = row?.querySelector('.chat-markdown')
    const code = row?.querySelector('figure pre code')
    const process = screen.getByRole('region', { name: '过程分组' })
    const toggle = process.querySelector('button')!
    // Make this an explicit user choice, which must survive the handoff.
    if (toggle.getAttribute('aria-expanded') === 'true') fireEvent.click(toggle)
    fireEvent.click(toggle)
    expect(toggle).toHaveAttribute('aria-expanded', 'true')
    expect(code?.textContent).toContain('const answer = 42')

    act(() => {
      patchSnapshot({ streaming: false, reasoningStreaming: false })
      setCoarse({ streaming: false, streamFrozen: true })
    })
    await flush()
    expect(container.querySelector(`[data-message-id="${assistant.id}"]`)).toBe(row)
    rerender(<MessageList messages={[user, assistant]} conversationId="continuous-c1" />)
    await flush()
    act(() => reset())
    await flush()

    const committed = container.querySelector(`[data-message-id="${assistant.id}"]`)
    expect(committed).toBe(row)
    expect(committed?.querySelector('.chat-markdown')).toBe(markdown)
    expect(committed?.querySelector('figure pre code')).toBe(code)
    expect(toggle).toBeInTheDocument()
    expect(toggle).toHaveAttribute('aria-expanded', 'true')
    expect(container.querySelectorAll(`[data-message-id="${assistant.id}"]`)).toHaveLength(1)
  })

  it('does not render a detached global agent plan row', async () => {
    const onExecute = vi.fn()
    render(
      <MessageList
        conversationId="c-plan"
        messages={[{
          id: 'msg-plan',
          role: 'assistant',
          content: '1. Read code\n2. Implement',
          timestamp: 1,
        }]}
        agentPlanState={{ mode: 'plan', status: 'draft', plan: '1. Read code\n2. Implement', updated_at: 1 }}
        onExecuteAgentPlan={onExecute}
      />,
    )
    await flush()

    expect(document.querySelector('[data-chat-message-list-item="plan"]')).not.toBeInTheDocument()
    const button = screen.getByRole('button', { name: '执行这条计划' })
    expect(button).toBeInTheDocument()
    await act(async () => {
      button.click()
    })
    expect(onExecute).toHaveBeenCalledWith('msg-plan')
  })

  it('preserves explicit legacy plan metadata without grading its prose', async () => {
    render(
      <MessageList
        conversationId="c-plan-fragment"
        messages={[{
          id: 'msg-plan-fragment',
          role: 'assistant',
          content: '没问题！积萌,',
          timestamp: 1,
        }]}
        agentPlanState={{ mode: 'plan', status: 'draft', plan: '没问题！积萌,', updated_at: 1 }}
        onExecuteAgentPlan={() => {}}
      />,
    )
    await flush()

    expect(screen.getByText('计划草案')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '执行这条计划' })).toBeInTheDocument()
  })

  it('恢复运行时不同时渲染同 messageId 的历史草稿和实时预览', async () => {
    render(
      <MessageList
        conversationId="c-recovered"
        messages={[{
          id: 'msg-recovered',
          role: 'assistant',
          content: '恢复中的回答',
          stream_outcome: 'interrupted',
          timestamp: 1,
        }]}
      />,
    )
    act(() => {
      setSnapshot(snapWith({
        messageId: 'msg-recovered',
        content: '恢复中的回答',
        streaming: true,
      }))
      setCoarse({ streaming: true, streamFrozen: false })
    })
    await flush()

    // Live row reuses the real message id when known (stable virtualizer key is separate).
    // History twin is filtered out so only one bubble is mounted.
    expect(document.querySelectorAll('[data-message-id="msg-recovered"]')).toHaveLength(1)
    expect(screen.getByText('恢复中的回答')).toBeInTheDocument()

    act(() => {
      reset()
      setCoarse({ streaming: false, streamFrozen: false })
    })
    await flush()
    expect(document.querySelectorAll('[data-message-id="msg-recovered"]')).toHaveLength(1)
  })

  it('流式逐帧更新只重渲 MessageList，不波及未订阅的兄弟节点', async () => {
    siblingRenders = 0
    mountList()
    const baseline = siblingRenders // 1

    act(() => setCoarse({ streaming: true }))
    // 连续多帧内容更新（模拟 RAF 每帧 setSnapshot）
    for (let i = 0; i < 5; i++) {
      act(() => setSnapshot(snapWith({ content: `frame ${i}`, streaming: true })))
    }
    await flush()
    expect(screen.getByText(/frame 4/)).toBeInTheDocument()
    // 兄弟节点渲染次数不变 —— 证明 store 把更新隔离到订阅者。
    expect(siblingRenders).toBe(baseline)
  })

  it('流式内容帧不重建历史导航索引', async () => {
    const buildNavigator = vi.spyOn(messageNavigator, 'buildMessageNavigatorNodes')
    const messages = Array.from({ length: 40 }, (_, index) => message(index))
    render(<MessageList messages={messages} conversationId="navigator-perf-c1" />)
    await flush()
    const baseline = buildNavigator.mock.calls.length

    act(() => setCoarse({ streaming: true }))
    for (let i = 0; i < 8; i++) {
      act(() => setSnapshot(snapWith({ content: `token frame ${i}`, streaming: true })))
    }
    await flush()

    expect(buildNavigator).toHaveBeenCalledTimes(baseline)
  })

  it('reset（clearStreamingPreview 等价）清掉预览但保留 streamError', async () => {
    mountList()
    act(() => {
      setSnapshot(snapWith({ content: 'to be cleared', streaming: true }))
      setCoarse({ streaming: true, streamError: 'boom' })
    })
    await flush()
    expect(screen.getByText(/to be cleared/)).toBeInTheDocument()

    act(() => reset())
    await flush()
    expect(screen.queryByText(/to be cleared/)).not.toBeInTheDocument()
    // streamError 不被 reset 清除（与原 clearStreamingPreview 语义一致），错误文案仍展示。
    expect(screen.getByText('boom')).toBeInTheDocument()
  })

  it('流读取错误使用独立错误卡片并保留原始详情', async () => {
    mountList()
    act(() => setCoarse({ streamError: 'Chat stream: stream_read_error' }))
    await flush()

    expect(screen.getByText('超时 / 连接中断')).toBeInTheDocument()
    expect(screen.getByText('Chat stream: stream_read_error')).toBeInTheDocument()
  })

  it('长列表只挂载可见窗口，而不是把所有历史消息留在 DOM', async () => {
    const messages = Array.from({ length: 100 }, (_, index) => message(index))
    render(<MessageList messages={messages} conversationId="long-c1" />)
    await flush()

    const mountedMessages = document.querySelectorAll('[data-chat-message-list-item="message"]')
    expect(mountedMessages.length).toBeGreaterThan(0)
    expect(mountedMessages.length).toBeLessThan(messages.length)
  })

  it('用户翻历史时，完成消息跨重会话阈值也不切换渲染模式', async () => {
    const initialMessages = Array.from({ length: 4 }, (_, index) => message(index))
    const { container, rerender } = render(
      <MessageList messages={initialMessages} conversationId="heavy-completion-c1" />,
    )
    await flush()

    const scroller = container.querySelector('.chat-motion-view-in.custom-scrollbar') as HTMLDivElement
    Object.defineProperties(scroller, {
      scrollHeight: { configurable: true, get: () => 2400 },
      scrollTop: { configurable: true, writable: true, value: 400 },
    })
    fireEvent.wheel(scroller, { deltaY: -40 })
    await flush()
    expect(screen.getByRole('button', { name: '回到底部' })).toBeInTheDocument()

    // 80 个围栏代码块的估算成本超过 1500；旧逻辑会在这一帧切到 heavyHistory，
    // 卸载上方行并让 scrollTop 被 clamp。本次仍应保留用户正在看的首条历史。
    const heavyAnswer = Array.from({ length: 80 }, (_, index) => (
      `\`\`\`text\nblock ${index}\n\`\`\``
    )).join('\n')
    rerender(
      <MessageList
        conversationId="heavy-completion-c1"
        messages={[
          ...initialMessages,
          { id: 'heavy-answer', role: 'assistant', content: heavyAnswer, timestamp: 5 },
        ]}
      />,
    )
    await flush()

    expect(container.querySelector('[data-message-id="m-0"]')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '回到底部' })).toBeInTheDocument()
  })

  it('仍在跟随时，assistant 落库后重新钉到底部', async () => {
    const initialMessages = [{
      id: 'completion-user',
      role: 'user' as const,
      content: 'question',
      timestamp: 1,
    }]
    const { container, rerender } = render(
      <MessageList messages={initialMessages} conversationId="completion-pin-c1" />,
    )
    await flush()

    const scroller = container.querySelector('.chat-motion-view-in.custom-scrollbar') as HTMLDivElement
    let scrollTop = 120
    let writes = 0
    Object.defineProperties(scroller, {
      scrollHeight: { configurable: true, get: () => 2400 },
      scrollTop: {
        configurable: true,
        get: () => scrollTop,
        set: (value: number) => {
          scrollTop = value
          writes += 1
        },
      },
    })

    rerender(
      <MessageList
        conversationId="completion-pin-c1"
        messages={[
          ...initialMessages,
          { id: 'completion-answer', role: 'assistant', content: 'answer', timestamp: 2 },
        ]}
      />,
    )
    await flush()

    expect(writes).toBeGreaterThan(0)
    expect(scrollTop).toBe(2400)
  })

  it('live→历史交接后继续钉底，不卡在用户提问', async () => {
    const user = {
      id: 'handoff-user',
      role: 'user' as const,
      content: 'question',
      timestamp: 1,
    }
    const assistant = {
      id: 'handoff-answer',
      role: 'assistant' as const,
      content: 'long answer body',
      timestamp: 2,
    }
    const { container } = render(
      <MessageList messages={[user, assistant]} conversationId="handoff-pin-c1" />,
    )
    await flush()

    // 流式中 history 过滤同 id 的 assistant；live 外置在 virtualizer 下方。
    act(() => {
      setSnapshot(snapWith({
        content: 'long answer body',
        streaming: true,
        messageId: 'handoff-answer',
      }))
      setCoarse({ streaming: true, streamFrozen: false })
    })
    await flush()

    const scroller = container.querySelector('.chat-motion-view-in.custom-scrollbar') as HTMLDivElement
    const scrollHeight = 3000
    let scrollTop = 0
    let writes = 0
    Object.defineProperties(scroller, {
      scrollHeight: { configurable: true, get: () => scrollHeight },
      clientHeight: { configurable: true, get: () => 500 },
      scrollTop: {
        configurable: true,
        get: () => scrollTop,
        set: (value: number) => {
          // 模拟浏览器 clamp：高度骤降时 scrollTop 被压到新 max。
          scrollTop = Math.max(0, Math.min(value, Math.max(0, scrollHeight - 500)))
          writes += 1
        },
      },
    })

    // 流式跟底。
    act(() => {
      setSnapshot(snapWith({
        content: 'long answer body more tokens',
        streaming: true,
        messageId: 'handoff-answer',
      }))
    })
    await flush()
    expect(writes).toBeGreaterThan(0)
    expect(scrollTop).toBe(2500)

    // LiveAgent path: settle reuses the live row key, so scrollHeight does not
    // collapse. Keep geometry continuous and assert we stay pinned at the bottom.
    const writesBeforeHandoff = writes
    act(() => {
      setSnapshot(snapWith({
        content: 'long answer body more tokens',
        streaming: false,
        messageId: 'handoff-answer',
      }))
      setCoarse({ streaming: false, streamFrozen: false })
    })
    await flush()

    expect(writes).toBeGreaterThanOrEqual(writesBeforeHandoff)
    expect(scrollTop).toBe(2500)
  })

  it('流式外置 live：只挂一个气泡，结束后落成历史消息', async () => {
    const user = {
      id: 'stable-key-user',
      role: 'user' as const,
      content: 'question',
      timestamp: 1,
    }
    const assistant = {
      id: 'stable-key-answer',
      role: 'assistant' as const,
      content: 'final answer body',
      timestamp: 2,
    }
    const { container } = render(
      <MessageList messages={[user, assistant]} conversationId="stable-key-c1" />,
    )
    await flush()

    act(() => {
      setSnapshot(snapWith({
        content: 'partial answer',
        streaming: true,
        messageId: 'stable-key-answer',
      }))
      setCoarse({ streaming: true, streamFrozen: false })
    })
    await flush()

    // External live: one bubble (history twin filtered), outside virtualizer.
    expect(container.querySelectorAll('[data-chat-message-list-item="streaming"]')).toHaveLength(1)
    expect(container.querySelectorAll('[data-message-id="stable-key-answer"]')).toHaveLength(1)
    expect(screen.getByText('partial answer')).toBeInTheDocument()

    act(() => {
      setSnapshot(snapWith({
        content: 'final answer body',
        streaming: false,
        messageId: 'stable-key-answer',
      }))
      setCoarse({ streaming: false, streamFrozen: false })
    })
    await flush()

    expect(container.querySelector('[data-chat-message-list-item="streaming"]')).toBeNull()
    expect(container.querySelectorAll('[data-message-id="stable-key-answer"]')).toHaveLength(1)
    expect(screen.getByText('final answer body')).toBeInTheDocument()
  })

  it('流式快照更新在 ResizeObserver 未报告时仍补钉到底部', async () => {
    const originalResizeObserver = window.ResizeObserver
    window.ResizeObserver = class {
      observe() {}
      disconnect() {}
    } as unknown as typeof ResizeObserver

    try {
      const { container } = render(
        <MessageList
          conversationId="streaming-fallback-pin-c1"
          messages={[{
            id: 'streaming-fallback-user',
            role: 'user',
            content: 'question',
            timestamp: 1,
          }]}
        />,
      )
      await flush()

      const scroller = container.querySelector('.chat-motion-view-in.custom-scrollbar') as HTMLDivElement
      let scrollTop = 0
      let writes = 0
      Object.defineProperties(scroller, {
        scrollHeight: { configurable: true, get: () => 2400 },
        clientHeight: { configurable: true, get: () => 500 },
        scrollTop: {
          configurable: true,
          get: () => scrollTop,
          set: (value: number) => {
            scrollTop = value
            writes += 1
          },
        },
      })

      act(() => {
        setSnapshot(snapWith({ content: 'streaming answer', streaming: true }))
        setCoarse({ streaming: true })
      })
      await flush()

      expect(writes).toBeGreaterThan(0)
      expect(scrollTop).toBe(2400)
    } finally {
      window.ResizeObserver = originalResizeObserver
    }
  })

  it('底部阈值内的小幅向上滚动不会闪现回到底部按钮', async () => {
    const { container } = render(
      <MessageList messages={[message(1)]} conversationId="wheel-threshold-c1" />,
    )
    await flush()

    const scroller = container.querySelector('.chat-motion-view-in.custom-scrollbar')
    expect(scroller).not.toBeNull()
    expect(screen.queryByRole('button', { name: '回到底部' })).not.toBeInTheDocument()

    fireEvent.wheel(scroller!, { deltaY: -2 })

    expect(screen.queryByRole('button', { name: '回到底部' })).not.toBeInTheDocument()
  })
})
