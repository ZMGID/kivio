import { render } from '@testing-library/react'
import { expect, it } from 'vitest'
import { CompactionDivider } from './CompactionDivider'

it.each([
  [{ token_estimate_before: 0, token_estimate_after: 39500 }, '', ['压缩后: 39,500']],
  [{ tokenEstimateBefore: 78000 }, '', ['压缩前: 78,000']],
  [{ tokenEstimateBefore: 78000, tokenEstimateAfter: 39500 }, '78.0k → 39.5k',
    ['压缩前: 78,000', '压缩后: 39,500', '释放: 38,500']],
  [{ token_estimate_before: 0, token_estimate_after: 0 }, '', []],
] as const)('displays only the available compaction counts for %j', (counts, detail, tooltipLines) => {
  const record = { id: 'boundary', trigger: 'auto', ...counts }
  const { getByRole } = render(<CompactionDivider boundary={{ afterIndex: 0, record }} />)
  const divider = getByRole('separator', { name: '上下文压缩边界' })
  expect(divider.textContent).toBe(`上下文已压缩${detail}`)
  expect(divider.title.trim().split(/\n+/)).toEqual(['上下文已压缩 (自动)', ...tooltipLines])
})
