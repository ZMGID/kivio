import { act, renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { getRememberedChatSidebarCollapsed, getRememberedSidebarWidth } from '../persistence'
import { isTauriRuntime } from '../utils'
import { useSidebarLayout } from './useSidebarLayout'

vi.mock('../chatPerformanceProbe', () => ({
  measureChatSurface: vi.fn(() => () => {}),
}))
vi.mock('../utils', () => ({
  isTauriRuntime: () => false,
}))

function mountShell() {
  const shell = document.createElement('div')
  shell.className = 'chat-window-shell'
  document.body.appendChild(shell)
  return shell
}

beforeEach(() => {
  window.localStorage.clear()
  document.body.innerHTML = ''
})

describe('useSidebarLayout', () => {
  it('round-trips collapse and width through localStorage', () => {
    const { result } = renderHook(() => useSidebarLayout())
    expect(result.current.collapsed).toBe(false)

    act(() => { result.current.setCollapsed(true) })
    expect(result.current.collapsed).toBe(true)
    expect(getRememberedChatSidebarCollapsed()).toBe(true)

    act(() => { result.current.setWidth(300) })
    expect(result.current.width).toBe(300)
    expect(getRememberedSidebarWidth()).toBe(300)

    act(() => { result.current.collapse() })
    expect(result.current.collapsed).toBe(true)
  })

  it('writes --chat-sidebar-width on the shell', () => {
    const shell = mountShell()
    const { result } = renderHook(() => useSidebarLayout())
    act(() => { result.current.setWidth(288) })
    expect(shell.style.getPropertyValue('--chat-sidebar-width')).toBe('288px')
  })

  it('does not touch the native window when not in Tauri', async () => {
    renderHook(() => useSidebarLayout())
    await act(async () => { await Promise.resolve() })
    expect(isTauriRuntime()).toBe(false)
  })
})
