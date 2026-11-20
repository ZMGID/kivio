import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { QueuedMessages } from './QueuedMessages'
import type { QueuedMessage } from './hooks/useMessageQueue'

function queued(overrides: Partial<QueuedMessage> = {}): QueuedMessage {
  return {
    id: 'q1',
    content: '改用 rg',
    attachments: [],
    steering: false,
    ...overrides,
  }
}

function setup(messages: QueuedMessage[]) {
  const onRemove = vi.fn()
  const onRestore = vi.fn()
  const onInsert = vi.fn()
  render(
    <QueuedMessages
      messages={messages}
      onRemove={onRemove}
      onRestore={onRestore}
      onInsert={onInsert}
      lang="zh"
    />,
  )
  return { onRemove, onRestore, onInsert }
}

describe('QueuedMessages', () => {
  it('空队列不渲染任何东西', () => {
    const { container } = render(
      <QueuedMessages
        messages={[]}
        onRemove={vi.fn()}
        onRestore={vi.fn()}
        lang="zh"
      />,
    )
    expect(container.firstChild).toBeNull()
  })

  it('shows queued user bubbles with an insert action below each message', () => {
    const { onInsert, onRemove } = setup([queued(), queued({ id: 'q2', content: '再看下测试' })])
    expect(screen.getByText('改用 rg').closest('.chat-user-bubble')).not.toBeNull()
    expect(screen.getByText('再看下测试').closest('.chat-user-bubble')).not.toBeNull()
    screen.getAllByLabelText('立即插入')[1].click()
    expect(onInsert).toHaveBeenCalledWith('q2')
    screen.getAllByLabelText('移出队列')[0].click()
    expect(onRemove).toHaveBeenCalledWith('q1')
  })

  it.each([{ steering: true }, { followingUp: true }])('submitted messages remain visible without queue controls: %j', (state) => {
    setup([queued(state)])
    expect(screen.getByText('改用 rg')).toBeVisible()
    expect(screen.queryByLabelText('移出队列')).toBeNull()
  })

  it('keeps attachments visible and allows restoring the whole draft', () => {
    const { onRestore } = setup([queued({
      attachments: [{ id: 'a1', type: 'image', name: 'a.png', path: '/tmp/a.png' }],
    })])
    screen.getByLabelText('点击撤回到输入框').click()
    expect(onRestore).toHaveBeenCalledWith('q1')
  })

  it('插入被拒后仍显示排队状态并允许重试和删除', () => {
    setup([queued({ steerRejected: true })])
    expect(screen.getByText('未能插入，仍在排队，可重试')).toBeTruthy()
    expect(screen.getByLabelText('移出队列')).toBeTruthy()
  })

  it('Pi follow-up 被拒时说明会降级为轮末发送', () => {
    setup([queued({ followUpRejected: true })])
    expect(screen.getByText('排队失败，轮末发送')).toBeTruthy()
    expect(screen.getByLabelText('移出队列')).toBeTruthy()
  })


})
