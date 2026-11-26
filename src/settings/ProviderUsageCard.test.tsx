import { act, fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { api, type ProviderOAuthUsage } from '../api/tauri'
import { ProviderUsageCard, ProviderUsageSummary } from './ProviderUsageCard'
import { makeProvider } from './tabs/testFixtures'
vi.mock('../api/tauri', async () => {
  const actual = await vi.importActual<typeof import('../api/tauri')>('../api/tauri')
  return { ...actual, api: { ...actual.api, providerOAuthUsage: vi.fn() } }
})
const baseProvider = makeProvider()
const account = (credentialId: string): typeof baseProvider => ({
  ...baseProvider,
  request: { ...baseProvider.request, oauth: { provider: 'kimi', credentialId } },
})
const provider = account('first-account')
const usage: ProviderOAuthUsage = { plan: null, fetchedAt: 1700000000, windows: [{ label: 'Weekly', usedPercent: 75, used: 75, limit: 100, resetsAt: 1700600000 }] }
beforeEach(() => { vi.resetAllMocks(); vi.mocked(api.providerOAuthUsage).mockResolvedValue(usage) })
describe('ProviderUsageCard', () => {
  it('loads Antigravity quota groups and preserves zero remaining', async () => {
    vi.mocked(api.providerOAuthUsage).mockResolvedValue({ ...usage, windows: [{ ...usage.windows[0], label: 'Gemini · Weekly', usedPercent: 100, used: null, limit: null, resetsAt: null, resetHint: 'Resets every week' }] })
    render(<ProviderUsageCard provider={{ ...provider, request: { ...provider.request, oauth: { provider: 'antigravity', credentialId: 'agy' } } }} lang="zh" />)
    expect(await screen.findByText('剩余 0%')).toBeInTheDocument()
    expect(screen.getByText('Resets every week')).toBeInTheDocument()
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '0')
    expect(api.providerOAuthUsage).toHaveBeenCalledTimes(1)
  })

  it('loads account limits and shows remaining rather than used percent', async () => {
    const view = render(<ProviderUsageCard provider={provider} lang="zh" />)
    expect(await screen.findByText('剩余 25%')).toBeInTheDocument()
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '25')
    view.unmount()
    render(<ProviderUsageCard provider={provider} lang="zh" />)
    expect(screen.getByText('剩余 25%')).toBeInTheDocument()
    expect(api.providerOAuthUsage).toHaveBeenCalledTimes(1)
  })
  it('shows unavailable data and errors without a misleading full bar', async () => {
    vi.mocked(api.providerOAuthUsage).mockResolvedValue({ ...usage, windows: [{ ...usage.windows[0], usedPercent: null, used: null }] })
    render(<ProviderUsageCard provider={account('no-percent')} lang="zh" />)
    expect(await screen.findByText('暂无比例')).toBeInTheDocument()
    expect(screen.queryByRole('progressbar')).not.toBeInTheDocument()
    vi.mocked(api.providerOAuthUsage).mockRejectedValue(new Error('network'))
    fireEvent.click(screen.getByRole('button', { name: '刷新' }))
    expect(screen.getByText('暂无比例')).toBeInTheDocument()
    expect(await screen.findByRole('alert')).toHaveTextContent('暂时无法获取')
    expect(screen.getByText('暂无比例')).toBeInTheDocument()
  })
  it('scopes snapshots and late refreshes to the credential that requested them', async () => {
    const first = account('original-account')
    const second = account('replacement-account')
    const view = render(<ProviderUsageCard provider={first} lang="zh" />)
    expect(await screen.findByText('剩余 25%')).toBeInTheDocument()

    let finish: (result: ProviderOAuthUsage) => void = () => {}
    vi.mocked(api.providerOAuthUsage)
      .mockImplementationOnce(() => new Promise(resolve => { finish = resolve }))
      .mockResolvedValueOnce({ ...usage, windows: [{ ...usage.windows[0], usedPercent: 10 }] })
    fireEvent.click(screen.getByRole('button', { name: '刷新' }))
    expect(screen.getByText('剩余 25%')).toBeInTheDocument()
    view.rerender(<ProviderUsageCard provider={second} lang="zh" />)
    expect(screen.queryByText('剩余 25%')).not.toBeInTheDocument()
    expect(await screen.findByText('剩余 90%')).toBeInTheDocument()
    await act(async () => finish({ ...usage, windows: [{ ...usage.windows[0], usedPercent: 60 }] }))
    expect(screen.getByText('剩余 90%')).toBeInTheDocument()
  })
  it('keeps separate snapshots when the same credential uses different request settings', async () => {
    const provider = account('shared-account')
    const direct = { ...provider, baseUrl: `${provider.baseUrl}/direct` }
    const proxied = { ...provider, baseUrl: `${provider.baseUrl}/relay` }
    const view = render(<ProviderUsageCard provider={direct} lang="zh" />)
    expect(await screen.findByText('剩余 25%')).toBeInTheDocument()

    let finishDirect: (result: ProviderOAuthUsage) => void = () => {}
    vi.mocked(api.providerOAuthUsage)
      .mockImplementationOnce(() => new Promise(resolve => { finishDirect = resolve }))
      .mockResolvedValueOnce({ ...usage, windows: [{ ...usage.windows[0], usedPercent: 10 }] })
    fireEvent.click(screen.getByRole('button', { name: '刷新' }))
    view.rerender(<ProviderUsageCard provider={proxied} lang="zh" />)
    expect(screen.queryByText('剩余 25%')).not.toBeInTheDocument()
    expect(await screen.findByText('剩余 90%')).toBeInTheDocument()

    await act(async () => finishDirect({ ...usage, windows: [{ ...usage.windows[0], usedPercent: 60 }] }))
    expect(screen.getByText('剩余 90%')).toBeInTheDocument()
    view.rerender(<ProviderUsageCard provider={direct} lang="zh" />)
    expect(screen.getByText('剩余 40%')).toBeInTheDocument()
    expect(api.providerOAuthUsage).toHaveBeenCalledTimes(3)
  })
  it('reads GLM Coding Plan quota with the selected API key', async () => {
    vi.mocked(api.providerOAuthUsage).mockResolvedValue({ plan: 'Lite', fetchedAt: 1700000000, windows: [
      { label: '5h', usedPercent: 20, used: null, limit: null, resetsAt: null },
      { label: 'MCP · Monthly', usedPercent: 4, used: 4, limit: 100, resetsAt: null },
    ] })
    const glm = { ...baseProvider, baseUrl: 'https://open.bigmodel.cn/api/coding/paas/v4', apiKeys: ['first', 'second'], activeKeyIndex: 1 }
    const view = render(<ProviderUsageCard provider={glm} lang="zh" />)
    expect(await screen.findByText('5 小时额度')).toBeInTheDocument()
    expect(screen.getByText('MCP 每月额度')).toBeInTheDocument()
    expect(screen.getByText('Lite')).toBeInTheDocument()
    view.rerender(<ProviderUsageCard provider={{ ...glm, activeKeyIndex: 0 }} lang="zh" />)
    await screen.findByText('5 小时额度')
    expect(api.providerOAuthUsage).toHaveBeenCalledTimes(2)
    view.unmount()
    render(<ProviderUsageSummary provider={{ ...glm, activeKeyIndex: 0 }} lang="zh" />)
    expect(screen.getByText('剩余 80%')).toBeInTheDocument()
    expect(api.providerOAuthUsage).toHaveBeenCalledTimes(2)
  })
  it('skips pay-as-you-go GLM endpoints and providers without a key', () => {
    render(<>
      <ProviderUsageCard provider={{ ...baseProvider, baseUrl: 'https://open.bigmodel.cn/api/paas/v4', apiKeys: ['key'] }} lang="zh" />
      <ProviderUsageSummary provider={{ ...baseProvider, baseUrl: 'https://api.z.ai/api/coding/paas/v4', apiKeys: [' '] }} lang="zh" />
    </>)
    expect(api.providerOAuthUsage).not.toHaveBeenCalled()
  })
  it('never fetches unsupported or disconnected accounts and ignores stale responses', async () => {
    let finish!: (result: ProviderOAuthUsage) => void
    vi.mocked(api.providerOAuthUsage).mockImplementation(() => new Promise(resolve => { finish = resolve }))
    const view = render(<ProviderUsageCard provider={account('disconnected-account')} lang="zh" />)
    view.rerender(<ProviderUsageCard provider={{ ...provider, request: { ...provider.request, oauth: { provider: 'chatgpt' } } }} lang="zh" />)
    await act(async () => finish(usage))
    expect(screen.queryByRole('region')).not.toBeInTheDocument()
    expect(api.providerOAuthUsage).toHaveBeenCalledTimes(1)
    view.rerender(<ProviderUsageCard provider={{ ...provider, request: { ...provider.request, oauth: { provider: 'antigravity' } } }} lang="zh" />)
    expect(api.providerOAuthUsage).toHaveBeenCalledTimes(1)
  })
})
