import { act, cleanup, render } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'

const native = vi.hoisted(() => ({ measure: vi.fn<(center: number) => Promise<number>>(async () => 30) }))
vi.mock('./platform', () => ({ isMac: true, isWindows: false, usesNativeTitlebar: true }))
vi.mock('./utils', () => ({ isTauriRuntime: () => true }))
vi.mock('../api/tauri', () => ({ api: {
  chatReportNotificationView: vi.fn(async () => {}),
  chatTrafficLightCenterY: native.measure,
} }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    isFullscreen: async () => false,
    onResized: async () => () => {},
    onFocusChanged: async () => () => {},
  }),
}))
import { ChatWindowHost } from './ChatWindowHost'

afterEach(() => { cleanup(); vi.useRealTimers(); document.documentElement.style.removeProperty('--chat-traffic-center-y') })

it('以页面按钮中心校准原生红绿灯，不把原生位置写回页面布局', async () => {
  vi.useFakeTimers()
  native.measure.mockResolvedValue(17)
  const view = render(<ChatWindowHost translucentSidebar={false}>
    <div className="chat-titlebar-row"><button>model</button></div>
  </ChatWindowHost>)
  const button = document.querySelector('button')!
  const bounds = vi.spyOn(button, 'getBoundingClientRect').mockReturnValue({ top: 15, height: 32, width: 32, right: 100 } as DOMRect)
  await act(async () => { window.dispatchEvent(new Event('focus')); await vi.advanceTimersByTimeAsync(300) })
  expect(native.measure).toHaveBeenLastCalledWith(31)
  expect(document.documentElement.style.getPropertyValue('--chat-traffic-center-y')).toBe('')
  bounds.mockReturnValue({ top: 20, height: 32, width: 32, right: 100 } as DOMRect)
  await act(async () => { window.dispatchEvent(new Event('blur')); await vi.advanceTimersByTimeAsync(300) })
  expect(native.measure).toHaveBeenLastCalledWith(36)
  await act(async () => { await vi.advanceTimersByTimeAsync(1500) })
  native.measure.mockClear()
  bounds.mockReturnValue({ top: 26, height: 32, width: 32, right: 100 } as DOMRect)
  await act(async () => { window.dispatchEvent(new Event('hashchange')); await vi.advanceTimersByTimeAsync(500) })
  expect(native.measure).toHaveBeenLastCalledWith(42)
  view.unmount()
  native.measure.mockClear()
  await act(async () => { await vi.advanceTimersByTimeAsync(1000) })
  expect(native.measure).not.toHaveBeenCalled()
})

it('标题栏在启动重试结束后出现或从设置返回时，仍会校准原生按钮', async () => {
  vi.useFakeTimers()
  native.measure.mockClear()
  native.measure.mockImplementation(async (center: number) => center)
  const view = render(<ChatWindowHost translucentSidebar={false}><div>settings</div></ChatWindowHost>)
  await act(async () => { await vi.advanceTimersByTimeAsync(1500) })
  expect(native.measure).not.toHaveBeenCalled()
  view.rerender(<ChatWindowHost translucentSidebar={false}>
    <div className="chat-titlebar-row"><button>model</button></div>
  </ChatWindowHost>)
  vi.spyOn(document.querySelector('button')!, 'getBoundingClientRect').mockReturnValue({ top: 10, height: 32, width: 32, right: 100 } as DOMRect)
  await act(async () => { await vi.advanceTimersByTimeAsync(500) })
  expect(native.measure).toHaveBeenLastCalledWith(26)
})
