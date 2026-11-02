import { act, fireEvent, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ChatMarkdown } from './ChatMarkdown'
import { MarkdownStreamingContext } from './markdownStreaming'
import { api } from '../api/tauri'

vi.mock('../api/tauri', () => ({ api: {
  createHtmlPreview: vi.fn(async () => ({ id: 'preview-id', url: 'kivio-preview://localhost/preview-id' })),
  releaseHtmlPreview: vi.fn(async () => {}),
  openHtmlPreview: vi.fn(async () => {}),
} }))

// 回归测试：流式生成中的 HTML 代码块不能挂 iframe —— srcDoc 每个 delta 换一次就是整篇重载，
// 页面闪 + 高度重测把聊天列表拽回底部。只有完成态才切回完整 Markdown/iframe。
const block = (body: string) => '```html\n<html><body>' + body + '\n'

const streaming = (content: string) => (
  <MarkdownStreamingContext.Provider value={true}>
    <ChatMarkdown content={content} />
  </MarkdownStreamingContext.Provider>
)

describe('HTML 预览挂载时机', () => {
  beforeEach(() => vi.useFakeTimers())
  afterEach(() => vi.useRealTimers())

  it('生成中只显示一行占位，完成后才挂 iframe', async () => {
    const { container, rerender, getByRole } = render(streaming(block('<p>a</p>')))
    expect(container.querySelector('iframe')).toBeNull()
    expect(container.querySelector('pre')).toBeNull()
    expect(getByRole('status')).toHaveTextContent('正在生成页面')

    rerender(streaming(block('<p>ab</p>')))
    act(() => void vi.advanceTimersByTime(400))
    rerender(streaming(block('<p>abc</p>')))
    act(() => void vi.advanceTimersByTime(400))
    expect(container.querySelector('iframe')).toBeNull() // 还没静默

    act(() => void vi.advanceTimersByTime(1_200))
    expect(container.querySelector('iframe')).toBeNull()

    rerender(<ChatMarkdown content={block('<p>abc</p>')} />)
    await act(async () => { await Promise.resolve() })
    expect(container.querySelector('iframe')?.getAttribute('src')).toMatch(/^kivio-preview:\/\/localhost\/preview-id#kivio-theme=/)
  })

  it('历史消息准备完成后挂独立源 iframe 并在卸载时释放', async () => {
    const { container, unmount } = render(<ChatMarkdown content={block('<p>done</p>')} />)
    await act(async () => { await Promise.resolve() })
    const iframe = container.querySelector('iframe')
    expect(iframe?.getAttribute('src')).toMatch(/^kivio-preview:\/\/localhost\/preview-id#kivio-theme=/)
    expect(iframe?.hasAttribute('srcdoc')).toBe(false)
    expect(iframe).toHaveAttribute('sandbox', 'allow-scripts')
    expect(iframe?.getAttribute('sandbox')).not.toContain('allow-same-origin')
    expect(iframe).toHaveAttribute('referrerpolicy', 'no-referrer')
    unmount()
    expect(api.releaseHtmlPreview).toHaveBeenCalledWith('preview-id')
  })

  it('creates a separate fullscreen preview and preserves the inline page when it fails', async () => {
    HTMLDialogElement.prototype.showModal = function () { this.setAttribute('open', '') }
    HTMLDialogElement.prototype.close = function () { this.removeAttribute('open'); this.dispatchEvent(new Event('close')) }
    vi.mocked(api.createHtmlPreview).mockClear()
      .mockResolvedValueOnce({ id: 'inline-id', url: 'kivio-preview://localhost/inline-id' })
      .mockResolvedValueOnce({ id: 'fullscreen-id', url: 'kivio-preview://localhost/fullscreen-id' })
    const { container, getByRole } = render(<ChatMarkdown content={block('<p>full</p>')} />)
    await act(async () => { await Promise.resolve() })
    const inline = container.querySelector('iframe')!
    fireEvent.click(getByRole('button', { name: '全屏查看' }))
    await act(async () => { await Promise.resolve() })
    const fullscreen = container.querySelectorAll('iframe')[1]
    expect(fullscreen.getAttribute('src')).toMatch(/^kivio-preview:\/\/localhost\/fullscreen-id#kivio-theme=/)
    act(() => { window.dispatchEvent(new MessageEvent('message', {
      data: { kivioHtmlPreview: 'error' }, source: fullscreen.contentWindow,
    })) })
    expect(container.querySelector('iframe')).toBe(inline)
    expect(container.querySelector('pre code')).toBeNull()
    expect(getByRole('status')).toHaveTextContent('全屏页面不可用')
    fireEvent.click(getByRole('button', { name: '关闭' }))
    expect(api.releaseHtmlPreview).toHaveBeenCalledWith('fullscreen-id')
  })

  it('switches between the rendered page and its source', async () => {
    const { container, getByRole } = render(<ChatMarkdown content={block('<p>source</p>')} />)
    await act(async () => { await Promise.resolve() })
    fireEvent.click(getByRole('button', { name: '查看源码' }))
    expect(container.querySelector('iframe')).toBeNull()
    expect(container.querySelector('pre code')).toHaveTextContent('<p>source</p>')
    fireEvent.click(getByRole('button', { name: '查看预览' }))
    await act(async () => { await Promise.resolve() })
    expect(container.querySelector('iframe')).not.toBeNull()
  })

  it('按页面上报高度撑开 iframe，并封顶；其他窗口的消息不生效', async () => {
    const { container } = render(<ChatMarkdown content={block('<p>tall</p>')} />)
    await act(async () => { await Promise.resolve() })
    const iframe = container.querySelector('iframe')!
    const post = (value: number, source: MessageEventSource | null) => act(() => {
      window.dispatchEvent(new MessageEvent('message', { data: { kivioHtmlPreview: 'height', value }, source }))
    })
    post(300, iframe.contentWindow)
    expect(iframe.style.height).toBe('300px')
    post(99_999, iframe.contentWindow)
    expect(iframe.style.height).toBe('2000px')
    post(50, window)
    expect(iframe.style.height).toBe('2000px')
  })

  it('首个高度到达前保持 pending，到达后清除', async () => {
    const { container } = render(<ChatMarkdown content={block('<p>p</p>')} />)
    await act(async () => { await Promise.resolve() })
    const iframe = container.querySelector('iframe')!
    expect(container.querySelector('[data-chat-async-pending="true"]')).not.toBeNull()
    act(() => {
      window.dispatchEvent(new MessageEvent('message', { data: { kivioHtmlPreview: 'height', value: 200 }, source: iframe.contentWindow }))
    })
    expect(container.querySelector('[data-chat-async-pending="true"]')).toBeNull()
  })

  it('预览创建失败时退回源码，不留空白框', async () => {
    vi.mocked(api.createHtmlPreview).mockRejectedValueOnce(new Error('Too many HTML previews are open'))
    const { container } = render(<ChatMarkdown content={block('<p>fail</p>')} />)
    await act(async () => { await Promise.resolve(); await Promise.resolve() })
    expect(container.querySelector('iframe')).toBeNull()
    expect(container.querySelector('pre code')?.textContent).toContain('<p>fail</p>')
    expect(container.querySelector('[data-chat-async-pending="true"]')).toBeNull()
  })

  it('passes original HTML to the backend without injecting frontend bootstrap code', async () => {
    const create = vi.mocked(api.createHtmlPreview)
    create.mockClear()
    const page = '<!doctype html><!-- example: <head> --><html><head><title>t</title></head><body><header>h</header></body></html>'
    render(<ChatMarkdown content={'```html\n' + page + '\n```'} />)
    await act(async () => { await Promise.resolve() })
    expect(create.mock.calls[0][0].trim()).toBe(page)
  })
})
