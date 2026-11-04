import { describe, expect, it } from 'vitest'
import { clampTaskMaxConcurrency, TASK_MAX_CONCURRENCY_PRESETS } from './chatToolsShared'

describe('task concurrency settings', () => {
  it.each([
    [undefined, 32],
    [null, 32],
    ['invalid', 32],
    [Number.POSITIVE_INFINITY, 32],
    [-1, 0],
    [0, 0],
    ['0', 0],
    [1, 1],
    [31.6, 32],
    [197, 197],
    [256, 256],
    [257, 256],
  ] as const)('normalizes %s to %s', (value, expected) => {
    expect(clampTaskMaxConcurrency(value)).toBe(expected)
  })

  it('offers the default and unlimited as presets', () => {
    expect(TASK_MAX_CONCURRENCY_PRESETS).toContain(32)
    expect(TASK_MAX_CONCURRENCY_PRESETS).toContain(0)
  })
})
