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

})
