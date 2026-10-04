import { describe, expect, it } from 'vitest'
import routeContract from './routeContract.json'
import {
  chatRouteKind,
  decodeChatRouteId,
  decodeConversationRouteId,
  encodeChatRouteId,
  isRememberableChatRoute,
  isRestorableChatRoute,
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

  it('decodes one route segment and contains malformed encoding', () => {
    expect(encodeChatRouteId('chat/popout/', 'a/b')).toBe('chat/popout/a%2Fb')
    expect(decodeChatRouteId('chat/', 'chat/a%2Fb')).toBe('a/b')
    expect(decodeChatRouteId('chat/', 'chat/a/b')).toBeNull()
    expect(decodeChatRouteId('chat/', 'chat/%E0%A4%A')).toBeNull()
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
