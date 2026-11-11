import { describe, expect, it } from 'vitest'
import { matchModelExact, resolveModelInfo } from './modelMatching'

describe('November model database refresh', () => {
  it('recognizes new models with their verified controls and limits', () => {
    for (const id of ['claude-fable-5-2', 'anthropic/claude-fable-5.2', 'claude-mythos-5-2']) {
      expect(matchModelExact(id), id).toMatchObject({
        contextWindow: 1000000, maxOutput: 128000,
        reasoningEfforts: ['low', 'medium', 'high', 'xhigh', 'max'],
        pricing: { input: 8, output: 40, cachedInput: 0.2 },
      })
    }
    for (const id of ['gemini-4-carbon', 'google/gemini-4-carbon']) {
      expect(matchModelExact(id), id).toMatchObject({
        contextWindow: 1048576, maxOutput: 65536,
        capabilities: { vision: true, videoInput: true, functionCalling: true },
        reasoningEfforts: ['low', 'medium', 'high'], pricing: { input: 2, output: 10 },
      })
    }
    for (const id of ['gpt-image-2.6-flare', 'gpt-image-2.6-sunburst']) {
      expect(matchModelExact(id)?.capabilities).toMatchObject({ vision: true, imageGeneration: true })
      expect(matchModelExact(id)?.pricing?.input).toBeUndefined()
      expect(matchModelExact(id)?.pricing?.output).toBeUndefined()
    }
    expect(matchModelExact('cohere/embed-5-pro')).toMatchObject({ dimensions: 2048, multilingual: true })
  })

  it('keeps corrected capacities and lets user settings override new defaults', () => {
    expect(matchModelExact('meta-llama/llama-3-8b-instruct')).toMatchObject({ contextWindow: 8192, maxOutput: 8192 })
    expect(matchModelExact('ministral-3b-2512')?.contextWindow).toBe(262144)
    expect(resolveModelInfo('gemini-4-carbon', {
      'gemini-4-carbon': { contextWindow: 2000000, pricing: { input: 9 } },
    })).toMatchObject({ contextWindow: 2000000, maxOutput: 65536, pricing: { input: 9, output: 10 } })
  })
})
