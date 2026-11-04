import { describe, expect, it } from 'vitest'
import { clampTaskMaxConcurrency } from './chatToolsShared'

describe('task concurrency settings', () => {
  it.each([
    ['invalid', 32],
    [Number.POSITIVE_INFINITY, 32],
    [-1, 0],
    [0, 0],
    ['0', 0],
    [1, 1],
    [31.6, 32],
    [197, 197],
    [256, 256],
    [257, 257],
    [1024, 1024],
    [Number.MAX_VALUE, Number.MAX_SAFE_INTEGER],
  ] as const)('normalizes %s to %s', (value, expected) => {
    expect(clampTaskMaxConcurrency(value)).toBe(expected)
  })

})
