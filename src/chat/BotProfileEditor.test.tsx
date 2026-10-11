// @vitest-environment jsdom
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, expect, it, vi } from 'vitest'
import { LangContext } from '../components/i18n'
import { chatApi } from './api'
import { abandonAssistantEdit, beginAssistantEdit, createBlankAssistant } from './assistantDraftStore'
import { BotProfileEditor } from './BotProfileEditor'

afterEach(() => { vi.restoreAllMocks(); abandonAssistantEdit(); localStorage.clear() })

it('edits a Bot beside the chat and keeps changes after a failed save', async () => {
  HTMLDialogElement.prototype.showModal = vi.fn()
  const assistant = await chatApi.createAssistant({ ...createBlankAssistant('研究员'), system_prompt: '原职责' })
  const conversation = await chatApi.openBot(assistant.id)
  await chatApi.sendMessage(conversation.id, '保留的历史')
  beginAssistantEdit(assistant, true)
  const onClose = vi.fn()
  let leave!: () => Promise<boolean>
  const user = userEvent.setup()
  render(<LangContext.Provider value="zh"><BotProfileEditor onSaved={async () => {}} onClose={onClose} registerLeaveGuard={guard => { if (guard) leave = guard }} /></LangContext.Provider>)
  expect(screen.getByRole('complementary', { name: 'Bot 详情' })).toBeVisible()
  expect(screen.queryByRole('textbox')).not.toBeInTheDocument()
  expect(screen.queryByRole('button', { name: '保存资料' })).not.toBeInTheDocument()
  screen.getByRole('tab', { name: '详情' }).focus()
  await user.keyboard('{ArrowRight}')
  expect(screen.getByRole('tab', { name: '成果' })).toHaveAttribute('aria-selected', 'true')
  expect(screen.queryByRole('textbox')).not.toBeInTheDocument()
  await user.tab()
  expect(screen.getByRole('tabpanel')).toHaveFocus()
  screen.getByRole('tab', { name: '成果' }).focus()
  await user.keyboard('{End}')
  expect(screen.getByRole('tab', { name: '设置' })).toHaveAttribute('aria-selected', 'true')
  expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
  let role = screen.getByRole('textbox', { name: '职责与长期指令' })
  await user.clear(role)
  await user.type(role, '先核验来源')
  await user.click(screen.getByRole('button', { name: '运行' }))
  expect(screen.queryByRole('textbox', { name: '职责与长期指令' })).not.toBeInTheDocument()
  await user.click(screen.getByRole('button', { name: '资料' }))
  expect(screen.getByRole('textbox', { name: '职责与长期指令' })).toHaveValue('先核验来源')
  vi.spyOn(chatApi, 'updateAssistant').mockRejectedValueOnce(new Error('保存失败'))
  await user.click(screen.getByRole('button', { name: '保存资料' }))
  expect(await screen.findByRole('alert')).toHaveTextContent('保存失败')
  expect(role).toHaveValue('先核验来源')
  expect(onClose).not.toHaveBeenCalled()
  await user.click(screen.getByRole('button', { name: '保存资料' }))
  expect((await chatApi.getAssistants()).find(item => item.id === assistant.id)?.system_prompt).toBe('先核验来源')
  expect((await chatApi.getConversation(conversation.id)).messages[0].content).toBe('保留的历史')
  expect(screen.getByRole('tab', { name: '设置' })).toHaveAttribute('aria-selected', 'true')
  await user.click(screen.getByRole('tab', { name: '详情' }))
  expect(screen.queryByRole('textbox')).not.toBeInTheDocument()
  await user.click(screen.getByRole('tab', { name: '设置' }))
  role = screen.getByRole('textbox', { name: '职责与长期指令' })
  await user.type(role, '，标注不确定性')
  vi.spyOn(window, 'confirm').mockReturnValueOnce(false).mockReturnValueOnce(true)
  expect(await leave()).toBe(false)
  expect(onClose).not.toHaveBeenCalled()
  expect(role).toHaveValue('先核验来源，标注不确定性')
  expect(await leave()).toBe(true)
  expect(onClose).toHaveBeenCalledOnce()
})
