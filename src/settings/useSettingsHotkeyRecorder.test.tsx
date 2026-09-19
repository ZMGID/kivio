import { act, renderHook } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import type { Settings } from '../api/tauri'
import { useSettingsHotkeyRecorder } from './useSettingsHotkeyRecorder'

describe('useSettingsHotkeyRecorder', () => {
  it('cancels on Escape without editing and captures one hotkey in its domain', () => {
    const edit = vi.fn()
    const { result } = renderHook(() => useSettingsHotkeyRecorder(edit))
    act(() => { result.current.toggle('screenshotTranslationText') })
    act(() => { window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })) })
    expect(result.current.target).toBeNull()
    expect(edit).not.toHaveBeenCalled()

    act(() => { result.current.toggle('screenshotTranslationText') })
    act(() => { window.dispatchEvent(new KeyboardEvent('keydown', { key: 't', code: 'KeyT', ctrlKey: true, bubbles: true })) })
    expect(result.current.target).toBeNull()
    expect(edit).toHaveBeenCalledOnce()
    const update = edit.mock.calls[0][0] as (settings: Settings) => Settings
    const original = { screenshotTranslation: { textHotkey: 'old' } } as Settings
    expect(update(original).screenshotTranslation.textHotkey).toBe('CommandOrControl+T')
    expect(original.screenshotTranslation.textHotkey).toBe('old')
  })

  it('only the active target receives a shortcut, and toggling it off removes capture', () => {
    const edit = vi.fn()
    const { result } = renderHook(() => useSettingsHotkeyRecorder(edit))
    act(() => { result.current.toggle('main'); result.current.toggle('main') })
    act(() => { window.dispatchEvent(new KeyboardEvent('keydown', { key: 'x', code: 'KeyX', ctrlKey: true, bubbles: true })) })
    expect(edit).not.toHaveBeenCalled()
    act(() => { result.current.toggle('lens') })
    act(() => { window.dispatchEvent(new KeyboardEvent('keydown', { key: 'l', code: 'KeyL', ctrlKey: true, bubbles: true })) })
    const update = edit.mock.calls[0][0] as (settings: Settings) => Settings
    const original = { lens: { hotkey: 'old' }, hotkey: 'old-main' } as Settings
    expect(update(original).lens.hotkey).toBe('CommandOrControl+L')
    expect(update(original).hotkey).toBe('old-main')
  })
})
