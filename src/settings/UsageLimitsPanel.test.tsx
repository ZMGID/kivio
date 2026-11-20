import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { expect, it, vi } from 'vitest'
import {
  externalCliSettingsApi,
  onExternalAgentsUpdated,
  onExternalCliAccountUpdated,
  type DetectedExternalAgent,
} from '../api/externalCliSettings'
import { UsageLimitsPanel } from './UsageLimitsPanel'

vi.mock('../api/externalCliSettings', () => ({
  externalCliSettingsApi: { detectExternalAgents: vi.fn(), getExternalCliAccount: vi.fn() },
  onExternalAgentsUpdated: vi.fn(async () => () => {}),
  onExternalCliAccountUpdated: vi.fn(async () => () => {}),
}))
vi.mock('../api/settingsCache', () => ({ peekSettings: () => null, subscribeSettings: () => () => {} }))

function agent(id: string, name: string, account: DetectedExternalAgent['capabilities']['account']): DetectedExternalAgent {
  return {
    id, name, available: true, models: [],
    capabilities: {
      steering: 'fallback', followUp: 'fallback', imageInput: 'unavailable',
      rewind: 'fallback', conversationBranch: 'fallback', importHistory: 'unavailable', account,
    },
  }
}

it('keeps usable limits across visits and updates just the affected CLI', async () => {
  const detect = vi.mocked(externalCliSettingsApi.detectExternalAgents)
  detect.mockResolvedValue([
    agent('codex', 'Codex', 'native'),
    agent('antigravity', 'Antigravity CLI', 'native'),
    agent('grok', 'Grok Build', 'native'),
    agent('kimi', 'Kimi', 'native'),
    agent('pi', 'Pi', 'unavailable'),
  ])
  const get = vi.mocked(externalCliSettingsApi.getExternalCliAccount)
  const reset = new Date(Date.now() + 3_600_000).toISOString()
  const windows = {
    codex: [{ name: 'Weekly', usedPercent: 100, resetsAt: null, windowDurationMins: 10080 }],
    antigravity: [{ name: 'Gemini Models · 5h', usedPercent: 22, resetsAt: reset, windowDurationMins: 300 }],
    grok: [{ name: 'Monthly', usedPercent: 0, resetsAt: reset, windowDurationMins: 40320 }],
  }
  get.mockImplementation(async (id, _project, mode) => {
    if (id === 'antigravity' && mode === 'refresh') throw new Error('IPC interrupted')
    return {
      agentId: id, account: null, credits: null, unavailable: null, checkedAt: 123,
      windows: id === 'codex' || id === 'antigravity' || id === 'grok' ? windows[id] : [],
      ...(id === 'codex' ? { refreshError: 'timeout' } : id === 'kimi' ? { unavailable: 'Kimi CLI sign-in required' } : {}),
    }
  })
  const firstVisit = render(<UsageLimitsPanel lang="en" />)
  const codex = await screen.findByRole('region', { name: 'Codex' })
  expect(await within(codex).findByRole('progressbar')).toHaveAttribute('aria-valuenow', '0')
  expect(within(codex).getByText('Refresh failed. Showing the last successful reading.')).toBeInTheDocument()
  expect(within(codex).getByText('Reset time unavailable')).toBeInTheDocument()
  const antigravity = await screen.findByRole('region', { name: 'Antigravity CLI' })
  expect(await within(antigravity).findByRole('progressbar', { name: 'Gemini Models · 5h' })).toHaveAttribute('aria-valuenow', '78')
  expect(within(antigravity).getByText(/^Resets in /)).toHaveAttribute('datetime', reset)
  const grok = await screen.findByRole('region', { name: 'Grok Build' })
  expect(within(grok).getByRole('progressbar', { name: 'Monthly' })).toHaveAttribute('aria-valuenow', '100')
  await waitFor(() => expect(get.mock.calls.some(([id]) => id === 'kimi')).toBe(true))
  expect(screen.queryByRole('button', { name: 'Filter local CLIs' })).not.toBeInTheDocument()
  expect(get.mock.calls.some(([id]) => id === 'pi')).toBe(false)

  const beforeReopen = get.mock.calls.length
  firstVisit.unmount()
  const secondVisit = render(<UsageLimitsPanel lang="en" />)
  const cached = screen.getByRole('progressbar', { name: 'Gemini Models · 5h' })
  expect(cached).toHaveAttribute('aria-valuenow', '78')
  expect(screen.queryByRole('region', { name: 'Kimi' })).not.toBeInTheDocument()
  await act(async () => {})
  expect(detect).toHaveBeenCalledTimes(1)
  expect(get).toHaveBeenCalledTimes(beforeReopen)

  windows.antigravity = [{ ...windows.antigravity[0], usedPercent: 36 }]
  const onUpdate = vi.mocked(onExternalCliAccountUpdated).mock.calls.at(-1)?.[0]
  await act(async () => onUpdate?.('antigravity'))
  expect(cached).toHaveAttribute('aria-valuenow', '64')
  expect(get.mock.calls.slice(beforeReopen).map(([id, , mode]) => [id, mode]))
    .toEqual([['antigravity', 'cached']])

  const refresh = screen.getByRole('button', { name: 'Refresh usage limits' })
  fireEvent.click(refresh)
  await waitFor(() => expect(refresh).toBeEnabled())
  expect(cached).toHaveAttribute('aria-valuenow', '64')
  expect(within(screen.getByRole('region', { name: 'Antigravity CLI' })).getByRole('status'))
    .toHaveTextContent('Refresh failed. Showing the last successful reading.')
  expect(screen.queryByRole('region', { name: 'Kimi' })).not.toBeInTheDocument()
  expect(get.mock.calls.filter(([, , mode]) => mode === 'refresh').map(([id]) => id).sort())
    .toEqual(['antigravity', 'codex', 'grok', 'kimi'])

  const beforeAvailabilityUpdate = get.mock.calls.length
  secondVisit.unmount()
  const onAvailability = vi.mocked(onExternalAgentsUpdated).mock.calls[0][0]
  await act(async () => onAvailability([agent('codex', 'Codex', 'native')]))
  render(<UsageLimitsPanel lang="en" />)
  expect(screen.getByRole('region', { name: 'Codex' })).toBeInTheDocument()
  expect(screen.queryByRole('region', { name: 'Antigravity CLI' })).not.toBeInTheDocument()
  await act(async () => {})
  expect(detect).toHaveBeenCalledTimes(1)
  expect(get).toHaveBeenCalledTimes(beforeAvailabilityUpdate)
})
