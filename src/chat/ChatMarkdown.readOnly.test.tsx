import { render } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ChatMarkdown } from './ChatMarkdown'
import { MarkdownStreamingContext } from './markdownStreaming'
import { api } from '../api/tauri'

vi.mock('../api/tauri', async importOriginal => ({
  ...await importOriginal<typeof import('../api/tauri')>(),
  api: { openExternal: vi.fn(), openLocalFile: vi.fn(), openHtmlPreview: vi.fn() },
}))

describe('read-only Markdown for private study material', () => {
  it('preserves math, emphasis, tables, and code as inert content', () => {
    const { container } = render(<ChatMarkdown readOnly content={'**One step:** solve $x^2=4$.\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n```python\nprint(2)\n```'} />)
    expect(container.querySelector('[data-streamdown="strong"]')?.textContent).toBe('One step:')
    expect(container.querySelector('.katex-mathml annotation')?.textContent).toBe('x^2=4')
    expect(container.querySelector('table')).not.toBeNull()
    expect(container.querySelector('pre')?.textContent).toContain('print(2)')
    expect(container.querySelector('button')).toBeNull()
  })

  it('never embeds remote, local, data, or artifact images or clickable file links', () => {
    const content = [
      '![remote](https://example.invalid/track?source=private)',
      '![local](file:///private/notes.png)',
      '![embedded](data:image/svg+xml;base64,PHN2Zz4=)',
      '![artifact](artifact:private-image)',
      '[open file](file:///private/notes.txt)',
      '[open remote](https://example.invalid/track)',
      '[artifact file](artifact:private-file)',
    ].join('\n\n')
    const { container } = render(<ChatMarkdown readOnly content={content} />)
    expect(container.querySelector('img, iframe, a, button')).toBeNull()
    expect(container.textContent).toContain('remote')
    expect(container.textContent).toContain('open file')
    expect(api.openLocalFile).not.toHaveBeenCalled()
    expect(api.openExternal).not.toHaveBeenCalled()
  })

  it('escapes raw HTML and never runs HTML/SVG/Mermaid/CLI previews', () => {
    const content = [
      '<img src="https://example.invalid/private"><iframe src="https://example.invalid"></iframe>',
      '```html\n<script>fetch("https://example.invalid")</script>\n```',
      '```svg\n<svg><image href="https://example.invalid/image" /></svg>\n```',
      '```mermaid\nflowchart LR\n A-->B\n```',
      '```kivio-cli-report\n{"command":"touch /private/file"}\n```',
      '$\\includegraphics{https://example.invalid/tex-image}$',
      '$\\href{https://example.invalid/tex-link}{click}$',
    ].join('\n\n')
    const { container } = render(<ChatMarkdown readOnly content={content} />)
    expect(container.querySelector('img, iframe, a, button, script, object, video, audio')).toBeNull()
    expect(container.querySelectorAll('pre')).toHaveLength(4)
    expect(container.textContent).toContain('fetch(')
    expect(api.openHtmlPreview).not.toHaveBeenCalled()
  })

  it('removes existing interactive nodes when switching to read-only with unchanged content', () => {
    const content = '![image](https://example.invalid/image.png)\n\n```html\n<p>preview</p>\n```'
    const { container, rerender } = render(<ChatMarkdown content={content} />)
    expect(container.querySelector('img')).not.toBeNull()
    expect(container.querySelector('iframe')).not.toBeNull()
    rerender(<ChatMarkdown readOnly content={content} />)
    expect(container.querySelector('img, iframe, a, button')).toBeNull()
  })

  it('keeps streamed HTML and links inert as partial syntax becomes complete', () => {
    const renderContent = (content: string) => <MarkdownStreamingContext.Provider value={true}><ChatMarkdown readOnly content={content} /></MarkdownStreamingContext.Provider>
    const { container, rerender } = render(renderContent('![source](https://example.invalid/private'))
    rerender(renderContent('![source](https://example.invalid/private)\n\n```html\n<img src="https://example.invalid">\n```'))
    expect(container.querySelector('img, iframe, a, button')).toBeNull()
  })
})
