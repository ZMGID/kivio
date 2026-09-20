import { renderHook, act } from '@testing-library/react'
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { useRef } from 'react'
import { useChatRouting } from './useChatRouting'
import {
  beginConversationTransition,
  invalidateConversationTransition,
  isCurrentConversationTransition,
} from '../conversationTransitionStore'

/**
 * 回归重点（搬迁时最容易破的三件事）：
 *   1. 分支顺序 —— 中心页判定必须早于会话解析，否则 '#chat/mcp' 会被当成会话 id
 *   2. 挂载即执行一次 + 订阅 hashchange 的时序
 *   3. 「已是当前会话则跳过重载」这条防双读逻辑
 */
function setup(initialHash = '#chat', opts?: {
  onOpenPluginsSettings?: () => void
  onOpenSessionsSettings?: () => void
  onLoadConversation?: (conversationId: string) => void
  onLeaveConversation?: () => void
}) {
  window.location.hash = initialHash
  const onViewChange = vi.fn()
  const onLoadConversation = vi.fn(opts?.onLoadConversation)
  const onResetConversation = vi.fn()
  const onLeaveConversation = vi.fn(opts?.onLeaveConversation)
  const onOpenPluginsSettings = opts?.onOpenPluginsSettings ?? vi.fn()
  const onOpenSessionsSettings = opts?.onOpenSessionsSettings ?? vi.fn()
  const setSettingsInitialTab = vi.fn()
  const setExtensionsNavItem = vi.fn()

  const rendered = renderHook(() => {
    const currentConversationIdRef = useRef<string | null>(null)
    const routing = useChatRouting({
      onViewChange,
      onLoadConversation,
      onResetConversation,
      currentConversationIdRef,
      onOpenPluginsSettings,
      onOpenSessionsSettings,
      onLeaveConversation,
      setSettingsInitialTab,
      setExtensionsNavItem,
    })
    return { routing, currentConversationIdRef }
  })

  return {
    ...rendered,
    onViewChange,
    onLoadConversation,
    onResetConversation,
    onOpenPluginsSettings,
    onOpenSessionsSettings,
    onLeaveConversation,
    setSettingsInitialTab,
    setExtensionsNavItem,
  }
}

describe('useChatRouting 挂载即解析', () => {
  beforeEach(() => {
    window.location.hash = '#chat'
  })

  it('挂载时立刻按当前 hash 解析一次', () => {
    const { onViewChange } = setup('#chat/settings')
    expect(onViewChange).toHaveBeenCalledWith('settings')
  })

  it('空会话路由触发 reset 而非 load', () => {
    const { onViewChange, onResetConversation, onLoadConversation } = setup('#chat')
    expect(onViewChange).toHaveBeenCalledWith('conversation')
    expect(onResetConversation).toHaveBeenCalled()
    expect(onLoadConversation).not.toHaveBeenCalled()
  })

  it('会话路由触发 load 且带 id', () => {
    const { onLoadConversation, onResetConversation } = setup('#chat/conv-1')
    expect(onLoadConversation).toHaveBeenCalledWith('conv-1')
    expect(onResetConversation).not.toHaveBeenCalled()
  })
})

describe('useChatRouting 分支顺序', () => {
  // 关键：中心页判定必须早于会话解析。若顺序反了，这些路由会被当作会话 id 去加载。
  const centerRoutes: Array<[string, string]> = [
    ['#chat/settings', 'settings'],
    ['#chat/assistants', 'assistants'],
    ['#chat/skill', 'skill'],
    ['#chat/mcp', 'mcp'],
    ['#chat/knowledge', 'knowledge'],
    ['#chat/notes', 'notes'],
    ['#chat/artifacts', 'artifacts'],
    ['#chat/automations', 'automations'],
    ['#chat/onboarding', 'onboarding'],
  ]

  for (const [hash, view] of centerRoutes) {
    it(`${hash} → view=${view} 且不当作会话加载`, () => {
      const { onViewChange, onLoadConversation, onResetConversation } = setup(hash)
      expect(onViewChange).toHaveBeenCalledWith(view)
      expect(onLoadConversation).not.toHaveBeenCalled()
      expect(onResetConversation).not.toHaveBeenCalled()
    })
  }

  it('#chat/automations/id → view=automations，不当作会话加载', () => {
    const { onViewChange, onLoadConversation } = setup('#chat/automations/auto-1')
    expect(onViewChange).toHaveBeenCalledWith('automations')
    expect(onLoadConversation).not.toHaveBeenCalled()
  })

  it('#chat/plugins → 走设置插件重定向，不当作会话加载', () => {
    const { onViewChange, onLoadConversation, onOpenPluginsSettings } = setup('#chat/plugins')
    expect(onOpenPluginsSettings).toHaveBeenCalled()
    expect(onViewChange).not.toHaveBeenCalled()
    expect(onLoadConversation).not.toHaveBeenCalled()
  })

  it('#chat/sessions → 走设置对话库重定向，不当作会话加载', () => {
    const { onViewChange, onLoadConversation, onOpenSessionsSettings } = setup('#chat/sessions')
    expect(onOpenSessionsSettings).toHaveBeenCalled()
    expect(onViewChange).not.toHaveBeenCalled()
    expect(onLoadConversation).not.toHaveBeenCalled()
  })
})

describe('useChatRouting hashchange', () => {
  beforeEach(() => {
    window.location.hash = '#chat'
    invalidateConversationTransition()
  })

  it('hash 变化后重新解析', () => {
    const { onViewChange } = setup('#chat')
    onViewChange.mockClear()
    act(() => {
      window.location.hash = '#chat/skill'
      window.dispatchEvent(new HashChangeEvent('hashchange'))
    })
    expect(onViewChange).toHaveBeenCalledWith('skill')
  })

  it('卸载后不再响应 hash 变化', () => {
    const { onViewChange, unmount } = setup('#chat')
    unmount()
    onViewChange.mockClear()
    act(() => {
      window.location.hash = '#chat/mcp'
      window.dispatchEvent(new HashChangeEvent('hashchange'))
    })
    expect(onViewChange).not.toHaveBeenCalled()
  })

  it('已是当前会话时跳过重载（防双读）', () => {
    const { result, onLoadConversation } = setup('#chat')
    act(() => {
      result.current.currentConversationIdRef.current = 'conv-9'
    })
    onLoadConversation.mockClear()
    act(() => {
      window.location.hash = '#chat/conv-9'
      window.dispatchEvent(new HashChangeEvent('hashchange'))
    })
    expect(onLoadConversation).not.toHaveBeenCalled()
  })

  it('切到不同会话时正常重载', () => {
    const { result, onLoadConversation } = setup('#chat')
    act(() => {
      result.current.currentConversationIdRef.current = 'conv-9'
    })
    onLoadConversation.mockClear()
    act(() => {
      window.location.hash = '#chat/conv-10'
      window.dispatchEvent(new HashChangeEvent('hashchange'))
    })
    expect(onLoadConversation).toHaveBeenCalledWith('conv-10')
  })

  it('进入中心页时先使旧 conversation transition 失效，迟到成功不提交', async () => {
    let finish: (() => void) | undefined
    const committed = vi.fn()
    const onLoadConversation = (conversationId: string) => {
      const requestId = beginConversationTransition(conversationId)
      void new Promise<void>((resolve) => { finish = resolve }).then(() => {
        if (isCurrentConversationTransition(requestId, conversationId)) committed(conversationId)
      })
    }
    const { onLeaveConversation, onViewChange } = setup('#chat/missing-a', {
      onLoadConversation,
      onLeaveConversation: invalidateConversationTransition,
    })
    onLeaveConversation.mockClear()
    onViewChange.mockClear()

    act(() => {
      window.location.hash = '#chat/settings'
      window.dispatchEvent(new HashChangeEvent('hashchange'))
    })
    expect(onLeaveConversation.mock.invocationCallOrder[0]).toBeLessThan(onViewChange.mock.invocationCallOrder[0]!)
    await act(async () => { finish?.(); await Promise.resolve() })

    expect(committed).not.toHaveBeenCalled()
    expect(window.location.hash).toBe('#chat/settings')
  })

  it('进入 legacy redirect 时使旧 transition 失效，迟到失败不改路由或错误', async () => {
    let fail: ((error: Error) => void) | undefined
    const publishError = vi.fn()
    const onLoadConversation = (conversationId: string) => {
      const requestId = beginConversationTransition(conversationId)
      void new Promise<void>((_resolve, reject) => { fail = reject }).catch((error: Error) => {
        if (!isCurrentConversationTransition(requestId, conversationId)) return
        publishError(error.message)
        window.location.hash = '#chat'
      })
    }
    setup('#chat/missing-a', {
      onLoadConversation,
      onLeaveConversation: invalidateConversationTransition,
    })

    act(() => {
      window.location.hash = '#chat/plugins'
      window.dispatchEvent(new HashChangeEvent('hashchange'))
    })
    await act(async () => { fail?.(new Error('not found')); await Promise.resolve() })

    expect(publishError).not.toHaveBeenCalled()
    expect(window.location.hash).toBe('#chat/plugins')
  })
})

describe('useChatRouting sync*Route', () => {
  beforeEach(() => {
    window.location.hash = '#chat'
  })

  it('九个 sync 各写对应 hash（不串）', () => {
    const { result } = setup('#chat')
    const r = result.current.routing
    const cases: Array<[() => void, string]> = [
      [r.syncSettingsRoute, '#chat/settings'],
      [r.syncOnboardingRoute, '#chat/onboarding'],
      [r.syncAssistantCenterRoute, '#chat/assistants'],
      [r.syncSkillCenterRoute, '#chat/skill'],
      [r.syncMcpCenterRoute, '#chat/mcp'],
      [r.syncKnowledgeCenterRoute, '#chat/knowledge'],
      [r.syncNotesRoute, '#chat/notes'],
      [r.syncAutomationsRoute, '#chat/automations'],
    ]
    for (const [sync, expected] of cases) {
      act(() => { sync() })
      expect(window.location.hash).toBe(expected)
    }
    act(() => { r.syncConversationRoute('conv-2') })
    expect(window.location.hash).toBe('#chat/conv-2')
    act(() => { r.syncConversationRoute(null) })
    expect(window.location.hash).toBe('#chat')
  })
})

describe('useChatRouting center openers', () => {
  beforeEach(() => {
    window.location.hash = '#chat'
  })

  it('openEmbeddedSettings writes the tab, view, and settings hash', () => {
    const { result, onViewChange, setSettingsInitialTab } = setup('#chat')
    act(() => { result.current.routing.openEmbeddedSettings('usage') })
    expect(setSettingsInitialTab).toHaveBeenCalledWith('usage')
    expect(onViewChange).toHaveBeenCalledWith('settings')
    expect(window.location.hash).toBe('#chat/settings')
  })

  it('openChatSettings is openEmbeddedSettings(chat)', () => {
    const { result, setSettingsInitialTab } = setup('#chat')
    act(() => { result.current.routing.openChatSettings() })
    expect(setSettingsInitialTab).toHaveBeenCalledWith('chat')
    expect(window.location.hash).toBe('#chat/settings')
  })

  it('openExtensionsItem records the nav item and opens that center', () => {
    const { result, onViewChange, setExtensionsNavItem } = setup('#chat')
    act(() => { result.current.routing.openExtensionsItem('mcp') })
    expect(setExtensionsNavItem).toHaveBeenCalledWith('mcp')
    expect(onViewChange).toHaveBeenCalledWith('mcp')
    expect(window.location.hash).toBe('#chat/mcp')
  })
})
