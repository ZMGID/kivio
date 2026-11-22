import { describe, expect, it, vi } from 'vitest'
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

it('mounts every table and code block in a settled answer before scrolling', () => {
  class OffscreenObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  vi.stubGlobal('IntersectionObserver', OffscreenObserver)
  try {
    const sections = Array.from({ length: 32 }, (_, i) => [
      `## Section ${i}`, '', '| Worker | Cost |', '| --- | ---: |', `| worker-${i} | **${i}** |`,
      '', '```ts', `const worker${i} = ${i}`, '```', '', 'Review the complete measurement for this worker.', '',
    ].join('\n'))
    const { container } = render(<div className="chat-scroll-viewport">
      <ChatMarkdown content={sections.join('\n')} />
    </div>)
    expect(container.querySelectorAll('table')).toHaveLength(sections.length)
    expect(container.querySelectorAll('pre')).toHaveLength(sections.length)
    expect(container.querySelectorAll('table')[31]).toHaveTextContent('worker-31')
    expect(container.querySelectorAll('pre')[31]).toHaveTextContent('const worker31 = 31')
  } finally {
    vi.unstubAllGlobals()
  }
})
