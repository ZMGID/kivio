// @vitest-environment jsdom
import { act, fireEvent, render, renderHook, screen, waitFor } from '@testing-library/react'
import { useState } from 'react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { MessageGroup } from './MessageGroup'
import { beginGroup, ensureGroupColumn, flushGroups, resetGroups } from './groupStreamingStore'
import { useMultiAnswerViewMode, type MultiAnswerViewMode } from './multiAnswerViewMode'
import type { ChatMessage, Conversation } from './types'
import { chatApi } from './api'

// 用公开 API 驱动展示模式（内存态 + storage 同步），替代已删除的测试专用导出。
function setMultiAnswerViewMode(mode: MultiAnswerViewMode) {
  const { result, unmount } = renderHook(() => useMultiAnswerViewMode())
  act(() => {
    result.current[1](mode)
  })
  unmount()
}

afterEach(() => {
  resetGroups()
  setMultiAnswerViewMode('tabs')
  window.localStorage.clear()
})

function assistant(id: string, content: string, providerId: string, model: string): ChatMessage {
  return {
    id,
    role: 'assistant',
    content,
    provider_id: providerId,
    model,
    group_id: 'g1',
    timestamp: 1,
  }
}

describe('MessageGroup — 删除单个回答', () => {
  it.each(['tabs', 'columns'] as const)('%s 删除指定回答并保留其它回答与续聊选择', async (mode) => {
    setMultiAnswerViewMode(mode)
    const initial: Conversation = {
      id: 'delete-group', revision: 0, title: 'Delete one answer',
      provider_id: 'openai', model: 'model-a', created_at: 1, updated_at: 1,
      messages: [
        { id: 'question', role: 'user', content: 'Keep this question', timestamp: 1 },
        assistant('a1', 'Answer A', 'openai', 'model-a'),
        assistant('a2', 'Answer B', 'openai', 'model-b'),
        assistant('a3', 'Answer C', 'openai', 'model-c'),
      ],
      group_selections: { g1: 'a2' },
    }
    window.localStorage.setItem('kivio-chat-dev-conversations', JSON.stringify([initial]))
    function ConversationView() {
      const [conversation, setConversation] = useState(initial)
      return (
        <MessageGroup
          conversationId={conversation.id}
          groupId="g1"
          messages={conversation.messages.filter(message => message.role === 'assistant')}
          selectedMessageId={conversation.group_selections?.g1}
          onDeleteMessage={async id => {
            setConversation(await chatApi.deleteMessage(conversation.id, id))
          }}
        />
      )
    }
    render(<ConversationView />)
    const user = userEvent.setup()
    await user.click(screen.getByRole('button', { name: '删除 model-c' }))
    await waitFor(() => expect(screen.queryByRole('button', { name: 'model-c' })).toBeNull())
    let saved = await chatApi.getConversation(initial.id)
    expect(saved.messages.map(message => message.id)).toEqual(['question', 'a1', 'a2'])
    expect(saved.group_selections).toEqual({ g1: 'a2' })
    expect(screen.getByRole('button', { name: 'model-b' })).toHaveAttribute('aria-pressed', 'true')

    await user.click(screen.getByRole('button', { name: '删除 model-b' }))
    await waitFor(() => expect(screen.queryByRole('button', { name: 'model-b' })).toBeNull())
    saved = await chatApi.getConversation(initial.id)
    expect(saved.messages.map(message => message.id)).toEqual(['question', 'a1'])
    expect(saved.group_selections).toEqual({})
    expect(screen.getByText('Answer A')).toBeVisible()
    expect(screen.getByRole('button', { name: 'model-a' })).toHaveAttribute('aria-pressed', 'true')
  })

  it('生成期间不允许删除已完成的同组回答', () => {
    act(() => {
      beginGroup('c1', 'g1', [{ providerId: 'openai', model: 'model-a' }])
      const column = ensureGroupColumn('c1', 'a1', 'openai', 'model-a')!
      column.streaming = false
      column.content = 'Already completed'
      flushGroups()
    })
    render(<MessageGroup conversationId="c1" groupId="g1" messages={[]} onDeleteMessage={vi.fn()} />)
    expect(screen.queryByRole('button', { name: '删除 model-a' })).toBeNull()
  })
})

describe('MessageGroup — columns 模式', () => {
  beforeEach(() => {
    setMultiAnswerViewMode('columns')
  })

  it.each(['{Enter}', ' '])('键盘聚焦非活动列后可用 %s 展开思考，其他列仍卸载正文', async (key) => {
    const user = userEvent.setup()
    act(() => {
      beginGroup('c1', 'g1', [
        { providerId: 'openai', model: 'gpt-4o' },
        { providerId: 'anthropic', model: 'claude-3' },
      ])
      const a = ensureGroupColumn('c1', 'msg_a', 'openai', 'gpt-4o')!
      a.streaming = true
      a.reasoning = 'Column A thought'
      const b = ensureGroupColumn('c1', 'msg_b', 'anthropic', 'claude-3')!
      b.streaming = true
      b.reasoning = 'Column B thought'
      flushGroups()
    })
    const { container } = render(<MessageGroup conversationId="c1" groupId="g1" messages={[]} />)
    const columns = container.querySelectorAll('.chat-message-group-col')
    const first = columns[0].querySelector<HTMLButtonElement>('button[title="展开完整思考"]')!
    fireEvent.click(first)
    expect(columns[0].querySelector('[data-testid="reasoning-text"]')).not.toBeNull()
    const second = columns[1].querySelector<HTMLButtonElement>('button[title="展开完整思考"]')!
    // Move keyboard focus without hovering or clicking the second column.
    act(() => second.focus())
    await user.keyboard(key)
    expect(second).toHaveAttribute('aria-expanded', 'true')
    expect(columns[1].querySelector('[data-testid="reasoning-text"]')).toHaveTextContent('Column B thought')
    expect(columns[0].querySelector('[data-testid="reasoning-text"]')).toBeNull()
    expect(columns[0].querySelector('[data-testid="reasoning-preview"]')).toBeNull()
  })

  it('落库态：渲染每列的「model | provider」标签', () => {
    render(
      <MessageGroup
        conversationId="c1"
        groupId="g1"
        messages={[
          assistant('a1', 'answer one', 'openai', 'gpt-4o'),
          assistant('a2', 'answer two', 'anthropic', 'claude-3'),
        ]}
      />,
    )
    // 列头 + footer chip 都含标签 → getAllByText。
    expect(screen.getAllByText('gpt-4o | openai').length).toBeGreaterThan(0)
    expect(screen.getAllByText('claude-3 | anthropic').length).toBeGreaterThan(0)
  })

  it('选中条：默认第一列高亮；点选其它列触发回调', async () => {
    const onSelect = vi.fn()
    render(
      <MessageGroup
        conversationId="c1"
        groupId="g1"
        messages={[
          assistant('a1', 'answer one', 'openai', 'gpt-4o'),
          assistant('a2', 'answer two', 'anthropic', 'claude-3'),
        ]}
        onSelectColumn={onSelect}
      />,
    )
    // 默认第一列已选（列头显示「已选」）。
    expect(screen.getByText('已选')).toBeInTheDocument()
    const continueButtons = screen.getAllByText('用这条继续')
    expect(continueButtons).toHaveLength(1)
    await act(async () => {
      continueButtons[0].click()
    })
    expect(onSelect).toHaveBeenCalledWith('g1', 'a2')
  })

  it('显式选中条：高亮所记列', () => {
    render(
      <MessageGroup
        conversationId="c1"
        groupId="g1"
        messages={[
          assistant('a1', 'answer one', 'openai', 'gpt-4o'),
          assistant('a2', 'answer two', 'anthropic', 'claude-3'),
        ]}
        selectedMessageId="a2"
        onSelectColumn={() => {}}
      />,
    )
    // a2 被选 → a1 显示「用这条继续」，a2 显示「已选」。
    expect(screen.getByText('已选')).toBeInTheDocument()
    expect(screen.getAllByText('用这条继续')).toHaveLength(1)
  })

  it('流式态：从 group store 读实时列，无选中标记', async () => {
    act(() => {
      beginGroup('c1', 'g1', [
        { providerId: 'openai', model: 'gpt-4o' },
        { providerId: 'anthropic', model: 'claude-3' },
      ])
      const a = ensureGroupColumn('c1', 'msg_a', 'openai', 'gpt-4o')!
      a.content = 'streaming A'
      // touchGroup 现在 rAF 合帧；测试用 flushGroups 立即同步通知订阅者。
      flushGroups()
    })
    render(<MessageGroup conversationId="c1" groupId="g1" messages={[]} />)
    expect(screen.getByText(/streaming A/)).toBeInTheDocument()
    // 流式态不显示选中标记（还没落库）。
    expect(screen.queryByText('已选')).not.toBeInTheDocument()
    expect(screen.queryByText('用这条继续')).not.toBeInTheDocument()
  })

  it('性能降级（R10）：非聚焦列折叠 reasoning（正文 hideBody），聚焦列展开流式思考', async () => {
    act(() => {
      beginGroup('c1', 'g1', [
        { providerId: 'openai', model: 'gpt-4o' },
        { providerId: 'anthropic', model: 'claude-3' },
      ])
      const a = ensureGroupColumn('c1', 'msg_a', 'openai', 'gpt-4o')!
      a.streaming = true
      a.reasoning = 'focused thinking'
      const b = ensureGroupColumn('c1', 'msg_b', 'anthropic', 'claude-3')!
      b.streaming = true
      b.reasoning = 'unfocused thinking'
      flushGroups()
    })
    const { container } = render(<MessageGroup conversationId="c1" groupId="g1" messages={[]} />)
    // Only the focused column mounts the latest-line preview; full text stays lazy.
    const columns = container.querySelectorAll('.chat-message-group-col')
    expect(columns[0].querySelector('[data-testid="reasoning-preview"]')).not.toBeNull()
    expect(columns[1].querySelector('[data-testid="reasoning-preview"]')).toBeNull()
    fireEvent.mouseEnter(columns[1])
    expect(columns[0].querySelector('[data-testid="reasoning-preview"]')).toBeNull()
    expect(columns[1].querySelector('[data-testid="reasoning-preview"]')).not.toBeNull()
    expect(columns[0].querySelector('[data-testid="reasoning-text"]')).toBeNull()
    fireEvent.mouseEnter(columns[0])
    expect(columns[0].querySelector('[data-testid="reasoning-preview"]')).not.toBeNull()
    expect(columns[1].querySelector('[data-testid="reasoning-preview"]')).toBeNull()
  })
})

describe('MessageGroup — tabs 模式（默认）', () => {
  it('默认只整宽渲染选中条（第一条），不显示其它条正文', () => {
    render(
      <MessageGroup
        conversationId="c1"
        groupId="g1"
        messages={[
          assistant('a1', 'answer one', 'openai', 'gpt-4o'),
          assistant('a2', 'answer two', 'anthropic', 'claude-3'),
        ]}
        onSelectColumn={() => {}}
      />,
    )
    // tabs 模式：只渲染第一条正文。
    expect(screen.getByText('answer one')).toBeInTheDocument()
    expect(screen.queryByText('answer two')).not.toBeInTheDocument()
    // 列头「用这条继续」按钮在 tabs 模式不渲染（交给 footer chip）。
    expect(screen.queryByText('用这条继续')).not.toBeInTheDocument()
    expect(screen.queryByText('已选')).not.toBeInTheDocument()
  })

  it('显式选中条：默认整宽显示所记列', () => {
    render(
      <MessageGroup
        conversationId="c1"
        groupId="g1"
        messages={[
          assistant('a1', 'answer one', 'openai', 'gpt-4o'),
          assistant('a2', 'answer two', 'anthropic', 'claude-3'),
        ]}
        selectedMessageId="a2"
        onSelectColumn={() => {}}
      />,
    )
    expect(screen.getByText('answer two')).toBeInTheDocument()
    expect(screen.queryByText('answer one')).not.toBeInTheDocument()
  })

  it('点 footer 模型 chip：切换显示条并触发 onSelectColumn（一举两用）', async () => {
    const onSelect = vi.fn()
    render(
      <MessageGroup
        conversationId="c1"
        groupId="g1"
        messages={[
          assistant('a1', 'answer one', 'openai', 'gpt-4o'),
          assistant('a2', 'answer two', 'anthropic', 'claude-3'),
        ]}
        onSelectColumn={onSelect}
      />,
    )
    // 初始显示第一条。
    expect(screen.getByText('answer one')).toBeInTheDocument()
    // footer 第二个模型 chip（claude-3）。
    const chip = screen.getByTitle('claude-3 | anthropic')
    await act(async () => {
      chip.click()
    })
    // 切换到第二条 + 触发续聊选中回调。
    expect(screen.getByText('answer two')).toBeInTheDocument()
    expect(screen.queryByText('answer one')).not.toBeInTheDocument()
    expect(onSelect).toHaveBeenCalledWith('g1', 'a2')
  })

  it('切到 columns 模式：N 列横向并排出现', async () => {
    render(
      <MessageGroup
        conversationId="c1"
        groupId="g1"
        messages={[
          assistant('a1', 'answer one', 'openai', 'gpt-4o'),
          assistant('a2', 'answer two', 'anthropic', 'claude-3'),
        ]}
        onSelectColumn={() => {}}
      />,
    )
    expect(screen.queryByText('answer two')).not.toBeInTheDocument()
    // 点 footer「并排」按钮。
    const columnsBtn = screen.getByTitle('并排显示（多列）')
    await act(async () => {
      columnsBtn.click()
    })
    // 两条都整列渲染出来。
    expect(screen.getByText('answer one')).toBeInTheDocument()
    expect(screen.getByText('answer two')).toBeInTheDocument()
  })

  it('切换和并排都能只删当前这一条，流式列不提供删除', async () => {
    const onDelete = vi.fn(async () => {})
    const messages = [
      assistant('a', 'answer a', 'dev-provider', 'model-a'),
      assistant('b', 'answer b', 'dev-provider', 'model-b'),
      assistant('c', 'answer c', 'dev-provider', 'model-c'),
    ]
    const view = render(
      <MessageGroup
        conversationId="c1"
        groupId="g1"
        messages={messages}
        selectedMessageId="b"
        onSelectColumn={() => {}}
        onDeleteMessage={onDelete}
      />,
    )
    expect(screen.getByText('answer b')).toBeInTheDocument()
    expect(screen.queryByText('answer a')).not.toBeInTheDocument()
    for (const model of ['model-a', 'model-b', 'model-c']) {
      expect(screen.getByRole('button', { name: `删除 ${model}` })).toBeInTheDocument()
    }
    fireEvent.click(screen.getByRole('button', { name: '删除 model-b' }))
    expect(onDelete).toHaveBeenCalledTimes(1)
    expect(onDelete).toHaveBeenCalledWith('b')
    expect(screen.getByText('answer b')).toBeInTheDocument()

    fireEvent.click(screen.getByTitle('并排显示（多列）'))
    expect(screen.getByText('answer a')).toBeInTheDocument()
    expect(screen.getByText('answer c')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '删除 model-c' }))
    expect(onDelete).toHaveBeenCalledTimes(2)
    expect(onDelete).toHaveBeenLastCalledWith('c')

    view.unmount()
    act(() => {
      beginGroup('c1', 'g1', [
        { providerId: 'dev-provider', model: 'model-a' },
        { providerId: 'dev-provider', model: 'model-b' },
      ])
      ensureGroupColumn('c1', 'live-a', 'dev-provider', 'model-a')
      flushGroups()
    })
    render(
      <MessageGroup
        conversationId="c1"
        groupId="g1"
        messages={[]}
        onDeleteMessage={onDelete}
      />,
    )
    expect(screen.queryByRole('button', { name: /删除/ })).not.toBeInTheDocument()
  })
})
