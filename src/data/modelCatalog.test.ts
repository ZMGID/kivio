import { describe, expect, it } from 'vitest'
import type { ModelProvider } from '../api/tauri'
import { applyModelCatalog } from './modelCatalog'
import { resolveModelInfo } from './modelMatching'

const provider: ModelProvider = {
  id: 'test', name: 'Test', baseUrl: 'https://example.com/v1', apiKeys: [],
  apiFormat: 'openai_chat', availableModels: [], enabledModels: [], enabled: true,
}

describe('imported model video capabilities', () => {
  it.each([
    'kimi-k2.5', 'kimi-k2.6', 'kimi-k2.7-code',
    'kimi-k2.7-code-highspeed', 'moonshotai/kimi-k2.7-code', 'kimi-k3',
    'gemini-3-pro-preview', 'models/gemini-3-pro-preview',
    'gemini-3.1-pro-preview', 'gemini-3.8-flash', 'google/gemini-2.5-pro',
  ])('enables video for %s when the catalog contains only IDs', (id) => {
    const imported = applyModelCatalog(provider, { models: [id], capabilities: {} })
    expect(resolveModelInfo(id, imported.modelOverrides, provider).capabilities?.videoInput).toBe(true)
  })

  it('preserves explicit settings while importing advertised capabilities', () => {
    const id = 'gemini-2.5-pro'
    const imported = applyModelCatalog({ ...provider, modelOverrides: {
      [id]: { capabilities: { videoInput: false } },
    } }, { models: [id], capabilities: { [id]: { videoInput: true } } })
    expect(resolveModelInfo(id, imported.modelOverrides, provider).capabilities?.videoInput).toBe(false)
  })

  it('uses advertised capabilities for unknown IDs and honors advertised false', () => {
    const imported = applyModelCatalog(provider, { models: ['private-model', 'kimi-k3'],
      capabilities: { 'private-model': { videoInput: true }, 'kimi-k3': { videoInput: false } },
    })
    expect(resolveModelInfo('private-model', imported.modelOverrides, provider).capabilities?.videoInput).toBe(true)
    expect(resolveModelInfo('kimi-k3', imported.modelOverrides, provider).capabilities?.videoInput).toBe(false)
  })

  it('does not enable video merely because a model accepts images', () => {
    for (const id of ['gemini-3-pro-image-preview', 'gemini-embedding-001', 'kimi-k2', 'kimi-k2.7', 'gpt-4o']) {
      const imported = applyModelCatalog(provider, { models: [id], capabilities: {} })
      expect(resolveModelInfo(id, imported.modelOverrides, provider).capabilities?.videoInput).not.toBe(true)
    }
  })
})
