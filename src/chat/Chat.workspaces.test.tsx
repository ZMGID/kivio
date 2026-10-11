import { act, fireEvent, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeAll, expect, it, vi } from 'vitest'
import { api } from '../api/tauri'
import { makeSettings } from '../settings/tabs/testFixtures'
import { chatApi } from './api'
import Chat from './Chat'
import * as workspaceNavigation from './workspace'

vi.mock('@xterm/xterm', () => ({ Terminal: class {} }))
vi.mock('@xterm/addon-webgl', () => ({ WebglAddon: class {} }))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ onFocusChanged: async () => () => {} }) }))
vi.mock('../api/settingsCache', async original => ({
  ...await original<typeof import('../api/settingsCache')>(),
  getSettingsCached: () => Promise.resolve(makeSettings({ settingsLanguage: 'zh' })),
}))

afterEach(() => { vi.restoreAllMocks(); window.localStorage.clear(); window.history.replaceState(null, '', '/') })
beforeAll(() => {
  // jsdom provides editor selection but no layout geometry.
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList
  Range.prototype.getBoundingClientRect = () => new DOMRect()
})

it.each(['agent', 'bot'])('switches away from the %s workspace through the footer without mixing navigation', async workspace => {
  // Only native transport and persisted data reads are substituted. The shell,
  // view menu, Bot header and composer exercise their actual shared flow.
  for (const key of Object.keys(api) as Array<keyof typeof api>) {
    if (key.startsWith('on') && typeof api[key] === 'function') vi.spyOn(api, key).mockResolvedValue(() => {})
  }
  vi.spyOn(api, 'chatSyncState').mockResolvedValue(undefined)
  vi.spyOn(api, 'scheduledTasksSnapshot').mockResolvedValue({ tasks: [], liveRuns: [] })
  const assistant = { id: 'mode_regression', name: '研究员', created_at: 1, updated_at: 1 }
  const conversation = {
    id: workspace === 'bot' ? 'conv_bot_mode_regression' : 'conv_agent_workspace', revision: 1, title: assistant.name,
    provider_id: '', model: '', messages: [{ id: 'existing', role: 'user' as const, content: '旧聊天内容', timestamp: 1 }],
    preview: '', message_count: 1, created_at: 1, updated_at: 1,
    assistant_id: assistant.id, assistant_snapshot: assistant,
  }
  vi.spyOn(chatApi, 'getConversationWindow').mockResolvedValue(conversation)
  vi.spyOn(chatApi, 'getContextStats').mockResolvedValue({ contextState: {}, conversation })
  vi.spyOn(chatApi, 'getConversations').mockResolvedValue([conversation])
  vi.spyOn(chatApi, 'getBotConversations').mockResolvedValue([conversation])
  vi.spyOn(chatApi, 'getProjects').mockResolvedValue([])
  vi.spyOn(chatApi, 'getSets').mockResolvedValue([])
  vi.spyOn(chatApi, 'getAssistants').mockResolvedValue([assistant])
  vi.spyOn(chatApi, 'getConversationPins').mockResolvedValue({})
  // Even an old Bot sidebar preference must not turn the Agent window into Bots.
  window.localStorage.setItem('kivio-chat-sidebar-view', 'bot')
  window.history.replaceState(null, '', `/${workspace === 'bot' ? '?workspace=bot' : ''}#chat/${conversation.id}`)
  let openRequest!: Parameters<typeof api.onChatOpenConversation>[0]
  vi.spyOn(api, 'onChatOpenConversation').mockImplementation(async handler => { openRequest = handler; return () => {} })
  const user = userEvent.setup()
  const view = render(<Chat onSettingsChange={() => {}} />)
  expect(await screen.findByText('旧聊天内容', {}, { timeout: 5000 })).toBeInTheDocument()
  fireEvent.paste(screen.getByRole('textbox'), { clipboardData: { getData: () => '保留我的草稿', files: [] } })
  if (workspace === 'agent') {
    const switchWorkspace = vi.spyOn(workspaceNavigation, 'switchChatWorkspace').mockResolvedValue()
    expect(screen.queryByRole('button', { name: 'Bots' })).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '扩展' }))
    expect(screen.getByRole('button', { name: '笔记' })).toBeVisible()
    await user.click(screen.getByRole('button', { name: '切换到 Bot 模式' }))
    expect(switchWorkspace).toHaveBeenCalledWith(true)
    expect(screen.queryByRole('button', { name: 'Bot 详情' })).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: /^Kivio Agent/ })).toBeVisible()
    await user.click(screen.getByRole('button', { name: '对话列表操作' }))
    expect(screen.queryByRole('menuitem', { name: '切换为 Bot 模式' })).not.toBeInTheDocument()
  } else {
    const switchWorkspace = vi.spyOn(workspaceNavigation, 'switchChatWorkspace').mockResolvedValue()
    await user.click(screen.getByRole('button', { name: '切换到 Agent 模式' }))
    expect(switchWorkspace).toHaveBeenCalledWith(false)
    expect(screen.getByRole('button', { name: 'Bot 详情' })).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Bot 详情' }))
    expect(await screen.findByRole('complementary', { name: 'Bot 详情' })).toBeVisible()
    expect(screen.queryByRole('textbox', { name: '职责与长期指令' })).not.toBeInTheDocument()
    await user.click(within(screen.getByRole('complementary', { name: 'Bot 详情' })).getByRole('tab', { name: '设置' }))
    await user.click(within(screen.getByRole('complementary', { name: 'Bot 详情' })).getByRole('button', { name: '关闭' }))
    expect(screen.queryByRole('complementary', { name: 'Bot 详情' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Bots' })).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '工作台' }))
    expect(screen.queryByRole('menuitem', { name: '切换为扁平视图' })).not.toBeInTheDocument()
  }
  await act(async () => openRequest({ conversationId: workspace === 'bot' ? 'conv_agent_workspace' : 'conv_bot_mode_regression' }))
  expect(window.location.hash).toBe(`#chat/${conversation.id}`)
  expect(screen.getByRole('textbox')).toHaveTextContent('保留我的草稿')
  expect(screen.getByText('旧聊天内容')).toBeInTheDocument()
  view.unmount()
})
