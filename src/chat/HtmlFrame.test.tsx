import { act, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { HtmlFrame } from './HtmlFrame'
import { api } from '../api/tauri'

vi.mock('../api/tauri', () => ({ api: {
  openExternal: vi.fn(async () => {}),
  createHtmlPreview: vi.fn(async () => ({ id: 'dialog-id', url: 'kivio-preview://localhost/dialog-id' })),
  releaseHtmlPreview: vi.fn(async () => {}),
} }))

const src = 'kivio-preview://localhost/render/conv_1/htmlrender-12345678-1234-1234-1234-123456789abc'
const openBrowser = vi.fn()
const activation = { isActive: true }
const originalActivation = Object.getOwnPropertyDescriptor(navigator, 'userActivation')

function frameTheme(iframe: HTMLIFrameElement): string {
  const encoded = iframe.getAttribute('src')!.split('#kivio-theme=')[1]
  const base64 = encoded.replace(/-/g, '+').replace(/_/g, '/')
  return new TextDecoder().decode(Uint8Array.from(atob(base64.padEnd(Math.ceil(base64.length / 4) * 4, '=')), char => char.charCodeAt(0)))
}

function send(source: MessageEventSource | null, type: string, value: unknown) {
  act(() => { window.dispatchEvent(new MessageEvent('message', { source, data: { kivioHtmlPreview: type, value } })) })
}

describe('HtmlFrame', () => {
  beforeEach(() => {
    activation.isActive = true
    Object.defineProperty(navigator, 'userActivation', { configurable: true, value: activation })
    HTMLDialogElement.prototype.showModal = function () { this.setAttribute('open', '') }
    HTMLDialogElement.prototype.close = function () { this.removeAttribute('open'); this.dispatchEvent(new Event('close')) }
  })
  afterEach(() => {
    document.documentElement.classList.remove('dark')
    document.documentElement.style.removeProperty('--font-sans')
    if (originalActivation) Object.defineProperty(navigator, 'userActivation', originalActivation)
    else Reflect.deleteProperty(navigator, 'userActivation')
    document.documentElement.style.removeProperty('--text')
    vi.clearAllMocks()
  })

  it('keeps the initial height pending until an inline height arrives and clamps measurements', () => {
    const { container } = render(<HtmlFrame src={src} title="Chart" initialHeight={480} onOpenInBrowser={openBrowser} />)
    const iframe = screen.getByTitle('Chart') as HTMLIFrameElement
    expect(iframe).toHaveAttribute('sandbox', 'allow-scripts')
    expect(iframe).toHaveAttribute('referrerpolicy', 'no-referrer')
    expect(iframe.style.height).toBe('480px')
    expect(container.querySelector('[data-chat-async-pending="true"]')).not.toBeNull()
    send(window, 'height', 400)
    send(iframe.contentWindow, 'height', NaN)
    expect(iframe.style.height).toBe('480px')
    expect(container.querySelector('[data-chat-async-pending="true"]')).not.toBeNull()
    send(iframe.contentWindow, 'height', 10)
    expect(iframe.style.height).toBe('80px')
    expect(container.querySelector('[data-chat-async-pending="true"]')).toBeNull()
    send(iframe.contentWindow, 'height', 5000)
    expect(iframe.style.height).toBe('2000px')
  })

  it('accepts links only from the inline or fullscreen frame, never resizes from fullscreen', () => {
    render(<HtmlFrame src={src} title="Chart" initialHeight={480} onOpenInBrowser={openBrowser} />)
    const inline = screen.getByTitle('Chart') as HTMLIFrameElement
    fireEvent.click(screen.getByRole('button', { name: '全屏查看' }))
    const frames = screen.getAllByTitle('Chart') as HTMLIFrameElement[]
    const fullscreenWindow = frames[1].contentWindow
    send(window, 'open', 'https://outside.test')
    send(inline.contentWindow, 'open', 'file:///private')
    send(fullscreenWindow, 'open', 'https://example.com')
    expect(api.openExternal).toHaveBeenCalledTimes(1)
    expect(api.openExternal).toHaveBeenCalledWith('https://example.com')
    send(fullscreenWindow, 'height', 900)
    expect(inline.style.height).toBe('480px')
    fireEvent.click(screen.getByRole('button', { name: '关闭' }))
    send(fullscreenWindow, 'open', 'https://after-close.test')
    expect(api.openExternal).toHaveBeenCalledTimes(1)
  })

  it('rejects unsolicited links unless a real user activation is active', () => {
    render(<HtmlFrame src={src} onOpenInBrowser={openBrowser} />)
    const inline = screen.getByTitle('HTML 预览') as HTMLIFrameElement
    activation.isActive = false
    send(inline.contentWindow, 'open', 'https://example.com')
    expect(api.openExternal).not.toHaveBeenCalled()
    Object.defineProperty(navigator, 'userActivation', { configurable: true, value: undefined })
    send(inline.contentWindow, 'open', 'https://example.com')
    expect(api.openExternal).not.toHaveBeenCalled()
    Object.defineProperty(navigator, 'userActivation', { configurable: true, value: activation })
    activation.isActive = true
    send(inline.contentWindow, 'open', 'https://example.com')
    expect(api.openExternal).toHaveBeenCalledWith('https://example.com')
  })

  it('supplies UTF-8 theme fragments on both frames without reloading on a theme change', async () => {
    document.documentElement.style.setProperty('--font-sans', '"示例字体", sans-serif')
    render(<HtmlFrame src={src} title="Chart" onOpenInBrowser={openBrowser} />)
    const inline = screen.getByTitle('Chart') as HTMLIFrameElement
    const originalSrc = inline.getAttribute('src')
    expect(originalSrc).toMatch(/^kivio-preview:\/\/localhost\/render\/.*#kivio-theme=[A-Za-z0-9_-]+$/)
    expect(frameTheme(inline)).toContain('--font-sans:"示例字体", sans-serif')
    await act(async () => {
      document.documentElement.style.setProperty('--text', '#123456')
      document.documentElement.classList.add('dark')
      await Promise.resolve()
    })
    expect(inline.getAttribute('src')).toBe(originalSrc)
    fireEvent.click(screen.getByRole('button', { name: '全屏查看' }))
    const fullscreen = screen.getAllByTitle('Chart')[1] as HTMLIFrameElement
    expect(frameTheme(fullscreen)).toContain('--foreground:#123456')
    expect(frameTheme(fullscreen)).toContain('color-scheme:dark')
  })

  it('pushes T3 theme variables on load and theme changes to both frames', async () => {
    render(<HtmlFrame src={src} title="Chart" onOpenInBrowser={openBrowser} />)
    const inline = screen.getByTitle('Chart') as HTMLIFrameElement
    const inlinePost = vi.spyOn(inline.contentWindow!, 'postMessage')
    fireEvent.load(inline)
    const css = inlinePost.mock.calls.at(-1)?.[0].value as string
    for (const name of ['background', 'foreground', 'muted', 'muted-foreground', 'card', 'card-foreground',
      'popover', 'popover-foreground', 'secondary', 'secondary-foreground', 'border', 'input', 'ring', 'primary',
      'primary-foreground', 'accent', 'accent-foreground', 'destructive', 'destructive-foreground', 'warning',
      'warning-foreground', 'success', 'success-foreground', 'info', 'info-foreground', 'code-background',
      'code-foreground', 'chart-1', 'chart-2', 'chart-3', 'chart-4', 'chart-5', 'chart-6', 'radius', 'font-sans', 'font-mono']) {
      expect(css).toContain(`--${name}:`)
    }
    expect(css).not.toContain('--kv-')
    fireEvent.click(screen.getByRole('button', { name: '全屏查看' }))
    const dialogFrame = screen.getAllByTitle('Chart')[1] as HTMLIFrameElement
    const dialogPost = vi.spyOn(dialogFrame.contentWindow!, 'postMessage')
    fireEvent.load(dialogFrame)
    expect(dialogPost).toHaveBeenCalledWith(expect.objectContaining({ kivioHtmlPreview: 'theme' }), '*')
    await act(async () => {
      document.documentElement.style.setProperty('--text', '#123456')
      document.documentElement.classList.add('dark')
      await Promise.resolve()
    })
    expect(inlinePost.mock.calls.at(-1)?.[0].value).toContain('--foreground:#123456')
    expect(dialogPost.mock.calls.at(-1)?.[0].value).toContain('color-scheme:dark')
  })

  it('shows the fallback when the isolated page reports a load error', () => {
    const { container } = render(<HtmlFrame src={src} onOpenInBrowser={openBrowser} fallback={<span>Unavailable</span>} />)
    const iframe = screen.getByTitle('HTML 预览') as HTMLIFrameElement
    send(window, 'error', null)
    expect(screen.queryByText('Unavailable')).toBeNull()
    send(iframe.contentWindow, 'error', null)
    expect(screen.getByText('Unavailable')).toBeVisible()
    expect(container.querySelector('[data-chat-async-pending="true"]')).toBeNull()
  })

  it('creates and releases a fresh fullscreen preview, isolating dialog load failures', async () => {
    render(<HtmlFrame src={src} title="Chart" fullscreenHtml="<p>preview</p>" onOpenInBrowser={openBrowser}
      fallback={<span>Inline unavailable</span>} />)
    const inline = screen.getByTitle('Chart') as HTMLIFrameElement
    fireEvent.click(screen.getByRole('button', { name: '全屏查看' }))
    await act(async () => { await Promise.resolve() })
    expect(api.createHtmlPreview).toHaveBeenCalledWith('<p>preview</p>')
    const dialogFrame = screen.getAllByTitle('Chart')[1] as HTMLIFrameElement
    expect(dialogFrame.getAttribute('src')).toMatch(/^kivio-preview:\/\/localhost\/dialog-id#kivio-theme=/)
    expect(frameTheme(dialogFrame)).toContain('--chart-1:')
    send(dialogFrame.contentWindow, 'error', null)
    expect(screen.getByRole('status')).toHaveTextContent('全屏页面不可用')
    expect(screen.queryByText('Inline unavailable')).toBeNull()
    expect(screen.getByTitle('Chart')).toBe(inline)
    fireEvent.click(screen.getByRole('button', { name: '关闭' }))
    expect(api.releaseHtmlPreview).toHaveBeenCalledWith('dialog-id')
    expect(screen.getByTitle('Chart')).toBe(inline)
  })

  it('releases a fullscreen preview that finishes after its dialog has closed', async () => {
    let resolve!: (value: { id: string; url: string }) => void
    vi.mocked(api.createHtmlPreview).mockReturnValueOnce(new Promise((done) => { resolve = done }))
    render(<HtmlFrame src={src} fullscreenHtml="<p>preview</p>" onOpenInBrowser={openBrowser} />)
    fireEvent.click(screen.getByRole('button', { name: '全屏查看' }))
    expect(screen.getByRole('status')).toHaveTextContent('正在加载页面')
    fireEvent.click(screen.getByRole('button', { name: '关闭' }))
    await act(async () => { resolve({ id: 'late-id', url: 'kivio-preview://localhost/late-id' }); await Promise.resolve() })
    expect(api.releaseHtmlPreview).toHaveBeenCalledWith('late-id')
    expect(screen.getAllByTitle('HTML 预览')).toHaveLength(1)
  })

  it('keeps the inline frame when fullscreen preview creation fails', async () => {
    vi.mocked(api.createHtmlPreview).mockRejectedValueOnce(new Error('preview unavailable'))
    render(<HtmlFrame src={src} fullscreenHtml="<p>preview</p>" onOpenInBrowser={openBrowser}
      fallback={<span>Inline unavailable</span>} />)
    const inline = screen.getByTitle('HTML 预览')
    fireEvent.click(screen.getByRole('button', { name: '全屏查看' }))
    await act(async () => { await Promise.resolve(); await Promise.resolve() })
    expect(screen.getByRole('status')).toHaveTextContent('全屏页面不可用')
    expect(screen.queryByText('Inline unavailable')).toBeNull()
    expect(screen.getByTitle('HTML 预览')).toBe(inline)
  })

  it('exposes optional source and browser actions and renders the supplied failure fallback', () => {
    const source = vi.fn()
    const { rerender } = render(<HtmlFrame src={src} onOpenInBrowser={openBrowser} onSource={source} fallback={<span>Unavailable</span>} />)
    fireEvent.click(screen.getByRole('button', { name: '查看源码' }))
    fireEvent.click(screen.getByRole('button', { name: '在浏览器打开' }))
    expect(source).toHaveBeenCalledOnce()
    expect(openBrowser).toHaveBeenCalledOnce()
    rerender(<HtmlFrame src={src} onOpenInBrowser={openBrowser} failed fallback={<span>Unavailable</span>} />)
    expect(screen.getByText('Unavailable')).toBeVisible()
    expect(screen.queryByTitle('HTML 预览')).toBeNull()
  })
})
