// @vitest-environment jsdom
import { afterEach, expect, it } from 'vitest'
import { chatApi } from './api'
import { createBlankAssistant } from './assistantDraftStore'

afterEach(() => {
  localStorage.removeItem('kivio-chat-dev-conversations')
  localStorage.removeItem('kivio-chat-dev-assistants')
})

it('keeps one durable Bot history through concurrent opens, paging, profile edits and hide/restore', async () => {
  const assistant = await chatApi.createAssistant({ ...createBlankAssistant('研究员'), system_prompt: '给出来源' })
  const ordinary = await chatApi.createConversation(undefined, undefined, undefined, null, assistant.id)
  const opens = await Promise.all(Array.from({ length: 3 }, () => chatApi.openBot(assistant.id)))
  expect(new Set(opens.map(item => item.id)).size).toBe(1)
  const id = opens[0].id
  expect(id).not.toBe(ordinary.id)
  expect(await chatApi.getBotConversations()).toHaveLength(1)
  for (let turn = 0; turn < 35; turn++) await chatApi.sendMessage(id, `研究问题 ${turn}`)
  await chatApi.sendMessage(ordinary.id, '研究问题 0')
  expect((await chatApi.searchConversations('研究问题 0', 1, true)).map(hit => hit.id)).toEqual([id])
  const window = await chatApi.openBot(assistant.id)
  expect(window.history_total).toBe(70)
  expect(window.messages).toHaveLength(60)
  const older = await chatApi.getConversationPage(id, window.history_start!)
  expect(older.messages[0].content).toBe('研究问题 0')
  expect(window.messages[window.messages.length - 2].content).toBe('研究问题 34')

  const edited = { ...assistant, name: '资料研究员', system_prompt: '先核验再回答', provider_id: 'profile-provider', model: 'profile-model' }
  await chatApi.updateAssistant(edited)
  expect((await chatApi.getConversation(id)).model).toBe('profile-model')
  await chatApi.updateConversation(id, { providerId: 'manual-provider', model: 'manual-model' })
  await chatApi.updateAssistant({ ...edited, description: '持续查证' })
  expect((await chatApi.getConversation(id)).model).toBe('manual-model')
  await chatApi.updateAssistant({ ...edited, provider_id: '', model: '' })
  expect((await chatApi.getConversation(id)).model).toBe('dev-model')
  expect((await chatApi.getConversation(id)).assistant_snapshot?.system_prompt).toBe('先核验再回答')
  expect((await chatApi.getConversation(ordinary.id)).assistant_snapshot?.system_prompt).toBe('给出来源')
  await chatApi.updateConversation(id, { pinned: true, archived: true })
  expect((await chatApi.getBotConversations())[0]).toMatchObject({ archived: true, pinned: true })
  const restored = await chatApi.openBot(assistant.id)
  expect(restored.id).toBe(id)
  expect(restored.archived).toBe(false)
  expect(restored.history_total).toBe(70)
  expect((await chatApi.getBotConversations())[0].assistant_name).toBe('资料研究员')
  await expect(chatApi.updateConversation(id, { assistantId: '' })).rejects.toThrow('身份不能更换')
  expect((await chatApi.getConversation(id)).messages).toHaveLength(70)
})
