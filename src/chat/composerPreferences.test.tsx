import { beforeEach, describe, expect, it, vi } from 'vitest'
import { updateSettingsCached } from '../api/settingsCache'
import {
  LAST_THINKING_KEY,
  LAST_WEB_SEARCH_MODE_KEY,
  loadLastThinkingLevel,
  loadLastWebSearchMode,
  persistLastChatModelToSettings,
  saveLastThinkingLevel,
  saveLastWebSearchMode,
} from './composerPreferences'

vi.mock('../api/settingsCache', () => ({
  updateSettingsCached: vi.fn(),
}))

const mockUpdate = vi.mocked(updateSettingsCached)

beforeEach(() => {
  window.localStorage.clear()
  mockUpdate.mockReset()
  mockUpdate.mockImplementation(async (mutate) => mutate({
    defaultModels: { chat: { providerId: 'old', model: 'old-model' } },
  } as never) as never)
})

describe('composerPreferences: thinking level', () => {
  it('round-trips a valid level and clears on null', () => {
    saveLastThinkingLevel('xhigh')
    expect(loadLastThinkingLevel()).toBe('xhigh')
    saveLastThinkingLevel(null)
    expect(window.localStorage.getItem(LAST_THINKING_KEY)).toBeNull()
    expect(loadLastThinkingLevel()).toBeNull()
  })

  it('rejects unknown stored values instead of trusting localStorage', () => {
    window.localStorage.setItem(LAST_THINKING_KEY, 'turbo')
    expect(loadLastThinkingLevel()).toBeNull()
  })
})

describe('composerPreferences: web search mode', () => {
  it('round-trips a valid mode', () => {
    saveLastWebSearchMode('builtin')
    expect(loadLastWebSearchMode()).toBe('builtin')
  })

  it('returns undefined for unknown stored values', () => {
    window.localStorage.setItem(LAST_WEB_SEARCH_MODE_KEY, 'everything')
    expect(loadLastWebSearchMode()).toBeUndefined()
  })
})

describe('persistLastChatModelToSettings', () => {
  it('ignores blank provider ids', async () => {
    await persistLastChatModelToSettings('  ', 'm')
    expect(mockUpdate).not.toHaveBeenCalled()
  })

  it('writes default chat model plus legacy fields when they differ', async () => {
    await persistLastChatModelToSettings('p', 'm')
    expect(mockUpdate).toHaveBeenCalledTimes(1)
    const mutate = mockUpdate.mock.calls[0][0]
    const next = mutate({ defaultModels: { chat: { providerId: 'old', model: 'old-model' } } } as never)
    expect(next).toMatchObject({
      defaultModels: { chat: { providerId: 'p', model: 'm' } },
      chatProviderId: 'p',
      chatModel: 'm',
    })
  })

  it('returns the same settings object when nothing changes', async () => {
    await persistLastChatModelToSettings('old', 'old-model')
    const mutate = mockUpdate.mock.calls[0][0]
    const settings = { defaultModels: { chat: { providerId: 'old', model: 'old-model' } } } as never
    expect(mutate(settings)).toBe(settings)
  })

  it('swallows persistence errors (best-effort mirror, not a user action)', async () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    mockUpdate.mockRejectedValueOnce(new Error('disk'))
    await expect(persistLastChatModelToSettings('p', 'm')).resolves.toBeUndefined()
    expect(error).toHaveBeenCalled()
    error.mockRestore()
  })
})
