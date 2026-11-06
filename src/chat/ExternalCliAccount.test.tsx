import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { ExternalCliAccount } from './ExternalCliAccount'
import { externalCliSettingsApi, type ExternalCliAccount as Account } from '../api/externalCliSettings'
import type { ProviderCapabilities } from '../generated/chatProtocol'
vi.mock('../api/externalCliSettings', () => ({ externalCliSettingsApi: { getExternalCliAccount: vi.fn() } }))

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
beforeEach(() => get.mockReset())

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
