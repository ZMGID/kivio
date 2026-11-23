import { render } from '../test/renderWithNotices'
import { act, cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { api, type UsageStatsResponse } from '../api/tauri'
import { UsageStatsPanel } from './UsageStatsPanel'
import { usageStatsCache } from './usageStatsCache'

vi.mock('../api/tauri', () => ({ api: { usageGetStats: vi.fn(), usageSyncCli: vi.fn(), usageClear: vi.fn() } }))
afterEach(() => { cleanup(); usageStatsCache.clear(); vi.useRealTimers(); vi.clearAllMocks() })

it('ignores a failed older query while the current range is still loading', async () => {
  let rejectOld!: (error: Error) => void
  let rejectCurrent!: (error: Error) => void
  vi.mocked(api.usageGetStats)
    .mockImplementationOnce(() => new Promise((_, reject) => { rejectOld = reject }))
    .mockImplementationOnce(() => new Promise((_, reject) => { rejectCurrent = reject }))
  render(<UsageStatsPanel lang="zh" view="calls" providers={[]} />)
  fireEvent.click(screen.getByRole('button', { name: '近 30 日' }))
  await waitFor(() => expect(api.usageGetStats).toHaveBeenCalledTimes(2))
  await act(async () => { rejectOld(new Error('过时查询失败')) })
  expect(screen.queryByText('过时查询失败')).toBeNull()
  expect(screen.getByRole('button', { name: '刷新' })).toBeDisabled()
  await act(async () => { rejectCurrent(new Error('当前查询失败')) })
  expect(screen.getByText('当前查询失败')).toBeTruthy()
  expect(screen.getByRole('button', { name: '刷新' })).not.toBeDisabled()
})


const report: UsageStatsResponse = {
  revision: 1,
  sampledAt: 0,
  validUntil: Number.MAX_SAFE_INTEGER,
  utcOffset: -new Date().getTimezoneOffset() * 60,
  summary: { totalRequests: 2, successfulRequests: 2, failedRequests: 0, missingUsageRequests: 0, providerReportedRequests: 2, totalTokens: 1100, inputTokens: 1000, outputTokens: 100, cachedInputTokens: 800, cacheCreationInputTokens: 0, reasoningTokens: 0, totalCostUsd: 1.25, sessionCount: 1, unpricedRequests: 1 },
  trend: [{ date: '2026-11-14 08', label: '08:00', requests: 2, totalTokens: 1100, inputTokens: 1000, outputTokens: 100, cachedInputTokens: 800, cacheCreationInputTokens: 0, costUsd: 1.25 }],
  logs: [], providerStats: [{ id: 'cli:codex', label: 'Codex', providerId: 'cli:codex', providerName: 'Codex', requestCount: 2, successCount: 2, totalTokens: 1100, inputTokens: 1000, outputTokens: 100, cachedInputTokens: 800, cacheCreationInputTokens: 0, costUsd: 1.25, sessionCount: 1, unpricedRequests: 1 }], modelStats: [], totalLogs: 2, skippedRecords: 0,
}

it('shows configured provider logos in the overview and provider breakdown', async () => {
  const [codex] = report.providerStats
  vi.mocked(api.usageGetStats).mockResolvedValue({
    ...report,
    providerStats: [
      codex,
      { ...codex, id: 'dsivio-id', providerId: 'dsivio-id', label: 'Dsivio API', providerName: 'Dsivio API' },
      { ...codex, id: 'relay-id', providerId: 'relay-id', label: 'Relay', providerName: 'Relay' },
    ],
  })
  vi.mocked(api.usageSyncCli).mockResolvedValue({ revision: 1, sources: [] })
  render(<UsageStatsPanel
    lang="zh"
    view="app"
    providers={[
      { id: 'dsivio-id', name: 'Dsivio API', baseUrl: 'https://relay.example/v1' },
      { id: 'relay-id', name: 'Relay', baseUrl: 'https://generativelanguage.googleapis.com/v1beta' },
    ]}
    providerIcons={{ 'dsivio-id': 'OpenAI' }}
  />)

  expect((await screen.findByTitle('筛选 Dsivio API')).querySelector('svg title')?.textContent).toBe('OpenAI')
  expect(screen.getByTitle('筛选 Relay').querySelector('svg title')?.textContent).toBe('Google')

  fireEvent.click(screen.getByRole('button', { name: 'Provider' }))
  expect(screen.getByRole('row', { name: /Dsivio API/ }).querySelector('svg title')?.textContent).toBe('OpenAI')
  expect(screen.getByRole('row', { name: /Relay/ }).querySelector('svg title')?.textContent).toBe('Google')
})

it('syncs native history, labels estimates and unpriced records, and switches metrics', async () => {
  vi.mocked(api.usageGetStats).mockResolvedValue(report)
  vi.mocked(api.usageSyncCli).mockResolvedValue({ revision: 1, sources: [{ id: 'codex', label: 'Codex', files: 1, records: 2, skipped: 0, errors: [] }] })
  render(<UsageStatsPanel lang="zh" view="app" providers={[]} />)
  await waitFor(() => expect(api.usageSyncCli).toHaveBeenCalledTimes(1))
  await waitFor(() => expect(screen.getByRole('button', { name: '刷新' })).not.toBeDisabled())
  expect(screen.getByText('不代表订阅账单或实际扣费。')).toBeTruthy()
  expect(screen.getByText('1 条记录未计价；Token 已计入，费用合计不包含这些记录。')).toBeTruthy()
  expect(screen.getByRole('img', { name: '费用趋势图（美元）' })).toBeTruthy()
  fireEvent.click(screen.getByRole('button', { name: 'Tokens' }))
  expect(screen.getByRole('button', { name: 'Tokens' })).toHaveAttribute('aria-pressed', 'true')
  expect(screen.queryByRole('img', { name: '费用趋势图（美元）' })).toBeNull()
  fireEvent.click(screen.getByRole('button', { name: '刷新' }))
  await waitFor(() => expect(api.usageSyncCli).toHaveBeenCalledTimes(2))
  await waitFor(() => expect(screen.getByRole('button', { name: '刷新' })).not.toBeDisabled())
  expect(api.usageSyncCli).toHaveBeenLastCalledWith(true)
  expect(api.usageGetStats).toHaveBeenCalledTimes(1)
})

it('filters provider details without rescanning CLI history on each query', async () => {
  vi.mocked(api.usageGetStats).mockResolvedValue(report)
  vi.mocked(api.usageSyncCli).mockResolvedValue({ revision: 1, sources: [] })
  render(<UsageStatsPanel lang="zh" view="app" providers={[]} />)
  await waitFor(() => expect(screen.getByRole('button', { name: '刷新' })).not.toBeDisabled())
  fireEvent.click(screen.getByTitle('筛选 Codex'))
  await waitFor(() => expect(api.usageGetStats).toHaveBeenLastCalledWith(expect.objectContaining({ providerSearch: 'cli:codex' })))
  expect(api.usageSyncCli).toHaveBeenCalledTimes(1)
})

it('releases syncing when leaving overview before the scan completes', async () => {
  let finish!: (value: Awaited<ReturnType<typeof api.usageSyncCli>>) => void
  vi.mocked(api.usageGetStats).mockResolvedValue(report)
  vi.mocked(api.usageSyncCli).mockImplementationOnce(() => new Promise(resolve => { finish = resolve }))
  const { rerender } = render(<UsageStatsPanel lang="en" view="app" providers={[]} />)
  await waitFor(() => expect(api.usageGetStats).toHaveBeenCalled())
  rerender(<UsageStatsPanel lang="en" view="calls" providers={[]} />)
  await waitFor(() => expect(screen.getByRole('button', { name: 'Refresh' })).not.toBeDisabled())
  await act(async () => { finish({ revision: 1, sources: [] }) })
  expect(screen.getByRole('button', { name: 'Refresh' })).not.toBeDisabled()
})

it('reopens with the existing chart and only revalidates the ledger once when the scan saw no writes', async () => {
  vi.mocked(api.usageGetStats).mockResolvedValue(report)
  vi.mocked(api.usageSyncCli).mockResolvedValue({ revision: 1, sources: [] })
  const first = render(<UsageStatsPanel lang="en" view="app" providers={[]} />)
  await waitFor(() => expect(screen.getByRole('button', { name: 'Refresh' })).not.toBeDisabled())
  const chartName = /cost trend/i
  expect(screen.getByRole('img', { name: chartName })).toBeVisible()
  expect(api.usageGetStats).toHaveBeenCalledTimes(1)
  first.unmount()

  render(<UsageStatsPanel lang="en" view="app" providers={[]} />)
  expect(screen.getByRole('img', { name: chartName })).toBeVisible()
  await waitFor(() => expect(screen.getByRole('button', { name: 'Refresh' })).not.toBeDisabled())
  expect(api.usageSyncCli).toHaveBeenCalledTimes(1)
  expect(api.usageGetStats).toHaveBeenCalledTimes(2)
})

it('updates the totals after a CLI sync changed the ledger and keeps them when paging logs', async () => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
  const refreshed = {
    ...report,
    revision: 2,
    totalLogs: 16,
    summary: { ...report.summary, totalRequests: 16 },
  }
  vi.mocked(api.usageGetStats).mockResolvedValueOnce(report).mockImplementation(async query => {
    const model = query?.offset === 15 ? 'second-page-model' : 'first-page-model'
    return {
      ...refreshed,
      logs: [{
        id: model, model, createdAt: 1794816000, completedAt: 1794816001, durationMs: 1000,
        source: 'cli', operation: 'codex', providerId: 'cli:codex', providerName: 'Codex',
        apiFormat: 'openai_chat', status: 'success', usageSource: 'provider_reported', costSource: 'litellm',
      }],
    }
  })
  vi.mocked(api.usageSyncCli).mockResolvedValue({ revision: 2, sources: [] })
  await act(async () => { render(<UsageStatsPanel lang="en" view="app" providers={[]} />) })
  expect(screen.getByText('16 requests')).toBeVisible()
  fireEvent.click(screen.getByRole('button', { name: 'Logs' }))
  expect(screen.getByText('first-page-model')).toBeVisible()
  await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Next' })) })
  expect(screen.getByText('second-page-model')).toBeVisible()
  await act(async () => { vi.advanceTimersByTime(250) })
  expect(screen.getByText('second-page-model')).toBeVisible()
  expect(screen.queryByText('first-page-model')).toBeNull()
  expect(screen.getByText('2 / 2')).toBeVisible()
  expect(screen.getByText('16 requests')).toBeVisible()
  expect(api.usageSyncCli).toHaveBeenCalledTimes(1)
})
