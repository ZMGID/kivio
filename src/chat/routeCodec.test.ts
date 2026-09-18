import { describe, expect, it } from 'vitest'
import {
  chatRouteKind,
  decodeChatRouteId,
  encodeChatRouteId,
  isChatOnboardingPath,
  isChatPath,
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
