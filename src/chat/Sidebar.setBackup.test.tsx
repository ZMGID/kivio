import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { open, save } from '@tauri-apps/plugin-dialog'
import { chatApi } from './api'
import { Sidebar, type SidebarProps } from './Sidebar'
import { getSettingsCached } from '../api/settingsCache'
import { alertDialog } from '../components/dialogQueue'

vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }))
vi.mock('../api/settingsCache', () => ({ getSettingsCached: vi.fn().mockResolvedValue({ chat: {} }) }))
vi.mock('../components/dialogQueue', () => ({ alertDialog: vi.fn(), confirmDialog: vi.fn() }))

const set = { id: 'set_one', name: '写作', created_at: 1, updated_at: 1 }
function setup() {
  vi.spyOn(chatApi, 'getConversations').mockResolvedValue([])
  vi.spyOn(chatApi, 'getProjects').mockResolvedValue([])
  vi.spyOn(chatApi, 'getSets').mockResolvedValue([set])
  vi.spyOn(chatApi, 'getAssistants').mockResolvedValue([])
  vi.spyOn(chatApi, 'getConversationPins').mockResolvedValue({})
  const props: SidebarProps = {
    lang: 'zh', selectedProject: null, selectedSet: null,
    onSelectProject: vi.fn(), onSelectSet: vi.fn(), onSelectConversation: vi.fn(),
    onNewConversation: vi.fn(), onOpenSettings: vi.fn(), onOpenExtensionsItem: vi.fn(),
    onSelectLang: vi.fn(), onOpenUsage: vi.fn(), collapsed: false, onToggleCollapsed: vi.fn(),
    refreshKey: 0, searchOpen: false, onSearchOpenChange: vi.fn(),
  }
  const view = render(<Sidebar {...props} />)
  return { ...view, props, user: userEvent.setup() }
}
beforeEach(() => { window.localStorage.removeItem('kivio-chat-sidebar-view'); vi.mocked(getSettingsCached).mockResolvedValue({ chat: {} } as Awaited<ReturnType<typeof getSettingsCached>>) })
afterEach(() => { vi.restoreAllMocks(); vi.clearAllMocks(); window.localStorage.removeItem('kivio-chat-sidebar-view') })

it('exports the selected set and allows cancellation then retry after a native save error', async () => {
  const exportBackup = vi.spyOn(chatApi, 'exportSetBackup').mockRejectedValueOnce(new Error('disk full')).mockResolvedValue()
  vi.mocked(save).mockResolvedValueOnce(null).mockResolvedValue('/tmp/set.json')
  const { user } = setup()
  await user.click(await screen.findByRole('button', { name: '集' }))
  const exportFromMenu = async () => {
    await user.click(screen.getByRole('button', { name: '集操作' }))
    await user.click(await screen.findByRole('menuitem', { name: '导出集备份' }))
    await waitFor(() => expect(screen.queryByRole('menu')).not.toBeInTheDocument())
  }
  await exportFromMenu()
  expect(exportBackup).not.toHaveBeenCalled()
  expect(alertDialog).not.toHaveBeenCalled()
  await exportFromMenu()
  expect(alertDialog).toHaveBeenCalledWith('导出失败：disk full')
  await exportFromMenu()
  expect(exportBackup).toHaveBeenLastCalledWith(set.id, '/tmp/set.json')
  expect(alertDialog).toHaveBeenLastCalledWith(expect.stringContaining('集备份已导出'))
})

it('imports from the conversation actions menu and selects the restored set after refreshing the catalog', async () => {
  const restored = { ...set, id: 'set_restored', name: '写作 (2)' }
  vi.spyOn(chatApi, 'importSetBackup').mockResolvedValue(restored)
  vi.mocked(open).mockResolvedValue('/tmp/set.json')
  const { user, props } = setup()
  await user.click(await screen.findByRole('button', { name: '对话列表操作' }))
  await user.click(await screen.findByRole('menuitem', { name: '导入集备份' }))
  await waitFor(() => expect(props.onSelectSet).toHaveBeenCalledWith(restored))
  expect(chatApi.importSetBackup).toHaveBeenCalledWith('/tmp/set.json')
  expect(chatApi.getSets).toHaveBeenCalledTimes(2)
  expect(alertDialog).toHaveBeenCalledWith(expect.stringContaining('集备份已导入'))
})
