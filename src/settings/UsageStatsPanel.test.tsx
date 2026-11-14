import { render } from '../test/renderWithNotices'
import { act, cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { api, type UsageStatsResponse } from '../api/tauri'
import { UsageStatsPanel } from './UsageStatsPanel'

vi.mock('../api/tauri', () => ({ api: { usageGetStats: vi.fn(), usageSyncCli: vi.fn(), usageClear: vi.fn() } }))
afterEach(() => { cleanup(); vi.clearAllMocks() })

it('ignores a failed older query while the current range is still loading', async () => {
  let rejectOld!: (error: Error) => void
  let rejectCurrent!: (error: Error) => void
  vi.mocked(api.usageGetStats)
    .mockImplementationOnce(() => new Promise((_, reject) => { rejectOld = reject }))
    .mockImplementationOnce(() => new Promise((_, reject) => { rejectCurrent = reject }))
  render(<UsageStatsPanel lang="zh" view="calls" />)
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
  summary: { totalRequests: 2, successfulRequests: 2, failedRequests: 0, missingUsageRequests: 0, providerReportedRequests: 2, totalTokens: 1100, inputTokens: 1000, outputTokens: 100, cachedInputTokens: 800, cacheCreationInputTokens: 0, reasoningTokens: 0, totalCostUsd: 1.25, sessionCount: 1, unpricedRequests: 1 },
  trend: [{ date: '2026-11-14 08', label: '08:00', requests: 2, totalTokens: 1100, inputTokens: 1000, outputTokens: 100, cachedInputTokens: 800, cacheCreationInputTokens: 0, costUsd: 1.25 }],
  logs: [], providerStats: [{ id: 'cli:codex', label: 'Codex', providerId: 'cli:codex', providerName: 'Codex', requestCount: 2, successCount: 2, totalTokens: 1100, inputTokens: 1000, outputTokens: 100, cachedInputTokens: 800, cacheCreationInputTokens: 0, costUsd: 1.25, sessionCount: 1, unpricedRequests: 1 }], modelStats: [], totalLogs: 2, skippedRecords: 0,
}

it('syncs native history, labels estimates and unpriced records, and switches metrics', async () => {
  vi.mocked(api.usageGetStats).mockResolvedValue(report)
  vi.mocked(api.usageSyncCli).mockResolvedValue({ sources: [{ id: 'codex', label: 'Codex', files: 1, records: 2, skipped: 0, errors: [] }] })
  render(<UsageStatsPanel lang="zh" view="app" />)
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
})

it('filters provider details without rescanning CLI history on each query', async () => {
  vi.mocked(api.usageGetStats).mockResolvedValue(report)
  vi.mocked(api.usageSyncCli).mockResolvedValue({ sources: [] })
  render(<UsageStatsPanel lang="zh" view="app" />)
  await waitFor(() => expect(screen.getByRole('button', { name: '刷新' })).not.toBeDisabled())
  fireEvent.click(screen.getByTitle('筛选 Codex'))
  await waitFor(() => expect(api.usageGetStats).toHaveBeenLastCalledWith(expect.objectContaining({ providerSearch: 'cli:codex' })))
  expect(api.usageSyncCli).toHaveBeenCalledTimes(1)
})

it('releases syncing when leaving overview before the scan completes', async () => {
  let finish!: (value: Awaited<ReturnType<typeof api.usageSyncCli>>) => void
  vi.mocked(api.usageGetStats).mockResolvedValue(report)
  vi.mocked(api.usageSyncCli).mockImplementationOnce(() => new Promise(resolve => { finish = resolve }))
  const { rerender } = render(<UsageStatsPanel lang="en" view="app" />)
  await waitFor(() => expect(api.usageGetStats).toHaveBeenCalled())
  rerender(<UsageStatsPanel lang="en" view="calls" />)
  await waitFor(() => expect(screen.getByRole('button', { name: 'Refresh' })).not.toBeDisabled())
  await act(async () => { finish({ sources: [] }) })
  expect(screen.getByRole('button', { name: 'Refresh' })).not.toBeDisabled()
})
