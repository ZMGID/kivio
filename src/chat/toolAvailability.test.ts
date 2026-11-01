import { describe, expect, it } from 'vitest'
import type { ChatToolDefinition } from '../api/tauri'
import { deriveToolStatusHint, findUnavailableRecommendedTools, type ToolStatusHintInput } from './toolAvailability'

const tools: ChatToolDefinition[] = [{
  id: 'mcp__notes__search',
  name: 'search',
  serverId: 'notes',
  source: 'mcp',
  description: 'Search notes',
  inputSchema: {},
  sensitive: false,
}]

describe('recommended tool availability', () => {
  it('does not treat an unconnected server as missing tools and disable skill sends', () => {
    expect(findUnavailableRecommendedTools(['notes:search'], [], true)).toEqual([])
  })

  it('does not reject a recommendation when only some servers have cached tools', () => {
    expect(findUnavailableRecommendedTools(['other:search'], tools, true)).toEqual([])
  })

  it('reports missing tools after discovery completes', () => {
    expect(findUnavailableRecommendedTools(['other:search'], tools, false)).toEqual(['other:search'])
    expect(findUnavailableRecommendedTools(['notes:search'], [], false)).toEqual(['notes:search'])
  })

  it('matches complete tool lists by name, id and server-qualified name', () => {
    expect(findUnavailableRecommendedTools(['search', 'mcp__notes__search', ' notes:search '], tools, false)).toEqual([])
  })
})

describe('deriveToolStatusHint', () => {
  const base: ToolStatusHintInput = {
    toolsDisabledReason: '',
    enabledToolCount: 3,
    toolsRequested: true,
    effectiveSkillId: null,
    recommendedTools: [],
    unavailableRecommendedTools: [],
  }

  it('is silent when tools are available and nothing is missing', () => {
    expect(deriveToolStatusHint(base)).toBe('')
  })

  it('surfaces the disabled reason when the user asked for tools and the catalog is empty', () => {
    const reason = 'MCP unavailable'
    expect(deriveToolStatusHint({ ...base, toolsDisabledReason: reason, enabledToolCount: 0 })).toBe(reason)
    expect(deriveToolStatusHint({ ...base, toolsDisabledReason: reason, enabledToolCount: null })).toBe(reason)
  })

  it('prefixes the skill requirement when the active skill needs tools', () => {
    const reason = 'MCP unavailable'
    const hint = deriveToolStatusHint({
      ...base, toolsDisabledReason: reason, enabledToolCount: 0, toolsRequested: false, recommendedTools: ['x'],
    })
    expect(hint.endsWith(reason)).toBe(true)
    expect(hint).not.toBe(reason)
  })

  it('passes the provider "不支持 tools" message through verbatim when a skill is mounted', () => {
    const reason = '该模型不支持 tools'
    expect(deriveToolStatusHint({
      ...base, toolsDisabledReason: reason, enabledToolCount: 0, effectiveSkillId: 'pdf', recommendedTools: ['x'],
    })).toBe(reason)
  })

  it('stays silent when the catalog is empty but nobody asked for tools', () => {
    expect(deriveToolStatusHint({ ...base, toolsDisabledReason: 'off', enabledToolCount: 0, toolsRequested: false }))
      .toBe('')
  })

  it('lists at most three missing recommended tools', () => {
    const missing = ['tool-a', 'tool-b', 'tool-c', 'tool-d']
    const hint = deriveToolStatusHint({ ...base, unavailableRecommendedTools: missing })
    for (const name of missing.slice(0, 3)) expect(hint).toContain(name)
    expect(hint).not.toContain(missing[3])
  })
})
