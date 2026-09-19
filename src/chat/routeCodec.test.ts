import { describe, expect, it } from 'vitest'
import routeContract from './routeContract.json'
import {
  chatRouteKind,
  decodeChatRouteId,
  decodeConversationRouteId,
  encodeChatRouteId,
  isChatOnboardingPath,
  isChatPath,
  isRememberableChatRoute,
  isRestorableChatRoute,
  isChatSettingsPath,
  pathFromHash,
} from './routeCodec'
import {
  isChatAssistantCenterPath,
  isChatAutomationsPath,
  isChatKnowledgeCenterPath,
  isChatMcpCenterPath,
  isChatNotesPath,
  isChatOnboardingRoute,
  isChatPluginCenterPath,
  isChatPopoutRoute,
  isChatSessionCenterPath,
  isChatSkillCenterPath,
} from './chatRoutes'

describe('chat route codec', () => {
  it.each(routeContract.cases)('matches the shared route contract: $name', (fixture) => {
    const path = pathFromHash(fixture.raw)
    expect(path).toBe(fixture.path)
    expect(chatRouteKind(path)).toBe(fixture.kind)
    expect(decodeConversationRouteId(path)).toBe(fixture.decodedConversationId)
    expect(isRememberableChatRoute(path)).toBe(fixture.rememberable)
    expect(isRestorableChatRoute(fixture.raw)).toBe(fixture.restorable)
  })

  it('normalizes hashes and classifies center routes in one place', () => {
    expect(pathFromHash('#chat/settings?tab=general')).toBe('chat/settings')
    expect(isChatPath('chat/conversation-1')).toBe(true)
    expect(isChatSettingsPath('chat/settings/providers')).toBe(true)
    expect(isChatOnboardingPath('chat/onboarding/step')).toBe(true)
    expect(chatRouteKind('chat/automations/a-1')).toBe('automations')
    expect(chatRouteKind('chat/conversation-1')).toBe('conversation')
  })

  it('decodes one route segment and contains malformed encoding', () => {
    expect(encodeChatRouteId('chat/popout/', 'a/b')).toBe('chat/popout/a%2Fb')
    expect(decodeChatRouteId('chat/', 'chat/a%2Fb')).toBe('a/b')
    expect(decodeChatRouteId('chat/', 'chat/a/b')).toBeNull()
    expect(decodeChatRouteId('chat/', 'chat/%E0%A4%A')).toBeNull()
  })

  it.each([
    ['chat/', 'other'],
    ['chat/a/b', 'other'],
    ['chat/%E0%A4%A', 'other'],
  ] as const)('does not classify invalid conversation path %s as a conversation', (path, kind) => {
    expect(chatRouteKind(path)).toBe(kind)
  })

  it('preserves rememberable center routes while rejecting transient and corrupt routes', () => {
    for (const path of [
      'chat/assistants',
      'chat/skill/item',
      'chat/plugins',
      'chat/sessions',
      'chat/automations/a-1',
      'chat/mcp',
      'chat/knowledge',
      'chat/notes/n-1',
    ]) {
      expect(isRememberableChatRoute(path), path).toBe(true)
    }
    for (const path of [
      'chat',
      'chat/settings',
      'chat/onboarding',
      'chat/popout/c-1',
      'chat/a/b',
      'chat/%E0%A4%A',
      'lens',
    ]) {
      expect(isRememberableChatRoute(path), path).toBe(false)
    }
  })

  it.each([
    ['chat/assistants/item', isChatAssistantCenterPath],
    ['chat/skill/item', isChatSkillCenterPath],
    ['chat/plugins/item', isChatPluginCenterPath],
    ['chat/sessions/item', isChatSessionCenterPath],
    ['chat/automations/item', isChatAutomationsPath],
    ['chat/mcp/item', isChatMcpCenterPath],
    ['chat/knowledge/item', isChatKnowledgeCenterPath],
    ['chat/notes/item', isChatNotesPath],
    ['chat/onboarding/item', isChatOnboardingRoute],
    ['chat/popout/item', isChatPopoutRoute],
  ] as const)('keeps the %s center wrapper behavior', (path, matches) => {
    expect(matches(path)).toBe(true)
    expect(matches('chat/conversation-1')).toBe(false)
  })
})
