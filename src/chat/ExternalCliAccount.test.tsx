import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { ExternalCliAccount } from './ExternalCliAccount'
import { externalCliSettingsApi, type ExternalCliAccount as Account } from '../api/externalCliSettings'
import type { ProviderCapabilities } from '../generated/chatProtocol'
import { peekSettings, subscribeSettings } from '../api/settingsCache'
import { makeSettings } from '../settings/tabs/testFixtures'
vi.mock('../api/externalCliSettings', () => ({ externalCliSettingsApi: { getExternalCliAccount: vi.fn() } }))
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
  vi.mocked(subscribeSettings).mockClear()
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
  const view = render(<ExternalCliAccount agent={{ id: 'codex', name: 'Codex', capabilities }} conversationId="c1" />)
  view.rerender(<ExternalCliAccount agent={{ id: 'test-cli', name: 'Test CLI', capabilities }} conversationId="c2" />)
  await waitFor(() => expect(screen.getByRole('button', { name: 'CLI 账号与额度' })).toHaveAttribute('title', '剩余 1%'))
  fireEvent.click(screen.getByRole('button', { name: 'CLI 账号与额度' }))
  await screen.findByText(/current@example\.test/)
  expect(screen.getByText('Test CLI')).toBeInTheDocument()
  expect(get).toHaveBeenLastCalledWith('test-cli', 'c2')
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
  render(<ExternalCliAccount agent={{ id: 'codex', name: 'Codex', capabilities }} conversationId="c1" />)
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
