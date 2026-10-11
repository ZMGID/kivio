import { afterEach, expect, it, vi } from 'vitest'

afterEach(() => {
  vi.restoreAllMocks()
  window.sessionStorage.clear()
  window.history.replaceState({}, '', '/')
})

it('restores each workspace draft across webview reloads, including new-chat drafts', async () => {
  const listeners = vi.spyOn(window, 'addEventListener')
  const unload = () => {
    window.dispatchEvent(new Event('pagehide'))
    for (const [type, listener] of listeners.mock.calls) {
      if (type === 'pagehide') window.removeEventListener(type, listener)
    }
    listeners.mockClear()
    vi.resetModules()
  }
  const agent = await import('./composerDraft')
  const draft = { input: 'Agent 草稿', quotes: ['引用'], attachments: [] }
  agent.setComposerDraft(agent.draftKey(null), draft)
  unload()
  window.history.replaceState({}, '', '/?workspace=bot#chat')
  const bot = await import('./composerDraft')
  expect(bot.getComposerDraft(bot.draftKey(null))).toBeUndefined()
  bot.setComposerDraft('conv_bot_writer', { ...draft, input: 'Bot 草稿' })
  unload()
  window.history.replaceState({}, '', '/#chat')
  const restoredAgent = await import('./composerDraft')
  expect(restoredAgent.getComposerDraft(restoredAgent.draftKey(null))).toEqual(draft)
  expect(restoredAgent.getComposerDraft('conv_bot_writer')).toBeUndefined()
  unload()
  window.history.replaceState({}, '', '/?workspace=bot#chat')
  const restoredBot = await import('./composerDraft')
  expect(restoredBot.getComposerDraft('conv_bot_writer')?.input).toBe('Bot 草稿')
  unload()
})
