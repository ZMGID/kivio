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
  render(
    <QueuedMessages
      messages={messages}
      onRemove={onRemove}
      onRestore={onRestore}
      lang="zh"
    />,
  )
  return { onRemove, onRestore }
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

  it('逐条列出，无需手动引导，未提交条目可以删除', () => {
    const { onRemove } = setup([queued(), queued({ id: 'q2', content: '再看下测试' })])
    expect(screen.getByText('改用 rg')).toBeTruthy()
    expect(screen.getByText('再看下测试')).toBeTruthy()
    expect(screen.queryByLabelText(/立刻引导/)).toBeNull()
    screen.getAllByLabelText('移出队列')[0].click()
    expect(onRemove).toHaveBeenCalledWith('q1')
  })

  it.each([{ steering: true }, { followingUp: true }])('submitted messages leave the composer strip: %j', (state) => {
    setup([queued(state)])
    expect(screen.queryByText('改用 rg')).toBeNull()
    expect(screen.queryByLabelText('移出队列')).toBeNull()
  })

  // 撤回只把文字还给输入框，所以带附件的不给撤——否则附件会静默消失。
  it('带附件的条目不能撤回，但仍标出附件数', () => {
    const { onRestore } = setup([queued({
      attachments: [{ id: 'a1', type: 'image', name: 'a.png', path: '/tmp/a.png' }],
    })])
    expect(screen.getByText('+1')).toBeTruthy()
    screen.getByTitle(/不能撤回/).click()
    expect(onRestore).not.toHaveBeenCalled()
  })

  it('引导被拒时说明轮末发送并允许删除', () => {
    setup([queued({ steerRejected: true })])
    expect(screen.getByText('暂不能插入，将在本轮结束后发送')).toBeTruthy()
    expect(screen.getByLabelText('移出队列')).toBeTruthy()
  })

  it('Pi follow-up 被拒时说明会降级为轮末发送', () => {
    setup([queued({ followUpRejected: true })])
    expect(screen.getByText('排队失败，轮末发送')).toBeTruthy()
    expect(screen.getByLabelText('移出队列')).toBeTruthy()
  })

  it('普通条目点文本即撤回', () => {
    const { onRestore } = setup([queued()])
    screen.getByTitle('点击撤回到输入框').click()
    expect(onRestore).toHaveBeenCalledWith('q1')
  })
})
