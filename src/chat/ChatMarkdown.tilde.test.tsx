import { act, render } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { ChatMarkdown } from './ChatMarkdown'
import { MarkdownStreamingContext } from './markdownStreaming'

const rangeText = '夜间停暖7~8小时后，室内温度从20°C跌至12~14°C。第二天连续满负荷燃烧40~60分钟，约消耗2~3小时的用气量。'

describe('ChatMarkdown 波浪线', () => {
  it.each([false, true])('范围中的单波浪线保留原文（streaming=%s）', (streaming) => {
    const { container } = render(
      <MarkdownStreamingContext.Provider value={streaming}>
        <ChatMarkdown content={rangeText} />
      </MarkdownStreamingContext.Provider>,
    )
    expect(container.querySelector('del')).toBeNull()
    expect(container.textContent).toBe(rangeText)
  })

  it.each([false, true])('普通符号混排不会变成强调、删除线、表格或公式（streaming=%s）', (streaming) => {
    const content = '7~8小时；file_name、__init__；价格 $10 和 $20；2 < 3，5 > 4；A | B；C++。'
    const { container } = render(
      <MarkdownStreamingContext.Provider value={streaming}>
        <ChatMarkdown content={content} />
      </MarkdownStreamingContext.Provider>,
    )
    expect(container.textContent).toBe(content)
    expect(container.querySelector('del, em, table, hr, .katex, [data-streamdown="strong"]')).toBeNull()
  })

  it('流式追加、结束和重新打开都不把范围变成删除线', async () => {
    const view = (content: string, streaming: boolean) => (
      <MarkdownStreamingContext.Provider value={streaming}>
        <ChatMarkdown content={content} />
      </MarkdownStreamingContext.Provider>
    )
    const { container, rerender, unmount } = render(view('', true))
    for (let end = 1; end <= rangeText.length; end += 1) {
      rerender(view(rangeText.slice(0, end), true))
      await act(async () => { await Promise.resolve() })
      expect(container.querySelector('del')).toBeNull()
      expect(container.textContent).toBe(rangeText.slice(0, end))
    }
    rerender(view(rangeText, false))
    await act(async () => { await Promise.resolve() })
    expect(container.querySelector('del')).toBeNull()
    expect(container.textContent).toBe(rangeText)
    unmount()
    const reopened = render(view(rangeText, false))
    expect(reopened.container.querySelector('del')).toBeNull()
    expect(reopened.container.textContent).toBe(rangeText)
  })

  it.each([false, true])('只有双波浪线表示删除线，代码和链接仍保留波浪线（streaming=%s）', (streaming) => {
    const { container } = render(
      <MarkdownStreamingContext.Provider value={streaming}>
        <ChatMarkdown content={'~普通文本~，~~删除内容。~~继续，`12~14°C`，[范围~说明](https://example.com/~user)'} />
      </MarkdownStreamingContext.Provider>,
    )
    expect(container.querySelectorAll('del')).toHaveLength(1)
    expect(container.querySelector('del')?.textContent).toBe('删除内容。')
    expect(container.textContent).toContain('~普通文本~')
    expect(container.querySelector('code')?.textContent).toBe('12~14°C')
    expect(container.querySelector('a')?.getAttribute('href')).toBe('https://example.com/~user')
    expect(container.querySelector('a')?.textContent).toBe('范围~说明')
  })

  it.each([false, true])('不改写链接地址、公式和波浪线围栏代码（streaming=%s）', async (streaming) => {
    const view = (content: string) => (
      <MarkdownStreamingContext.Provider value={streaming}>
        <ChatMarkdown content={content} />
      </MarkdownStreamingContext.Provider>
    )
    const { container, rerender } = render(view('[范围](https://example.com/a~b)'))
    expect(container.querySelector('a')?.getAttribute('href')).toBe('https://example.com/a~b')
    rerender(view('$x~y$'))
    await act(async () => { await Promise.resolve() })
    expect(container.querySelector('.katex-mathml annotation')?.textContent).toBe('x~y')
    rerender(view('~~~text\n7~8小时，12~14°C\n~~~'))
    await act(async () => { await Promise.resolve() })
    expect(container.querySelector('pre code')?.textContent).toBe('7~8小时，12~14°C')
  })
})
