import { act, renderHook } from '@testing-library/react'
import { useState } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Settings, SettingsSnapshot } from '../../api/tauri'
import { SettingsEditorController } from '../../settings/SettingsEditorController'
import { useSettingsExit } from './useSettingsExit'

type View = Parameters<typeof useSettingsExit>[0]['chatView']

function setup(initialView: View = 'settings') {
  const currentConversationIdRef = { current: 'c1' as string | null }
  const settingsRef = { current: { requestClose: vi.fn() } }
  const syncConversationRoute = vi.fn()
  const onReturnedToConversation = vi.fn()
  const rendered = renderHook(() => {
    const [chatView, setChatView] = useState<View>(initialView)
    const exit = useSettingsExit({
      chatView,
      setChatView,
      settingsRef,
      currentConversationIdRef,
      syncConversationRoute,
      onReturnedToConversation,
    })
    return { chatView, setChatView, exit }
  })
  return { ...rendered, currentConversationIdRef, settingsRef, syncConversationRoute, onReturnedToConversation }
}

beforeEach(() => {
  vi.useFakeTimers()
})

afterEach(() => {
  vi.useRealTimers()
})

describe('useSettingsExit', () => {
  it('plays the 220ms exit, then returns to conversation and flushes the pending action', () => {
    const { result, onReturnedToConversation, syncConversationRoute, currentConversationIdRef } = setup()
    act(() => { result.current.exit.closeSettings() })
    expect(result.current.exit.settingsExiting).toBe(true)
    expect(result.current.chatView).toBe('settings')
    expect(onReturnedToConversation).not.toHaveBeenCalled()

    act(() => { vi.advanceTimersByTime(219) })
    expect(result.current.exit.settingsExiting).toBe(true)

    act(() => { vi.advanceTimersByTime(1) })
    expect(result.current.exit.settingsExiting).toBe(false)
    expect(result.current.chatView).toBe('conversation')
    expect(syncConversationRoute.mock.calls).toEqual([['c1']])
    expect(onReturnedToConversation).toHaveBeenCalledOnce()
    expect(currentConversationIdRef.current).toBe('c1')
  })

  it('runs the action immediately when not on the settings view', () => {
    const { result, settingsRef } = setup('conversation')
    const action = vi.fn()
    act(() => { result.current.exit.runAfterLeavingSettings(action) })
    expect(action).toHaveBeenCalledTimes(1)
    expect(settingsRef.current.requestClose).not.toHaveBeenCalled()
  })

  it('queues the action and only requestClose when SettingsShell is mounted', () => {
    const { result, settingsRef, onReturnedToConversation, syncConversationRoute } = setup()
    const action = vi.fn(() => syncConversationRoute('target'))
    act(() => { result.current.exit.runAfterLeavingSettings(action, { restoreCurrentRoute: false }) })
    expect(action).not.toHaveBeenCalled()
    expect(settingsRef.current.requestClose).toHaveBeenCalledTimes(1)

    act(() => { result.current.exit.closeSettings() })
    act(() => { vi.advanceTimersByTime(220) })
    expect(syncConversationRoute.mock.calls).toEqual([['target']])
    expect(action).toHaveBeenCalledOnce()
    expect(onReturnedToConversation).toHaveBeenCalledOnce()
  })

  it('exits immediately and runs the action when SettingsShell is gone', () => {
    const { result, settingsRef, syncConversationRoute } = setup()
    settingsRef.current = null as never
    const action = vi.fn(() => expect(syncConversationRoute.mock.calls).toEqual([['c1']]))
    act(() => { result.current.exit.runAfterLeavingSettings(action) })
    expect(result.current.chatView).toBe('conversation')
    expect(action).toHaveBeenCalledOnce()
  })

  it('refreshes when returning from skill/mcp/assistants/knowledge/settings, but not notes', () => {
    const { result, onReturnedToConversation } = setup('skill')
    act(() => { result.current.setChatView('conversation') })
    expect(onReturnedToConversation).toHaveBeenCalledTimes(1)

    for (const view of ['mcp', 'assistants', 'knowledge', 'settings'] as const) {
      act(() => { result.current.setChatView(view) })
      onReturnedToConversation.mockClear()
      act(() => { result.current.setChatView('conversation') })
      expect(onReturnedToConversation).toHaveBeenCalledTimes(1)
    }

    act(() => { result.current.setChatView('notes') })
    onReturnedToConversation.mockClear()
    act(() => { result.current.setChatView('conversation') })
    expect(onReturnedToConversation).not.toHaveBeenCalled()
  })

  it('restores the latest current route before a non-navigation action', () => {
    const { result, syncConversationRoute, currentConversationIdRef } = setup()
    const action = vi.fn(() => expect(syncConversationRoute.mock.calls).toEqual([['c2']]))
    act(() => { result.current.exit.runAfterLeavingSettings(action) })
    act(() => { result.current.exit.closeSettings() })
    currentConversationIdRef.current = 'c2'
    act(() => { vi.advanceTimersByTime(220) })
    expect(action).toHaveBeenCalledOnce()
  })

  it('does not refresh conversation tools when the queued action opens another center', () => {
    const { result, onReturnedToConversation } = setup()
    act(() => {
      result.current.exit.runAfterLeavingSettings(() => result.current.setChatView('mcp'))
      result.current.exit.closeSettings()
    })
    act(() => { vi.advanceTimersByTime(220) })
    expect(result.current.chatView).toBe('mcp')
    expect(onReturnedToConversation).not.toHaveBeenCalled()
    act(() => { result.current.setChatView('conversation') })
    expect(onReturnedToConversation).toHaveBeenCalledOnce()
  })

  it('coalesces repeated close callbacks and executes only the latest queued action', () => {
    const { result, onReturnedToConversation } = setup()
    const first = vi.fn()
    const latest = vi.fn()
    act(() => {
      result.current.exit.runAfterLeavingSettings(first)
      result.current.exit.closeSettings()
    })
    act(() => { vi.advanceTimersByTime(100) })
    act(() => {
      result.current.exit.runAfterLeavingSettings(latest)
      result.current.exit.closeSettings()
    })
    act(() => { vi.advanceTimersByTime(120) })
    expect(latest).toHaveBeenCalledOnce()
    act(() => { vi.advanceTimersByTime(220) })
    expect(first).not.toHaveBeenCalled()
    expect(latest).toHaveBeenCalledOnce()
    expect(onReturnedToConversation).toHaveBeenCalledOnce()
  })

  it('cancels the old exit when another view takes over, including a keep-alive reopen', () => {
    const { result, syncConversationRoute } = setup()
    const staleAction = vi.fn()
    act(() => {
      result.current.exit.runAfterLeavingSettings(staleAction)
      result.current.exit.closeSettings()
    })
    act(() => { result.current.setChatView('notes') })
    expect(result.current.exit.settingsExiting).toBe(false)
    act(() => { result.current.setChatView('settings') })
    act(() => { vi.advanceTimersByTime(220) })
    expect(result.current.chatView).toBe('settings')
    expect(staleAction).not.toHaveBeenCalled()
    expect(syncConversationRoute).not.toHaveBeenCalled()
    act(() => { result.current.exit.closeSettings() })
    act(() => { vi.advanceTimersByTime(220) })
    expect(result.current.chatView).toBe('conversation')
    expect(staleAction).not.toHaveBeenCalled()
  })

  it('cancels pending navigation when the host unmounts', () => {
    const { result, unmount, syncConversationRoute, onReturnedToConversation } = setup()
    const action = vi.fn()
    act(() => {
      result.current.exit.runAfterLeavingSettings(action)
      result.current.exit.closeSettings()
    })
    unmount()
    act(() => { vi.advanceTimersByTime(220) })
    expect(action).not.toHaveBeenCalled()
    expect(syncConversationRoute).not.toHaveBeenCalled()
    expect(onReturnedToConversation).not.toHaveBeenCalled()
  })

  it('keeps navigation queued on save failure and leaves only after the retry commits', async () => {
    const initial: SettingsSnapshot = {
      settings: { theme: 'light', providers: [], chatTools: { servers: [] } } as unknown as Settings,
      version: { epoch: 'test', revision: 1 },
    }
    let commit!: (value: SettingsSnapshot) => void
    const pendingSave = new Promise<SettingsSnapshot>((resolve) => { commit = resolve })
    const save = vi.fn().mockRejectedValueOnce(new Error('disk unavailable')).mockReturnValueOnce(pendingSave)
    const controller = new SettingsEditorController({
      peek: () => initial,
      load: async () => initial,
      refresh: async () => initial,
      save,
      subscribe: () => () => {},
    })
    const { result, settingsRef, onReturnedToConversation } = setup()
    let closing: Promise<void> | undefined
    settingsRef.current.requestClose.mockImplementation(() => {
      closing = controller.requestClose(result.current.exit.closeSettings)
    })
    const action = vi.fn()
    try {
      controller.start()
      await Promise.resolve()
      controller.edit((draft) => ({ ...draft, theme: 'dark' }))
      await act(async () => {
        result.current.exit.runAfterLeavingSettings(action)
        await closing
      })
      act(() => { vi.advanceTimersByTime(500) })
      expect(result.current.chatView).toBe('settings')
      expect(result.current.exit.settingsExiting).toBe(false)
      expect(controller.snapshot.saveError).toContain('disk unavailable')
      expect(action).not.toHaveBeenCalled()

      act(() => { result.current.exit.runAfterLeavingSettings(action) })
      expect(result.current.exit.settingsExiting).toBe(false)
      await act(async () => {
        commit({ settings: { ...initial.settings, theme: 'dark' }, version: { epoch: 'test', revision: 2 } })
        await closing
      })
      expect(controller.snapshot.hasUnsavedChanges).toBe(false)
      expect(result.current.exit.settingsExiting).toBe(true)
      expect(action).not.toHaveBeenCalled()
      act(() => { vi.advanceTimersByTime(220) })
      expect(result.current.chatView).toBe('conversation')
      expect(action).toHaveBeenCalledOnce()
      expect(onReturnedToConversation).toHaveBeenCalledOnce()
    } finally {
      controller.dispose()
    }
  })
})
