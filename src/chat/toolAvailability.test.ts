import { describe, expect, it } from 'vitest'
import type { ChatToolDefinition } from '../api/tauri'
import { findUnavailableRecommendedTools } from './toolAvailability'

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
