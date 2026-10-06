import { render } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { ChatMarkdown } from './ChatMarkdown'
import { MarkdownStreamingContext } from './markdownStreaming'

function renderMarkdown(content: string, streaming = false) {
  return render(
    <MarkdownStreamingContext.Provider value={streaming}>
      <ChatMarkdown content={content} />
    </MarkdownStreamingContext.Provider>,
  )
}

describe('助手 Markdown 保留普通下划线', () => {
  it('普通单下划线、多个下划线、文件名和标识符按原文字显示', () => {
    const content = '___ _name_ file_name __value__'
    const { container } = renderMarkdown(content)
    expect(container.querySelector('hr')).toBeNull()
    expect(container.querySelector('em')).toBeNull()
    expect(container.querySelector('[data-streamdown="strong"]')).toBeNull()
    expect(container.textContent).toContain(content)
  })

  it('单独成行的下划线不是分隔线', () => {
    const { container } = renderMarkdown('___\n\n_\n\n__init__')
    expect(container.querySelector('hr')).toBeNull()
    expect(container.textContent).toContain('___')
    expect(container.textContent).toContain('__init__')
  })

  it('星号强调和横线分隔线仍然生效', () => {
    const { container } = renderMarkdown('*斜体* **加粗**\n\n---\n\n***')
    expect(container.querySelector('em')?.textContent).toBe('斜体')
    expect(container.querySelector('[data-streamdown="strong"]')?.textContent).toBe('加粗')
    expect(container.querySelectorAll('hr')).toHaveLength(2)
    expect(container.textContent).not.toContain('*斜体*')
  })

  it('下划线包住的星号强调仍保留星号格式', () => {
    const { container } = renderMarkdown('_**name**_')
    expect(container.querySelector('[data-streamdown="strong"]')?.textContent).toBe('name')
    expect(container.textContent).toContain('_')
    expect(container.textContent).toContain('name')
  })

  it('代码、链接和公式里的下划线不被改写', () => {
    const { container } = renderMarkdown('`file_name` [a_b](https://example.com/my_file) $Z_1$')
    expect(container.querySelector('code')?.textContent).toBe('file_name')
    expect(container.querySelector('a')?.getAttribute('href')).toBe('https://example.com/my_file')
    expect(container.querySelector('a')?.textContent).toBe('a_b')
    expect(container.querySelector('.katex-mathml annotation')?.textContent).toBe('Z_1')
  })

  it('流式补全不会给未写完的标识符多补一个下划线', () => {
    const { container } = renderMarkdown('see _name and __init', true)
    expect(container.textContent).toContain('see _name and __init')
    expect(container.textContent).not.toContain('_name_')
    expect(container.textContent).not.toContain('__init__')
  })
})
