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
    expect(deriveToolStatusHint({ ...base, toolsDisabledReason: 'MCP 未连接', enabledToolCount: 0 }))
      .toBe('MCP 未连接')
    expect(deriveToolStatusHint({ ...base, toolsDisabledReason: 'MCP 未连接', enabledToolCount: null }))
      .toBe('MCP 未连接')
  })

  it('prefixes the skill requirement when the active skill needs tools', () => {
    expect(deriveToolStatusHint({
      ...base, toolsDisabledReason: 'MCP 未连接', enabledToolCount: 0, toolsRequested: false, recommendedTools: ['x'],
    })).toBe('当前 Skill 需要工具，但MCP 未连接')
  })

  it('passes the provider "不支持 tools" message through verbatim when a skill is mounted', () => {
    expect(deriveToolStatusHint({
      ...base, toolsDisabledReason: '该模型不支持 tools', enabledToolCount: 0, effectiveSkillId: 'pdf', recommendedTools: ['x'],
    })).toBe('该模型不支持 tools')
  })

  it('stays silent when the catalog is empty but nobody asked for tools', () => {
    expect(deriveToolStatusHint({ ...base, toolsDisabledReason: 'off', enabledToolCount: 0, toolsRequested: false }))
      .toBe('')
  })

  it('lists at most three missing recommended tools', () => {
    expect(deriveToolStatusHint({ ...base, unavailableRecommendedTools: ['a', 'b', 'c', 'd'] }))
      .toBe('当前 Skill 推荐的工具不可用：a, b, c')
  })
})
