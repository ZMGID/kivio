import { render, waitFor } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { ChatMarkdown } from './ChatMarkdown'

// 同一文件内共享模块状态：用例按顺序覆盖“首次加载前 → 加载后 → 再次挂载”。
describe('公式按需排版', () => {
  it('KaTeX 未加载时显示 TeX 源码，加载后替换为公式', async () => {
    const { container } = render(<ChatMarkdown content={'目标 $Z_1$ 最小化'} />)
    const placeholder = container.querySelector('[data-markdown-math="inline"]')
    expect(placeholder).toHaveTextContent('$Z_1$')
    expect(container.querySelector('.katex')).toBeNull()

    await waitFor(() => expect(container.querySelector('.katex-mathml annotation')).toHaveTextContent('Z_1'))
    // 同一节点原地换成排版结果，不改变外层结构。
    expect(container.querySelector('[data-markdown-math="inline"]')).toBe(placeholder)
  })

  it('加载过一次后，新挂载的公式同步排版，不再闪源码', () => {
    const { container } = render(<ChatMarkdown content={'$$\n\\frac{a}{b}\n$$'} />)
    const display = container.querySelector('[data-markdown-math="display"]')
    expect(display?.querySelector('.katex-display')).not.toBeNull()
    expect(display).not.toHaveTextContent('$$')
  })

  it('无法解析的公式保留源码', () => {
    const { container } = render(<ChatMarkdown content={'$\\frac{a$'} />)
    expect(container.querySelector('.katex-error')).toHaveTextContent('\\frac{a')
  })
})
