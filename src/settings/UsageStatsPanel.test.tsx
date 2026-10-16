import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { api } from '../api/tauri'
import { UsageStatsPanel } from './UsageStatsPanel'

vi.mock('../api/tauri', () => ({ api: { usageGetStats: vi.fn() } }))
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
