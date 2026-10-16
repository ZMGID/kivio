/**
 * @vitest-environment jsdom
 *
 * 路由判定读 window.location.hash，需要 DOM 环境。
 * vite.config.ts 只给 *.test.tsx 配了 jsdom，这里按文件声明。
 */
import { describe, expect, it } from 'vitest'
import {
  conversationHash,
  extensionsNavItemForView,
  getRouteAutomationId,
  getRouteConversationId,
  hashPath,
  isChatAssistantCenterPath,
  isChatAutomationsPath,
  isChatSchedulesPath,
  isChatKnowledgeCenterPath,
  isChatMcpCenterPath,
  isChatNotesPath,
  isChatOnboardingRoute,
  isChatPluginCenterPath,
  isChatPopoutRoute,
  isChatSessionCenterPath,
  isChatSettingsPath,
  isChatSkillCenterPath,
} from './chatRoutes'

function withHash(hash: string) {
  window.location.hash = hash
}

describe('chatRoutes 判定', () => {
  it('各中心页判定互不误命中', () => {
    const cases: Array<[string, (p: string) => boolean]> = [
      ['chat/settings', isChatSettingsPath],
      ['chat/assistants', isChatAssistantCenterPath],
      ['chat/skill', isChatSkillCenterPath],
      ['chat/plugins', isChatPluginCenterPath],
      ['chat/sessions', isChatSessionCenterPath],
      ['chat/automations', isChatAutomationsPath],
      ['chat/schedules', isChatSchedulesPath],
      ['chat/mcp', isChatMcpCenterPath],
      ['chat/knowledge', isChatKnowledgeCenterPath],
      ['chat/notes', isChatNotesPath],
      ['chat/onboarding', isChatOnboardingRoute],
      ['chat/popout', isChatPopoutRoute],
    ]
    for (const [path, predicate] of cases) {
      expect(predicate(path)).toBe(true)
      // 任一判定不应命中其他路由
      for (const [otherPath, otherPredicate] of cases) {
        if (otherPath === path) continue
        expect(otherPredicate(path)).toBe(false)
      }
    }
  })

  it('子路径也命中（chat/settings/xxx）', () => {
    expect(isChatSettingsPath('chat/settings/providers')).toBe(true)
    expect(isChatSkillCenterPath('chat/skill/store')).toBe(true)
  })

  it('前缀相近但不同的路径不命中', () => {
    // 'chat/settingsx' 不是 settings 的子路径
    expect(isChatSettingsPath('chat/settingsx')).toBe(false)
    expect(isChatNotesPath('chat/notesarchive')).toBe(false)
    expect(isChatSchedulesPath('chat/schedulesarchive')).toBe(false)
  })

  it('会话路径不被任何中心页判定命中', () => {
    const convPath = 'chat/abc-123'
    for (const predicate of [
      isChatSettingsPath, isChatAssistantCenterPath, isChatSkillCenterPath,
      isChatPluginCenterPath, isChatSessionCenterPath, isChatAutomationsPath, isChatSchedulesPath, isChatMcpCenterPath,
      isChatKnowledgeCenterPath, isChatNotesPath, isChatOnboardingRoute, isChatPopoutRoute,
    ]) {
      expect(predicate(convPath)).toBe(false)
    }
  })
})

describe('hashPath', () => {

  it('空 hash 返回空串', () => {
    withHash('')
    expect(hashPath()).toBe('')
  })
})

describe('getRouteConversationId', () => {

  it('URL 编码的 id 被解码', () => {
    withHash(`#chat/${encodeURIComponent('a/b c')}`)
    expect(getRouteConversationId()).toBe('a/b c')
  })

  it('非 chat 路由返回 null', () => {
    withHash('#settings')
    expect(getRouteConversationId()).toBeNull()
  })

  it('排除清单里的中心页返回 null', () => {
    for (const seg of [
      'settings', 'assistants', 'skill', 'knowledge', 'onboarding',
      'mcp', 'notes', 'plugins', 'sessions', 'automations', 'schedules', 'popout',
    ]) {
      withHash(`#chat/${seg}`)
      expect(getRouteConversationId()).toBeNull()
    }
  })

  it('popout 路由不当成主窗会话 id', () => {
    withHash('#chat/popout/conv_abc')
    expect(getRouteConversationId()).toBeNull()
  })
})

describe('getRouteAutomationId', () => {
  it('列表页返回 null', () => {
    withHash('#chat/automations')
    expect(getRouteAutomationId()).toBeNull()
  })

  it('编辑页返回解码后的 id', () => {
    withHash('#chat/automations/auto-1')
    expect(getRouteAutomationId()).toBe('auto-1')
  })

  it('多层路径返回 null', () => {
    withHash('#chat/automations/a/b')
    expect(getRouteAutomationId()).toBeNull()
  })
})

describe('conversationHash', () => {

  it('conversationHash 编码特殊字符', () => {
    expect(conversationHash('a/b')).toBe('#chat/a%2Fb')
  })

})

describe('extensionsNavItemForView', () => {
  it('maps center views to the extensions nav item and ignores the rest', () => {
    expect(extensionsNavItemForView('assistants')).toBe('assistants')
    expect(extensionsNavItemForView('skill')).toBe('plugins')
    expect(extensionsNavItemForView('mcp')).toBe('plugins')
    expect(extensionsNavItemForView('knowledge')).toBe('knowledge')
    expect(extensionsNavItemForView('notes')).toBe('notes')
    expect(extensionsNavItemForView('automations')).toBe('tasks')
    expect(extensionsNavItemForView('schedules')).toBe('tasks')
    expect(extensionsNavItemForView('settings')).toBeNull()
    expect(extensionsNavItemForView('conversation')).toBeNull()
    expect(extensionsNavItemForView('onboarding')).toBeNull()
  })
})
