import { describe, expect, it } from 'vitest'
import type { Settings } from '../api/tauri'
import { applyProviderDraftIntent } from './providerDraftIntents'

function settings(): Settings {
  return {
    providers: [
      { id: 'one', name: 'One', enabled: true, enabledModels: ['m1', 'm2'], availableModels: [] },
      { id: 'two', name: 'Two', enabled: true, enabledModels: ['m3'], availableModels: [] },
    ],
    providerIcons: { one: 'custom', two: 'other' },
    translatorProviderId: 'one',
    translatorModel: 'm1',
    chatProviderId: 'one',
    chatModel: 'm1',
    defaultModels: {
      chat: { providerId: 'one', model: 'm1' },
      vision: { providerId: 'one', model: 'm2' },
      videoAnalysis: { providerId: '', model: '' },
      titleSummary: { providerId: '', model: '' },
      compression: { providerId: '', model: '' },
      imageGeneration: { providerId: '', model: '' },
      promptOptimize: { providerId: '', model: '' },
      advisor: { providerId: '', model: '' },
    },
    screenshotTranslation: { providerId: 'one', model: 'm2' },
    lens: { providerId: 'one', model: 'm1' },
  } as unknown as Settings
}

describe('provider draft intents', () => {
  it('deleting a provider cascades model selection without leaving dangling references', () => {
    const next = applyProviderDraftIntent(settings(), { type: 'delete', id: 'one' })

    expect(next.providers.map((provider) => provider.id)).toEqual(['two'])
    expect(next.providerIcons).toEqual({ two: 'other' })
    expect(next.translatorProviderId).toBe('two')
    expect(next.translatorModel).toBe('m3')
    expect(next.defaultModels.chat).toEqual({ providerId: '', model: '' })
    expect(next.chatProviderId).toBe('')
    expect(next.screenshotTranslation.providerId).toBe('two')
    expect(next.lens.providerId).toBe('two')
  })

  it('removing a model chooses the next enabled model for every dependent selection', () => {
    const next = applyProviderDraftIntent(settings(), { type: 'remove-model', id: 'one', model: 'm1' })

    expect(next.providers[0].enabledModels).toEqual(['m2'])
    expect(next.translatorModel).toBe('m2')
    expect(next.defaultModels.chat).toEqual({ providerId: 'one', model: 'm2' })
    expect(next.chatModel).toBe('m2')
    expect(next.lens.model).toBe('m2')
    expect(next.screenshotTranslation.model).toBe('m2')
  })

  it('bulk model addition trims and deduplicates by case while keeping input order', () => {
    const next = applyProviderDraftIntent(settings(), {
      type: 'add-models', id: 'one', models: [' M2 ', 'm4', 'M4', '', ' m5 '],
    })

    expect(next.providers[0].enabledModels).toEqual(['m1', 'm2', 'm4', 'm5'])
  })

  it('ignores an edit to an unknown provider without creating a dirty draft', () => {
    const original = settings()
    expect(applyProviderDraftIntent(original, { type: 'update', id: 'missing', updates: { name: 'Absent' } })).toBe(original)
  })
})
