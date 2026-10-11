// @vitest-environment jsdom
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, expect, it, vi } from 'vitest'
import { BotRuntimePanel } from './BotRuntimePanel'
import { profileApi, type BotMemoryView } from './profile'

vi.mock('../../api/tauri', () => ({
  isTauriRuntime: () => true,
  api: {
    getSettings: async () => ({ settings: { chatTools: { servers: [] } } }),
    chatSkillsList: async () => ({ skills: [] }),
  },
}))

const assistantId = 'asst_memory'

function memory(overrides: Partial<BotMemoryView> = {}): BotMemoryView {
  return {
    assistantId,
    privateDirectory: '',
    globalInjection: '',
    l1: '旧的近期',
    l2: '旧的长期',
    sharedMemory: [],
    ...overrides,
  }
}

afterEach(() => { vi.restoreAllMocks() })

async function renderMemory() {
  vi.spyOn(profileApi, 'get').mockResolvedValue({
    assistantId,
    name: 'Ada',
    description: '',
    icon: 'bot',
    color: '#6A8FBD',
    systemPrompt: '',
    providerId: '',
    model: '',
    conversationId: 'conv',
    persistedProfile: true,
    resolvedProviderId: 'provider',
    resolvedModel: 'model',
    modelSource: 'inherit_global',
    modelExplanation: '',
    profile: {
      version: 1,
      assistantId,
      primary: false,
      capability: { source: 'granted_defaults', explanation: '' },
      model: { source: 'inherit_global', explanation: '' },
      approval: { source: 'inherit_global', policy: null, explanation: '' },
      sharedMemory: [],
      revision: 1,
      createdAt: 1,
      updatedAt: 1,
    },
    resolved: {
      source: 'granted_defaults',
      explanation: '',
      mcpServerIds: [],
      skillIds: [],
      droppedMcpServerIds: [],
      droppedSkillIds: [],
      rewriteSnapshot: false,
    },
  })
  vi.spyOn(profileApi, 'memory').mockResolvedValue(memory())
  vi.spyOn(profileApi, 'onChanged').mockResolvedValue(() => {})
  const user = userEvent.setup()
  render(<BotRuntimePanel section="memory" assistantId={assistantId} />)
  expect(await screen.findByRole('textbox', { name: '近期记忆' })).toHaveValue('旧的近期')
  return user
}

it('keeps an unsaved long-term draft when recent memory is saved or cleared', async () => {
  const user = await renderMemory()
  const save = vi.spyOn(profileApi, 'saveMemory').mockImplementation(async (_id, layer, content) => memory({
    l1: layer === 'l1' ? content : '服务器近期',
    l2: layer === 'l2' ? content : '服务器长期',
  }))
  const clear = vi.spyOn(profileApi, 'clearMemory').mockImplementation(async (_id, layer) => memory({
    l1: layer === 'l1' ? null : '服务器近期',
    l2: layer === 'l2' ? null : '服务器长期',
  }))
  const l2 = screen.getByRole('textbox', { name: '长期记忆' })
  await user.clear(l2)
  await user.type(l2, '还没保存的长期')
  await user.click(screen.getAllByRole('button', { name: '保存这段记忆' })[0])
  expect(save).toHaveBeenCalledWith(assistantId, 'l1', '旧的近期')
  expect(l2).toHaveValue('还没保存的长期')
  expect(screen.getByRole('textbox', { name: '近期记忆' })).toHaveValue('旧的近期')
  vi.spyOn(window, 'confirm').mockReturnValueOnce(true)
  await user.click(screen.getAllByRole('button', { name: '清除' })[0])
  expect(clear).toHaveBeenCalledWith(assistantId, 'l1')
  expect(screen.getByRole('textbox', { name: '近期记忆' })).toHaveValue('')
  expect(l2).toHaveValue('还没保存的长期')
  const l1 = screen.getByRole('textbox', { name: '近期记忆' })
  await user.type(l1, '还没保存的近期')
  await user.click(screen.getAllByRole('button', { name: '保存这段记忆' })[1])
  expect(save).toHaveBeenLastCalledWith(assistantId, 'l2', '还没保存的长期')
  expect(l1).toHaveValue('还没保存的近期')
})
