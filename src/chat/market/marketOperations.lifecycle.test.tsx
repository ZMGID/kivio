import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { marketApi, marketplaceApi, type MarketPlugin, type MarketSnapshot } from '../../api/market'
import { listen } from '@tauri-apps/api/event'
import { packageApi, type PluginPackage } from '../../api/pluginPackages'
import { open } from '@tauri-apps/plugin-dialog'
import { MarketPage } from './MarketPage'
import { resetMarketWindow } from './marketOperations'

vi.mock('../../api/pluginPackages', () => ({ packageApi: { describe: vi.fn(), list: vi.fn(), import: vi.fn(), setEnabled: vi.fn(), remove: vi.fn() } }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }))
vi.mock('../../api/settingsCache', () => ({ refreshSettings: vi.fn() }))
vi.mock('../../components/dialogQueue', () => ({ confirmDialog: vi.fn().mockResolvedValue(true) }))
vi.mock('../../api/market', () => ({
  MARKET_CHANGED_EVENT: 'kivio-market-changed',
  marketApi: { snapshot: vi.fn(), install: vi.fn(), uninstall: vi.fn(), setEnabled: vi.fn() },
  marketplaceApi: { describe: vi.fn(), list: vi.fn(), add: vi.fn(), refresh: vi.fn(), remove: vi.fn(), install: vi.fn() },
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }))
vi.mock('../../api/tauri', () => ({ isTauriRuntime: () => true, api: { openExternal: vi.fn() } }))

function plugin(local: MarketPlugin['local'] = null): MarketPlugin {
  return {
    manifest: {
      id: 'feishu-cli', name: '飞书 CLI', summary: '在飞书中处理消息。', categoryIds: ['productivity'],
      icon: '', welcome: '', inputHint: '', startPrompt: '使用飞书 CLI', setupSkillId: 'feishu-cli-setup',
      mainSkillId: 'feishu-cli', skillIds: ['feishu-cli'], checkCommand: null, repository: null, revision: 'ceace6d9349f',
    },
    local,
  }
}

const catalog = (item: MarketPlugin): MarketSnapshot => ({ categories: [{ id: 'productivity', name: '效率办公' }], plugins: [item] })

function deferred<T>() {
  let resolve: (value: T) => void = () => {}
  let reject: (reason?: unknown) => void = () => {}
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej })
  return { promise, resolve, reject }
}

function imported(id: string, name: string, source: string): PluginPackage {
  return {
    id, name, description: name, version: '1', source, revision: null, format: 'kivio',
    enabled: false, components: { skills: 1 }, diagnostics: [],
  }
}

beforeEach(() => {
  resetMarketWindow()
  vi.resetAllMocks()
  vi.mocked(packageApi.describe).mockResolvedValue({ author: null, version: null, homepage: null, license: null, groups: [], diagnostics: [] })
  vi.mocked(marketplaceApi.describe).mockResolvedValue({ author: null, version: null, homepage: null, license: null, groups: [], diagnostics: [] })
  vi.mocked(listen).mockResolvedValue(() => {})
  vi.mocked(packageApi.list).mockResolvedValue([])
  vi.mocked(marketplaceApi.list).mockResolvedValue([])
  vi.mocked(marketApi.snapshot).mockResolvedValue(catalog(plugin()))
  HTMLDialogElement.prototype.showModal = function () { this.setAttribute('open', '') }
  window.location.hash = '#chat/plugins'
})

describe('market window lifetime', () => {
  it('drops a reset install flight so the next install can start and ignore the stale result', async () => {
    const pending = deferred<MarketSnapshot>()
    const installed = catalog(plugin({ status: 'ready', enabled: true, error: null }))
    vi.mocked(marketApi.install).mockReturnValue(pending.promise)
    const onSkillsChanged = vi.fn()
    const first = render(<MarketPage onUse={vi.fn()} onSkillsChanged={onSkillsChanged} />)
    fireEvent.click(await screen.findByRole('button', { name: '安装' }))
    await waitFor(() => expect(marketApi.install).toHaveBeenCalledTimes(1))
    first.unmount()
    resetMarketWindow()

    vi.mocked(marketApi.install).mockResolvedValue(installed)
    render(<MarketPage onUse={vi.fn()} onSkillsChanged={onSkillsChanged} />)
    fireEvent.click(await screen.findByRole('button', { name: '安装' }))
    await waitFor(() => expect(marketApi.install).toHaveBeenCalledTimes(2))
    await screen.findByRole('button', { name: '飞书 CLI' })
    expect(screen.queryByRole('button', { name: '安装' })).not.toBeInTheDocument()
    await act(async () => { pending.resolve(catalog(plugin())) })
    expect(screen.queryByRole('button', { name: '安装' })).not.toBeInTheDocument()
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
    expect(onSkillsChanged).toHaveBeenCalledTimes(1)
  })

  it('keeps a typed import source when a closed folder picker returns', async () => {
    const pending = deferred<string | null>()
    vi.mocked(open).mockReturnValue(pending.promise)
    render(<MarketPage onUse={vi.fn()} onSkillsChanged={vi.fn()} />)
    fireEvent.click(await screen.findByRole('button', { name: '添加' }))
    fireEvent.click(screen.getByRole('menuitem', { name: '从本地目录导入' }))
    fireEvent.click(screen.getByRole('button', { name: '选择目录' }))
    fireEvent.click(screen.getByRole('button', { name: '取消' }))
    fireEvent.click(screen.getByRole('button', { name: '添加' }))
    fireEvent.click(screen.getByRole('menuitem', { name: '从本地目录导入' }))
    fireEvent.change(screen.getByLabelText('插件来源'), { target: { value: '/plugins/kept' } })
    await act(async () => { pending.resolve('/late/folder') })
    expect(screen.getByLabelText('插件来源')).toHaveValue('/plugins/kept')
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
  })

  it('runs a local import beside an in-flight git import and only navigates for the open dialog', async () => {
    const git = deferred<PluginPackage>()
    const local = deferred<PluginPackage>()
    const gitPackage = imported('git-plugin', 'Git plugin', 'https://github.com/example/git')
    const localPackage = imported('local-plugin', 'Local plugin', '/plugins/local')
    vi.mocked(packageApi.import).mockReturnValueOnce(git.promise).mockReturnValueOnce(local.promise)
    render(<MarketPage onUse={vi.fn()} onSkillsChanged={vi.fn()} />)
    fireEvent.click(await screen.findByRole('button', { name: '添加' }))
    fireEvent.click(screen.getByRole('menuitem', { name: '从 Git 仓库导入' }))
    fireEvent.change(screen.getByLabelText('插件来源'), { target: { value: 'https://github.com/example/git' } })
    fireEvent.click(screen.getByRole('button', { name: '导入' }))
    await waitFor(() => expect(packageApi.import).toHaveBeenCalledTimes(1))
    fireEvent.click(screen.getByRole('button', { name: '取消' }))
    fireEvent.click(screen.getByRole('button', { name: '添加' }))
    fireEvent.click(screen.getByRole('menuitem', { name: '从本地目录导入' }))
    fireEvent.change(screen.getByLabelText('插件来源'), { target: { value: '/plugins/local' } })
    fireEvent.click(screen.getByRole('button', { name: '导入' }))
    await waitFor(() => expect(packageApi.import).toHaveBeenCalledTimes(2))
    expect(packageApi.import).toHaveBeenNthCalledWith(1, 'https://github.com/example/git', undefined)
    expect(packageApi.import).toHaveBeenNthCalledWith(2, '/plugins/local', undefined)
    await act(async () => { git.resolve(gitPackage) })
    expect(screen.getByRole('dialog')).toBeInTheDocument()
    expect(screen.getByLabelText('插件来源')).toHaveValue('/plugins/local')
    expect(screen.getByRole('button', { name: '正在导入…' })).toBeDisabled()
    expect(screen.queryByRole('heading', { name: 'Git plugin' })).not.toBeInTheDocument()
    await act(async () => { local.resolve(localPackage) })
    expect(await screen.findByRole('heading', { name: 'Local plugin' })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '插件市场' }))
    expect(await screen.findByRole('button', { name: 'Git plugin' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Local plugin' })).toBeInTheDocument()
  })

  it('does not let an older package list drop an import that already landed', async () => {
    const packages = deferred<PluginPackage[]>()
    const pluginPackage = imported('landed', 'Landed plugin', 'https://github.com/example/landed')
    vi.mocked(packageApi.list).mockResolvedValueOnce([]).mockImplementationOnce(() => packages.promise)
    vi.mocked(packageApi.import).mockResolvedValue(pluginPackage)
    render(<MarketPage onUse={vi.fn()} onSkillsChanged={vi.fn()} />)
    await screen.findByRole('button', { name: '安装' })
    fireEvent(window, new Event('focus'))
    await waitFor(() => expect(packageApi.list).toHaveBeenCalledTimes(2))
    expect(vi.mocked(packageApi.list).mock.results[1]?.value).toBe(packages.promise)
    fireEvent.click(screen.getByRole('button', { name: '添加' }))
    fireEvent.click(screen.getByRole('menuitem', { name: '从 Git 仓库导入' }))
    fireEvent.change(screen.getByLabelText('插件来源'), { target: { value: 'https://github.com/example/landed' } })
    fireEvent.click(screen.getByRole('button', { name: '导入' }))
    expect(await screen.findByRole('heading', { name: 'Landed plugin' })).toBeInTheDocument()
    await act(async () => { packages.resolve([]) })
    expect(screen.getByRole('heading', { name: 'Landed plugin' })).toBeInTheDocument()
    expect(screen.queryByText('找不到这个插件')).not.toBeInTheDocument()
  })
})
