import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { MessageNavigator } from './ChatMessageNavigator'
import type { MessageNavigatorNode } from './messageNavigator'

const nodes: MessageNavigatorNode[] = [
  {
    kind: 'turn',
    id: 'turn-u1',
    targetRenderIndex: 1,
    userMessageId: 'u1',
    title: '第一轮问题',
    answerPreview: '第一轮回答',
    modelLabel: 'gpt-5',
  },
  {
    kind: 'compaction',
    id: 'compaction-c1',
    targetRenderIndex: 3,
    title: '已压缩此前上下文',
    answerPreview: '压缩摘要',
    modelLabel: '',
  },
]

describe('MessageNavigator', () => {
  it('渲染独立节点、当前状态和悬停预览', () => {
    render(
      <MessageNavigator
        nodes={nodes}
        activeNodeId="turn-u1"
        visibleNodeIds={['turn-u1', 'compaction-c1']}
        onNavigate={() => {}}
        onNavigateStep={() => {}}
      />,
    )
    const turn = screen.getByRole('button', { name: '第 1 轮：第一轮问题' })
    expect(turn).toHaveAttribute('aria-current', 'location')
    expect(turn).toHaveClass('is-visible')
    expect(screen.getByRole('button', { name: '上下文压缩摘要' })).toHaveClass('is-visible')
    fireEvent.mouseEnter(turn)
    expect(screen.getByRole('tooltip')).toHaveTextContent('第一轮问题')
    expect(screen.getByRole('tooltip')).toHaveTextContent('第一轮回答')
    expect(screen.getByRole('tooltip')).toHaveTextContent('gpt-5')
    expect(screen.getByRole('button', { name: '上下文压缩摘要' })).toBeInTheDocument()
  })

  it('点击节点定位，轨道滚轮发出逐轮导航且阻止冒泡', () => {
    const onNavigate = vi.fn()
    const onNavigateStep = vi.fn()
    render(
      <MessageNavigator
        nodes={nodes}
        activeNodeId="turn-u1"
        visibleNodeIds={['turn-u1']}
        onNavigate={onNavigate}
        onNavigateStep={onNavigateStep}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: '上下文压缩摘要' }))
    expect(onNavigate).toHaveBeenCalledWith(nodes[1])

    const rail = screen.getByLabelText('对话轮次导航').firstElementChild!
    fireEvent.wheel(rail, { deltaY: 80 })
    expect(onNavigateStep).toHaveBeenCalledWith(1)
  })
})
