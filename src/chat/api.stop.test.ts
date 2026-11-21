import { beforeEach, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { dismissNotice, getNotices } from '../components/appNoticeStore'
import { chatApi } from './api'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('./utils', () => ({ isTauriRuntime: () => true }))

beforeEach(() => {
  vi.mocked(invoke).mockReset()
  for (const notice of getNotices()) dismissNotice(notice.id)
})

it('surfaces persistence failure without rejecting a completed stop', async () => {
  vi.mocked(invoke).mockResolvedValue({ warning: 'Generation stopped. Goal state could not be saved.' })
  await expect(chatApi.cancelStream('c1')).resolves.toBeUndefined()
  expect(getNotices().map(({ message }) => message))
    .toEqual(['Generation stopped. Goal state could not be saved.'])
})

it('rejects an IPC failure so the caller can restore still-live controls', async () => {
  vi.mocked(invoke).mockRejectedValue(new Error('Stop IPC failed'))
  await expect(chatApi.cancelStream('c1')).rejects.toThrow('Stop IPC failed')
  expect(getNotices()).toHaveLength(0)
})
