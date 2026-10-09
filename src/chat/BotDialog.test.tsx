import { useState } from 'react'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { BotDialog } from './BotDialog'
import { chatApi } from './api'
import { abandonAssistantEdit, beginAssistantEdit, createBlankAssistant, resetAssistantDraftStore } from './assistantDraftStore'

beforeEach(() => {
  resetAssistantDraftStore()
  HTMLDialogElement.prototype.showModal ??= function () { this.setAttribute('open', '') }
})
afterEach(() => {
  vi.restoreAllMocks()
  resetAssistantDraftStore()
  localStorage.removeItem('kivio-chat-dev-assistants')
  localStorage.removeItem('kivio-chat-dev-conversations')
})

function Creation() {
  const [open, setOpen] = useState(false)
  const [selected, setSelected] = useState('')
  return <>
    <button onClick={() => { beginAssistantEdit(createBlankAssistant('资料 Bot'), false); setOpen(true) }}>添加</button>
    <output>{selected}</output>
    {open && <BotDialog onClose={() => { abandonAssistantEdit(); setOpen(false) }} onAdvanced={() => {}}
      onSaved={async assistant => { const conversation = await chatApi.openBot(assistant.id); setSelected(conversation.assistant_snapshot?.name ?? '') }} />}
  </>
}

it('cancels without creating a contact and retries an open failure without duplicating the saved Bot', async () => {
  const user = userEvent.setup()
  render(<Creation />)
  await user.click(screen.getByRole('button', { name: '添加' }))
  expect(screen.getByRole('textbox', { name: 'Bot 名称' })).toHaveFocus()
  await user.click(screen.getByRole('button', { name: '取消' }))
  expect((await chatApi.getAssistants()).filter(item => item.name === '资料 Bot')).toHaveLength(0)
  expect(await chatApi.getBotConversations()).toHaveLength(0)

  await user.click(screen.getByRole('button', { name: '添加' }))
  vi.spyOn(chatApi, 'openBot').mockRejectedValueOnce(new Error('暂时无法打开'))
  await user.click(screen.getByRole('button', { name: '创建并开始聊天' }))
  expect(await screen.findByRole('alert')).toHaveTextContent('暂时无法打开')
  expect((await chatApi.getAssistants()).filter(item => item.name === '资料 Bot')).toHaveLength(1)
  await user.click(screen.getByRole('button', { name: '创建并开始聊天' }))
  await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
  expect(screen.getByRole('status')).toHaveTextContent('资料 Bot')
  expect((await chatApi.getAssistants()).filter(item => item.name === '资料 Bot')).toHaveLength(1)
  expect(await chatApi.getBotConversations()).toHaveLength(1)
})
