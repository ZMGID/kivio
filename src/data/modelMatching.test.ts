import { describe, expect, it, vi } from 'vitest'
import { matchModel, matchModelExact, resolveModelInfo } from './modelMatching'

vi.mock('./modelDatabase.json', () => ({
  default: {
    _meta: { version: 'test' },
    'model-4': { displayName: 'Model 4', contextWindow: 100 },
    'model-4.6': {
      displayName: 'Model 4.6', contextWindow: 200, maxOutput: 60, temperature: 0.7,
      capabilities: { vision: true, videoInput: true, functionCalling: true, reasoning: true, streaming: true, embedding: true },
      dimensions: 1024, multilingual: true, reasoningEfforts: ['low', 'high'],
      pricing: { input: 2, output: 4, cachedInput: 0.2 },
    },
    'model-4.6-fast': { displayName: 'Model 4.6 Fast', contextWindow: 300 },
    'model-4.6-free': { displayName: 'Model 4.6 Free', contextWindow: 300 },
    'vendor/model-4.6': { displayName: 'Vendor Model 4.6', contextWindow: 400 },
    '[self-hosted]model/model-4.6-private': { displayName: 'Private Model 4.6', contextWindow: 500 },
    'kimi-code/k3': { displayName: 'Coding Model', contextWindow: 600, reasoningEfforts: ['low', 'high'] },
    'kimi-code/k3-256k': { displayName: 'Coding Model Small', contextWindow: 700 },
  },
}))

describe('matchModel', () => {
  it('prefers qualified entries and normalizes version separators without losing the route', () => {
    expect(matchModelExact('vendor/model-4-6')?.displayName).toBe('Vendor Model 4.6')
    expect(matchModelExact('model-4-6')?.displayName).toBe('Model 4.6')
    expect(matchModel('vendor/model-4-6')).toEqual(matchModelExact('vendor/model-4.6'))
  })

  it('matches provider-prefixed and case-insensitive IDs without inferring exact metadata for private aliases', () => {
    expect(matchModelExact(' Other/MODEL-4.6 ')).toEqual(matchModelExact('model-4.6'))
    expect(matchModelExact('company-model-4.6-fast-special')).toBeNull()
  })

  it('uses the longest normalized prefix for dated and parameterized variants', () => {
    expect(matchModel('model-4-6-fast-20260101')).toEqual(matchModelExact('model-4.6-fast'))
    expect(matchModel('model-4.6-high')).toEqual(matchModelExact('model-4.6'))
    expect(matchModel('model-4.6[fast=true]')).toEqual(matchModelExact('model-4.6'))
  })

  it('prefers a prefix to a contained match and otherwise picks the longest contained match', () => {
    expect(matchModel('model-4-alias-model-4.6')).toEqual(matchModelExact('model-4'))
    expect(matchModel('company-model-4.6-fast-special')).toEqual(matchModelExact('model-4.6-fast'))
  })

  it('keeps major and known minor versions distinct and does not collapse an unknown minor', () => {
    expect(matchModel('model-4')).toEqual(matchModelExact('model-4'))
    expect(matchModel('model-4.6')).toEqual(matchModelExact('model-4.6'))
    expect(matchModel('model-4.7-nebula')).toBeNull()
  })

  it('does not inherit commercial prices for exact free or self-hosted variants', () => {
    const free = matchModelExact('model-4.6-free')
    expect(free?.displayName).toBe('Model 4.6 Free')
    expect(free?.pricing?.input).toBeUndefined()
    expect(free?.pricing?.output).toBeUndefined()
    expect(matchModel('opencode/model-4.6-free')).toEqual(free)
    const privateModel = matchModel('[self-hosted]model/model-4.6-private')
    expect(privateModel?.displayName).toBe('Private Model 4.6')
    expect(privateModel?.pricing?.input).toBeUndefined()
    expect(privateModel?.pricing?.output).toBeUndefined()
  })

  it('returns null for blank model names', () => {
    expect(matchModel('')).toBeNull()
    expect(matchModel('   ')).toBeNull()
  })

  it('recognizes uncatalogued image generation naming patterns', () => {
    for (const id of ['gpt-image-new', 'grok-imagine-image-new', 'nano-banana-new', 'qwen-image-new']) {
      expect(matchModelExact(id)).toBeNull()
      expect(matchModel(id)?.capabilities).toMatchObject({ vision: true, imageGeneration: true })
    }
    expect(matchModel('dall-e-3')?.capabilities).toMatchObject({ vision: false, imageGeneration: true })
    expect(matchModel('other/gpt-image-new')?.displayName).toBe('GPT Image New')
  })

  it('copies catalog metadata including embedding fields and prices', () => {
    expect(matchModelExact('model-4.6')).toMatchObject({
      displayName: 'Model 4.6', contextWindow: 200, maxOutput: 60, temperature: 0.7,
      capabilities: { vision: true, videoInput: true, functionCalling: true, reasoning: true, streaming: true, embedding: true },
      dimensions: 1024, multilingual: true, reasoningEfforts: ['low', 'high'],
      pricing: { input: 2, output: 4, cachedInput: 0.2 },
    })
    expect(matchModelExact('model-4')?.capabilities).toMatchObject({ vision: false, videoInput: false, embedding: false })
  })
})

describe('resolveModelInfo', () => {
  it('resolves Kimi Code short IDs only with provider context and preserves overrides', () => {
    const provider = { baseUrl: 'https://api.kimi.com/coding/v1' }
    expect(resolveModelInfo('k3', undefined, provider)).toEqual(matchModelExact('kimi-code/k3'))
    expect(resolveModelInfo('k3-256k', undefined, provider)).toEqual(matchModelExact('kimi-code/k3-256k'))
    expect(resolveModelInfo('k3', undefined, { baseUrl: '', request: { oauth: { provider: 'kimi' } } }))
      .toEqual(matchModelExact('kimi-code/k3'))
    expect(resolveModelInfo('k3', { k3: { contextWindow: 1234 } }, provider).contextWindow).toBe(1234)
    expect(matchModel('k3')).toBeNull()
    expect(resolveModelInfo('k3', undefined, { baseUrl: 'https://other.example/v1' })).toEqual({})
  })

  it('uses catalog defaults for different providers while preserving explicit overrides', () => {
    const codex = { baseUrl: 'https://chatgpt.com/backend-api/codex', request: { oauth: { provider: 'codex' as const } } }
    for (const provider of [undefined, codex, { baseUrl: 'https://api.openai.com/v1' }, { baseUrl: 'https://gateway.example/v1' }]) {
      expect(resolveModelInfo('model-4.6', undefined, provider)).toEqual(matchModelExact('model-4.6'))
      expect(resolveModelInfo('model-4.6', { 'model-4.6': { contextWindow: 8192 } }, provider).contextWindow).toBe(8192)
    }
  })

  it('merges user overrides with untouched database defaults, including nested fields', () => {
    const resolved = resolveModelInfo('model-4.6', {
      'model-4.6': {
        displayName: 'Custom', capabilities: { vision: false }, pricing: { input: 9 }, reasoningEfforts: [],
      },
    })
    expect(resolved).toMatchObject({
      displayName: 'Custom', contextWindow: 200, maxOutput: 60,
      capabilities: { vision: false, videoInput: true, embedding: true }, dimensions: 1024, multilingual: true,
      pricing: { input: 9, output: 4, cachedInput: 0.2 }, reasoningEfforts: [],
    })
  })

  it('returns override-only info when the database has no match', () => {
    const resolved = resolveModelInfo('custom-local-model', {
      'custom-local-model': { displayName: 'Local', contextWindow: 8192 },
    })
    expect(resolved).toEqual({ displayName: 'Local', contextWindow: 8192 })
  })

  it('leaves temperature absent when neither the database nor overrides define it', () => {
    expect(resolveModelInfo('model-4').temperature).toBeUndefined()
  })

  it('uses a numeric temperature override', () => {
    const resolved = resolveModelInfo('model-4.6', { 'model-4.6': { temperature: 0.4 } })
    expect(resolved.temperature).toBe(0.4)
    expect(resolved.omitTemperature).toBeUndefined()
  })

  it('uses omitTemperature as an explicit blank tombstone', () => {
    const resolved = resolveModelInfo('model-4.6', { 'model-4.6': { temperature: 0.4, omitTemperature: true } })
    expect(resolved.temperature).toBeUndefined()
    expect(resolved.omitTemperature).toBe(true)
  })
})
