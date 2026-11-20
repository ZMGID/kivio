import { beforeEach, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { chatApi } from './api'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('./utils', () => ({ isTauriRuntime: () => true }))
beforeEach(() => vi.mocked(invoke).mockReset())

it.each(['steerMessage', 'followUpMessage'] as const)('%s preserves disk and virtual text attachments at IPC', async (method) => {
  vi.mocked(invoke).mockResolvedValue(true)
  expect(await chatApi[method]('c1', 'q1', 'Look', [
    { id: 'image', type: 'image', name: 'screen.png', path: '/tmp/screen.png' },
    { id: 'doc', type: 'file', name: 'notes.pdf', path: '/tmp/notes.pdf' },
    { id: 'paste', type: 'file', name: 'paste.txt', path: 'memory://paste', content: 'source' },
  ])).toBe(true)
  expect(invoke).toHaveBeenCalledWith(method === 'steerMessage' ? 'chat_steer_message' : 'chat_follow_up_message', {
    conversationId: 'c1', [method === 'steerMessage' ? 'steerId' : 'followUpId']: 'q1', content: 'Look',
    attachments: ['/tmp/screen.png', '/tmp/notes.pdf'],
    textAttachments: [{ name: 'paste.txt', content: 'source' }],
  })
})
