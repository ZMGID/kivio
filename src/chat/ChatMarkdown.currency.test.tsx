import { render } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { ChatMarkdown } from './ChatMarkdown'
import { MarkdownStreamingContext } from './markdownStreaming'

describe('currency and math', () => {
  it.each([false, true])('preserves amounts and bold emphasis (streaming=%s)', (streaming) => {
    const content = '这次共花费 **$0.011 美元**：首次搜索 $0.01，后续美国站搜索 $0.001。'
    const { container } = render(
      <MarkdownStreamingContext.Provider value={streaming}>
        <ChatMarkdown content={content} />
      </MarkdownStreamingContext.Provider>,
    )
    expect(container.querySelector('.katex')).toBeNull()
    expect(container.querySelector('[data-streamdown="strong"]')?.textContent).toBe('$0.011 美元')
    expect(container.textContent).toContain('首次搜索 $0.01，后续美国站搜索 $0.001。')
  })

  it.each([false, true])('renders Brazilian prices with spaces as money (streaming=%s)', (streaming) => {
    const content = '- Adorée：同品牌 15 ml 香水，售价 **R$ 20.00**（原价 R$ 28.90，优惠 31%）；销量字段未提供。\n- Inspire Girl：选择首屏里较接近的**5 支装女士香水**作为类似款，售价 **R$ 74.99**（原价 R$ 99.99，优惠 25%），销量 **1,658**。'
    const { container } = render(
      <MarkdownStreamingContext.Provider value={streaming}>
        <ChatMarkdown content={content} />
      </MarkdownStreamingContext.Provider>,
    )
    expect(container.querySelector('.katex')).toBeNull()
    expect(Array.from(container.querySelectorAll('[data-streamdown="strong"]'), node => node.textContent))
      .toEqual(['R$ 20.00', '5 支装女士香水', 'R$ 74.99', '1,658'])
    expect(container.textContent).toContain('原价 R$ 28.90，优惠 31%')
    expect(container.textContent).toContain('原价 R$ 99.99，优惠 25%')
  })

  it.each([' ', '\u00a0', '\u202f'])('preserves currency spacing %j alongside math', (space) => {
    const { container } = render(<ChatMarkdown content={`**R$${space}20.00** e R$${space}28.90; $ 2 + 3 = 5 $`} />)
    expect(container.querySelector('[data-streamdown="strong"]')?.textContent).toBe(`R$${space}20.00`)
    expect(Array.from(container.querySelectorAll('.katex-mathml annotation'), node => node.textContent))
      .toEqual(['2 + 3 = 5'])
  })

  it.each([false, true])('keeps standalone currency symbols out of math (streaming=%s)', (streaming) => {
    const content = '**一个数字要你确认**：供应商标 R$（雷亚尔），第 2 页起标 **D$**。我按美元理解记成了 US$ 9.50，并且**没有换算汇率**。如果 D$ 是别的单位，用 R$ 17,90 对着 US$ 9.50 看会低估成本。'
    const { container } = render(
      <MarkdownStreamingContext.Provider value={streaming}>
        <ChatMarkdown content={content} />
      </MarkdownStreamingContext.Provider>,
    )
    expect(container.querySelector('.katex')).toBeNull()
    expect(Array.from(container.querySelectorAll('[data-streamdown="strong"]'), node => node.textContent))
      .toEqual(['一个数字要你确认', 'D$', '没有换算汇率'])
    expect(container.textContent).toContain('R$（雷亚尔）')
    expect(container.textContent).toContain('R$ 17,90 对着 US$ 9.50')
  })

  it('keeps decimal-comma prices in their table cells', () => {
    const { container } = render(<ChatMarkdown content={'| 成本 | 竞品 |\n| --- | --- |\n| R$ 8,30 | R$ 19,22 |\n| US$ 9,50 | R$ 17,90 |'} />)
    expect(container.querySelector('.katex')).toBeNull()
    expect(Array.from(container.querySelectorAll('td'), node => node.textContent))
      .toEqual(['R$ 8,30', 'R$ 19,22', 'US$ 9,50', 'R$ 17,90'])
  })

  it('keeps currency beside real numeric, symbolic and display math', () => {
    const { container } = render(<ChatMarkdown content={'Costs $10 and $20; $2 + 3 = 5$, $x^2$, and \\(2x\\).\n\n$$\n2 + 3 = 5\n$$'} />)
    expect(container.textContent).toContain('Costs $10 and $20;')
    expect(Array.from(container.querySelectorAll('.katex-mathml annotation'), (node) => node.textContent))
      .toEqual(['2 + 3 = 5', 'x^2', '2x', '2 + 3 = 5'])
  })

  it('preserves escaped dollars and literal code', () => {
    const { container } = render(<ChatMarkdown content={'\\$10 and `$20`\n\n```text\n$30 and $40\n```'} />)
    expect(container.querySelector('.katex')).toBeNull()
    expect(container.textContent).toContain('$10')
    expect(container.querySelector('code')?.textContent).toBe('$20')
    expect(container.textContent).toContain('$30 and $40')
  })

  it('preserves numeric math with punctuation and LaTeX commands', () => {
    const { container } = render(<ChatMarkdown content={'$2,3$ and $2^{10}$ and $2 \\text{USD}$'} />)
    expect(Array.from(container.querySelectorAll('.katex-mathml annotation'), (node) => node.textContent))
      .toEqual(['2,3', '2^{10}', '2 \\text{USD}'])
  })

  it('does not rewrite link destinations or tilde-fenced code', () => {
    const { container } = render(<ChatMarkdown content={'[Price $10](https://example.com/$10)\n\n~~~text\n$30 and $40\n~~~'} />)
    expect(container.querySelector('a')?.getAttribute('href')).toBe('https://example.com/$10')
    expect(container.querySelector('a')?.textContent).toBe('Price $10')
    expect(container.textContent).toContain('$30 and $40')
    expect(container.textContent).not.toContain('\\$')
  })

  it('preserves signed amounts beside a real formula', () => {
    const { container } = render(<ChatMarkdown content={'退款 $-10，余额 $+20；公式 $-2 + 3 = 1$。'} />)
    expect(container.textContent).toContain('退款 $-10，余额 $+20；')
    expect(Array.from(container.querySelectorAll('.katex-mathml annotation'), node => node.textContent))
      .toEqual(['-2 + 3 = 1'])
  })

  it('keeps a price literal as its answer streams and then settles', () => {
    const content = '售价 **R$ 20.00**，原价 R$ 28.90。'
    const view = render(<ChatMarkdown content="" />)
    for (let end = content.indexOf('$') + 1; end <= content.length; end++) {
      view.rerender(<MarkdownStreamingContext.Provider value><ChatMarkdown content={content.slice(0, end)} /></MarkdownStreamingContext.Provider>)
      expect(view.container.querySelector('.katex')).toBeNull()
    }
    view.rerender(<ChatMarkdown content={content} />)
    expect(view.container.querySelector('[data-streamdown="strong"]')?.textContent).toBe('R$ 20.00')
  })

})
