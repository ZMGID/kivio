// @vitest-environment jsdom
import { act, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { abandonAssistantEdit, assistantDraftStore, beginAssistantEdit, createBlankAssistant } from '../assistantDraftStore'
import { BotProfileEditor } from '../BotProfileEditor'
import { decodeProfileDetail, profileApi } from './profile'

vi.mock('../../api/tauri', () => ({
  isTauriRuntime: () => true,
  api: {
    getSettings: async () => ({ settings: { chatTools: { servers: [] } } }),
    chatSkillsList: async () => ({ skills: [] }),
  },
}))

/** Shape `chat_get_bot_profile` returns after load, including a legacy revision of 0. */
function loadedDetail(assistantId: string, name = 'Ada') {
  return {
    assistantId,
    name,
    description: '',
    icon: 'bot',
    color: '#6A8FBD',
    systemPrompt: '原职责',
    providerId: '',
    model: '',
    conversationId: `conv_bot_${assistantId}`,
    persistedProfile: true,
    resolvedProviderId: 'provider',
    resolvedModel: 'model',
    modelSource: 'inherit_global',
    modelExplanation: '沿用这位 Bot 打开时的模型',
    profile: {
      version: 1,
      assistantId,
      primary: false,
      capability: { source: 'granted_defaults', explanation: '使用已授予的默认能力' },
      model: { source: 'inherit_global', explanation: '沿用这位 Bot 打开时的模型' },
      approval: { source: 'inherit_global', policy: null, explanation: '沿用这位 Bot 打开时的审批' },
      sharedMemory: [],
      revision: 0,
      createdAt: 1,
      updatedAt: 1,
    },
    resolved: {
      source: 'granted_defaults',
      explanation: '使用已授予的默认能力',
      mcpServerIds: [],
      skillIds: [],
      droppedMcpServerIds: [],
      droppedSkillIds: [],
      rewriteSnapshot: true,
    },
  }
}

afterEach(() => {
  vi.restoreAllMocks()
  abandonAssistantEdit()
})

describe('profile detail decoding', () => {
  it('keeps revision 0 from a fully loaded profile', () => {
    const decoded = decodeProfileDetail(loadedDetail('asst_a'))
    expect(decoded?.profile.revision).toBe(0)
    expect(decoded?.profile.assistantId).toBe('asst_a')
    expect(decoded?.profile.model.source).toBe('inherit_global')
    expect(decoded?.resolved.rewriteSnapshot).toBe(true)
  })

  it('rejects a detail that omits fields the backend supplies on load', () => {
    const partial = {
      assistantId: 'asst_a',
      name: 'Ada',
      description: '',
      icon: 'bot',
      color: '#6A8FBD',
      systemPrompt: '',
      conversationId: 'conv_bot_asst_a',
      persistedProfile: true,
      modelSource: 'inherit_global',
      modelExplanation: '沿用全局',
      providerId: '',
      model: '',
      resolvedProviderId: 'p',
      resolvedModel: 'm',
      profile: {
        primary: false,
        capability: { source: 'granted_defaults' },
        approval: { source: 'inherit_global', policy: null, explanation: '沿用全局审批' },
      },
      resolved: { explanation: '已授予', mcpServerIds: [], skillIds: [] },
    }
    expect(decodeProfileDetail(partial)).toBeNull()
    const missingRevision = loadedDetail('asst_a')
    delete (missingRevision.profile as { revision?: number }).revision
    expect(decodeProfileDetail(missingRevision)).toBeNull()
  })
})

it('keeps the open draft when a stale profile save is rejected', async () => {
  const assistant = createBlankAssistant('Ada')
  const opened = decodeProfileDetail(loadedDetail(assistant.id, '已保存的名字'))
  expect(opened).not.toBeNull()
  vi.spyOn(profileApi, 'get').mockResolvedValue(opened!)
  vi.spyOn(profileApi, 'memory').mockResolvedValue({
    assistantId: assistant.id,
    privateDirectory: '',
    globalInjection: '',
    l1: null,
    l2: null,
    sharedMemory: [],
  })
  vi.spyOn(profileApi, 'onChanged').mockResolvedValue(() => {})
  const update = vi.spyOn(profileApi, 'updatePersona').mockRejectedValue(new Error('配置已经更新，这次没有覆盖'))
  const onSaved = vi.fn()
  beginAssistantEdit(assistant, true)
  const user = userEvent.setup()
  render(<BotProfileEditor onSaved={onSaved} onClose={() => {}} />)
  expect(await screen.findByRole('heading', { name: '已保存的名字' })).toBeVisible()
  await user.click(screen.getByRole('tab', { name: '设置' }))
  const name = screen.getByRole('textbox', { name: 'Bot 名称' })
  expect(name).toHaveValue('已保存的名字')
  await user.type(name, ' 草稿')
  await user.click(screen.getByRole('button', { name: '保存资料' }))
  expect(await screen.findByRole('alert')).toHaveTextContent('这次没有覆盖')
  expect(name).toHaveValue('已保存的名字 草稿')
  expect(onSaved).not.toHaveBeenCalled()
  await waitFor(() => expect(update).toHaveBeenCalledTimes(1))
  expect(update).toHaveBeenCalledWith(assistant.id, expect.objectContaining({
    name: '已保存的名字 草稿',
    systemPrompt: '原职责',
  }), 0)
  const latest = { ...opened!, name: '新的已保存名字', profile: { ...opened!.profile, revision: 2 } }
  vi.mocked(profileApi.get).mockResolvedValue(latest)
  vi.spyOn(window, 'confirm').mockReturnValueOnce(false).mockReturnValueOnce(true)
  await user.click(screen.getByRole('button', { name: '重新读取' }))
  expect(name).toHaveValue('已保存的名字 草稿')
  await user.click(screen.getByRole('button', { name: '重新读取' }))
  await waitFor(() => expect(name).toHaveValue('新的已保存名字'))
  expect(screen.queryByRole('alert')).not.toBeInTheDocument()
  await user.type(name, ' 二稿')
  update.mockResolvedValue({ ...latest, name: '新的已保存名字 二稿', profile: { ...latest.profile, revision: 3 } })
  await user.click(screen.getByRole('button', { name: '保存资料' }))
  await waitFor(() => expect(onSaved).toHaveBeenCalledOnce())
  expect(update).toHaveBeenLastCalledWith(assistant.id, expect.objectContaining({ name: '新的已保存名字 二稿' }), 2)
})

it('does not save an older draft with a revision that arrived after the edit', async () => {
  const assistant = createBlankAssistant('Ada')
  const opened = decodeProfileDetail(loadedDetail(assistant.id, '已保存的名字'))
  expect(opened).not.toBeNull()
  const resolvers: Array<(detail: NonNullable<typeof opened>) => void> = []
  vi.spyOn(profileApi, 'get').mockImplementation(() => new Promise((resolve) => { resolvers.push(resolve) }))
  vi.spyOn(profileApi, 'memory').mockResolvedValue({
    assistantId: assistant.id, privateDirectory: '', globalInjection: '', l1: null, l2: null, sharedMemory: [],
  })
  vi.spyOn(profileApi, 'onChanged').mockResolvedValue(() => {})
  const update = vi.spyOn(profileApi, 'updatePersona').mockResolvedValue(opened!)
  beginAssistantEdit(assistant, true)
  const user = userEvent.setup()
  render(<BotProfileEditor onSaved={vi.fn()} onClose={() => {}} />)
  await user.click(await screen.findByRole('tab', { name: '设置' }))
  const name = screen.getByRole('textbox', { name: 'Bot 名称' })
  await user.type(name, ' 草稿')
  await act(async () => { for (const resolve of resolvers) resolve(opened!) })
  await user.click(screen.getByRole('button', { name: '保存资料' }))
  expect(await screen.findByRole('alert')).toHaveTextContent('还没有读到这位 Bot 的配置版本，这次没有保存。')
  expect(update).not.toHaveBeenCalled()
  expect(name).toHaveValue('Ada 草稿')
  expect(screen.queryByText('已保存的名字')).not.toBeInTheDocument()
})

it('retries opening a created Bot after navigation fails instead of creating another', async () => {
  HTMLDialogElement.prototype.showModal = function () { this.setAttribute('open', '') }
  const opened = decodeProfileDetail(loadedDetail('asst_server', '资料 Bot'))
  expect(opened).not.toBeNull()
  const create = vi.spyOn(profileApi, 'create').mockResolvedValue(opened!)
  const update = vi.spyOn(profileApi, 'updatePersona').mockResolvedValue(opened!)
  beginAssistantEdit(createBlankAssistant('资料 Bot'), false)
  const onSaved = vi.fn().mockRejectedValueOnce(new Error('暂时无法打开')).mockResolvedValueOnce(undefined)
  const onClose = vi.fn()
  const user = userEvent.setup()
  render(<BotProfileEditor onSaved={onSaved} onClose={onClose} />)
  await user.click(screen.getByRole('button', { name: '创建并开始聊天' }))
  expect(await screen.findByRole('alert')).toHaveTextContent('暂时无法打开')
  expect(screen.getByRole('dialog', { name: '新建 Bot' })).toBeInTheDocument()
  expect(assistantDraftStore.getSnapshot().editing).toBe(true)
  expect(assistantDraftStore.getSnapshot().draft?.id).toBe('asst_server')
  expect(create).toHaveBeenCalledTimes(1)
  await user.click(screen.getByRole('button', { name: '创建并开始聊天' }))
  await waitFor(() => expect(onSaved).toHaveBeenCalledTimes(2))
  expect(create).toHaveBeenCalledTimes(1)
  expect(update).not.toHaveBeenCalled()
  expect(onSaved).toHaveBeenLastCalledWith(expect.objectContaining({ id: 'asst_server' }), true)
  expect(onClose).toHaveBeenCalledOnce()
})

it('updates the created Bot instead of creating another when the draft changes after a failed open', async () => {
  HTMLDialogElement.prototype.showModal = function () { this.setAttribute('open', '') }
  const opened = decodeProfileDetail(loadedDetail('asst_server', '资料 Bot'))
  expect(opened).not.toBeNull()
  const create = vi.spyOn(profileApi, 'create').mockResolvedValue(opened!)
  const update = vi.spyOn(profileApi, 'updatePersona').mockResolvedValue({ ...opened!, profile: { ...opened!.profile, revision: 1 } })
  beginAssistantEdit(createBlankAssistant('资料 Bot'), false)
  const onSaved = vi.fn().mockRejectedValueOnce(new Error('暂时无法打开')).mockResolvedValueOnce(undefined)
  const user = userEvent.setup()
  render(<BotProfileEditor onSaved={onSaved} onClose={() => {}} />)
  await user.click(screen.getByRole('button', { name: '创建并开始聊天' }))
  expect(await screen.findByRole('alert')).toHaveTextContent('暂时无法打开')
  const name = screen.getByRole('textbox', { name: 'Bot 名称' })
  await user.type(name, ' 二稿')
  await user.click(screen.getByRole('button', { name: '创建并开始聊天' }))
  await waitFor(() => expect(update).toHaveBeenCalledTimes(1))
  expect(create).toHaveBeenCalledTimes(1)
  expect(update).toHaveBeenCalledWith('asst_server', expect.objectContaining({ name: '资料 Bot 二稿' }), 0)
})

it('guards settings drafts and busy memory saves when returning to details', async () => {
  const assistant = createBlankAssistant('Ada')
  const opened = decodeProfileDetail(loadedDetail(assistant.id, 'Ada'))
  expect(opened).not.toBeNull()
  vi.spyOn(profileApi, 'get').mockResolvedValue(opened!)
  vi.spyOn(profileApi, 'memory').mockResolvedValue({
    assistantId: assistant.id, privateDirectory: '', globalInjection: '', l1: '旧的近期', l2: '旧的长期', sharedMemory: [],
  })
  vi.spyOn(profileApi, 'onChanged').mockResolvedValue(() => {})
  let releaseSave: (value: Awaited<ReturnType<typeof profileApi.memory>>) => void = () => {}
  vi.spyOn(profileApi, 'saveMemory').mockImplementation(() => new Promise((resolve) => { releaseSave = resolve }))
  beginAssistantEdit(assistant, true)
  const onClose = vi.fn()
  let leave!: () => Promise<boolean>
  const user = userEvent.setup()
  render(<BotProfileEditor onSaved={async () => {}} onClose={onClose} registerLeaveGuard={(guard) => { if (guard) leave = guard }} />)
  await user.click(screen.getByRole('tab', { name: '设置' }))
  await user.click(screen.getByRole('button', { name: '记忆' }))
  const l2 = await screen.findByRole('textbox', { name: '长期记忆' })
  await user.clear(l2)
  await user.type(l2, '还没保存')
  vi.spyOn(profileApi, 'updatePersona').mockResolvedValue({ ...opened!, name: 'Ada 更新', profile: { ...opened!.profile, revision: 1 } })
  await user.click(screen.getByRole('button', { name: '资料' }))
  await user.type(screen.getByRole('textbox', { name: 'Bot 名称' }), ' 更新')
  await user.click(screen.getByRole('button', { name: '保存资料' }))
  expect(screen.getByRole('tab', { name: '设置' })).toHaveAttribute('aria-selected', 'true')
  await user.click(screen.getByRole('button', { name: '记忆' }))
  expect(l2).toHaveValue('还没保存')
  vi.spyOn(window, 'confirm').mockReturnValueOnce(false).mockReturnValueOnce(false).mockReturnValueOnce(true)
  expect(await leave()).toBe(false)
  expect(onClose).not.toHaveBeenCalled()
  expect(l2).toHaveValue('还没保存')
  await user.click(screen.getByRole('tab', { name: '详情' }))
  expect(screen.getByRole('textbox', { name: '长期记忆' })).toBeVisible()
  expect(l2).toHaveValue('还没保存')
  await user.click(screen.getByRole('tab', { name: '详情' }))
  expect(screen.queryByRole('textbox', { name: 'Bot 名称' })).not.toBeInTheDocument()
  expect(screen.queryByRole('textbox', { name: '长期记忆' })).not.toBeInTheDocument()
  await user.click(screen.getByRole('tab', { name: '设置' }))
  await user.click(screen.getByRole('button', { name: '记忆' }))
  await user.click(screen.getAllByRole('button', { name: '保存这段记忆' })[0])
  expect(await leave()).toBe(false)
  expect(screen.getByRole('tab', { name: '详情' })).toBeDisabled()
  expect(onClose).not.toHaveBeenCalled()
  await act(async () => {
    releaseSave({
      assistantId: assistant.id, privateDirectory: '', globalInjection: '', l1: '已写入', l2: '服务器长期', sharedMemory: [],
    })
  })
})
