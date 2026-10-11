// @vitest-environment jsdom
import { act, renderHook, waitFor } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { api } from '../../api/tauri'
import { chatApi } from '../api'
import { createBlankAssistant } from '../assistantDraftStore'
import type { ChatAssistant } from '../types'
import { useCurrentBotAppearance } from './useCurrentBotAppearance'

afterEach(() => vi.restoreAllMocks())

it('refreshes saved identity without flashing a default, and fences late reads after contact changes', async () => {
  const a = { ...createBlankAssistant('Ada'), id: 'a', icon: 'kv1:213', color: '#6E9E80' }
  const b = { ...createBlankAssistant('Bea'), id: 'b', icon: 'kv1:162', color: '#9A7CB8' }
  let changed: (() => void) | undefined
  vi.spyOn(api, 'onChatAssistantsChanged').mockImplementation(async handler => {
    changed = () => handler('b')
    return () => {}
  })
  let finishOld: ((profiles: ChatAssistant[]) => void) | undefined
  const read = vi.spyOn(chatApi, 'getAssistants').mockResolvedValue([a, b])
  const { result, rerender } = renderHook(({ id }) => useCurrentBotAppearance(id), { initialProps: { id: 'a' as string | null } })
  await waitFor(() => expect(result.current?.icon).toBe('kv1:213'))
  read.mockImplementationOnce(() => new Promise(resolve => { finishOld = resolve }))
  act(() => changed?.())
  expect(result.current?.color).toBe('#6E9E80')
  rerender({ id: 'b' })
  expect(result.current).toBeUndefined()
  await waitFor(() => expect(result.current?.icon).toBe('kv1:162'))
  await act(async () => finishOld?.([{ ...a, icon: 'kv1:000' }]))
  expect(result.current?.id).toBe('b')
  read.mockResolvedValue([{ ...b, icon: 'kv1:500', color: '#C39B55' }])
  act(() => changed?.())
  await waitFor(() => expect(result.current?.icon).toBe('kv1:500'))
  expect(result.current?.color).toBe('#C39B55')
  rerender({ id: null })
  expect(result.current).toBeUndefined()
})
