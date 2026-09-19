import { act, renderHook } from '@testing-library/react'
import { useState } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { completeSettingsExit } from '../settingsExit'
import { useSettingsExit } from './useSettingsExit'

vi.mock('../settingsExit', () => ({
  completeSettingsExit: vi.fn(),
}))

const mockComplete = vi.mocked(completeSettingsExit)

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
  mockComplete.mockReset()
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
    expect(mockComplete).toHaveBeenCalledWith('c1', null, syncConversationRoute)
    expect(onReturnedToConversation).toHaveBeenCalled()
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
    const { result, settingsRef, onReturnedToConversation } = setup()
    const action = vi.fn()
    act(() => { result.current.exit.runAfterLeavingSettings(action, { restoreCurrentRoute: false }) })
    expect(action).not.toHaveBeenCalled()
    expect(settingsRef.current.requestClose).toHaveBeenCalledTimes(1)

    act(() => { result.current.exit.closeSettings() })
    act(() => { vi.advanceTimersByTime(220) })
    expect(mockComplete).toHaveBeenCalledWith(
      'c1',
      { action, restoreCurrentRoute: false },
      expect.any(Function),
    )
    expect(action).not.toHaveBeenCalled()
    expect(onReturnedToConversation).toHaveBeenCalled()
  })

  it('exits immediately and runs the action when SettingsShell is gone', () => {
    const { result, settingsRef, syncConversationRoute } = setup()
    settingsRef.current = null as never
    const action = vi.fn()
    act(() => { result.current.exit.runAfterLeavingSettings(action) })
    expect(result.current.chatView).toBe('conversation')
    expect(mockComplete).toHaveBeenCalledWith(
      'c1',
      { action, restoreCurrentRoute: true },
      syncConversationRoute,
    )
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
})
