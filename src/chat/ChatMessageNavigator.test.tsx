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
      />,
    )
    const turn = screen.getByRole('button', { name: '第 1 轮：第一轮问题' })
    expect(turn).toHaveAttribute('aria-current', 'location')
    fireEvent.mouseEnter(turn)
    expect(screen.getByRole('tooltip')).toHaveTextContent('第一轮问题')
    expect(screen.getByRole('tooltip')).toHaveTextContent('第一轮回答')
    expect(screen.getByRole('tooltip')).toHaveTextContent('gpt-5')
    expect(screen.getByRole('button', { name: '上下文压缩摘要' })).toBeInTheDocument()
  })

  it('在长轨道滚到中段时保留原轮次编号、预览和跳转目标', () => {
    const turns = Array.from({ length: 500 }, (_, index) => ({
      ...nodes[0], id: String(index), title: `问题 ${index + 1}`,
    }))
    const navigate = vi.fn()
    const height = vi.spyOn(HTMLElement.prototype, 'offsetHeight', 'get').mockReturnValue(110)
    try {
      const navigator = (activeNodeId: string) => (
        <MessageNavigator nodes={turns} activeNodeId={activeNodeId} visibleNodeIds={[]} onNavigate={navigate} />
      )
      const view = render(navigator('0'))
      const rail = screen.getByLabelText('对话轮次导航').firstElementChild as HTMLElement
      expect(screen.queryByRole('button', { name: '第 251 轮：问题 251' })).toBeNull()
      fireEvent.wheel(rail, { deltaY: 250 * 11 })
      fireEvent.scroll(rail)
      // A new active turn in the transcript must not move the rail out from under the pointer.
      view.rerender(navigator('1'))
      expect(rail.scrollTop).toBe(250 * 11)
      const middle = screen.getByRole('button', { name: '第 251 轮：问题 251' })
      fireEvent.focus(middle)
      expect(screen.getByRole('tooltip')).toHaveTextContent('问题 251')
      fireEvent.click(middle)
      expect(navigate).toHaveBeenCalledWith(turns[250])
      view.rerender(navigator('2'))
      expect(rail.scrollTop).toBe(2 * 11)
    } finally {
      height.mockRestore()
    }
  })

  it('点击节点定位；轨道滚轮只滚导航条，不切换消息、不冒泡', () => {
    const onNavigate = vi.fn()
    const parentWheel = vi.fn()
    render(
      <div onWheel={parentWheel}>
        <MessageNavigator
          nodes={nodes}
          activeNodeId="turn-u1"
          visibleNodeIds={['turn-u1']}
          onNavigate={onNavigate}
        />
      </div>,
    )
    fireEvent.click(screen.getByRole('button', { name: '上下文压缩摘要' }))
    expect(onNavigate).toHaveBeenCalledWith(nodes[1])

    const rail = screen.getByLabelText('对话轮次导航').firstElementChild as HTMLElement
    Object.defineProperty(rail, 'clientHeight', { configurable: true, value: 40 })
    Object.defineProperty(rail, 'scrollHeight', { configurable: true, value: 200 })
    let scrollTop = 0
    Object.defineProperty(rail, 'scrollTop', {
      configurable: true,
      get: () => scrollTop,
      set: (value: number) => { scrollTop = value },
    })

    const event = new WheelEvent('wheel', { deltaY: 80, bubbles: true, cancelable: true })
    const notCanceled = rail.dispatchEvent(event)
    expect(notCanceled).toBe(false)
    expect(scrollTop).toBe(80)
    expect(parentWheel).not.toHaveBeenCalled()
    // 滚轮不再触发消息跳转
    expect(onNavigate).toHaveBeenCalledTimes(1)
  })
})
