import { describe, expect, it } from 'vitest'
import { render } from '@testing-library/react'
import { ChatMarkdown } from './ChatMarkdown'

describe('表格', () => {
  it('列对齐透传到单元格（remark-gfm 的 text-align 内联样式）', () => {
    const md = ['| 左 | 中 | 右 |', '| :--- | :---: | ---: |', '| a | b | c |'].join('\n')
    const { container } = render(<ChatMarkdown content={md} />)
    const ths = Array.from(container.querySelectorAll('th'))
    expect(ths.map((th) => th.style.textAlign)).toEqual(['left', 'center', 'right'])
    const tds = Array.from(container.querySelectorAll('td'))
    expect(tds.map((td) => td.style.textAlign)).toEqual(['left', 'center', 'right'])
  })

  it('keeps large result tables complete and renders inline Markdown when a cell uses it', () => {
    const lines = ['| 分片 | 延迟 |', '| :--- | ---: |', ...Array.from({ length: 350 },
      (_, i) => `| worker-${i} | ${i} |`)]
    const { container, rerender } = render(<ChatMarkdown content={lines.join('\n')} />)
    expect(container.querySelectorAll('tbody tr')).toHaveLength(350)
    expect(container.querySelector('tbody tr:last-child td:last-child')).toHaveTextContent('349')
    expect(container.querySelector('tbody tr:last-child td:last-child')).toHaveStyle({ textAlign: 'right' })
    rerender(<ChatMarkdown content={[...lines.slice(0, -1), '| final-worker | 901 |'].join('\n')} />)
    expect(container.querySelectorAll('tbody tr')).toHaveLength(350)
    expect(container.querySelector('tbody tr:last-child td:last-child')).toHaveTextContent('901')
    rerender(<ChatMarkdown content={[...lines, '| **重点** | 400 |'].join('\n')} />)
    expect(container.querySelector('tbody tr:last-child td:first-child')).toHaveTextContent('重点')
  })

})
