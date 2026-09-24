import { act, fireEvent, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { MessageBubble } from './MessageBubble'
import type { ChatMessage } from './types'

describe('assistant body visibility', () => {
  it.each(['cancelled', undefined])('folds cancelled progress with outcome %s and preserves the partial answer', outcome => {
    render(<MessageBubble message={{
      id: 'partial-answer', role: 'assistant', timestamp: 1, content: 'Useful partial answer', stream_outcome: outcome,
      segments: [
        { id: 'progress', kind: 'text', phase: 'tool_loop', order: 1, text: 'Reading files now' },
        { id: 'call', kind: 'tool', phase: 'tool_loop', order: 2, tool_call_id: 'call' },
        { id: 'partial', kind: 'text', phase: 'synthesis', order: 3, text: 'Useful partial answer' },
        { id: 'seg_4_cancelled_synthesis', kind: 'text', phase: 'synthesis', order: 4, text: '已停止生成。' },
      ],
    }} />)
    expect(screen.queryByText('Reading files now')).not.toBeInTheDocument()
    expect(screen.getByText('Useful partial answer')).toBeVisible()
    expect(screen.getByText('已停止生成。')).toBeVisible()
  })

  it.each(['cancelled', undefined])('keeps interrupted browser commentary inside Worked with outcome %s', outcome => {
    const progress = ['Chrome 起来了，但扩展还是没连上。', '扩展自己重连了，再试一次。', '我找一下 Kivio 的日志。']
    render(<MessageBubble message={{
      id: 'browser-cancelled', role: 'assistant', timestamp: 1,
      content: `${progress.join('\n\n')}\n\n正在检查日志。\n\n已停止生成。`, stream_outcome: outcome,
      segments: [
        ...progress.flatMap((text, index) => [
          { id: `note-${index}`, kind: 'text' as const, phase: 'tool_loop' as const, order: index * 2, text },
          { id: `call-${index}`, kind: 'tool' as const, phase: 'tool_loop' as const, order: index * 2 + 1, tool_call_id: `call-${index}` },
        ]),
        { id: 'trailing-note', kind: 'text', phase: 'tool_loop', order: 6, text: '正在检查日志。' },
        { id: 'seg_7_cancelled_synthesis', kind: 'text', phase: 'synthesis', order: 7, text: '已停止生成。' },
      ],
    }} />)
    const worked = screen.getByRole('button', { name: /^Worked/ })
    expect(worked).toHaveAttribute('aria-expanded', 'false')
    for (const text of progress) expect(screen.queryByText(text)).not.toBeInTheDocument()
    expect(screen.queryByText('正在检查日志。')).not.toBeInTheDocument()
    expect(screen.getByText('已停止生成。')).toBeVisible()
    fireEvent.click(worked)
    for (const text of progress) expect(screen.getByText(text)).toBeVisible()
    expect(screen.getByText('正在检查日志。')).toBeVisible()
  })

  it('uses one stable Work for subagents, waiting, final answer and history', () => {
    const message: ChatMessage = {
      id: 'live-subagents', role: 'assistant', timestamp: 1, content: '',
      segments: [
        { id: 'r1', kind: 'reasoning', phase: 'tool_loop', order: 0, text: 'Launch plan' },
        { id: 'intro', kind: 'text', phase: 'tool_loop', order: 1, text: '先启动子代理，我来读 README。' },
        { id: 'a', kind: 'tool', phase: 'tool_loop', order: 2, tool_call_id: 'a' },
        { id: 'b', kind: 'tool', phase: 'tool_loop', order: 3, tool_call_id: 'b' },
        { id: 'r2', kind: 'reasoning', phase: 'tool_loop', order: 4, text: 'Read plan' },
        { id: 'read-note', kind: 'text', phase: 'tool_loop', order: 5, text: 'README 只有 9 字节，再看文档。' },
        { id: 'read', kind: 'tool', phase: 'tool_loop', order: 6, tool_call_id: 'read' },
        { id: 'wait-note', kind: 'text', phase: 'tool_loop', order: 7, text: '项目用途已了解，等待子代理。' },
        { id: 'wait', kind: 'tool', phase: 'tool_loop', order: 8, tool_call_id: 'wait' },
      ],
      tool_calls: [
        { id: 'a', name: 'agent', source: 'native', status: 'completed' },
        { id: 'b', name: 'agent', source: 'native', status: 'completed' },
        { id: 'read', name: 'read_file', source: 'native', status: 'completed' },
        { id: 'wait', name: 'agent', source: 'native', status: 'running', arguments: '{"operation":"wait"}' },
      ],
    }
    const { rerender } = render(<MessageBubble message={message} messageStreaming />)
    const work = screen.getByRole('button', { name: 'Working' })
    expect(screen.getAllByLabelText('过程分组')).toHaveLength(1)
    expect(screen.getByText('先启动子代理，我来读 README。')).toBeVisible()
    rerender(<MessageBubble message={{ ...message, segments: [...message.segments!,
      { id: 'final', kind: 'text', phase: 'plain', order: 9, text: '最终项目汇总' },
    ] }} />)
    expect(screen.getByText('最终项目汇总')).toBeVisible()
    expect(screen.getAllByLabelText('过程分组')).toHaveLength(1)
    expect(screen.getByRole('button', { name: /^Worked/ })).toBe(work)
    expect(work).toHaveAttribute('aria-expanded', 'false')
    expect(screen.queryByText('先启动子代理，我来读 README。')).not.toBeInTheDocument()
    fireEvent.click(work)
    expect(screen.getByText('README 只有 9 字节，再看文档。')).toBeVisible()
    expect(screen.getByText('项目用途已了解，等待子代理。')).toBeVisible()
  })

  it.each(['cancelled', 'error', 'interrupted', 'recovered'])('folds progress after a %s outcome', outcome => {
    const message: ChatMessage = {
      id: 'stopped-run', role: 'assistant', timestamp: 1, content: 'Run stopped',
      stream_outcome: outcome,
      segments: [
        { id: 'partial', kind: 'text', phase: 'tool_loop', order: 1, text: 'Partial findings' },
        { id: 'tool', kind: 'tool', phase: 'tool_loop', order: 2, tool_call_id: 'call' },
        { id: 'notice', kind: 'text', phase: 'synthesis', order: 3, text: 'Run stopped' },
      ],
    }
    const { rerender } = render(<MessageBubble message={message} />)
    expect(screen.queryByText('Partial findings')).not.toBeInTheDocument()
    expect(screen.getByText('Run stopped')).toBeVisible()
    rerender(<MessageBubble message={{ ...message, stream_outcome: undefined, streamOutcome: outcome }} />)
    expect(screen.queryByText('Partial findings')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: /^Worked/ }))
    expect(screen.getByText('Partial findings')).toBeVisible()
  })

  it('folds intermediate search updates into one Worked group after completion', () => {
    const message: ChatMessage = {
      id: 'news-search', role: 'assistant', timestamp: 1, content: '最终新闻汇总',
      stream_outcome: 'completed',
      segments: [
        { id: 'r1', kind: 'reasoning', phase: 'tool_loop', order: 0, text: 'search plan' },
        { id: 'n1', kind: 'text', phase: 'tool_loop', order: 1, text: '先搜一圈最近几天的 AI 新闻，稍等。' },
        { id: 't1', kind: 'tool', phase: 'tool_loop', order: 2, tool_call_id: 'search' },
        { id: 'n2', kind: 'text', phase: 'tool_loop', order: 3, text: '再补两条其他方向的。' },
        { id: 'r2', kind: 'reasoning', phase: 'tool_loop', order: 4, text: 'summarize' },
        { id: 'final', kind: 'text', phase: 'plain', order: 5, text: '最终新闻汇总' },
      ],
      tool_calls: [{ id: 'search', name: 'web_search', source: 'native', status: 'completed' }],
    }
    const { rerender } = render(<MessageBubble message={message} messageStreaming />)
    expect(screen.getByText('再补两条其他方向的。')).toBeVisible()
    rerender(<MessageBubble message={message} />)
    expect(screen.getByText('最终新闻汇总')).toBeVisible()
    expect(screen.queryByText('再补两条其他方向的。')).not.toBeInTheDocument()
    const worked = screen.getByRole('button', { name: /^Worked/ })
    expect(worked).toHaveAttribute('aria-expanded', 'false')
    fireEvent.click(worked)
    expect(screen.getByText('先搜一圈最近几天的 AI 新闻，稍等。')).toBeVisible()
    expect(screen.getByText('再补两条其他方向的。')).toBeVisible()
  })

  it('copies the questions and final supplement without private reasoning or tool output', async () => {
    const user = userEvent.setup()
    const message: ChatMessage = {
      id: 'copy-questions', role: 'assistant', timestamp: 1, content: '请回答上面的问题。',
      segments: [
        { id: 'questions', kind: 'text', phase: 'tool_loop', order: 1, text: '目标用户是谁？' },
        { id: 'reasoning', kind: 'reasoning', phase: 'tool_loop', order: 2, text: 'private reasoning' },
        { id: 'final', kind: 'text', phase: 'plain', order: 3, text: '请回答上面的问题。' },
      ],
    }
    render(<MessageBubble message={message} />)
    await user.click(screen.getByRole('button', { name: '复制' }))
    expect(await navigator.clipboard.readText()).toBe('目标用户是谁？\n\n请回答上面的问题。')
  })

  it('keeps current text visible and moves it into Worked when execution advances', () => {
    const questions = 'Q1 目标用户是谁？Q2 有何差异？Q3 首个交付是什么？Q4 有哪些约束？'
    const message: ChatMessage = {
      id: 'questions-before-tools', role: 'assistant', content: '', timestamp: 1,
      segments: [{ id: 'questions', kind: 'text', phase: 'tool_loop', order: 1, text: questions }],
    }
    const { rerender } = render(<MessageBubble message={message} messageStreaming />)
    expect(screen.getByText(questions)).toBeVisible()
    const completed: ChatMessage = {
      ...message,
      content: '环境检查完成。请回答上面四个问题。',
      segments: [
        ...message.segments!,
        { id: 'check', kind: 'tool', phase: 'tool_loop', order: 2, tool_call_id: 'check' },
        { id: 'final', kind: 'text', phase: 'plain', order: 3, text: '环境检查完成。请回答上面四个问题。' },
      ],
      tool_calls: [{ id: 'check', name: 'bash', source: 'native', status: 'completed' }],
    }
    rerender(<MessageBubble message={completed} messageStreaming />)
    expect(screen.getByText(questions)).toBeVisible()
    rerender(<MessageBubble message={completed} />)
    expect(screen.queryByText(questions)).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: /^Worked/ })).toHaveAttribute('aria-expanded', 'false')
    fireEvent.click(screen.getByRole('button', { name: /^Worked/ }))
    expect(screen.getByText(questions)).toBeVisible()
  })
})

describe('MessageBubble mount motion', () => {
  const assistantMessage: ChatMessage = {
    id: 'assistant-motion',
    role: 'assistant',
    content: 'answer',
    timestamp: 1,
  }

  it('does not replay entrance motion for historical messages', () => {
    const { container, rerender } = render(<MessageBubble message={assistantMessage} />)
    expect(container.firstElementChild).not.toHaveClass('chat-motion-bubble-in')

    rerender(<MessageBubble message={{ ...assistantMessage, id: 'user-motion', role: 'user' }} />)
    expect(container.firstElementChild).not.toHaveClass('chat-motion-bubble-in')
  })

  it('keeps entrance motion for the live streaming preview', () => {
    const { container } = render(<MessageBubble message={assistantMessage} messageStreaming />)
    expect(container.firstElementChild).toHaveClass('chat-motion-bubble-in')
  })

  // settle 冻结帧：messageStreaming 翻 false 但 live 行还在。Markdown 若跟着切 static，
  // Streamdown 整树重挂、代码块退回未高亮 fallback（180ms 后再补）——「生成完闪一下」。
  it('keeps live markdown in streaming mode on the frozen frame when markdownStreaming stays true', async () => {
    const message: ChatMessage = {
      id: 'assistant-frozen',
      role: 'assistant',
      content: '前言\n\n```ts\nconst x = 1\n```\n\n```py\nprint(1)\n```',
      timestamp: 1,
    }
    const { container, rerender } = render(<MessageBubble message={message} messageStreaming markdownStreaming />)
    await act(async () => { await Promise.resolve() })
    const liveCodes = [...container.querySelectorAll('figure pre code')]
    expect(liveCodes).toHaveLength(2)
    const liveSpanCount = liveCodes.reduce((n, code) => n + code.querySelectorAll('span').length, 0)
    expect(liveSpanCount).toBeGreaterThan(0)

    rerender(<MessageBubble message={message} messageStreaming={false} markdownStreaming />)
    await act(async () => { await Promise.resolve() })
    // 没退回 fallback：island 仍是 hydrated，高亮 span 原样保留，代码块 DOM 节点也没换。
    expect(container.querySelector('[data-chat-heavy-hydrated="false"]')).toBeNull()
    const frozenCodes = [...container.querySelectorAll('figure pre code')]
    expect(frozenCodes).toEqual(liveCodes)
    expect(frozenCodes.reduce((n, code) => n + code.querySelectorAll('span').length, 0)).toBe(liveSpanCount)
  })

  // 曾经 messageStreaming 翻 false 会把 Streamdown 切成 static 模式并换 key，整树重挂、
  // 代码块退回未 hydrate 的 fallback（就是「生成完闪一下」）。现在 Markdown 终身留在
  // streaming 模式、key 不随 streaming 翻转，这个翻转不再触碰已挂载的岛。
  it('keeps code blocks hydrated when messageStreaming flips to false (no static remount)', async () => {
    const message: ChatMessage = {
      id: 'assistant-static-switch',
      role: 'assistant',
      content: '```ts\nconst x = 1\n```',
      timestamp: 1,
    }
    const { container, rerender } = render(<MessageBubble message={message} messageStreaming />)
    await act(async () => { await Promise.resolve() })
    expect(container.querySelector('[data-chat-heavy-island="true"]')?.getAttribute('data-chat-heavy-hydrated')).toBe('true')

    const before = container.querySelector('[data-chat-heavy-island="true"]')
    rerender(<MessageBubble message={message} messageStreaming={false} />)
    await act(async () => { await Promise.resolve() })
    const island = container.querySelector('[data-chat-heavy-island="true"]')
    expect(island).toBe(before)
    expect(island?.getAttribute('data-chat-heavy-hydrated')).toBe('true')
  })

  // 元信息条 hover 显隐：鼠标在这条消息上显示、移走隐藏。React 合成 pointer 事件挂在
  // 消息根元素上（不用 CSS group-hover——macOS WKWebView 的 :hover 移出后粘滞不消）。
  it('reveals the assistant meta row while the message is hovered', () => {
    const { container } = render(<MessageBubble message={assistantMessage} />)
    const root = container.firstElementChild as HTMLElement

    // 显隐不走 React state（滚动时消息滑过光标会连环 enter/leave，state 会整棵重渲），
    // 走根元素 data-msg-hovered 属性 + index.css 的 `[data-msg-hovered] .msg-hover-reveal`。
    // jsdom 不算样式，这里断言属性翻转 + 行挂着约定的 reveal 类。
    const meta = screen.getByLabelText('复制').closest('.transition-opacity') as HTMLElement
    expect(meta).toHaveClass('msg-hover-reveal')
    expect(meta).toHaveClass('opacity-0')
    expect(root).not.toHaveAttribute('data-msg-hovered')

    fireEvent.pointerEnter(root)
    expect(root).toHaveAttribute('data-msg-hovered')

    fireEvent.pointerLeave(root)
    expect(root).not.toHaveAttribute('data-msg-hovered')

    // WKWebView 会间歇性吞掉 pointerleave（实测最后一条消息上状态卡在显示）——
    // 兜底：悬停期间任何落在消息外的指针移动都收起，不依赖边界事件。
    fireEvent.pointerEnter(root)
    expect(root).toHaveAttribute('data-msg-hovered')
    fireEvent.pointerMove(document.body)
    expect(root).not.toHaveAttribute('data-msg-hovered')

    // 消息内的移动不收起。
    fireEvent.pointerEnter(root)
    fireEvent.pointerMove(root)
    expect(root).toHaveAttribute('data-msg-hovered')
  })

  // 用户气泡下的三个操作图标（复制/回到这里/建分支）：同一套显隐。
  it('reveals the user bubble actions while the bubble is hovered', () => {
    const { container } = render(
      <MessageBubble
        message={{ ...assistantMessage, id: 'user-hover', role: 'user', content: '你好' }}
        onForkMessage={async () => {}}
      />,
    )
    const root = container.firstElementChild as HTMLElement

    const actions = screen.getByLabelText('复制').closest('.transition-opacity') as HTMLElement
    expect(actions).toHaveClass('msg-hover-reveal')
    expect(actions).toHaveClass('opacity-0')
    expect(root).not.toHaveAttribute('data-msg-hovered')

    fireEvent.pointerEnter(root)
    expect(root).toHaveAttribute('data-msg-hovered')

    fireEvent.pointerLeave(root)
    expect(root).not.toHaveAttribute('data-msg-hovered')
  })
})

describe('MessageBubble thinking', () => {
  it('does not paint Thinking until reasoning summary text exists', () => {
    render(
      <MessageBubble
        message={{ id: 'a', role: 'assistant', content: '', timestamp: 1 }}
        messageStreaming
      />,
    )
    expect(screen.queryByLabelText('Thinking')).not.toBeInTheDocument()
  })
})

describe('MessageBubble agent plan action', () => {
  it('renders execute action for a message-scoped draft plan', async () => {
    const user = userEvent.setup()
    const calls: string[] = []
    const message: ChatMessage = {
      id: 'msg-plan',
      role: 'assistant',
      content: '1. Read code\n2. Implement',
      agent_plan: {
        mode: 'plan',
        status: 'draft',
        plan: '1. Read code\n2. Implement',
        updated_at: 1,
      },
      timestamp: 1,
    }

    render(<MessageBubble message={message} onExecuteAgentPlan={(messageId) => { calls.push(messageId) }} />)

    expect(screen.getByText('计划草案')).toBeInTheDocument()
    expect(screen.queryByLabelText('计划内容')).not.toBeInTheDocument()
    const button = screen.getByRole('button', { name: '执行这条计划' })
    expect(
      button.compareDocumentPosition(screen.getByText('Read code')),
    ).toBe(Node.DOCUMENT_POSITION_PRECEDING)
    await user.click(button)
    expect(calls).toEqual(['msg-plan'])
  })

  it('keeps process timeline outside the plan label and renders the action at the bottom', () => {
    const message: ChatMessage = {
      id: 'msg-plan-with-process',
      role: 'assistant',
      content: '## 执行计划\n\n1. 调研\n2. 实现',
      agent_plan: {
        mode: 'plan',
        status: 'draft',
        plan: '## 执行计划\n\n1. 调研\n2. 实现',
        updated_at: 1,
      },
      segments: [
        { id: 'seg-reasoning', kind: 'reasoning', phase: 'plain', order: 1, text: '先调研一下' },
        { id: 'seg-tool', kind: 'tool', phase: 'tool_loop', order: 2, tool_call_id: 'tool-search' },
        { id: 'seg-text', kind: 'text', phase: 'synthesis', order: 3, text: '## 执行计划\n\n1. 调研\n2. 实现' },
      ],
      tool_calls: [
        {
          id: 'tool-search',
          name: 'web_search',
          source: 'native',
          status: 'completed',
          arguments: '{"query":"AI chat frameworks"}',
        },
      ],
      timestamp: 1,
    }

    render(<MessageBubble message={message} onExecuteAgentPlan={() => {}} />)

    expect(screen.queryByLabelText('计划内容')).not.toBeInTheDocument()
    const button = screen.getByRole('button', { name: '执行这条计划' })
    expect(
      button.compareDocumentPosition(screen.getByText('执行计划')),
    ).toBe(Node.DOCUMENT_POSITION_PRECEDING)
    expect(screen.getByText('计划草案')).toBeInTheDocument()
  })

  it('keeps legacy plans usable without claiming implementation happened', () => {
    const message: ChatMessage = {
      id: 'msg-plan-approved',
      role: 'assistant',
      content: '1. Read code\n2. Edit',
      agent_plan: {
        mode: 'act',
        status: 'approved',
        plan: '1. Read code\n2. Edit',
        updated_at: 1,
      },
      timestamp: 1,
    }

    render(<MessageBubble message={message} onExecuteAgentPlan={() => {}} />)

    expect(screen.queryByText('已按这条计划执行')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: '执行这条计划' })).toBeInTheDocument()
  })

  it('uses document identity without requiring steps in the reply', () => {
    const execute = vi.fn()
    render(<MessageBubble message={{
      id: 'document-plan', role: 'assistant', content: '方案已保存。', timestamp: 1,
      agent_plan: { mode: 'plan', status: 'draft', document: { id: 'p1', title: '登录方案', path: 'E:/project/docs/plans/登录方案.md' } },
    }} onExecuteAgentPlan={execute} />)
    expect(screen.getByRole('button', { name: '登录方案.md' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '打开编辑' })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '执行这条计划' }))
    expect(execute).toHaveBeenCalledWith('document-plan')
  })

  it('does not turn an ordinary list into a plan', () => {
    render(<MessageBubble message={{ id: 'list', role: 'assistant', content: '1. Finding A\n2. Finding B', timestamp: 1 }} onExecuteAgentPlan={() => {}} />)
    expect(screen.queryByRole('button', { name: '执行这条计划' })).not.toBeInTheDocument()
  })
})

describe('MessageBubble timeline orphan tools', () => {
  it('renders tool calls that are missing tool segments', () => {
    const message: ChatMessage = {
      id: 'msg-1',
      role: 'assistant',
      content: 'done',
      reasoning: 'thinking',
      segments: [
        {
          id: 'seg-reasoning',
          kind: 'reasoning',
          phase: 'plain',
          order: 1,
          text: 'thinking',
        },
        {
          id: 'seg-text',
          kind: 'text',
          phase: 'plain',
          order: 2,
          text: 'done',
        },
      ],
      tool_calls: [
        {
          id: 'tool-1',
          name: 'Read',
          source: 'external_cli',
          status: 'success',
          arguments: '{"path":"README.md"}',
        },
      ],
      timestamp: 1,
    }

    render(<MessageBubble message={message} />)
    expect(screen.queryByText('Read')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: /^Worked/ }))
    expect(screen.getByText('Read')).toBeInTheDocument()
  })
})

describe('MessageBubble timeline grouping', () => {
  it.each(['segments', 'legacy-records', 'manual-open'])('folds a seen live thought and tool process on completion (%s), unless explicitly opened', (mode) => {
    const message: ChatMessage = { id: 'thought-then-tools', role: 'assistant', timestamp: 1, content: '', segments: [
      { id: 'thought', kind: 'reasoning', phase: 'tool_loop', order: 0, text: 'Plan before running tools' },
    ] }
    const { rerender } = render(<MessageBubble message={message} messageStreaming reasoningStreaming />)
    expect(screen.getByTestId('reasoning-preview')).toBeVisible()
    const work = screen.getByRole('button', { name: /^Working/ })
    if (mode === 'manual-open') {
      fireEvent.click(work)
      fireEvent.click(work)
    }
    const answered: ChatMessage = { ...message, content: 'The final answer', stream_outcome: 'completed',
      tool_calls: [{ id: 'read', name: 'read_file', source: 'native', status: 'completed' }],
      segments: [
        ...message.segments!,
        { id: 'progress', kind: 'text', phase: 'tool_loop', order: 1, text: 'Reading the source file' },
        ...(mode === 'legacy-records' ? [] : [{ id: 'call', kind: 'tool' as const, phase: 'tool_loop' as const, order: 2, tool_call_id: 'read' }]),
        { id: 'answer', kind: 'text', phase: 'synthesis', order: 3, text: 'The final answer' },
      ],
    }
    rerender(<MessageBubble message={answered} messageStreaming />)
    expect(work).toHaveAttribute('aria-expanded', 'true')
    rerender(<MessageBubble message={answered} />)
    expect(screen.getByRole('button', { name: /^Worked/ })).toBe(work)
    expect(work).toHaveAttribute('aria-expanded', mode === 'manual-open' ? 'true' : 'false')
    expect(screen.getByText('The final answer')).toBeVisible()
    if (mode !== 'manual-open') {
      expect(screen.queryByText('Reading the source file')).not.toBeInTheDocument()
      expect(screen.queryByLabelText('Thinking')).not.toBeInTheDocument()
      fireEvent.click(work)
    }
    expect(screen.getByText('Reading the source file')).toBeVisible()
  })

  it('keeps a seen thinking preview through answer start and whole-turn completion', () => {
    const message: ChatMessage = { id: 'seen-thinking', role: 'assistant', timestamp: 1, content: '', segments: [
      { id: 'thought', kind: 'reasoning', phase: 'plain', order: 0, text: 'Visible live thought' },
    ] }
    const { rerender } = render(<MessageBubble message={message} messageStreaming reasoningStreaming />)
    const preview = screen.getByTestId('reasoning-preview')
    const answered: ChatMessage = { ...message, content: 'Final answer', stream_outcome: 'completed', segments: [
      ...message.segments!, { id: 'answer', kind: 'text', phase: 'synthesis', order: 1, text: 'Final answer' },
    ] }
    rerender(<MessageBubble message={answered} messageStreaming />)
    expect(preview).toBeVisible()
    rerender(<MessageBubble message={answered} />)
    expect(screen.getByTestId('reasoning-preview')).toBe(preview)
    expect(preview).toBeVisible()
    expect(screen.getByRole('button', { name: /^Worked/ })).toHaveAttribute('aria-expanded', 'true')
    expect(screen.getByText('Final answer')).toBeVisible()
    fireEvent.click(screen.getByRole('button', { name: /^Worked/ }))
    expect(screen.queryByText('Visible live thought')).not.toBeInTheDocument()
  })

  it('collapses a completed group into a one-line summary by default', () => {
    const message: ChatMessage = {
      id: 'msg-2',
      role: 'assistant',
      content: 'answer',
      segments: [
        { id: 'seg-r', kind: 'reasoning', phase: 'plain', order: 1, text: 'planning' },
        { id: 'seg-t', kind: 'tool', phase: 'tool_loop', order: 2, tool_call_id: 'tool-1' },
        { id: 'seg-text', kind: 'text', phase: 'plain', order: 3, text: 'answer' },
      ],
      tool_calls: [
        {
          id: 'tool-1',
          name: 'read_file',
          source: 'native',
          status: 'completed',
          arguments: '{"path":"a.ts"}',
        },
      ],
      timestamp: 1,
    }

    render(<MessageBubble message={message} />)
    expect(screen.getByRole('button', { name: /Worked/ })).toBeInTheDocument()
    // collapsed historical groups keep only the summary mounted
    expect(screen.getByLabelText('过程分组')).toHaveAttribute('aria-label', '过程分组')
    expect(screen.queryByText('planning')).not.toBeInTheDocument()
    expect(screen.queryByText('read_file')).not.toBeInTheDocument()
    // final answer text still renders
    expect(screen.getByText('answer')).toBeInTheDocument()
  })

  it('uses a Worked-for duration title when tool timestamps are present', () => {
    const message: ChatMessage = {
      id: 'msg-worked',
      role: 'assistant',
      content: 'answer',
      segments: [
        { id: 'seg-t', kind: 'tool', phase: 'tool_loop', order: 1, tool_call_id: 'tool-1' },
        { id: 'seg-text', kind: 'text', phase: 'synthesis', order: 2, text: 'answer' },
      ],
      tool_calls: [
        {
          id: 'tool-1',
          name: 'read_file',
          source: 'native',
          status: 'completed',
          started_at: 1_700_000_000,
          completed_at: 1_700_000_012,
        },
      ],
      timestamp: 1,
    }

    render(<MessageBubble message={message} />)
    expect(screen.getByRole('button', { name: /Worked for 12s/ })).toHaveAttribute('aria-expanded', 'false')
    expect(screen.getByText('answer')).toBeInTheDocument()
  })

  it('folds tool-loop text with tool details after completion', async () => {
    const user = userEvent.setup()
    const message: ChatMessage = {
      id: 'msg-commentary',
      role: 'assistant',
      content: 'answer',
      segments: [
        { id: 'seg-t', kind: 'tool', phase: 'tool_loop', order: 1, tool_call_id: 'tool-1' },
        { id: 'seg-note', kind: 'text', phase: 'tool_loop', order: 2, text: 'looking around' },
        { id: 'seg-text', kind: 'text', phase: 'synthesis', order: 3, text: 'answer' },
      ],
      tool_calls: [
        {
          id: 'tool-1',
          name: 'read_file',
          source: 'native',
          status: 'completed',
        },
      ],
      timestamp: 1,
    }

    render(<MessageBubble message={message} />)
    expect(screen.queryByText('looking around')).not.toBeInTheDocument()
    expect(screen.getByText('answer')).toBeInTheDocument()
    expect(screen.getAllByLabelText('过程分组')).toHaveLength(1)

    await user.click(screen.getByRole('button', { name: /Worked/ }))
    expect(screen.getByText('looking around')).toBeInTheDocument()
  })

  it('mounts completed group details only after the user expands it', async () => {
    const user = userEvent.setup()
    const message: ChatMessage = {
      id: 'msg-expand',
      role: 'assistant',
      content: 'answer',
      segments: [
        { id: 'seg-r', kind: 'reasoning', phase: 'plain', order: 1, text: 'planning details' },
        { id: 'seg-t', kind: 'tool', phase: 'tool_loop', order: 2, tool_call_id: 'tool-1' },
        { id: 'seg-text', kind: 'text', phase: 'plain', order: 3, text: 'answer' },
      ],
      tool_calls: [
        {
          id: 'tool-1',
          name: 'read_file',
          source: 'native',
          status: 'completed',
          arguments: '{"path":"a.ts"}',
        },
      ],
      timestamp: 1,
    }

    render(<MessageBubble message={message} />)
    const toggle = screen.getByRole('button', { name: /Worked/ })
    expect(toggle).toHaveAttribute('aria-expanded', 'false')
    expect(screen.queryByText('planning details')).not.toBeInTheDocument()
    expect(screen.queryByText('read_file')).not.toBeInTheDocument()

    await user.click(toggle)

    expect(toggle).toHaveAttribute('aria-expanded', 'true')
    await user.click(screen.getByRole('button', { name: /^Thought/ }))
    expect(screen.getByText('planning details')).toBeInTheDocument()
    // 展开后组内工具块挂载：Cursor 式动词 Read + 目标（文件名）
    expect(screen.getByText('a.ts')).toBeInTheDocument()

    await user.click(toggle)

    expect(toggle).toHaveAttribute('aria-expanded', 'false')
    expect(screen.queryByText('planning details')).not.toBeInTheDocument()
    expect(screen.queryByText('a.ts')).not.toBeInTheDocument()
  })

  it('keeps many collapsed history tools out of the DOM until expanded', async () => {
    const user = userEvent.setup()
    const toolCount = 20
    const message: ChatMessage = {
      id: 'msg-heavy',
      role: 'assistant',
      content: 'final answer',
      segments: [
        ...Array.from({ length: toolCount }, (_, index) => ({
          id: `seg-tool-${index}`,
          kind: 'tool' as const,
          phase: 'tool_loop' as const,
          order: index,
          tool_call_id: `tool-${index}`,
        })),
        {
          id: 'seg-answer',
          kind: 'text',
          phase: 'plain',
          order: toolCount,
          text: 'final answer',
        },
      ],
      tool_calls: Array.from({ length: toolCount }, (_, index) => ({
        id: `tool-${index}`,
        name: 'write',
        source: 'native',
        status: 'completed',
        structured_content: {
          operation: 'write',
          resolvedPath: `file-${index}.ts`,
          additions: index + 1,
          removals: 0,
          diff: `diff payload ${index}`,
        },
      })),
      timestamp: 1,
    }

    render(<MessageBubble message={message} />)

    expect(screen.getByRole('button', { name: /Worked/ })).toHaveAttribute(
      'aria-expanded',
      'false',
    )
    expect(screen.queryByText('write')).not.toBeInTheDocument()
    expect(screen.queryByText('diff payload 0')).not.toBeInTheDocument()
    expect(screen.getByText('final answer')).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: /Worked/ }))

    expect(screen.getAllByText('Write')).toHaveLength(toolCount)
    expect(screen.getAllByText('file-0.ts').length).toBeGreaterThan(0)
  })

  it('folds text between tools when a legacy content fallback supplies the final answer', () => {
    const message: ChatMessage = {
      id: 'msg-3',
      role: 'assistant',
      content: 'final',
      segments: [
        { id: 'g1', kind: 'tool', phase: 'tool_loop', order: 1, tool_call_id: 'c1' },
        { id: 'txt', kind: 'text', phase: 'plain', order: 2, text: 'middle' },
        { id: 'g2', kind: 'tool', phase: 'tool_loop', order: 3, tool_call_id: 'c2' },
      ],
      tool_calls: [
        { id: 'c1', name: 'run_command', source: 'native', status: 'completed' },
        { id: 'c2', name: 'web_fetch', source: 'native', status: 'completed' },
      ],
      timestamp: 1,
    }

    render(<MessageBubble message={message} />)
    expect(screen.getAllByLabelText('过程分组')).toHaveLength(1)
    expect(screen.queryByText('middle')).not.toBeInTheDocument()
    expect(screen.getByText('final')).toBeVisible()
  })

  it('keeps the last group expanded while the message is streaming', () => {
    const message: ChatMessage = {
      id: 'msg-4',
      role: 'assistant',
      content: '',
      segments: [
        { id: 'seg-t', kind: 'tool', phase: 'tool_loop', order: 1, tool_call_id: 'tool-1' },
      ],
      tool_calls: [
        {
          id: 'tool-1',
          name: 'run_command',
          source: 'native',
          // 工具已完成、但消息整体仍在流式：末组应保持展开，不折叠抖动
          status: 'completed',
        },
      ],
      timestamp: 1,
    }

    render(<MessageBubble message={message} messageStreaming />)
    expect(screen.getByText('Working')).toBeInTheDocument()
    // 展开态：组内工具块细节仍渲染（动词 Run）
    expect(screen.getByText('Run')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Working' })).toHaveAttribute(
      'aria-expanded',
      'true',
    )
  })

  it('unmounts an automatically expanded group when streaming finishes', () => {
    const message: ChatMessage = {
      id: 'msg-stream-finish',
      role: 'assistant',
      content: '',
      segments: [
        { id: 'seg-r', kind: 'reasoning', phase: 'plain', order: 1, text: 'live details' },
        { id: 'seg-t', kind: 'tool', phase: 'tool_loop', order: 2, tool_call_id: 'tool-1' },
      ],
      tool_calls: [
        { id: 'tool-1', name: 'run_command', source: 'native', status: 'completed' },
      ],
      timestamp: 1,
    }

    const { rerender } = render(<MessageBubble message={message} messageStreaming />)
    expect(screen.queryByText('live details')).not.toBeInTheDocument()
    expect(screen.getByText('Run')).toBeInTheDocument()

    rerender(<MessageBubble message={message} messageStreaming={false} />)

    expect(screen.queryByText('live details')).not.toBeInTheDocument()
    expect(screen.queryByText('Run')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Worked/ })).toHaveAttribute(
      'aria-expanded',
      'false',
    )
  })

  it('shares one live Work control across process sections separated by presentations', () => {
    const message: ChatMessage = {
      id: 'msg-5',
      role: 'assistant',
      content: '',
      segments: [
        { id: 'g1', kind: 'tool', phase: 'tool_loop', order: 1, tool_call_id: 'c1' },
        { id: 'present', kind: 'tool', phase: 'tool_loop', order: 2, tool_call_id: 'present-1' },
        { id: 'g2', kind: 'tool', phase: 'tool_loop', order: 3, tool_call_id: 'c2' },
      ],
      tool_calls: [
        { id: 'c1', name: 'run_command', source: 'native', status: 'completed' },
        {
          id: 'present-1',
          name: 'present_artifacts',
          source: 'native',
          status: 'completed',
          structured_content: { type: 'artifact_presentation', artifactIds: [] },
        },
        { id: 'c2', name: 'web_fetch', source: 'native', status: 'running' },
      ],
      timestamp: 1,
    }

    render(<MessageBubble message={message} messageStreaming />)
    const groups = screen.getAllByLabelText('过程分组')
    expect(groups).toHaveLength(2)
    expect(screen.getAllByRole('button', { name: 'Working' })).toHaveLength(1)
    expect(screen.queryByRole('button', { name: /^Worked/ })).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Working' })).toHaveAttribute(
      'aria-expanded',
      'true',
    )
  })

  it('collapses every group once streaming has finished', () => {
    const message: ChatMessage = {
      id: 'msg-6',
      role: 'assistant',
      content: '',
      segments: [
        { id: 'seg-t', kind: 'tool', phase: 'tool_loop', order: 1, tool_call_id: 'tool-1' },
      ],
      tool_calls: [
        { id: 'tool-1', name: 'run_command', source: 'native', status: 'completed' },
      ],
      timestamp: 1,
    }

    // messageStreaming 默认 false（历史消息）→ 末组也折叠
    render(<MessageBubble message={message} />)
    expect(screen.getByRole('button', { name: /Worked/ })).toHaveAttribute(
      'aria-expanded',
      'false',
    )
  })
})

describe('MessageBubble 多模型所发模型标签（R8）', () => {
  const userMessage: ChatMessage = {
    id: 'msg-user',
    role: 'user',
    content: '比较这几个模型',
    group_id: 'grp-1',
    timestamp: 1,
  }

  it('多模型（≥2）时在 user 气泡顶部渲染所发模型标签', () => {
    render(
      <MessageBubble
        message={userMessage}
        sentModels={[
          { providerId: 'deepseek', model: 'deepseek-chat' },
          { providerId: 'qwen', model: 'qwen-max' },
        ]}
      />,
    )
    expect(screen.getByText('@deepseek-chat')).toBeInTheDocument()
    expect(screen.getByText('@qwen-max')).toBeInTheDocument()
  })

  it('单模型 / 缺省时不渲染标签行（无回归）', () => {
    const { rerender } = render(
      <MessageBubble message={userMessage} sentModels={[{ providerId: 'deepseek', model: 'deepseek-chat' }]} />,
    )
    expect(screen.queryByText('@deepseek-chat')).not.toBeInTheDocument()
    rerender(<MessageBubble message={userMessage} />)
    expect(screen.queryByText(/^@/)).not.toBeInTheDocument()
  })
})

describe('MessageBubble 一键 rewind', () => {
  const userMessage: ChatMessage = {
    id: 'msg-user-rewind',
    role: 'user',
    content: '原始问题',
    timestamp: 1,
  }

  it('点击直接回调 rewind，不弹编辑框', async () => {
    const onRewindMessage = vi.fn().mockResolvedValue(undefined)
    render(<MessageBubble message={userMessage} onRewindMessage={onRewindMessage} />)

    await userEvent.click(screen.getByRole('button', { name: '回到这里' }))

    expect(onRewindMessage).toHaveBeenCalledWith('msg-user-rewind')
    expect(screen.queryByRole('textbox')).not.toBeInTheDocument()
  })

  it('无回调时不渲染按钮（生成中被 MessageList 收走）', () => {
    render(<MessageBubble message={userMessage} />)
    expect(screen.queryByRole('button', { name: '回到这里' })).not.toBeInTheDocument()
  })
})

describe('MessageBubble 建分支', () => {
  const userMessage: ChatMessage = {
    id: 'msg-user-fork',
    role: 'user',
    content: '用户问题',
    timestamp: 1,
  }
  const assistantMessage: ChatMessage = {
    id: 'msg-asst-fork',
    role: 'assistant',
    content: '助手回答',
    timestamp: 2,
  }

  it('用户消息点分支按钮调用 onForkMessage(id)', async () => {
    const onForkMessage = vi.fn().mockResolvedValue(undefined)
    render(<MessageBubble message={userMessage} onForkMessage={onForkMessage} />)

    await userEvent.click(screen.getByRole('button', { name: '建分支' }))
    expect(onForkMessage).toHaveBeenCalledWith('msg-user-fork')
  })

  it('助手消息点分支按钮调用 onForkMessage(id)', async () => {
    const onForkMessage = vi.fn().mockResolvedValue(undefined)
    render(<MessageBubble message={assistantMessage} onForkMessage={onForkMessage} />)

    await userEvent.click(screen.getByRole('button', { name: '建分支' }))
    expect(onForkMessage).toHaveBeenCalledWith('msg-asst-fork')
  })

  it('无 onForkMessage 时用户消息不渲染分支按钮', () => {
    render(<MessageBubble message={userMessage} />)
    expect(screen.queryByRole('button', { name: '建分支' })).not.toBeInTheDocument()
  })
})



describe('MessageBubble explicit artifact presentation', () => {
  it.each(['completed', 'cancelled'])('keeps subsequent work below delivered artifacts through %s and re-expansion', outcome => {
    const message: ChatMessage = {
      id: 'delivery-during-work', role: 'assistant', timestamp: 1, content: '',
      artifacts: [
        { id: 'preview', name: 'preview.png', mime_type: 'image/png', data_url: 'data:image/png;base64,aA==' },
        { id: 'report', name: 'report.txt', mime_type: 'text/plain', data_url: 'data:text/plain;base64,aA==' },
      ],
      tool_calls: [{ id: 'present', name: 'present_artifacts', source: 'native', status: 'completed',
        structured_content: { type: 'artifact_presentation', artifactIds: ['preview'], caption: 'First delivery' } }],
      segments: [
        { id: 'before', kind: 'reasoning', phase: 'tool_loop', order: 0, text: 'Prepare preview' },
        { id: 'present', kind: 'tool', phase: 'tool_loop', order: 1, tool_call_id: 'present' },
      ],
    }
    const { container, rerender } = render(<MessageBubble message={message} messageStreaming />)
    const preview = container.querySelector('img')!
    const work = screen.getByRole('button', { name: 'Working' })
    const continuing: ChatMessage = { ...message, segments: [...message.segments!,
      { id: 'after', kind: 'text', phase: 'tool_loop', order: 2, text: 'Verify video after delivery' },
      { id: 'verify', kind: 'tool', phase: 'tool_loop', order: 3, tool_call_id: 'verify' },
      { id: 'second', kind: 'tool', phase: 'tool_loop', order: 4, tool_call_id: 'second' },
      { id: 'cleanup', kind: 'text', phase: 'tool_loop', order: 5, text: 'Cleanup after second delivery' },
      { id: 'finish', kind: 'reasoning', phase: 'tool_loop', order: 6, text: 'Ready to finish' },
    ], tool_calls: [...message.tool_calls!,
      { id: 'second', name: 'present_artifacts', source: 'native', status: 'completed',
        structured_content: { type: 'artifact_presentation', artifactIds: ['report'], caption: 'Second delivery' } },
    ] }
    const assertOrder = () => {
      const after = screen.getByText('Verify video after delivery')
      const second = screen.getByText('Second delivery')
      const cleanup = screen.getByText('Cleanup after second delivery')
      expect(preview.compareDocumentPosition(after) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
      expect(after.compareDocumentPosition(second) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
      expect(second.compareDocumentPosition(cleanup) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
      expect(container.querySelector('img')).toBe(preview)
    }
    rerender(<MessageBubble message={continuing} messageStreaming />)
    assertOrder()
    expect(screen.getAllByRole('button', { name: 'Working' })).toEqual([work])
    fireEvent.click(work)
    expect(screen.queryByText('Verify video after delivery')).not.toBeInTheDocument()
    expect(screen.queryByText('Cleanup after second delivery')).not.toBeInTheDocument()
    expect(preview).toBeVisible()
    expect(screen.getByText('Second delivery')).toBeVisible()
    const finished: ChatMessage = { ...continuing, stream_outcome: outcome, segments: [...continuing.segments!,
      { id: 'final', kind: 'text', phase: 'plain', order: 7, text: 'Final test results' },
    ] }
    rerender(<MessageBubble message={finished} />)
    expect(screen.getByRole('button', { name: /^Worked/ })).toBe(work)
    expect(screen.getByText('Final test results')).toBeVisible()
    expect(container.querySelector('img')).toBe(preview)
    fireEvent.click(work)
    assertOrder()
    expect(screen.getByText('Final test results').closest('[aria-label="过程分组"]')).toBeNull()
  })

  it('keeps streaming answer text below the presented image before completion', () => {
    const message: ChatMessage = {
      id: 'streaming-image-answer', role: 'assistant', timestamp: 1, content: '图片说明正在生成',
      artifacts: [{ id: 'preview', name: 'preview.png', mime_type: 'image/png', data_url: 'data:image/png;base64,aA==' }],
      tool_calls: [{ id: 'present', name: 'present_artifacts', source: 'native', status: 'completed',
        structured_content: { type: 'artifact_presentation', artifactIds: ['preview'] } }],
      segments: [
        { id: 'present', kind: 'tool', phase: 'tool_loop', order: 0, tool_call_id: 'present' },
        { id: 'answer', kind: 'text', phase: 'tool_loop', order: 1, text: '图片说明正在生成' },
      ],
    }
    const { container, rerender } = render(<MessageBubble message={message} messageStreaming />)
    const assertAnswerPosition = () => {
      const answer = screen.getByText('图片说明正在生成')
      expect(answer.closest('[aria-label="过程分组"]')).toBeNull()
      expect(container.querySelector('img')!.compareDocumentPosition(answer) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    }
    assertAnswerPosition()
    rerender(<MessageBubble message={message} />)
    assertAnswerPosition()
  })

  it.each(['completed', 'cancelled', 'error', 'interrupted'])('keeps late deliveries visible with Work manually closed after %s', outcome => {
    const message: ChatMessage = {
      id: 'late-delivery', role: 'assistant', timestamp: 1, content: '动画已完成。',
      segments: [
        { id: 'read', kind: 'tool', phase: 'tool_loop', order: 0, tool_call_id: 'read' },
        { id: 'answer', kind: 'text', phase: 'plain', order: 1, text: '动画已完成。' },
        { id: 'present', kind: 'tool', phase: 'tool_loop', order: 2, tool_call_id: 'present' },
      ],
      toolCalls: [
        { id: 'read', name: 'read_file', source: 'native', status: 'completed' },
        { id: 'present', toolName: 'present_artifacts', source: 'native', status: 'running' },
      ],
    }
    const { container, rerender } = render(<MessageBubble message={message} messageStreaming />)
    const work = screen.getByRole('button', { name: 'Working' })
    fireEvent.click(work)
    const presented: ChatMessage = { ...message, toolCalls: [message.toolCalls![0], {
      ...message.toolCalls![1], status: 'completed',
      structuredContent: { type: 'artifact_presentation', artifact_ids: ['preview', 'html'], caption: '动画与预览' },
    }] }
    rerender(<MessageBubble message={presented} messageStreaming />)
    expect(screen.getByLabelText('展示文件')).toHaveTextContent('2 个文件不可用')
    const delivered: ChatMessage = { ...presented, artifacts: [
      { id: 'preview', name: 'preview.png', mime_type: 'image/png', data_url: 'data:image/png;base64,aA==' },
      { id: 'html', name: 'pelican-bike.html', mime_type: 'text/html', path: 'C:/workspace/pelican-bike.html' },
    ] }
    rerender(<MessageBubble message={delivered} messageStreaming />)
    expect(work).toHaveAttribute('aria-expanded', 'false')
    expect(screen.getByLabelText('展示文件').closest('[aria-label="过程分组"]')).toBeNull()
    expect(screen.getByRole('button', { name: /pelican-bike\.html/ })).toBeVisible()
    expect(screen.getByText('动画已完成。')).toBeVisible()
    expect(screen.queryByText(/个文件不可用/)).not.toBeInTheDocument()
    rerender(<MessageBubble message={{ ...delivered, streamOutcome: outcome }} />)
    expect(screen.getByRole('button', { name: /^Worked/ })).toBe(work)
    expect(work).toHaveAttribute('aria-expanded', 'false')
    expect(screen.getByText('动画已完成。')).toBeVisible()
    expect(screen.getByRole('button', { name: /pelican-bike\.html/ })).toBeVisible()
    expect(container.querySelector('img')).toBeVisible()
    fireEvent.click(work)
    expect(screen.getAllByLabelText('展示文件')).toHaveLength(1)
    expect(screen.getAllByRole('button', { name: /pelican-bike\.html/ })).toHaveLength(1)
    expect(container.querySelectorAll('img')).toHaveLength(1)
  })

  it.each(['timeline', 'orphan', 'legacy'])('keeps delivered preview and HTML outside the single Work (%s)', shape => {
    const message: ChatMessage = {
      id: 'delivered-html', role: 'assistant', timestamp: 1, content: '动画已完成。',
      artifacts: [
        { id: 'preview', name: 'preview.png', mime_type: 'image/png', data_url: 'data:image/png;base64,aA==' },
        { id: 'html', name: 'pelican-bike.html', mime_type: 'text/html', data_url: 'data:text/html;base64,aA==' },
      ],
      tool_calls: [
        { id: 'read', name: 'read_file', source: 'native', status: 'completed' },
        { id: 'present', name: 'present_artifacts', source: 'native', status: 'completed',
          structured_content: { type: 'artifact_presentation', artifactIds: ['preview', 'html'], caption: '动画与预览' } },
      ],
      segments: shape === 'legacy' ? undefined : [
        { id: 'read', kind: 'tool', phase: 'tool_loop', order: 0, tool_call_id: 'read' },
        ...(shape === 'timeline' ? [{ id: 'present', kind: 'tool' as const, phase: 'tool_loop' as const, order: 1, tool_call_id: 'present' }] : []),
        { id: 'final', kind: 'text', phase: 'plain', order: 2, text: '动画已完成。' },
      ],
    }
    const { container, rerender } = render(<MessageBubble message={message} messageStreaming />)
    const file = screen.getByRole('button', { name: /pelican-bike\.html/ })
    expect(file.closest('[aria-label="过程分组"]')).toBeNull()
    rerender(<MessageBubble message={message} />)
    expect(screen.getByRole('button', { name: /pelican-bike\.html/ })).toBeVisible()
    expect(container.querySelector('img')).toBeVisible()
    expect(screen.getByText('动画与预览')).toBeVisible()
    const worked = screen.getByRole('button', { name: /^Worked/ })
    expect(worked).toHaveAttribute('aria-expanded', 'false')
    fireEvent.click(worked)
    expect(screen.getAllByRole('button', { name: /pelican-bike\.html/ })).toHaveLength(1)
    expect(container.querySelectorAll('img')).toHaveLength(1)
    expect(screen.getAllByLabelText('过程分组')).toHaveLength(1)
  })

  const artifact = {
    id: 'art_report',
    name: 'report.txt',
    mime_type: 'text/plain',
    data_url: 'data:text/plain;base64,cmVwb3J0',
    size_bytes: 6,
  }

  it('does not automatically show newly identified artifacts', () => {
    const message: ChatMessage = {
      id: 'msg-hidden-artifact',
      role: 'assistant',
      content: 'The file is ready.',
      artifacts: [artifact],
      timestamp: 1,
    }

    render(<MessageBubble message={message} />)

    expect(screen.queryByRole('button', { name: /report\.txt/ })).not.toBeInTheDocument()
  })

  it('shows only selected artifacts at the presentation segment position', () => {
    const message: ChatMessage = {
      id: 'msg-present-artifact',
      role: 'assistant',
      content: 'before\n\nafter',
      artifacts: [
        artifact,
        { ...artifact, id: 'art_hidden', name: 'hidden.txt' },
      ],
      segments: [
        { id: 'before', kind: 'text', phase: 'plain', order: 1, text: 'before' },
        { id: 'present', kind: 'tool', phase: 'tool_loop', order: 2, tool_call_id: 'call-present' },
        { id: 'after', kind: 'text', phase: 'synthesis', order: 3, text: 'after' },
      ],
      tool_calls: [
        {
          id: 'call-present',
          name: 'present_artifacts',
          source: 'native',
          status: 'completed',
          structured_content: {
            type: 'artifact_presentation',
            artifactIds: ['art_report'],
            caption: 'Download report',
          },
        },
      ],
      timestamp: 1,
    }

    render(<MessageBubble message={message} />)

    const file = screen.getByRole('button', { name: /report\.txt/ })
    const after = screen.getByText('after')
    expect(file).toBeVisible()
    expect(after).toBeVisible()
    const before = screen.getByText('before')
    expect(screen.getByText('Download report')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /hidden\.txt/ })).not.toBeInTheDocument()
    expect(before.compareDocumentPosition(file) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    expect(file.compareDocumentPosition(after) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
  })

  it('renders presentations for legacy messages without timeline segments', () => {
    const message: ChatMessage = {
      id: 'msg-present-without-segments',
      role: 'assistant',
      content: 'See the report.',
      artifacts: [artifact],
      tool_calls: [
        {
          id: 'call-present',
          name: 'present_artifacts',
          source: 'native',
          status: 'completed',
          structured_content: {
            type: 'artifact_presentation',
            artifact_ids: ['art_report'],
          },
        },
      ],
      timestamp: 1,
    }

    render(<MessageBubble message={message} />)

    expect(screen.getByRole('button', { name: /report\.txt/ })).toBeInTheDocument()
  })

  it('reports unavailable artifact IDs without falling back to paths', () => {
    const message: ChatMessage = {
      id: 'msg-missing-artifact',
      role: 'assistant',
      content: '',
      artifacts: [artifact],
      segments: [
        { id: 'present', kind: 'tool', phase: 'tool_loop', order: 1, tool_call_id: 'call-present' },
      ],
      tool_calls: [
        {
          id: 'call-present',
          name: 'present_artifacts',
          source: 'native',
          status: 'completed',
          structured_content: {
            type: 'artifact_presentation',
            artifactIds: ['art_missing'],
          },
        },
      ],
      timestamp: 1,
    }

    render(<MessageBubble message={message} />)

    expect(screen.getByText(/^1 /)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /report\.txt/ })).not.toBeInTheDocument()
  })

  it('keeps historical artifacts without IDs visible', () => {
    const message: ChatMessage = {
      id: 'msg-legacy-artifact',
      role: 'assistant',
      content: 'Legacy message',
      artifacts: [{ ...artifact, id: undefined }],
      timestamp: 1,
    }

    render(<MessageBubble message={message} />)

    expect(screen.getByRole('button', { name: /report\.txt/ })).toBeInTheDocument()
  })

  it('lays presented images out as wrapping tiles and files as compact chips', () => {
    const message: ChatMessage = {
      id: 'msg-mixed-artifacts',
      role: 'assistant',
      content: '',
      artifacts: [
        {
          id: 'art_img',
          name: 'shot.jpg',
          mime_type: 'image/jpeg',
          data_url: 'data:image/jpeg;base64,/9j/4AAQ',
        },
        {
          id: 'art_img2',
          name: 'shot-2.jpg',
          mime_type: 'image/jpeg',
          data_url: 'data:image/jpeg;base64,/9j/4AAQ',
        },
        {
          id: 'art_pdf',
          name: '简历.pdf',
          mime_type: 'application/pdf',
          data_url: 'data:application/pdf;base64,JVBERi0xLjc=',
          path: '/tmp/简历.pdf',
        },
        {
          id: 'art_md',
          name: 'notes.md',
          mime_type: 'text/markdown',
          data_url: 'data:text/markdown;base64,YQ==',
          path: '/tmp/notes.md',
        },
      ],
      segments: [
        { id: 'present', kind: 'tool', phase: 'tool_loop', order: 1, tool_call_id: 'call-present' },
      ],
      tool_calls: [
        {
          id: 'call-present',
          name: 'present_artifacts',
          source: 'native',
          status: 'completed',
          structured_content: {
            type: 'artifact_presentation',
            artifactIds: ['art_img', 'art_img2', 'art_pdf', 'art_md'],
          },
        },
      ],
      timestamp: 1,
    }

    const { container } = render(<MessageBubble message={message} />)

    const images = container.querySelectorAll('img')
    expect(images).toHaveLength(2)
    expect(images[0]?.closest('.flex-wrap')).not.toBeNull()
    expect(images[0]?.closest('button')?.style.width).toBe('128px')
    expect(images[0]?.closest('button')?.className ?? '').not.toContain('h-16')
    expect(screen.getByRole('button', { name: '打开文件 简历.pdf' }).className).toContain('h-16')
    expect(screen.getByRole('button', { name: '打开文件 notes.md' }).className).toContain('h-16')
    expect(container.textContent).not.toContain('%PDF')
  })
})
