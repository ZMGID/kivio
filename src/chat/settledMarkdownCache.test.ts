import { beforeEach, expect, it, vi } from 'vitest'
import { clearSettledMarkdownCache, getSettledMarkdownCacheEntry } from './settledMarkdownCache'

beforeEach(clearSettledMarkdownCache)
it('skips oversized entries', () => {
  const content = 'x'.repeat(3 * 1024 * 1024)
  const build = vi.fn(() => ({ normalized: content }))
  getSettledMarkdownCacheEntry(content, build)
  getSettledMarkdownCacheEntry(content, build)
  expect(build).toHaveBeenCalledTimes(2)
})
it('evicts by bytes below the entry count limit', () => {
  const content = 'x'.repeat(150_000)
  const build = vi.fn(() => ({ normalized: content }))
  getSettledMarkdownCacheEntry('first' + content, build)
  for (let i = 0; i < 20; i++) getSettledMarkdownCacheEntry(i + content, build)
  getSettledMarkdownCacheEntry('first' + content, build)
  expect(build).toHaveBeenCalledTimes(22)
})
