import { useState } from 'react'
import { render } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { usePanelShiftMotion } from './usePanelShiftMotion'

type Geometry = { left: number; width: number }

function Row({ panels, view, onRow, anchor }: {
  panels: string
  view: string
  onRow: (row: HTMLDivElement | null) => void
  anchor?: 'center' | 'start'
}) {
  const [row, setRow] = useState<HTMLDivElement | null>(null)
  usePanelShiftMotion(row, panels, view, anchor)
  return (
    <div ref={(node) => { setRow(node); onRow(node) }}>
      <div className="chat-panel-shift-host">
        <div data-testid="view" />
      </div>
    </div>
  )
}

describe('usePanelShiftMotion', () => {
  afterEach(() => vi.restoreAllMocks())

  function setup(anchor?: 'center' | 'start') {
    let geometry: Geometry = { left: 0, width: 1000 }
    let rowEl: HTMLDivElement | null = null
    const animate = vi.fn(() => {
      const listeners: Record<string, () => void> = {}
      return {
        cancel: vi.fn(),
        addEventListener: (name: string, cb: () => void) => { listeners[name] = cb },
      } as unknown as Animation
    })
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
      return { left: geometry.left, width: geometry.width, top: 0, height: 500, right: geometry.left + geometry.width, bottom: 500, x: geometry.left, y: 0, toJSON() {} } as DOMRect
    })
    const view = render(<Row panels="10" view="conversation" anchor={anchor} onRow={(node) => { rowEl = node }} />)
    const target = view.getByTestId('view') as HTMLElement & { animate: typeof animate }
    target.animate = animate
    return {
      setGeometry: (next: Geometry) => { geometry = next },
      rerender: (panels: string, chatView = 'conversation') =>
        view.rerender(<Row panels={panels} view={chatView} anchor={anchor} onRow={(node) => { rowEl = node }} />),
      animate,
      host: () => rowEl!.querySelector('.chat-panel-shift-host')!,
    }
  }

  it('slides the main view from its old center when a panel opens', () => {
    const { setGeometry, rerender, animate, host } = setup()
    // Sidebar opens: the main view starts 240px further right and gets narrower.
    setGeometry({ left: 240, width: 760 })
    rerender('11')
    expect(animate).toHaveBeenCalledTimes(1)
    const [keyframes] = animate.mock.calls[0] as unknown as [Keyframe[]]
    // Old center 500, new center 620 → start 120px to the left.
    expect(keyframes[0].transform).toBe('translateX(-120px)')
    expect(keyframes[1].transform).toBe('translateX(0)')
    expect(host().classList.contains('is-panel-shifting')).toBe(true)
  })

  it('slides left-aligned titlebar controls by their left edge', () => {
    const { setGeometry, rerender, animate } = setup('start')
    // Sidebar opens: titlebar controls move from x=0 to x=240.
    setGeometry({ left: 240, width: 760 })
    rerender('11')
    const [keyframes] = animate.mock.calls[0] as unknown as [Keyframe[]]
    expect(keyframes[0].transform).toBe('translateX(-240px)')
  })

  it('does not animate when the view changes at the same time or nothing moved', () => {
    const { setGeometry, rerender, animate } = setup()
    setGeometry({ left: 0, width: 1000 })
    rerender('11', 'settings')
    expect(animate).not.toHaveBeenCalled()
    rerender('10', 'settings')
    expect(animate).not.toHaveBeenCalled()
  })
})
