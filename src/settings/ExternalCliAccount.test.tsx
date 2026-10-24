import { act, fireEvent, render, screen } from '@testing-library/react'
import { expect, it, vi } from 'vitest'
import { ExternalCliAccount } from './ExternalCliAccount'
import { externalCliSettingsApi } from '../api/externalCliSettings'
vi.mock('../api/externalCliSettings', () => ({ externalCliSettingsApi: { getExternalCliAccount: vi.fn() } }))

it('shows remaining quota and ignores identity responses from a previous CLI', async () => {
  let oldReply!: (value: Awaited<ReturnType<typeof externalCliSettingsApi.getExternalCliAccount>>) => void
  const get = vi.mocked(externalCliSettingsApi.getExternalCliAccount)
  get.mockReturnValueOnce(new Promise(resolve => { oldReply = resolve }))
  const view = render(<ExternalCliAccount agentId="codex" conversationId="c1" />)
  fireEvent.click(screen.getByRole('button', { name: 'CLI 账号与额度' }))
  expect(screen.getByRole('status')).toHaveTextContent('正在读取')
  view.rerender(<ExternalCliAccount agentId="claude" conversationId="c2" />)
  get.mockResolvedValueOnce({ agentId: 'claude', account: { email: 'current@example.test', type: 'firstParty', plan: 'max' },
    windows: [{ name: 'five_hour', usedPercent: 99, resetsAt: null, windowDurationMins: null }],
    credits: null, unavailable: null, checkedAt: 123 })
  fireEvent.click(screen.getByRole('button', { name: 'CLI 账号与额度' }))
  await screen.findByText('current@example.test')
  expect(screen.getByText('1% 剩余')).toBeInTheDocument()
  expect(screen.getByText('重置时间未知')).toBeInTheDocument()
  await act(async () => { oldReply({ agentId: 'codex', account: { email: 'stale@example.test', type: null, plan: null }, windows: [], credits: null, unavailable: null, checkedAt: 1 }) })
  expect(screen.queryByText('stale@example.test')).not.toBeInTheDocument()
  expect(screen.getByText('current@example.test')).toBeInTheDocument()
})
