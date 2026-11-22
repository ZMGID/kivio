import { fireEvent, render, screen, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { unified } from 'unified'

const openExternal = vi.fn(() => Promise.resolve())
const openLocalFile = vi.fn(() => Promise.resolve())
const kbOpenSource = vi.hoisted(() => vi.fn(() => Promise.resolve()))
vi.mock('./knowledgeBase', () => ({ kbOpenSource }))
vi.mock('../api/tauri', () => ({
  api: {
    get openExternal() {
      return openExternal
    },
    get openLocalFile() {
      return openLocalFile
    },
  },
  isTauriRuntime: () => true,
}))

import { ChatMarkdown } from './ChatMarkdown'
import { buildCitationMap } from './citations'

/**
 * 链接点击的**底线**：除了系统 scheme（mailto:/tel:/sms:），默认导航一律不许发生。
 * <a> 的默认行为会把 Tauri webview 自己导航走，整个聊天 UI 连同未落盘的会话状态一起消失
 * —— 实测点一条 CLI 生成的 `file://…/index.html` 就把窗口点白了。
 */
function clickLink(content: string) {
  const { container } = render(<ChatMarkdown content={content} artifacts={[]} />)
  const link = container.querySelector('a')!
  const event = new MouseEvent('click', { bubbles: true, cancelable: true })
  link.dispatchEvent(event)
  return event
}

describe('ChatMarkdown 链接点击', () => {
  beforeEach(() => {
    openExternal.mockClear()
    openLocalFile.mockClear()
  })

  it('opens the cited snapshot for the second knowledge search even when both searches return source one', () => {
    const first = 'kb:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-1'
    const second = 'kb:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-1'
    render(<ChatMarkdown
      content={`First [${first}], second [${second}], web [1].`}
      citations={buildCitationMap([
        { id: 'first', toolName: 'knowledge_search', status: 'success', structured_content: {
          hits: [{ n: 1, citation: first, kbId: 'kb_a', docId: 'doc_a', docName: 'A.md', score: 1, text: 'First excerpt' }],
        } },
        { id: 'second', toolName: 'knowledge_search', status: 'success', structured_content: {
          hits: [{ n: 1, citation: second, kbId: 'kb_b', docId: 'doc_b', docName: 'B.pdf', page: 7, score: 1, text: 'Second excerpt' }],
        } },
        { id: 'web', toolName: 'web_search', status: 'success', structured_content: {
          type: 'builtin_web_search', citations: [{ title: 'Web', url: 'https://example.com' }],
        } },
      ])}
    />)
    fireEvent.click(screen.getByRole('button', { name: /(?:Source|来源) 3/ }))
    const source = within(screen.getByRole('dialog'))
    expect(source.getByText('Second excerpt')).toBeTruthy()
    expect(source.getByText(/p\. 7/)).toBeTruthy()
    fireEvent.click(source.getByRole('button', { name: 'B.pdf' }))
    expect(kbOpenSource).toHaveBeenCalledWith('kb_b', 'doc_b')
  })

  it('本地文件链接交给 openLocalFile，且不导航 webview', () => {
    // Streamdown 默认会把 `file:` 剥成空 href（死链），应用层需要保留并交给后端。
    const event = clickLink('[看板](file:///tmp/board/index.html)')
    expect(event.defaultPrevented).toBe(true)
    expect(openLocalFile).toHaveBeenCalledWith('file:///tmp/board/index.html', null)
    expect(openExternal).not.toHaveBeenCalled()
  })

  it('裸绝对路径同样不导航（这条才是把窗口点白的那种链接）', () => {
    const event = clickLink('[看板](/tmp/board/index.html)')
    expect(event.defaultPrevented).toBe(true)
    expect(openLocalFile).toHaveBeenCalledWith('/tmp/board/index.html', null)
  })

  it('相对路径也交给后端（按会话工作目录解析），绝不放行默认导航', () => {
    const event = clickLink('[看板](assets/index.html)')
    expect(event.defaultPrevented).toBe(true)
    expect(openLocalFile).toHaveBeenCalledWith('assets/index.html', null)
    expect(openExternal).not.toHaveBeenCalled()
  })

  it('重开相同表格复用已清洗的解析结果，文件仍由当前对话打开', () => {
    const table = '| **文件** | 备注 |\n| --- | --- |\n| [记录](notes/session.md) | <script>unsafe()</script>已确认 [1] |\n| [附件](artifact:art_report) | 已交付 |'
    const parser = vi.spyOn(Object.getPrototypeOf(unified) as typeof unified, 'parse')
    try {
      const first = render(<ChatMarkdown content={table} conversationId="first" />)
      expect(first.container.querySelector('table [data-streamdown="strong"]')).toHaveTextContent('文件')
      expect(parser).toHaveBeenCalled()
      first.unmount()
      parser.mockClear()
      const second = render(<ChatMarkdown content={table} conversationId="second" />)
      expect(second.container.querySelector('table [data-streamdown="strong"]')).toHaveTextContent('文件')
      expect(second.container.querySelector('script')).toBeNull()
      fireEvent.click(screen.getByRole('link', { name: /记录/ }))
      expect(openLocalFile).toHaveBeenCalledWith('notes/session.md', 'second')
      expect(screen.getByRole('status')).toHaveTextContent('附件')
      second.rerender(<ChatMarkdown content={table} conversationId="second"
        artifacts={[{ id: 'art_report', name: 'report.pdf', path: '/B/report.pdf', mime_type: 'application/pdf' }]}
        citations={new Map([[1, { kind: 'web', n: 1, title: 'B 来源', url: 'https://example.com/b', host: 'example.com' }]])}
      />)
      expect(screen.getByRole('button', { name: '打开文件 report.pdf' })).toBeVisible()
      fireEvent.click(screen.getByRole('button', { name: '来源 1' }))
      expect(within(screen.getByRole('dialog')).getByText('B 来源')).toBeVisible()
      expect(parser).not.toHaveBeenCalled()
    } finally {
      parser.mockRestore()
    }
  })

  it('不止 html：docx / zip 这类同样走默认程序', () => {
    clickLink('[报告](/tmp/out/report.docx)')
    expect(openLocalFile).toHaveBeenCalledWith('/tmp/out/report.docx', null)
  })

  it('http(s) 走系统浏览器', () => {
    const event = clickLink('[官网](https://example.com/a)')
    expect(event.defaultPrevented).toBe(true)
    expect(openExternal).toHaveBeenCalledWith('https://example.com/a')
  })

  it('mailto: 保留默认行为交给系统', () => {
    const event = clickLink('[联系](mailto:a@b.com)')
    expect(event.defaultPrevented).toBe(false)
    expect(openExternal).not.toHaveBeenCalled()
    expect(openLocalFile).not.toHaveBeenCalled()
  })
})

describe('ChatMarkdown 链接图标', () => {
  function linkOf(content: string) {
    const { container } = render(<ChatMarkdown content={content} artifacts={[]} />)
    return container.querySelector('a')!
  }

  it('已收录站点（GitHub）在链接前显示品牌图标', () => {
    const link = linkOf('[仓库](https://github.com/a/b)')
    const icon = link.querySelector('svg')!
    expect(icon.getAttribute('aria-hidden')).toBe('true')
    icon.remove()
    expect(link.textContent).toBe('\u2060仓库')
  })

  it('子域名同样命中', () => {
    expect(linkOf('[页面](https://docs.github.com/x)').querySelector('svg')).not.toBeNull()
  })

  it('未收录站点保持纯文字链接', () => {
    expect(linkOf('[说明](https://example.com/a)').querySelector('svg')).toBeNull()
  })

  it('仿冒域名不命中', () => {
    expect(linkOf('[x](https://github.com.evil.io/a)').querySelector('svg')).toBeNull()
  })

  it('本地文件链接显示文件类型图标', () => {
    const link = linkOf('[planning_recovery.rs](/repo/tests/planning_recovery.rs:12)')
    const icon = link.querySelector('svg')!
    expect(icon.getAttribute('aria-hidden')).toBe('true')
    expect(link.textContent).toBe('\u2060planning_recovery.rs')
  })

  it('页内锚点和 mailto 不加图标', () => {
    expect(linkOf('[跳转](#intro)').querySelector('svg')).toBeNull()
    expect(linkOf('[联系](mailto:a@b.com)').querySelector('svg')).toBeNull()
  })
})
