import { expect, it } from 'vitest'
import { isOpenCodeFree } from './tauri'
import { makeProvider } from '../settings/tabs/testFixtures'

it('enables anonymous OpenCode Free only on the official endpoint and chat protocol', () => {
  const provider = { ...makeProvider(), baseUrl: 'https://opencode.ai/zen/v1/', apiKeys: [] }
  expect(isOpenCodeFree(provider)).toBe(true)
  for (const baseUrl of ['https://opencode.ai/zen/go/v1', 'https://opencode.ai.evil/zen/v1', 'http://opencode.ai/zen/v1']) {
    expect(isOpenCodeFree({ ...provider, baseUrl })).toBe(false)
  }
  expect(isOpenCodeFree({ ...provider, apiKeys: ['paid-key'] })).toBe(false)
  expect(isOpenCodeFree({ ...provider, apiFormat: 'anthropic_messages' })).toBe(false)
  expect(isOpenCodeFree({ ...provider, request: { ...provider.request, oauth: { provider: 'kimi' } } })).toBe(false)
})
