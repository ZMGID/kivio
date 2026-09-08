import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { observeNotificationView } from './notificationView'

describe('notification viewing state', () => {
  let stop: (() => void) | undefined
  const report = vi.fn<(route: string, viewing: boolean) => Promise<void>>()
  const route = (hash: string) => window.history.replaceState(null, '', hash)

  beforeEach(() => {
    report.mockReset().mockResolvedValue(undefined)
    vi.spyOn(document, 'hasFocus').mockReturnValue(true)
    vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('visible')
    route('#chat/conv_one')
  })

  afterEach(() => {
    stop?.()
    stop = undefined
    vi.restoreAllMocks()
  })

  it('reports initial viewing state, conversation switches and settings routes', () => {
    stop = observeNotificationView(report)
    expect(report).toHaveBeenLastCalledWith('#chat/conv_one', true)
    route('#chat/conv_two?mode=chat')
    window.dispatchEvent(new Event('hashchange'))
    expect(report).toHaveBeenLastCalledWith('#chat/conv_two?mode=chat', true)
    route('#chat/settings')
    window.dispatchEvent(new Event('hashchange'))
    expect(report).toHaveBeenLastCalledWith('#chat/settings', true)
  })

  it('clears on blur, minimize/hide and restores on focus or visibility', () => {
    stop = observeNotificationView(report)
    window.dispatchEvent(new Event('blur'))
    expect(report).toHaveBeenLastCalledWith('#chat/conv_one', false)
    window.dispatchEvent(new Event('focus'))
    expect(report).toHaveBeenLastCalledWith('#chat/conv_one', true)
    vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden')
    document.dispatchEvent(new Event('visibilitychange'))
    expect(report).toHaveBeenLastCalledWith('#chat/conv_one', false)
    vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('visible')
    document.dispatchEvent(new Event('visibilitychange'))
    expect(report).toHaveBeenLastCalledWith('#chat/conv_one', true)
  })

  it('an unfocused popout does not claim it is being viewed', () => {
    route('#chat/popout/conv_pop')
    vi.spyOn(document, 'hasFocus').mockReturnValue(false)
    stop = observeNotificationView(report)
    expect(report).toHaveBeenLastCalledWith('#chat/popout/conv_pop', false)
  })

  it('does not wait for a stalled IPC and clears on pagehide and unmount', () => {
    report.mockImplementation(() => new Promise(() => {}))
    stop = observeNotificationView(report)
    window.dispatchEvent(new Event('pagehide'))
    expect(report).toHaveBeenLastCalledWith('#chat/conv_one', false)
    stop()
    stop = undefined
    report.mockClear()
    window.dispatchEvent(new Event('focus'))
    window.dispatchEvent(new Event('hashchange'))
    document.dispatchEvent(new Event('visibilitychange'))
    expect(report).not.toHaveBeenCalled()
  })

  it('handles rejected IPC without an unhandled rejection', async () => {
    report.mockRejectedValue(new Error('window closed'))
    stop = observeNotificationView(report)
    await Promise.resolve()
    expect(report).toHaveBeenCalledOnce()
  })
})
