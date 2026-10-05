import { act, cleanup, render } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'

const native = vi.hoisted(() => ({ measure: vi.fn(async () => 30) }))
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
  view.unmount()
  native.measure.mockClear()
  await act(async () => { await vi.advanceTimersByTimeAsync(1000) })
  expect(native.measure).not.toHaveBeenCalled()
})
