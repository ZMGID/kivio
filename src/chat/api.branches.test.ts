import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { chatApi } from './api'
import { dismissNotice, getNotices } from '../components/appNoticeStore'
import type { Conversation, ConversationContextState } from './types'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('./utils', () => ({ isTauriRuntime: () => true }))

const conversation: Conversation = {
  id: 'c1', revision: 2, title: 'Branch', provider_id: 'p', model: 'm', created_at: 1, updated_at: 2,
  messages: [{ id: 'm1', role: 'user', content: 'Original question', timestamp: 1 }],
}
const contextState: ConversationContextState = { estimated_input_tokens: 0, context_window_tokens: 1000 }

beforeEach(() => {
  vi.mocked(invoke).mockReset()
  getNotices().forEach(notice => dismissNotice(notice.id))
})

describe('branch command adapter', () => {
  it('loads only presentation IDs, labels and active state', async () => {
    const branches = [{ id: 'app-a', label: 'Original', active: false }, { id: 'app-b', label: 'Edited', active: true }]
    vi.mocked(invoke).mockResolvedValue({ success: true, branches })
    expect(await chatApi.getBranches('c1')).toEqual(branches)
    expect(invoke).toHaveBeenCalledWith('chat_get_branches', { conversationId: 'c1' })
  })

  it('sends the selected presentation ID and returns the confirmed conversation', async () => {
    vi.mocked(invoke).mockResolvedValue({ success: true, conversation })
    expect(await chatApi.switchBranch('c1', 'app-b')).toBe(conversation)
    expect(invoke).toHaveBeenCalledWith('chat_switch_branch', { conversationId: 'c1', branchId: 'app-b' })
  })

  it('rejects a failed switch without a replacement conversation', async () => {
    vi.mocked(invoke).mockResolvedValue({ success: false, error: 'Branch unavailable' })
    await expect(chatApi.switchBranch('c1', 'missing')).rejects.toThrow('Branch unavailable')
  })

  const mutations = [
    () => chatApi.switchBranch('c1', 'app-b'),
    () => chatApi.updateMessage('c1', 'm1', 'Edited'),
    () => chatApi.deleteMessage('c1', 'm2'),
    () => chatApi.setGroupSelection('c1', 'g1', 'm2'),
    () => chatApi.sendMessage('c1', 'Follow up'),
    () => chatApi.regenerateMessage('c1', 'm1'),
    () => chatApi.replyWithModel('c1', 'm2', 'p2', 'other'),
    () => chatApi.forkConversation('c1', 'm1'),
    async () => (await chatApi.rewindToMessage('c1', 'm1')).conversation,
    async () => (await chatApi.clearContext('c1')).conversation,
  ]
  it.each(mutations)('preserves a committed result when index repair reports a warning', async mutate => {
    vi.mocked(invoke).mockResolvedValue({ success: true, conversation, contextState, warning: 'Branch index repair needed' })
    expect(await mutate()).toBe(conversation)
    expect(getNotices().map(notice => notice.message)).toContain('Branch index repair needed')
  })
})
