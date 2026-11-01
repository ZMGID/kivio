import { describe, expect, it } from 'vitest'
import {
  buildModelPairOptions,
  formatHotkey,
  modelPairValue,
  parseModelPairValue,
} from './utils'
import type { ModelProvider } from '../api/tauri'

describe('formatHotkey', () => {
  it('renders macOS modifier glyphs', () => {
    expect(formatHotkey('CommandOrControl+Shift+T', 'macos')).toEqual(['⌘', '⇧', 'T'])
    expect(formatHotkey('Alt+Space', 'macos')).toEqual(['⌥', 'Space'])
  })

  it('renders Windows modifier labels', () => {
    expect(formatHotkey('CommandOrControl+Shift+T', 'windows')).toEqual(['Ctrl', 'Shift', 'T'])
    expect(formatHotkey('Alt+Space', 'windows')).toEqual(['Alt', 'Space'])
  })

  it('maps arrow keys to symbols on macOS', () => {
    expect(formatHotkey('ArrowUp', 'macos')).toEqual(['↑'])
    expect(formatHotkey('ArrowDown', 'macos')).toEqual(['↓'])
  })
})

describe('modelPairValue', () => {
  it('serializes provider and model as JSON array', () => {
    expect(modelPairValue('openai', 'gpt-4o')).toBe('["openai","gpt-4o"]')
  })
})

describe('parseModelPairValue', () => {
  it('parses JSON array values', () => {
    expect(parseModelPairValue('["openai","gpt-4o"]')).toEqual(['openai', 'gpt-4o'])
  })

  it('parses legacy provider:model values', () => {
    expect(parseModelPairValue('openai:gpt-4o')).toEqual(['openai', 'gpt-4o'])
  })

  it('returns model-less pair when no separator exists', () => {
    expect(parseModelPairValue('openai')).toEqual(['openai', ''])
  })
})

describe('buildModelPairOptions', () => {
  const provider = {
    id: 'p1',
    name: 'Proxy',
    enabled: true,
    enabledModels: ['image-model', 'chat-model', 'other-model'],
  } as unknown as ModelProvider

  it('lists all enabled models without a filter', () => {
    expect(buildModelPairOptions([provider]).map(o => o.value)).toEqual([
      '["p1","image-model"]',
      '["p1","chat-model"]',
      '["p1","other-model"]',
    ])
  })

  it('keeps only models passing the filter predicate', () => {
    const opts = buildModelPairOptions([provider], (_p, model) => model.includes('image'))
    expect(opts.map(o => o.value)).toEqual(['["p1","image-model"]'])
  })
})
