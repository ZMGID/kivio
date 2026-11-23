import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { ExternalCliAccount } from './ExternalCliAccount'
import { WorkspaceUsageSection } from './dock/WorkspaceUsageSection'
import { externalCliSettingsApi, onExternalCliAccountUpdated, type ExternalCliAccount as Account } from '../api/externalCliSettings'
import type { ProviderCapabilities } from '../generated/chatProtocol'
import { peekSettings, subscribeSettings } from '../api/settingsCache'
import { refreshCliAccount } from '../api/useCliAccount'
import { makeSettings } from '../settings/tabs/testFixtures'
vi.mock('../api/externalCliSettings', () => ({
  externalCliSettingsApi: { getExternalCliAccount: vi.fn() },
  onExternalCliAccountUpdated: vi.fn(async () => () => {}),
}))
vi.mock('../api/settingsCache', () => ({ peekSettings: vi.fn(), subscribeSettings: vi.fn(() => () => {}) }))

const capabilities: ProviderCapabilities = {
  steering: 'unavailable',
  followUp: 'fallback',
  imageInput: 'unavailable',
  rewind: 'fallback',
  conversationBranch: 'fallback',
  importHistory: 'unavailable',
  account: 'native',
}

const get = vi.mocked(externalCliSettingsApi.getExternalCliAccount)
beforeEach(() => {
  get.mockReset()
  vi.mocked(peekSettings).mockReset().mockReturnValue(null)
})

it.each(['fallback', 'unavailable'] as const)('skips the quota request when account support is %s', (account) => {
  render(<ExternalCliAccount agent={{ id: 'codex', name: 'Codex', capabilities: { ...capabilities, account } }} />)
  expect(screen.queryByRole('button', { name: 'CLI 账号与额度' })).not.toBeInTheDocument()
  expect(get).not.toHaveBeenCalled()
})

it('reads quota on mount and ignores identity responses from a previous CLI', async () => {
  let oldReply!: (value: Account) => void
  // tsconfig lib 未到 es2024，没有 Promise.withResolvers。
  get.mockReturnValueOnce(new Promise<Account>(resolve => { oldReply = resolve }))
  get.mockResolvedValue({ agentId: 'test-cli', account: { email: 'current@example.test', type: 'firstParty', plan: 'max' },
    windows: [{ name: 'five_hour', usedPercent: 99, resetsAt: null, windowDurationMins: null }],
    credits: null, unavailable: null, checkedAt: 123 })
  const view = render(<ExternalCliAccount agent={{ id: 'codex', name: 'Codex', capabilities }} conversation={{ id: 'c1', folder: 'Legacy' }} />)
  await waitFor(() => expect(get).toHaveBeenCalledTimes(1))
  view.rerender(<ExternalCliAccount agent={{ id: 'test-cli', name: 'Test CLI', capabilities }} conversation={{ id: 'c2', folder: 'Legacy' }} />)
  await waitFor(() => expect(screen.getByRole('button', { name: 'CLI 账号与额度' })).toHaveAttribute('title', '剩余 1%'))
  fireEvent.click(screen.getByRole('button', { name: 'CLI 账号与额度' }))
  await screen.findByText(/current@example\.test/)
  expect(screen.getByText('Test CLI')).toBeInTheDocument()
  expect(get).toHaveBeenLastCalledWith('test-cli', 'c2', 'automatic')
  expect(get).toHaveBeenCalledTimes(2)
  expect(screen.getByText('1% 剩余')).toBeInTheDocument()
  expect(screen.getByText('重置时间未知')).toBeInTheDocument()
  await act(async () => { oldReply({ agentId: 'codex', account: { email: 'stale@example.test', type: null, plan: null }, windows: [], credits: null, unavailable: null, checkedAt: 1 }) })
  expect(screen.queryByText('stale@example.test')).not.toBeInTheDocument()
  expect(screen.getByText(/current@example\.test/)).toBeInTheDocument()
})

it('refreshes changed CLI settings and discards the previous account response', async () => {
  let stale!: (value: Account) => void
  get.mockReturnValueOnce(new Promise<Account>((resolve) => { stale = resolve }))
  const current: Account = {
    agentId: 'codex', account: null,
    windows: [{ name: 'five_hour', usedPercent: 20, resetsAt: null, windowDurationMins: null }],
    credits: null, unavailable: null, checkedAt: 1,
  }
  get.mockResolvedValue(current)
  render(<ExternalCliAccount agent={{ id: 'codex', name: 'Codex', capabilities }} conversation={{ id: 'configured', folder: 'Legacy' }} />)
  await waitFor(() => expect(get).toHaveBeenCalledTimes(1))
  const listener = vi.mocked(subscribeSettings).mock.calls[0][0]
  const settings = makeSettings()
  act(() => listener(settings))
  expect(get).toHaveBeenCalledTimes(1)
  settings.chat = { ...settings.chat, externalCliAgents: { codex: { path: '/new/codex' } } }
  act(() => listener(settings))
  await waitFor(() => expect(screen.getByRole('button', { name: 'CLI 账号与额度' })).toHaveAttribute('title', '剩余 80%'))
  expect(get).toHaveBeenCalledTimes(2)
  await act(async () => { stale({ ...current, windows: [{ ...current.windows[0], usedPercent: 99 }] }) })
  expect(screen.getByRole('button', { name: 'CLI 账号与额度' })).toHaveAttribute('title', '剩余 80%')
})

it('renders a newer shared snapshot without requesting another probe', async () => {
  const reading: Account = {
    agentId: 'event-cli', account: null, credits: null, unavailable: null, checkedAt: 123,
    windows: [{ name: 'five_hour', usedPercent: 5, resetsAt: null, windowDurationMins: 300 }],
  }
  get.mockResolvedValueOnce(reading)
  render(<ExternalCliAccount agent={{ id: 'event-cli', name: 'Event CLI', capabilities }} conversation={{ id: 'c1', folder: 'Legacy' }} />)
  await waitFor(() => expect(screen.getByRole('button', { name: 'CLI 账号与额度' })).toHaveAttribute('title', '剩余 95%'))

  const listener = vi.mocked(onExternalCliAccountUpdated).mock.calls[0][0]
  get.mockResolvedValueOnce({ ...reading, windows: [{ ...reading.windows[0], usedPercent: 40 }] })
  await act(async () => listener('event-cli'))
  expect(get).toHaveBeenLastCalledWith('event-cli', 'c1', 'cached')
  expect(screen.getByRole('button', { name: 'CLI 账号与额度' })).toHaveAttribute('title', '剩余 60%')

  // The desktop no longer has a reading for this identity, for example after logout.
  get.mockResolvedValueOnce(null).mockResolvedValueOnce({ ...reading, windows: [], unavailable: 'Sign-in required' })
  await act(async () => listener('event-cli'))
  expect(screen.queryByRole('button', { name: 'CLI 账号与额度' })).toBeNull()
  expect(get).toHaveBeenLastCalledWith('event-cli', 'c1', 'automatic')
})

it('shows the saved global account and shares its in-flight refresh across entry points', async () => {
  let reply!: (value: Account) => void
  const id = 'maintenance-cli'
  get.mockImplementation(async (_, __, mode) => mode === 'cached' ? {
    agentId: id, account: { email: null, type: null, plan: 'Pro 20×' }, credits: null, unavailable: null, checkedAt: 122,
    windows: [{ name: 'five_hour', usedPercent: 40, resetsAt: null, windowDurationMins: 300 }], nextRefreshAt: 0,
  } : new Promise<Account>(resolve => { reply = resolve }))
  render(<ExternalCliAccount agent={{ id, name: 'Maintenance CLI', capabilities }} />)
  await waitFor(() => expect(get).toHaveBeenCalledWith(id, null, 'automatic'))
  fireEvent.click(await screen.findByRole('button', { name: 'CLI 账号与额度' }))
  expect(screen.getByText('Pro 20×')).toBeVisible()

  await act(async () => {
    const refreshed = refreshCliAccount(id)
    reply({
      agentId: id, account: null, credits: null, unavailable: null, checkedAt: 123,
      windows: [{ name: 'five_hour', usedPercent: 20, resetsAt: null, windowDurationMins: 300 }],
    })
    await refreshed
  })

  expect(get.mock.calls.map(([, , mode]) => mode)).toEqual(['cached', 'automatic'])
  expect(screen.getByRole('button', { name: 'CLI 账号与额度' })).toHaveAttribute('title', '剩余 80%')
})

it('checks the local quota owner on focus and drops a late reply from the previous login', async () => {
  const id = 'focus-owner-cli'
  const account = (usedPercent: number): Account => ({
    agentId: id, account: null, credits: null, unavailable: null, checkedAt: 123,
    nextRefreshAt: Date.now() + 300_000,
    windows: [{ name: 'Weekly', usedPercent, resetsAt: null, windowDurationMins: null }],
  })
  let initial = true
  let previousReply!: (value: Account) => void
  get.mockImplementation(async (_, __, mode) => {
    if (mode === 'cached') return initial ? account(10) : null
    if (mode === 'refresh') return new Promise<Account>(resolve => { previousReply = resolve })
    return account(80)
  })
  const panel = <ExternalCliAccount agent={{ id, name: 'Local CLI', capabilities }} />
  const view = render(panel)
  const button = await screen.findByRole('button', { name: 'CLI 账号与额度' })
  expect(button).toHaveAttribute('title', '剩余 90%')
  let refreshing!: Promise<void>
  await act(async () => { refreshing = refreshCliAccount(id) })
  initial = false
  await act(async () => { window.dispatchEvent(new Event('focus')) })
  await waitFor(() => expect(screen.getByRole('button', { name: 'CLI 账号与额度' })).toHaveAttribute('title', '剩余 20%'))
  await act(async () => { previousReply(account(10)); await refreshing })
  expect(screen.getByRole('button', { name: 'CLI 账号与额度' })).toHaveAttribute('title', '剩余 20%')

  view.unmount()
  const readsBeforeFocus = get.mock.calls.length
  await act(async () => { window.dispatchEvent(new Event('focus')) })
  expect(get).toHaveBeenCalledTimes(readsBeforeFocus)
  get.mockResolvedValueOnce(account(65))
  render(panel)
  await waitFor(() => expect(screen.getByRole('button', { name: 'CLI 账号与额度' })).toHaveAttribute('title', '剩余 35%'))
})

it.each([null, 'first'])('keeps the quota on first send in scope %s and isolates a different project', async projectId => {
  const agent = { id: `project-codex-${projectId}`, name: 'Codex', capabilities }
  const report: Account = {
    agentId: agent.id, account: null, credits: null, unavailable: null, checkedAt: 123,
    windows: [{ name: 'Weekly', usedPercent: 32, resetsAt: null, windowDurationMins: null }],
  }
  get.mockResolvedValueOnce(report)
  const view = render(<ExternalCliAccount agent={agent} draftProjectId={projectId} />)
  const ring = await screen.findByRole('button', { name: 'CLI 账号与额度' })
  expect(ring).toHaveAttribute('title', '剩余 68%')
  const panel = render(<WorkspaceUsageSection source={{
    providerId: '', externalAgentId: agent.id, externalAccount: true, conversation: null, draftProjectId: projectId,
  }} active lang="en" />)
  expect(await screen.findByText('68% left')).toBeVisible()
  expect(get).toHaveBeenCalledTimes(1)
  panel.unmount()
  get.mockResolvedValueOnce(report)
  view.rerender(<ExternalCliAccount agent={agent} conversation={{ id: 'first-message', project_id: projectId }} />)
  expect(screen.getByRole('button', { name: 'CLI 账号与额度' })).toHaveAttribute('title', '剩余 68%')
  await waitFor(() => expect(get).toHaveBeenLastCalledWith(agent.id, projectId ? { projectId } : null, 'cached'))
  expect(screen.getByRole('button', { name: 'CLI 账号与额度' })).toHaveAttribute('title', '剩余 68%')

  get.mockResolvedValueOnce({ ...report, windows: [{ ...report.windows[0], usedPercent: 90 }] })
  view.rerender(<ExternalCliAccount agent={agent} draftProjectId="second" />)
  expect(screen.queryByRole('button', { name: 'CLI 账号与额度' })).toBeNull()
  expect(await screen.findByRole('button', { name: 'CLI 账号与额度' })).toHaveAttribute('title', '剩余 10%')
})
