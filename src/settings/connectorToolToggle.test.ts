import { describe, expect, it } from 'vitest'
import { allowTool, disableTool, isToolAllowed, toggleTool } from './connectorToolToggle'

const ALL = ['a', 'b', 'c']

describe('connector tool toggle', () => {
  it('treats empty enabledTools as all allowed', () => {
    expect(isToolAllowed([], 'a')).toBe(true)
    expect(isToolAllowed([], 'z')).toBe(true)
  })

  it('treats non-empty enabledTools as a whitelist', () => {
    expect(isToolAllowed(['a'], 'a')).toBe(true)
    expect(isToolAllowed(['a'], 'b')).toBe(false)
  })

  it('disable from whitelist just removes', () => {
    expect(disableTool(ALL, ['a', 'b'], 'a')).toEqual(['b'])
  })

  it('allow on all-allowed stays empty', () => {
    expect(allowTool(ALL, [], 'a')).toEqual([])
  })

  it('allow adds to whitelist', () => {
    expect(allowTool(ALL, ['a'], 'b').sort()).toEqual(['a', 'b'])
  })

  it('toggle dispatches allow/disable', () => {
    expect(toggleTool(ALL, [], 'a', false).sort()).toEqual(['b', 'c'])
    expect(toggleTool(ALL, ['a', 'b'], 'c', true)).toEqual([])
  })

  it('round-trips disable then allow back to all-allowed', () => {
    const afterDisable = disableTool(ALL, [], 'b') // ['a','c']
    expect(afterDisable.sort()).toEqual(['a', 'c'])
    const afterAllow = allowTool(ALL, afterDisable, 'b')
    expect(afterAllow).toEqual([])
  })
})
