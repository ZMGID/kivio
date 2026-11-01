import { describe, expect, it, vi } from 'vitest'
import type { ModelProvider } from '../api/tauri'
import { applyModelCatalog } from './modelCatalog'
import { resolveModelInfo } from './modelMatching'
import { createProviderRequestDraft } from '../settings/public/providerDraft'

vi.mock('./modelDatabase.json', () => ({
  default: {
    _meta: { version: 'test' },
    'known-video': { capabilities: { videoInput: true } },
    'known-image': { capabilities: { vision: true } },
  },
}))

const provider: ModelProvider = {
  id: 'test', name: 'Test', baseUrl: 'https://example.com/v1', apiKeys: [],
  apiFormat: 'openai_chat', availableModels: [], enabledModels: [], enabled: true,
  request: createProviderRequestDraft(),
}

describe('imported model video capabilities', () => {
  it('retains catalog video capability when an imported model has only an ID', () => {
    const imported = applyModelCatalog(provider, { models: ['known-video'], capabilities: {} })
    expect(resolveModelInfo('known-video', imported.modelOverrides, provider).capabilities?.videoInput).toBe(true)
  })

  it('preserves explicit settings while importing advertised capabilities', () => {
    const id = 'known-video'
    const imported = applyModelCatalog({ ...provider, modelOverrides: {
      [id]: { capabilities: { videoInput: false } },
    } }, { models: [id], capabilities: { [id]: { videoInput: true } } })
    expect(resolveModelInfo(id, imported.modelOverrides, provider).capabilities?.videoInput).toBe(false)
  })

  it('uses advertised capabilities for unknown IDs and honors advertised false over catalog defaults', () => {
    const imported = applyModelCatalog(provider, { models: ['private-model', 'known-video'],
      capabilities: { 'private-model': { videoInput: true }, 'known-video': { videoInput: false } },
    })
    expect(resolveModelInfo('private-model', imported.modelOverrides, provider).capabilities?.videoInput).toBe(true)
    expect(resolveModelInfo('known-video', imported.modelOverrides, provider).capabilities?.videoInput).toBe(false)
  })

  it('does not enable video merely because a model accepts images', () => {
    const imported = applyModelCatalog(provider, { models: ['known-image'], capabilities: {} })
    expect(resolveModelInfo('known-image', imported.modelOverrides, provider).capabilities)
      .toMatchObject({ vision: true, videoInput: false })
  })
})
