import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import type { OAuthDevicePrompt, Settings } from '../api/tauri'
import { LangContext } from '../components/i18n'

const mocks = vi.hoisted(() => ({
  oauth: vi.fn(),
  test: vi.fn(),
  settings: {} as Settings,
}))

vi.mock('../api/tauri', () => ({ api: {
  connectorOauthConnect: mocks.oauth,
  chatMcpTestServer: mocks.test,
  onMcpServerState: async () => () => {},
  chatMcpServerStatus: async () => ({ state: { kind: 'disconnected' } }),
} }))
vi.mock('../api/settingsCache', () => ({
  peekSettings: () => mocks.settings,
  refreshSettings: async () => mocks.settings,
  subscribeSettings: () => () => {},
  updateSettingsCached: async (update: (settings: Settings) => Settings) => {
    mocks.settings = update(mocks.settings)
    return mocks.settings
  },
}))
vi.mock('./McpRegistryBrowser', () => ({ McpRegistryBrowser: () => null }))

import { McpCenter } from './McpCenter'

beforeEach(() => {
  vi.resetAllMocks()
  mocks.settings = { chatTools: { servers: [{
    id: 'github', name: 'github', enabled: true, transport: 'streamable_http',
    url: 'https://api.githubcopilot.com/mcp', command: '', args: [], env: {},
    headers: { 'X-Custom': 'keep' }, enabledTools: [],
  }] } } as unknown as Settings
  mocks.test.mockResolvedValue({ success: true, tools: [] })
})

it('offers app configuration after missing-DCR failure and uses it for browser authorization', async () => {
  mocks.settings.chatTools.servers[0].headers.authorization = 'Bearer expired-token'
  mocks.oauth.mockRejectedValueOnce('OAUTH_CLIENT_REQUIRED: registered client required')
  render(<LangContext.Provider value="zh"><McpCenter /></LangContext.Provider>)
  fireEvent.click(await screen.findByRole('button', { name: /github/ }))
  fireEvent.click(screen.getByRole('button', { name: 'OAuth 授权' }))
  const clientId = await screen.findByLabelText('Client ID')
  await waitFor(() => expect(clientId.closest('details')?.open).toBe(true))
  fireEvent.change(clientId, { target: { value: 'kivio-app' } })
  fireEvent.change(screen.getByLabelText('Client Secret'), { target: { value: 'test-secret' } })
  await userEvent.type(screen.getByLabelText('Scopes'), 'repo read:org')
  const auth = { kind: 'oauth', accessToken: 'test-token', clientId: 'kivio-app', clientSecret: 'test-secret', scopes: ['repo', 'read:org'] }
  mocks.oauth.mockResolvedValueOnce({ headers: { Authorization: 'Bearer test-token' }, auth })
  fireEvent.click(screen.getByRole('button', { name: 'OAuth 授权' }))
  await waitFor(() => expect(mocks.oauth).toHaveBeenLastCalledWith({
    url: 'https://api.githubcopilot.com/mcp', name: 'github',
    client: { clientId: 'kivio-app', clientSecret: 'test-secret', scopes: ['repo', 'read:org'] },
  }, expect.any(Function), expect.any(AbortSignal)))
  await waitFor(() => expect(mocks.settings.chatTools.servers[0].auth).toEqual(auth))
  expect(mocks.settings.chatTools.servers[0].headers).toEqual({ 'X-Custom': 'keep', Authorization: 'Bearer test-token' })
  expect(mocks.test).toHaveBeenCalledWith(mocks.settings.chatTools.servers[0], undefined)
})

it('uses built-in GitHub authorization without reusing the maintainers app secret', async () => {
  mocks.settings.chatTools.servers[0].auth = { kind: 'oauth', accessToken: 'old-token', clientId: 'legacy-app', clientSecret: 'legacy-secret' }
  let finish!: (value: unknown) => void
  mocks.oauth.mockImplementationOnce((_args: unknown, onPrompt: (prompt: OAuthDevicePrompt) => void) => {
    onPrompt({ userCode: 'ABCD-EFGH', verificationUri: 'https://github.com/login/device', expiresIn: 900 })
    return new Promise(resolve => { finish = resolve })
  })
  render(<LangContext.Provider value="zh"><McpCenter /></LangContext.Provider>)
  fireEvent.click(await screen.findByRole('button', { name: /github/ }))
  fireEvent.click(screen.getByRole('button', { name: 'OAuth 授权' }))
  expect(await screen.findByRole('dialog', { name: '连接 GitHub' })).toHaveTextContent('ABCD-EFGH')
  expect(mocks.oauth.mock.calls[0][0].client).toBeUndefined()
  expect(screen.getByRole('button', { name: '取消' })).toBeEnabled()
  const auth = { kind: 'oauth', accessToken: 'new-token', clientId: 'built-in-app', scopes: ['public_repo'] }
  finish({ headers: { Authorization: 'Bearer new-token' }, auth })
  await waitFor(() => expect(mocks.settings.chatTools.servers[0].auth).toEqual(auth))
  expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
})

it.each(['cancel', 'unmount'])('does not persist a device authorization result after %s', async (action) => {
  let finish!: (value: unknown) => void
  let signal!: AbortSignal
  mocks.oauth.mockImplementationOnce((_args: unknown, onPrompt: (prompt: OAuthDevicePrompt) => void, abort: AbortSignal) => {
    signal = abort
    onPrompt({ userCode: 'ABCD-EFGH', verificationUri: 'https://github.com/login/device', expiresIn: 900 })
    return new Promise(resolve => { finish = resolve })
  })
  const view = render(<LangContext.Provider value="zh"><McpCenter /></LangContext.Provider>)
  fireEvent.click(await screen.findByRole('button', { name: /github/ }))
  fireEvent.click(screen.getByRole('button', { name: 'OAuth 授权' }))
  await screen.findByRole('dialog')
  if (action === 'cancel') fireEvent.click(screen.getByRole('button', { name: '取消' }))
  else view.unmount()
  expect(signal.aborted).toBe(true)
  finish({ headers: { Authorization: 'Bearer late-token' }, auth: { kind: 'oauth', accessToken: 'late-token' } })
  await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
  expect(mocks.settings.chatTools.servers[0].auth).toBeUndefined()
  expect(mocks.test).not.toHaveBeenCalled()
})

it('does not save a late authorization result onto a changed server URL', async () => {
  let finish!: (value: unknown) => void
  mocks.oauth.mockImplementationOnce(() => new Promise((resolve) => { finish = resolve }))
  render(<LangContext.Provider value="zh"><McpCenter /></LangContext.Provider>)
  fireEvent.click(await screen.findByRole('button', { name: /github/ }))
  fireEvent.click(screen.getByRole('button', { name: 'OAuth 授权' }))
  await waitFor(() => expect(mocks.oauth).toHaveBeenCalledOnce())
  mocks.settings.chatTools.servers[0].url = 'https://another.example/mcp'
  finish({ headers: { Authorization: 'Bearer old-site-token' }, auth: { kind: 'oauth', accessToken: 'old-site-token' } })
  await screen.findByText('服务器地址已更改，请重新授权。')
  expect(mocks.settings.chatTools.servers[0].auth).toBeUndefined()
  expect(mocks.settings.chatTools.servers[0].headers).toEqual({ 'X-Custom': 'keep' })
  expect(mocks.test).not.toHaveBeenCalled()
})
