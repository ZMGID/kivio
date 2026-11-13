import { render } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { ChatDotGridBackground } from './ChatDotGridBackground'
import { ChatRouteActiveContext } from './chatRouteActiveContext'

afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals() })

it('releases a hidden route canvas and restores it when returning', () => {
  const context = {
    clearRect: vi.fn(), setTransform: vi.fn(), beginPath: vi.fn(),
    moveTo: vi.fn(), arc: vi.fn(), fill: vi.fn(),
  }
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(() => context as unknown as CanvasRenderingContext2D)
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} })
  vi.stubGlobal('IntersectionObserver', class { observe() {} disconnect() {} })
  const frame = vi.spyOn(window, 'requestAnimationFrame').mockReturnValue(1)
  const cancel = vi.spyOn(window, 'cancelAnimationFrame')
  const view = (active: boolean) => <ChatRouteActiveContext.Provider value={active}><ChatDotGridBackground /></ChatRouteActiveContext.Provider>
  const { container, rerender, unmount } = render(view(true))
  const canvas = container.querySelector('canvas')!
  expect(canvas.width).toBeGreaterThan(0)
  rerender(view(false))
  expect(canvas.width).toBe(0)
  expect(canvas.height).toBe(0)
  if (frame.mock.calls.length) expect(cancel).toHaveBeenCalled()
  rerender(view(true))
  expect(canvas.width).toBeGreaterThan(0)
  unmount()
  expect(canvas.width).toBe(0)
})
