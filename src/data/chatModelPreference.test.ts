import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  LAST_MODEL_KEY,
  loadLastModel,
  resolvePreferredChatModel,
  saveLastModel,
} from './chatModelPreference'

const google = { providerId: 'google', model: 'models/gemini-3.1-flash-lite' }
const deepseek = { providerId: 'ds', model: 'deepseek-v4-flash' }
const lens = { providerId: 'google', model: 'gemini-flash' }
const translator = { providerId: 'google', model: 'gemini-flash-lite' }
const providers = [
  { id: 'google', enabledModels: [google.model, lens.model, translator.model] },
  { id: 'ds', enabledModels: [deepseek.model] },
]

describe('resolvePreferredChatModel', () => {
  it('优先用聊天界面上次选的模型，而不是 settings 里的 defaultModels.chat', () => {
    expect(resolvePreferredChatModel({
      providers,
      last: deepseek,
      storedChat: google,
      legacyChat: google,
      lens,
      translator,
    })).toEqual(deepseek)
  })

  it('上次选择的供应商已删则回落到已写入的 last-used', () => {
    expect(resolvePreferredChatModel({
      providers: [providers[0]],
      last: deepseek,
      storedChat: google,
      legacyChat: { providerId: '', model: '' },
      lens,
      translator,
    })).toEqual(google)
  })

  it('没有上次选择时回落 Lens，而不是要求引导里的 Chat 默认模型', () => {
    expect(resolvePreferredChatModel({
      providers,
      last: null,
      storedChat: { providerId: '', model: '' },
      legacyChat: { providerId: '', model: '' },
      lens,
      translator,
    })).toEqual(lens)
  })

  it.each([
    { id: 'ds', enabledModels: [deepseek.model] },
    { id: 'ds', enabledModels: ['gpt-4o'], enabled: false },
    { id: 'ds', enabledModels: [] },
  ])('ignores stale or disabled last-used models before the configured Lens model ($enabledModels)', provider => {
    expect(resolvePreferredChatModel({
      providers: [providers[0], provider],
      last: { providerId: 'ds', model: 'gpt-4o' },
      storedChat: { providerId: 'ds', model: 'gpt-4o' },
      legacyChat: { providerId: 'ds', model: 'gpt-4o' },
      lens,
      translator,
    })).toEqual(lens)
  })

  it('uses an enabled model when every saved binding is stale, and stays empty without models', () => {
    const input = {
      last: null,
      storedChat: { providerId: '', model: '' },
      legacyChat: { providerId: '', model: '' },
      lens: { providerId: '', model: '' },
      translator: { providerId: 'ds', model: 'gpt-4o' },
    }
    expect(resolvePreferredChatModel({ ...input, providers: [providers[1]] })).toEqual(deepseek)
    expect(resolvePreferredChatModel({ ...input, providers: [] })).toEqual({ providerId: '', model: '' })
  })
})

describe('loadLastModel / saveLastModel', () => {
  const memory = new Map<string, string>()

  beforeEach(() => {
    memory.clear()
    vi.stubGlobal('window', {
      localStorage: {
        getItem: (key: string) => memory.get(key) ?? null,
        setItem: (key: string, value: string) => {
          memory.set(key, String(value))
        },
        removeItem: (key: string) => {
          memory.delete(key)
        },
      },
    })
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('读写顶栏最后一次选择', () => {
    saveLastModel('ds', 'deepseek-v4-flash')
    expect(loadLastModel()).toEqual(deepseek)
  })

  it('垃圾数据返回 null', () => {
    memory.set(LAST_MODEL_KEY, '{')
    expect(loadLastModel()).toBeNull()
  })
})
