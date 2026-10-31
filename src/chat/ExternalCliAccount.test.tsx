import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { expect, it, vi } from 'vitest'
import { ExternalCliAccount } from './ExternalCliAccount'
import { externalCliSettingsApi, type ExternalCliAccount as Account } from '../api/externalCliSettings'
vi.mock('../api/externalCliSettings', () => ({ externalCliSettingsApi: { getExternalCliAccount: vi.fn() } }))

it('reads quota on mount and ignores identity responses from a previous CLI', async () => {
  let oldReply!: (value: Account) => void
  const get = vi.mocked(externalCliSettingsApi.getExternalCliAccount)
  // tsconfig lib 未到 es2024，没有 Promise.withResolvers。
  get.mockReturnValueOnce(new Promise<Account>(resolve => { oldReply = resolve }))
  get.mockResolvedValue({ agentId: 'claude', account: { email: 'current@example.test', type: 'firstParty', plan: 'max' },
    windows: [{ name: 'five_hour', usedPercent: 99, resetsAt: null, windowDurationMins: null }],
    credits: null, unavailable: null, checkedAt: 123 })
  const view = render(<ExternalCliAccount agentId="codex" conversationId="c1" />)
  view.rerender(<ExternalCliAccount agentId="claude" conversationId="c2" />)
  await waitFor(() => expect(screen.getByRole('button', { name: 'CLI 账号与额度' })).toHaveAttribute('title', '剩余 1%'))
  fireEvent.click(screen.getByRole('button', { name: 'CLI 账号与额度' }))
  await screen.findByText(/current@example\.test/)
  expect(screen.getByText('1% 剩余')).toBeInTheDocument()
  expect(screen.getByText('重置时间未知')).toBeInTheDocument()
  await act(async () => { oldReply({ agentId: 'codex', account: { email: 'stale@example.test', type: null, plan: null }, windows: [], credits: null, unavailable: null, checkedAt: 1 }) })
  expect(screen.queryByText('stale@example.test')).not.toBeInTheDocument()
  expect(screen.getByText(/current@example\.test/)).toBeInTheDocument()
})
