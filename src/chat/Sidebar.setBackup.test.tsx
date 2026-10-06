import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { chatApi } from './api'
import { Sidebar, type SidebarProps } from './Sidebar'
import { getSettingsCached } from '../api/settingsCache'

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

it('keeps backup actions out of the sidebar and set menus', async () => {
  const { user } = setup()
  await user.click(await screen.findByRole('button', { name: '集' }))
  expect(screen.queryByRole('button', { name: '导入集备份' })).not.toBeInTheDocument()
  await user.click(screen.getByRole('button', { name: '集操作' }))
  expect(screen.queryByRole('menuitem', { name: '导出集备份' })).not.toBeInTheDocument()
  expect(screen.getByRole('menuitem', { name: '重命名 / 设置' })).toBeInTheDocument()
  await user.keyboard('{Escape}')
  await waitFor(() => expect(screen.queryByRole('menu')).not.toBeInTheDocument())
  await user.click(screen.getByRole('button', { name: '最近' }))
  await user.click(screen.getByRole('button', { name: '对话列表操作' }))
  expect(screen.queryByRole('menuitem', { name: '导入集备份' })).not.toBeInTheDocument()
})
