/** @vitest-environment jsdom */
import { beforeEach, expect, it, vi } from 'vitest'
import { api } from './tauri'
import { captureBackupInterface, restoreBackupInterface } from './backupInterface'

vi.mock('./tauri', () => ({ api: { backupInterface: vi.fn() } }))
beforeEach(() => { window.localStorage.clear(); vi.resetAllMocks() })

it('restores portable preferences once while retaining this computer window geometry', async () => {
  window.localStorage.setItem('kivio-chat-sidebar-view', 'classic')
  window.localStorage.setItem('kivio-chat-window-geometry', 'local monitor coordinates')
  window.localStorage.setItem('unrelated', 'leave alone')
  vi.mocked(api.backupInterface).mockResolvedValue({ id: 'restore-1', values: {
    'kivio-chat-sidebar-view': 'flat',
    'kivio:lens-history:v1': '[{"answer":"saved answer"}]',
    'kivio-chat-window-geometry': 'other computer coordinates',
    unrelated: 'foreign value',
  } })
  await restoreBackupInterface()
  expect(window.localStorage.getItem('kivio-chat-sidebar-view')).toBe('flat')
  expect(window.localStorage.getItem('kivio:lens-history:v1')).toBe('[{"answer":"saved answer"}]')
  expect(window.localStorage.getItem('kivio-chat-window-geometry')).toBe('local monitor coordinates')
  expect(window.localStorage.getItem('unrelated')).toBe('leave alone')
  window.localStorage.setItem('kivio-chat-sidebar-view', 'classic')
  await restoreBackupInterface()
  expect(window.localStorage.getItem('kivio-chat-sidebar-view')).toBe('classic')
  expect(captureBackupInterface()).toEqual({
    'kivio-chat-sidebar-view': 'classic', 'kivio:lens-history:v1': '[{"answer":"saved answer"}]',
  })
})
