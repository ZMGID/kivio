import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { MessageBubble } from './MessageBubble'
import type { ChatMessage } from './types'

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

    expect(screen.getByText('計畫草案')).toBeInTheDocument()
    expect(screen.queryByLabelText('計畫內容')).not.toBeInTheDocument()
    const button = screen.getByRole('button', { name: '執行這條計畫' })
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
      content: '## 執行計畫\n\n1. 調研\n2. 實現',
      agent_plan: {
        mode: 'plan',
        status: 'draft',
        plan: '## 執行計畫\n\n1. 調研\n2. 實現',
        updated_at: 1,
      },
      segments: [
        { id: 'seg-reasoning', kind: 'reasoning', phase: 'plain', order: 1, text: '先調研一下' },
        { id: 'seg-tool', kind: 'tool', phase: 'tool_loop', order: 2, tool_call_id: 'tool-search' },
        { id: 'seg-text', kind: 'text', phase: 'synthesis', order: 3, text: '## 執行計畫\n\n1. 調研\n2. 實現' },
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

    expect(screen.queryByLabelText('計畫內容')).not.toBeInTheDocument()
    const button = screen.getByRole('button', { name: '執行這條計畫' })
    expect(
      button.compareDocumentPosition(screen.getByText('執行計畫')),
    ).toBe(Node.DOCUMENT_POSITION_PRECEDING)
    expect(screen.getByText('計畫草案')).toBeInTheDocument()
  })

  it('shows approved state without an execute button', () => {
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

    expect(screen.getByText('已按這條計畫執行')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '執行這條計畫' })).not.toBeInTheDocument()
  })

  it('does not render execute action for an incomplete non-plan fragment', () => {
    const message: ChatMessage = {
      id: 'msg-plan-fragment',
      role: 'assistant',
      content: '沒問題！積萌,',
      agent_plan: {
        mode: 'plan',
        status: 'draft',
        plan: '沒問題！積萌,',
        updated_at: 1,
      },
      stream_outcome: 'interrupted',
      timestamp: 1,
    }

    render(<MessageBubble message={message} onExecuteAgentPlan={() => {}} />)

    expect(screen.queryByText('計畫草案')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '執行這條計畫' })).not.toBeInTheDocument()
  })

  it('does not render execute action for a non-plan sentence even if persisted as draft', () => {
    const message: ChatMessage = {
      id: 'msg-plan-sentence',
      role: 'assistant',
      content: '計畫：我會處理這個問題。',
      agent_plan: {
        mode: 'plan',
        status: 'draft',
        plan: '計畫：我會處理這個問題。',
        updated_at: 1,
      },
      timestamp: 1,
    }

    render(<MessageBubble message={message} onExecuteAgentPlan={() => {}} />)

    expect(screen.queryByText('計畫草案')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '執行這條計畫' })).not.toBeInTheDocument()
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
    expect(screen.getByText('Read')).toBeInTheDocument()
  })
})

describe('MessageBubble timeline grouping', () => {
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
    expect(screen.getByText(/讀取 1 個檔案/)).toBeInTheDocument()
    // collapsed historical groups keep only the summary mounted
    expect(screen.getByLabelText('過程分組')).toHaveAttribute('aria-label', '過程分組')
    expect(screen.queryByText('planning')).not.toBeInTheDocument()
    expect(screen.queryByText('read_file')).not.toBeInTheDocument()
    // final answer text still renders
    expect(screen.getByText('answer')).toBeInTheDocument()
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
    const toggle = screen.getByRole('button', { name: /讀取 1 個檔案/ })
    expect(toggle).toHaveAttribute('aria-expanded', 'false')
    expect(screen.queryByText('planning details')).not.toBeInTheDocument()
    expect(screen.queryByText('read_file')).not.toBeInTheDocument()

    await user.click(toggle)

    expect(toggle).toHaveAttribute('aria-expanded', 'true')
    expect(screen.getByText('planning details')).toBeInTheDocument()
    // 展开后组内工具块挂载：Cursor 式动词 Read + 目标（文件名）
    expect(screen.getByText('a.ts')).toBeInTheDocument()
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

    expect(screen.getByRole('button', { name: /編輯 20 個檔案/ })).toHaveAttribute(
      'aria-expanded',
      'false',
    )
    expect(screen.queryByText('write')).not.toBeInTheDocument()
    expect(screen.queryByText('diff payload 0')).not.toBeInTheDocument()
    expect(screen.getByText('final answer')).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: /編輯 20 個檔案/ }))

    expect(screen.getAllByText('Write')).toHaveLength(toolCount)
    expect(screen.getAllByText('file-0.ts').length).toBeGreaterThan(0)
  })

  it('renders tool → text → tool as two separate groups', () => {
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
    expect(screen.getAllByLabelText('過程分組')).toHaveLength(2)
    expect(screen.getByText('middle')).toBeInTheDocument()
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
          // 工具已完成、但訊息整體仍在流式：末組應保持展開，不折疊抖動
          status: 'completed',
        },
      ],
      timestamp: 1,
    }

    render(<MessageBubble message={message} messageStreaming />)
    expect(screen.getByText(/執行 1 條命令/)).toBeInTheDocument()
    // 展開態：組內工具區塊細節仍渲染
    expect(screen.getByText('執行')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /執行 1 條命令/ })).toHaveAttribute(
      'aria-expanded',
      'true',
    )
  })

  it('collapses non-last groups even while streaming', () => {
    const message: ChatMessage = {
      id: 'msg-5',
      role: 'assistant',
      content: '',
      segments: [
        { id: 'g1', kind: 'tool', phase: 'tool_loop', order: 1, tool_call_id: 'c1' },
        { id: 'txt', kind: 'text', phase: 'plain', order: 2, text: 'middle' },
        { id: 'g2', kind: 'tool', phase: 'tool_loop', order: 3, tool_call_id: 'c2' },
      ],
      tool_calls: [
        { id: 'c1', name: 'run_command', source: 'native', status: 'completed' },
        { id: 'c2', name: 'web_fetch', source: 'native', status: 'running' },
      ],
      timestamp: 1,
    }

    render(<MessageBubble message={message} messageStreaming />)
    const groups = screen.getAllByLabelText('過程分組')
    expect(groups).toHaveLength(2)
    // 前組（被正文打斷、非末組）摺疊；末組展開
    expect(screen.getByRole('button', { name: /執行 1 條命令/ })).toHaveAttribute(
      'aria-expanded',
      'false',
    )
    expect(screen.getByRole('button', { name: /正在讀取 1 個網頁/ })).toHaveAttribute(
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

    // messageStreaming 預設 false（歷史訊息）→ 末組也摺疊
    render(<MessageBubble message={message} />)
    expect(screen.getByRole('button', { name: /執行 1 條命令/ })).toHaveAttribute(
      'aria-expanded',
      'false',
    )
  })
})

describe('MessageBubble 多模型所發模型標籤（R8）', () => {
  const userMessage: ChatMessage = {
    id: 'msg-user',
    role: 'user',
    content: '比較這幾個模型',
    group_id: 'grp-1',
    timestamp: 1,
  }

  it('多模型（≥2）時在 user 氣泡頂部渲染所發模型標籤', () => {
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

  it('單模型 / 預設時不渲染標籤行（無迴歸）', () => {
    const { rerender } = render(
      <MessageBubble message={userMessage} sentModels={[{ providerId: 'deepseek', model: 'deepseek-chat' }]} />,
    )
    expect(screen.queryByText('@deepseek-chat')).not.toBeInTheDocument()
    rerender(<MessageBubble message={userMessage} />)
    expect(screen.queryByText(/^@/)).not.toBeInTheDocument()
  })
})

describe('MessageBubble 使用者訊息編輯並重新生成', () => {
  const userMessage: ChatMessage = {
    id: 'msg-user-edit',
    role: 'user',
    content: '原始問題',
    timestamp: 1,
  }

  it('點選編輯進入編輯態，儲存並重新生成攜帶新內容', async () => {
    const onRegenerateMessage = vi.fn().mockResolvedValue(undefined)
    render(<MessageBubble message={userMessage} onRegenerateMessage={onRegenerateMessage} />)

    await userEvent.click(screen.getByRole('button', { name: '編輯並重新生成' }))
    const textarea = screen.getByRole('textbox')
    expect(textarea).toHaveValue('原始問題')

    await userEvent.clear(textarea)
    await userEvent.type(textarea, '改過的問題')
    await userEvent.click(screen.getByRole('button', { name: '儲存並重新生成' }))

    expect(onRegenerateMessage).toHaveBeenCalledWith('msg-user-edit', '改過的問題')
  })

  it('內容未改動時儲存走純重新生成（不帶 newContent）', async () => {
    const onRegenerateMessage = vi.fn().mockResolvedValue(undefined)
    render(<MessageBubble message={userMessage} onRegenerateMessage={onRegenerateMessage} />)

    await userEvent.click(screen.getByRole('button', { name: '編輯並重新生成' }))
    await userEvent.click(screen.getByRole('button', { name: '儲存並重新生成' }))

    expect(onRegenerateMessage).toHaveBeenCalledWith('msg-user-edit', undefined)
  })

  it('取消恢復原文並退出編輯態；無回撥時不渲染編輯按鈕', async () => {
    const onRegenerateMessage = vi.fn().mockResolvedValue(undefined)
    const { rerender } = render(
      <MessageBubble message={userMessage} onRegenerateMessage={onRegenerateMessage} />,
    )

    await userEvent.click(screen.getByRole('button', { name: '編輯並重新生成' }))
    await userEvent.type(screen.getByRole('textbox'), '不想要的修改')
    await userEvent.click(screen.getByRole('button', { name: '取消' }))

    expect(screen.queryByRole('textbox')).not.toBeInTheDocument()
    expect(screen.getByText('原始問題')).toBeInTheDocument()
    expect(onRegenerateMessage).not.toHaveBeenCalled()

    rerender(<MessageBubble message={userMessage} />)
    expect(screen.queryByRole('button', { name: '編輯並重新生成' })).not.toBeInTheDocument()
  })
})
