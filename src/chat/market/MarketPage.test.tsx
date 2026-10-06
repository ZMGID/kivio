import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { MarketPage } from './MarketPage'
import { resetMarketWindow } from './marketOperations'
import { claudeMarketplaceIcon, pluginAction } from './marketModel'
import { marketApi, marketplaceApi, type Marketplace, type MarketPlugin, type MarketSnapshot } from '../../api/market'
import { listen } from '@tauri-apps/api/event'
import { packageApi, type PluginPackage } from '../../api/pluginPackages'
import { confirmDialog } from '../../components/dialogQueue'
import { open } from '@tauri-apps/plugin-dialog'

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
      icon: 'data:image/svg+xml;base64,PHN2Zy8+', welcome: '告诉我你想做什么', inputHint: '看看日程',
      startPrompt: '使用飞书 CLI', setupSkillId: 'feishu-cli-setup', mainSkillId: 'feishu-cli',
      skillIds: ['feishu-cli'], checkCommand: null, repository: 'larksuite/cli', revision: 'ceace6d9349f',
    },
    local,
  }
}
const snapshot = (item: MarketPlugin): MarketSnapshot => ({ categories: [{ id: 'productivity', name: '效率办公' }], plugins: [item] })

function deferred<T>() {
  let resolve: (value: T) => void = () => {}
  let reject: (reason?: unknown) => void = () => {}
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej })
  return { promise, resolve, reject }
}

beforeEach(() => {
  resetMarketWindow()
  vi.resetAllMocks()
  const details = { author: null, version: null, homepage: null, license: null, groups: [], diagnostics: [] }
  vi.mocked(packageApi.describe).mockResolvedValue(details)
  vi.mocked(marketplaceApi.describe).mockResolvedValue(details)
  vi.mocked(listen).mockResolvedValue(() => {})
  vi.mocked(packageApi.list).mockResolvedValue([])
  vi.mocked(marketplaceApi.list).mockResolvedValue([])
  vi.mocked(confirmDialog).mockResolvedValue(true)
  HTMLDialogElement.prototype.showModal = function () { this.setAttribute('open', '') }
  window.location.hash = '#chat/plugins'
})
afterEach(() => { window.location.hash = '' })

describe('pluginAction', () => {
  it('limits supplemental branding to the Claude official repository and known entries', () => {
    expect(claudeMarketplaceIcon('https://github.com/anthropics/claude-plugins-official.git#main', 'adobe-for-creativity')).toContain('/adobe-for-creativity/icon.png')
    expect(claudeMarketplaceIcon('https://github.com/someone/claude-plugins-official', 'adobe-for-creativity')).toBeUndefined()
    expect(claudeMarketplaceIcon('https://github.com/anthropics/claude-plugins-official', 'unknown-entry')).toBeUndefined()
    expect(claudeMarketplaceIcon(undefined, 'adobe-for-creativity')).toBeUndefined()
  })
  it('maps install state to the primary action', () => {
    expect(pluginAction(plugin())).toBe('install')
    expect(pluginAction(plugin({ status: 'failed', enabled: false, error: 'x' }))).toBe('repair')
    expect(pluginAction(plugin({ status: 'ready', enabled: false, error: null }))).toBe('enable-use')
    expect(pluginAction(plugin({ status: 'ready', enabled: true, error: null }))).toBe('use')
  })
})

describe('MarketPage', () => {
  it('shows native bundled capabilities without a principal skill or duplicate installed package', async () => {
    const native = plugin({ status: 'ready', enabled: true, error: null, packageId: 'bundled-id' })
    native.manifest = { ...native.manifest, id: 'github', name: 'GitHub', mainSkillId: null, skillIds: [], setupSkillId: 'github:setup',
      details: { author: 'Kivio', version: '1.0.0', homepage: null, license: null, diagnostics: [], groups: [
        { kind: 'skills', items: [{ name: 'setup', description: '验证环境' }, { name: 'pull-requests', description: '审阅 Pull Request' }] },
      ] } }
    vi.mocked(marketApi.snapshot).mockResolvedValue(snapshot(native))
    vi.mocked(packageApi.list).mockResolvedValue([{ id: 'bundled-id', name: 'github', description: 'Managed package', version: '1.0.0', source: '/plugins/bundled/github', revision: null, enabled: true, format: 'kivio', components: { skills: 2 }, diagnostics: [] }])
    const onUse = vi.fn()
    render(<MarketPage onUse={onUse} onSkillsChanged={vi.fn()} />)
    const installed = await screen.findByRole('button', { name: 'GitHub' })
    expect(installed.querySelector('img')).toHaveAttribute('src', native.manifest.icon)
    fireEvent.click(installed)
    const heading = await screen.findByRole('heading', { name: 'GitHub' })
    expect(heading.parentElement?.parentElement?.querySelector('img')).toHaveAttribute('src', native.manifest.icon)
    expect(screen.getByText('pull-requests')).toBeInTheDocument()
    expect(screen.getByText('验证环境')).toBeInTheDocument()
    expect(screen.queryByText('正在读取插件内容…')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '使用' }))
    await waitFor(() => expect(onUse).toHaveBeenCalledWith(native))
    fireEvent.click(screen.getByRole('button', { name: '插件' }))
    await screen.findByRole('tab', { name: '个人' })
    expect(screen.queryByRole('button', { name: 'github' })).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('tab', { name: '个人' }))
    expect(screen.queryByText('Managed package')).not.toBeInTheDocument()
  })

  it('shows Claude logos in the catalog, details and installed row and falls back on image errors', async () => {
    const source = 'https://github.com/anthropics/claude-plugins-official'
    const adobe = { name: 'adobe-for-creativity', displayName: 'Adobe', description: 'Creative tools', version: null, category: 'design', unavailableReason: null }
    vi.mocked(marketApi.snapshot).mockResolvedValue(snapshot(plugin()))
    vi.mocked(marketplaceApi.list).mockResolvedValue([{ id: 'claude', name: 'claude-plugins-official', source, description: '', plugins: [adobe] }])
    const { container } = render(<MarketPage onUse={vi.fn()} onSkillsChanged={vi.fn()} />)
    await screen.findByRole('button', { name: '安装' })
    fireEvent.click(screen.getByRole('tab', { name: '个人' }))
    const logo = container.querySelector('img')!
    expect(logo).toHaveAttribute('src', 'https://cdn-zcode.z.ai/zcode/official-plugin/assets/adobe-for-creativity/icon.png')
    fireEvent.error(logo)
    expect(container.querySelector('img')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Adobe Creative tools' }))
    await screen.findByRole('heading', { name: 'Adobe' })
    expect(container.querySelector('img')).toHaveAttribute('src', expect.stringContaining('/adobe-for-creativity/icon.png'))
    vi.mocked(packageApi.list).mockResolvedValue([{ id: 'adobe', name: 'Adobe', description: 'Creative tools', source: '/copy', format: 'claude', enabled: false, version: null, revision: null, components: {}, diagnostics: [], marketplace: { source, name: 'claude-plugins-official', plugin: adobe.name } }])
    // Removing a market must not make the installed plugin lose its known logo.
    vi.mocked(marketplaceApi.list).mockResolvedValue([])
    fireEvent(window, new Event('focus'))
    fireEvent.click(screen.getByRole('button', { name: '插件市场' }))
    const installed = await screen.findByRole('button', { name: 'Adobe' })
    expect(installed.querySelector('img')).toHaveAttribute('src', expect.stringContaining('/adobe-for-creativity/icon.png'))
  })

  const customMarket: Marketplace = { id: 'team-market', name: 'team-tools', description: 'Team plugins', source: 'https://github.com/team/tools', plugins: [
    { name: 'demo', displayName: 'Demo plugin', description: 'Build a demo', version: '1', category: 'development', unavailableReason: null },
  ] }

  it('adds a custom market, shows its catalog in Personal and installs into package management', async () => {
    vi.mocked(marketApi.snapshot).mockResolvedValue(snapshot(plugin()))
    vi.mocked(marketplaceApi.add).mockResolvedValue([customMarket])
    vi.mocked(marketplaceApi.install).mockResolvedValue({ id: 'installed-demo', name: 'demo', description: 'Build a demo', version: '1', source: '/cache/demo', revision: null, enabled: false, format: 'claude', components: { skills: 1 }, diagnostics: [], marketplace: { name: customMarket.name, source: customMarket.source, plugin: 'demo' } })
    const changed = vi.fn()
    render(<MarketPage onUse={vi.fn()} onSkillsChanged={changed} />)
    fireEvent.click(await screen.findByRole('button', { name: '添加' }))
    fireEvent.click(screen.getByRole('menuitem', { name: '添加插件市场' }))
    fireEvent.click(screen.getByRole('button', { name: 'Claude 官方市场' }))
    expect(screen.getByLabelText('市场来源')).toHaveValue('anthropics/claude-plugins-official')
    fireEvent.change(screen.getByLabelText('市场来源'), { target: { value: 'team/tools' } })
    fireEvent.click(screen.getByRole('button', { name: '添加市场' }))
    await screen.findByRole('heading', { name: /team-tools/ })
    expect(marketplaceApi.add).toHaveBeenCalledWith('team/tools')
    expect(screen.getByRole('tab', { name: '个人' })).toHaveAttribute('aria-selected', 'true')
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '安装' }))
    await screen.findByRole('heading', { name: 'demo' })
    expect(marketplaceApi.install).toHaveBeenCalledWith('team-market', 'demo')
    expect(screen.getByRole('switch', { name: '加载 demo' })).not.toBeChecked()
    expect(changed).toHaveBeenCalledOnce()
    expect(packageApi.setEnabled).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: '插件市场' }))
    await screen.findByRole('heading', { name: /team-tools/ })
    expect(screen.queryByRole('button', { name: '安装' })).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Demo plugin 更多操作' })).toBeInTheDocument()
  })

  it('keeps failed add input and preserves installed plugins when removing a market source', async () => {
    vi.mocked(marketApi.snapshot).mockResolvedValue(snapshot(plugin()))
    vi.mocked(marketplaceApi.add).mockRejectedValue('Invalid marketplace')
    vi.mocked(marketplaceApi.list).mockResolvedValue([customMarket])
    vi.mocked(marketplaceApi.remove).mockResolvedValue([])
    vi.mocked(packageApi.list).mockResolvedValue([{ id: 'demo', name: 'demo', description: 'Owned copy', version: '1', source: '/owned', revision: null, enabled: false, format: 'claude', components: {}, diagnostics: [], marketplace: { name: customMarket.name, source: customMarket.source, plugin: 'demo' } }])
    render(<MarketPage onUse={vi.fn()} onSkillsChanged={vi.fn()} />)
    await screen.findByRole('button', { name: 'demo' })
    fireEvent.click(screen.getByRole('button', { name: '添加' }))
    fireEvent.click(screen.getByRole('menuitem', { name: '添加插件市场' }))
    fireEvent.change(screen.getByLabelText('市场来源'), { target: { value: 'invalid/repo' } })
    fireEvent.click(screen.getByRole('button', { name: '添加市场' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('Invalid marketplace')
    expect(screen.getByLabelText('市场来源')).toHaveValue('invalid/repo')
    fireEvent.click(screen.getByRole('button', { name: '取消' }))
    fireEvent.click(screen.getByRole('button', { name: '管理插件市场' }))
    fireEvent.click(screen.getByRole('button', { name: '移除 team-tools' }))
    expect(screen.getByText('移除此市场来源？已安装的插件会保留。')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '确认移除' }))
    await screen.findByText('还没有添加市场。通过“添加”菜单添加第一个市场。')
    fireEvent.click(screen.getByRole('button', { name: '完成' }))
    fireEvent.click(screen.getByRole('tab', { name: '个人' }))
    expect(screen.getByRole('button', { name: 'demo Owned copy' })).toBeInTheDocument()
    expect(packageApi.remove).not.toHaveBeenCalled()
  })

  it('shows unsupported sources without invoking an installer', async () => {
    vi.mocked(marketApi.snapshot).mockResolvedValue(snapshot(plugin()))
    vi.mocked(marketplaceApi.list).mockResolvedValue([{ ...customMarket, plugins: [{ ...customMarket.plugins[0], unavailableReason: 'Unsupported source: npm' }] }])
    render(<MarketPage onUse={vi.fn()} onSkillsChanged={vi.fn()} />)
    await screen.findByRole('button', { name: '安装' })
    fireEvent.click(screen.getByRole('tab', { name: '个人' }))
    fireEvent.click(await screen.findByRole('button', { name: '查看原因' }))
    expect(await screen.findByText('Unsupported source: npm')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '安装' })).toBeDisabled()
    expect(marketplaceApi.install).not.toHaveBeenCalled()
  })

  it('preserves the catalog after a failed refresh and supersedes an older focus read after add', async () => {
    vi.mocked(marketApi.snapshot).mockResolvedValue(snapshot(plugin()))
    vi.mocked(marketplaceApi.list).mockResolvedValue([customMarket])
    vi.mocked(marketplaceApi.refresh).mockRejectedValue('Offline')
    render(<MarketPage onUse={vi.fn()} onSkillsChanged={vi.fn()} />)
    await screen.findByRole('button', { name: '安装' })
    fireEvent.click(screen.getByRole('button', { name: '管理插件市场' }))
    fireEvent.click(await screen.findByRole('button', { name: '刷新 team-tools' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('Offline')
    expect(screen.getByText(customMarket.source)).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '完成' }))
    let finishRead!: (next: Marketplace[]) => void
    vi.mocked(marketplaceApi.list).mockImplementationOnce(() => new Promise(resolve => { finishRead = resolve }))
    fireEvent(window, new Event('focus'))
    vi.mocked(marketplaceApi.add).mockResolvedValue([customMarket, { ...customMarket, id: 'second', name: 'second-tools', source: 'https://github.com/team/second' }])
    fireEvent.click(screen.getByRole('button', { name: '添加' }))
    fireEvent.click(screen.getByRole('menuitem', { name: '添加插件市场' }))
    fireEvent.change(screen.getByLabelText('市场来源'), { target: { value: 'team/second' } })
    fireEvent.click(screen.getByRole('button', { name: '添加市场' }))
    await screen.findByRole('heading', { name: /second-tools/ })
    await act(async () => finishRead([customMarket]))
    expect(screen.getByRole('heading', { name: /second-tools/ })).toBeInTheDocument()
  })

  it('unifies installed icons and keeps package diagnostics in its detail', async () => {
    const imported: PluginPackage = { id: 'video', name: 'Video plugin', version: '1', description: 'Video tools', source: '/video', revision: null, format: 'codex', enabled: false, components: { skills: 6 }, diagnostics: ['Missing API variable'] }
    vi.mocked(marketApi.snapshot).mockResolvedValue(snapshot(plugin({ status: 'ready', enabled: true, error: null })))
    vi.mocked(packageApi.list).mockResolvedValue([imported])
    render(<MarketPage onUse={vi.fn()} onSkillsChanged={vi.fn()} />)
    await screen.findByRole('button', { name: 'Video plugin' })
    expect(screen.getByRole('button', { name: '飞书 CLI' })).toBeInTheDocument()
    expect(screen.queryByText('Missing API variable')).not.toBeInTheDocument()
    expect(screen.queryByText('通用插件')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('tab', { name: '个人' }))
    expect(screen.getByRole('button', { name: 'Video plugin Video tools' })).toBeInTheDocument()
    expect(screen.queryByText('效率办公')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Video plugin' }))
    await screen.findByRole('heading', { name: 'Video plugin' })
    expect(screen.getByText('Missing API variable')).toBeInTheDocument()
    expect(screen.getByRole('switch', { name: '加载 Video plugin' })).toBeDisabled()
    expect(screen.getByRole('button', { name: '移除' })).toBeEnabled()
  })

  it('keeps the import form and entered source open when import fails', async () => {
    vi.mocked(marketApi.snapshot).mockResolvedValue(snapshot(plugin()))
    vi.mocked(packageApi.import).mockRejectedValue('Invalid manifest')
    render(<MarketPage onUse={vi.fn()} onSkillsChanged={vi.fn()} />)
    fireEvent.click(await screen.findByRole('button', { name: '添加' }))
    fireEvent.click(screen.getByRole('menuitem', { name: '从 Git 仓库导入' }))
    fireEvent.change(screen.getByLabelText('插件来源'), { target: { value: 'https://github.com/example/plugin' } })
    fireEvent.click(screen.getByRole('button', { name: '导入' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('Invalid manifest')
    expect(screen.getByLabelText('插件来源')).toHaveValue('https://github.com/example/plugin')
    fireEvent.click(screen.getByRole('button', { name: '取消' }))
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
  })

  it('still lists imported plugins when the public catalog cannot load', async () => {
    vi.mocked(marketApi.snapshot).mockRejectedValue('Catalog unavailable')
    vi.mocked(packageApi.list).mockResolvedValue([{ id: 'local', name: 'Local plugin', version: null, description: '', source: '/local', revision: null, format: 'kivio', enabled: false, components: {}, diagnostics: [] }])
    render(<MarketPage onUse={vi.fn()} onSkillsChanged={vi.fn()} />)
    expect(await screen.findByRole('button', { name: 'Local plugin' })).toBeInTheDocument()
    expect(await screen.findByRole('status')).toHaveTextContent('Catalog unavailable')
  })

  it('manages imported packages alongside catalog plugins and refreshes chat capabilities', async () => {
    const onSkillsChanged = vi.fn()
    const imported: PluginPackage = {
      id: 'local-example', name: 'Local example', version: '1', description: 'Example skill',
      source: '/plugins/example', revision: null, format: 'kivio', enabled: false,
      components: { skills: 1 }, diagnostics: [],
    }
    vi.mocked(marketApi.snapshot).mockResolvedValue(snapshot(plugin()))
    vi.mocked(open).mockResolvedValue('/plugins/example')
    vi.mocked(packageApi.import).mockImplementation(async () => {
      vi.mocked(packageApi.list).mockResolvedValue([imported])
      return imported
    })
    vi.mocked(packageApi.setEnabled).mockImplementation(async () => {
      vi.mocked(packageApi.list).mockResolvedValue([{ ...imported, enabled: true }])
      return { ...imported, enabled: true }
    })
    vi.mocked(packageApi.remove).mockImplementation(async () => {
      vi.mocked(packageApi.list).mockResolvedValue([])
    })
    render(<MarketPage onUse={vi.fn()} onSkillsChanged={onSkillsChanged} />)
    await screen.findByRole('button', { name: '安装' })
    expect(screen.queryByLabelText('插件来源')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '添加' }))
    fireEvent.click(screen.getByRole('menuitem', { name: '从本地目录导入' }))
    fireEvent.click(screen.getByRole('button', { name: '选择目录' }))
    await waitFor(() => expect(screen.getByLabelText('插件来源')).toHaveValue('/plugins/example'))
    fireEvent.click(screen.getByRole('button', { name: '导入' }))
    await screen.findByRole('heading', { name: 'Local example' })
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    expect(packageApi.setEnabled).not.toHaveBeenCalled()
    expect(packageApi.import).toHaveBeenCalledWith('/plugins/example', undefined)
    fireEvent.click(screen.getByRole('switch', { name: '加载 Local example' }))
    await waitFor(() => expect(screen.getByRole('switch')).toHaveAttribute('aria-checked', 'true'))
    fireEvent.click(screen.getByRole('button', { name: '移除' }))
    await waitFor(() => expect(screen.queryByText('Local example')).not.toBeInTheDocument())
    expect(onSkillsChanged).toHaveBeenCalledTimes(3)
    expect(packageApi.remove).toHaveBeenCalledWith('local-example')
    fireEvent.click(screen.getByRole('tab', { name: '公开' }))
    expect(screen.getByRole('button', { name: '安装' })).toBeInTheDocument()
  })

  it('keeps the installed state when an older refresh finishes after installation', async () => {
    const initial = snapshot(plugin())
    const installed = snapshot(plugin({ status: 'ready', enabled: true, error: null }))
    let finishRefresh!: (value: MarketSnapshot) => void
    vi.mocked(marketApi.snapshot).mockResolvedValueOnce(initial)
      .mockImplementationOnce(() => new Promise((resolve) => { finishRefresh = resolve }))
    vi.mocked(marketApi.install).mockResolvedValue(installed)
    render(<MarketPage onUse={vi.fn()} onSkillsChanged={vi.fn()} />)
    await screen.findByRole('button', { name: '安装' })
    fireEvent.focus(window)
    fireEvent.click(screen.getByRole('button', { name: '安装' }))
    fireEvent.click(await screen.findByRole('button', { name: '飞书 CLI' }))
    await screen.findByRole('button', { name: '使用' })
    await act(async () => { finishRefresh(initial) })
    expect(screen.getByRole('button', { name: '使用' })).toBeEnabled()
    expect(screen.queryByRole('button', { name: '安装' })).not.toBeInTheDocument()
  })

  it('ignores an older failed refresh after a newer refresh succeeds', async () => {
    let failRefresh!: (reason: string) => void
    vi.mocked(marketApi.snapshot)
      .mockImplementationOnce(() => new Promise((_resolve, reject) => { failRefresh = reject }))
      .mockResolvedValue(snapshot(plugin()))
    render(<MarketPage onUse={vi.fn()} onSkillsChanged={vi.fn()} />)
    fireEvent.focus(window)
    await screen.findByRole('button', { name: '安装' })
    await act(async () => { failRefresh('old network failure') })
    expect(screen.queryByRole('status')).not.toBeInTheDocument()
  })

  it('lists catalog plugins by category and installs one', async () => {
    const onSkillsChanged = vi.fn()
    vi.mocked(marketApi.snapshot).mockResolvedValue(snapshot(plugin()))
    vi.mocked(marketApi.install).mockResolvedValue(snapshot(plugin({ status: 'ready', enabled: true, error: null })))
    render(<MarketPage onUse={vi.fn()} onSkillsChanged={onSkillsChanged} />)
    await screen.findByText('效率办公')
    expect(screen.getByText('还没有安装插件')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '安装' }))
    await waitFor(() => expect(marketApi.install).toHaveBeenCalledWith('feishu-cli'))
    fireEvent.click(await screen.findByRole('button', { name: '飞书 CLI' }))
    await screen.findByRole('button', { name: '使用' })
    expect(screen.getByRole('heading', { name: '飞书 CLI' })).toBeInTheDocument()
    expect(onSkillsChanged).toHaveBeenCalled()
  })

  it('surfaces install failures without claiming success', async () => {
    vi.mocked(marketApi.snapshot).mockResolvedValue(snapshot(plugin()))
    vi.mocked(marketApi.install).mockRejectedValue('网络错误')
    render(<MarketPage onUse={vi.fn()} onSkillsChanged={vi.fn()} />)
    fireEvent.click(await screen.findByRole('button', { name: '安装' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('网络错误')
    expect(screen.getByRole('button', { name: '安装' })).toBeEnabled()
  })

  it('loads an unloaded plugin before starting a chat with it', async () => {
    window.location.hash = '#chat/plugins/feishu-cli'
    const onUse = vi.fn().mockResolvedValue(undefined)
    const ready = plugin({ status: 'ready', enabled: true, error: null })
    vi.mocked(marketApi.snapshot).mockResolvedValue(snapshot(plugin({ status: 'ready', enabled: false, error: null })))
    vi.mocked(marketApi.setEnabled).mockResolvedValue(snapshot(ready))
    render(<MarketPage onUse={onUse} onSkillsChanged={vi.fn()} />)
    fireEvent.click(await screen.findByRole('button', { name: '加载并使用' }))
    await waitFor(() => expect(onUse).toHaveBeenCalled())
    expect(marketApi.setEnabled).toHaveBeenCalledWith('feishu-cli', true)
  })

  it('shows plugin detail with its skills from the route', async () => {
    window.location.hash = '#chat/plugins/feishu-cli'
    vi.mocked(marketApi.snapshot).mockResolvedValue(snapshot(plugin({ status: 'ready', enabled: true, error: null })))
    render(<MarketPage onUse={vi.fn()} onSkillsChanged={vi.fn()} />)
    await screen.findByRole('heading', { name: '飞书 CLI', level: 1 })
    expect(screen.getByText('feishu-cli-setup')).toBeInTheDocument()
    expect(screen.getByRole('switch', { name: '加载 飞书 CLI' })).toHaveAttribute('aria-checked', 'true')
  })

  function clickTwice(element: HTMLElement) {
    act(() => {
      element.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }))
      element.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }))
    })
  }

  function openFeishuDetail() {
    window.location.hash = '#chat/plugins/feishu-cli'
  }

  it('keeps a catalog install across leaving, rejects a second start, and applies the result on return', async () => {
    const onSkillsChanged = vi.fn()
    const pending = deferred<MarketSnapshot>()
    const installed = snapshot(plugin({ status: 'ready', enabled: true, error: null }))
    vi.mocked(marketApi.snapshot).mockResolvedValue(snapshot(plugin()))
    vi.mocked(marketApi.install).mockReturnValue(pending.promise)
    const first = render(<MarketPage onUse={vi.fn()} onSkillsChanged={onSkillsChanged} />)
    const install = await screen.findByRole('button', { name: '安装' })
    clickTwice(install)
    await waitFor(() => expect(marketApi.install).toHaveBeenCalledTimes(1))
    expect(install).toBeDisabled()
    first.unmount()
    const second = render(<MarketPage onUse={vi.fn()} onSkillsChanged={onSkillsChanged} />)
    const again = await screen.findByRole('button', { name: '安装' })
    expect(again).toBeDisabled()
    fireEvent.click(again)
    expect(marketApi.install).toHaveBeenCalledTimes(1)
    second.unmount()
    vi.mocked(marketApi.snapshot).mockResolvedValue(installed)
    await act(async () => { pending.resolve(installed) })
    render(<MarketPage onUse={vi.fn()} onSkillsChanged={onSkillsChanged} />)
    await waitFor(() => expect(onSkillsChanged).toHaveBeenCalled())
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: '飞书 CLI' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '安装' })).not.toBeInTheDocument()
  })

  it('shows a queued install failure after leaving and keeps retry available', async () => {
    const pending = deferred<MarketSnapshot>()
    const installed = snapshot(plugin({ status: 'ready', enabled: true, error: null }))
    vi.mocked(marketApi.snapshot).mockResolvedValue(snapshot(plugin()))
    vi.mocked(marketApi.install).mockReturnValue(pending.promise)
    const view = render(<MarketPage onUse={vi.fn()} onSkillsChanged={vi.fn()} />)
    fireEvent.click(await screen.findByRole('button', { name: '安装' }))
    await waitFor(() => expect(marketApi.install).toHaveBeenCalledTimes(1))
    view.unmount()
    await act(async () => { pending.reject('queued') })
    render(<MarketPage onUse={vi.fn()} onSkillsChanged={vi.fn()} />)
    expect(await screen.findByRole('alert')).toHaveTextContent('queued')
    const retry = screen.getByRole('button', { name: '安装' })
    expect(retry).toBeEnabled()
    vi.mocked(marketApi.install).mockResolvedValue(installed)
    fireEvent.click(retry)
    await waitFor(() => expect(marketApi.install).toHaveBeenCalledTimes(2))
    await waitFor(() => expect(screen.queryByRole('alert')).not.toBeInTheDocument())
  })

  it('keeps marketplace add in flight when the dialog or page closes without pretending it was cancelled', async () => {
    const pending = deferred<Marketplace[]>()
    vi.mocked(marketApi.snapshot).mockResolvedValue(snapshot(plugin()))
    vi.mocked(marketplaceApi.add).mockReturnValue(pending.promise)
    const first = render(<MarketPage onUse={vi.fn()} onSkillsChanged={vi.fn()} />)
    await screen.findByRole('button', { name: '安装' })
    fireEvent.click(screen.getByRole('button', { name: '添加' }))
    fireEvent.click(screen.getByRole('menuitem', { name: '添加插件市场' }))
    fireEvent.change(screen.getByLabelText('市场来源'), { target: { value: 'team/tools' } })
    const add = screen.getByRole('button', { name: '添加市场' })
    clickTwice(add)
    await waitFor(() => expect(marketplaceApi.add).toHaveBeenCalledTimes(1))
    expect(marketplaceApi.add).toHaveBeenCalledWith('team/tools')
    expect(screen.getByText('正在读取市场，请稍候…')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '取消' }))
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
    first.unmount()
    render(<MarketPage onUse={vi.fn()} onSkillsChanged={vi.fn()} />)
    await screen.findByRole('button', { name: '安装' })
    fireEvent.click(screen.getByRole('button', { name: '添加' }))
    fireEvent.click(screen.getByRole('menuitem', { name: '添加插件市场' }))
    expect(screen.getByLabelText('市场来源')).toHaveValue('team/tools')
    expect(screen.getByText('正在读取市场，请稍候…')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '添加市场' })).toBeDisabled()
    fireEvent.click(screen.getByRole('button', { name: '添加市场' }))
    expect(marketplaceApi.add).toHaveBeenCalledTimes(1)
    fireEvent.click(screen.getByRole('button', { name: '取消' }))
    openFeishuDetail()
    expect(await screen.findByRole('heading', { name: '飞书 CLI', level: 1 })).toBeInTheDocument()
    await act(async () => { pending.resolve([customMarket]) })
    expect(screen.getByRole('heading', { name: '飞书 CLI', level: 1 })).toBeInTheDocument()
    expect(screen.queryByRole('heading', { name: /team-tools/ })).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '插件' }))
    fireEvent.click(await screen.findByRole('tab', { name: '个人' }))
    expect(await screen.findByRole('heading', { name: /team-tools/ })).toBeInTheDocument()
  })

  it('does not let a closed folder picker overwrite a newer marketplace source', async () => {
    const pending = deferred<string | null>()
    vi.mocked(marketApi.snapshot).mockResolvedValue(snapshot(plugin()))
    vi.mocked(open).mockReturnValue(pending.promise)
    render(<MarketPage onUse={vi.fn()} onSkillsChanged={vi.fn()} />)
    await screen.findByRole('button', { name: '安装' })
    fireEvent.click(screen.getByRole('button', { name: '添加' }))
    fireEvent.click(screen.getByRole('menuitem', { name: '添加插件市场' }))
    fireEvent.click(screen.getByRole('button', { name: '本地目录' }))
    fireEvent.click(screen.getByRole('button', { name: '取消' }))
    fireEvent.click(screen.getByRole('button', { name: '添加' }))
    fireEvent.click(screen.getByRole('menuitem', { name: '添加插件市场' }))
    fireEvent.change(screen.getByLabelText('市场来源'), { target: { value: 'team/kept' } })
    await act(async () => { pending.resolve('/late/folder') })
    expect(screen.getByLabelText('市场来源')).toHaveValue('team/kept')
  })

  it('applies a no-op marketplace install without opening it over a plugin the user already chose', async () => {
    const pending = deferred<PluginPackage>()
    const installedPackage: PluginPackage = {
      id: 'installed-demo', name: 'demo', description: 'Build a demo', version: '1', source: '/cache/demo',
      revision: null, enabled: false, format: 'claude', components: { skills: 1 }, diagnostics: [],
      marketplace: { name: customMarket.name, source: customMarket.source, plugin: 'demo' },
    }
    vi.mocked(marketApi.snapshot).mockResolvedValue(snapshot(plugin()))
    vi.mocked(marketplaceApi.list).mockResolvedValue([customMarket])
    vi.mocked(marketplaceApi.install).mockReturnValue(pending.promise)
    render(<MarketPage onUse={vi.fn()} onSkillsChanged={vi.fn()} />)
    await screen.findByRole('button', { name: '安装' })
    fireEvent.click(screen.getByRole('tab', { name: '个人' }))
    const install = await screen.findByRole('button', { name: '安装' })
    clickTwice(install)
    await waitFor(() => expect(marketplaceApi.install).toHaveBeenCalledTimes(1))
    expect(marketplaceApi.install).toHaveBeenCalledWith('team-market', 'demo')
    openFeishuDetail()
    expect(await screen.findByRole('heading', { name: '飞书 CLI', level: 1 })).toBeInTheDocument()
    await act(async () => { pending.resolve(installedPackage) })
    expect(screen.getByRole('heading', { name: '飞书 CLI', level: 1 })).toBeInTheDocument()
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '插件' }))
    fireEvent.click(await screen.findByRole('tab', { name: '个人' }))
    expect(screen.getByRole('button', { name: 'Demo plugin 更多操作' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '安装' })).not.toBeInTheDocument()
  })

  it('keeps an import draft across closing the dialog and does not navigate when that dialog is gone', async () => {
    const pending = deferred<PluginPackage>()
    const imported: PluginPackage = {
      id: 'local-example', name: 'Local example', version: '1', description: 'Example skill',
      source: 'https://github.com/example/plugin', revision: null, format: 'kivio', enabled: false,
      components: { skills: 1 }, diagnostics: [],
    }
    vi.mocked(marketApi.snapshot).mockResolvedValue(snapshot(plugin()))
    vi.mocked(packageApi.import)
      .mockReturnValueOnce(pending.promise)
      .mockRejectedValueOnce('queued')
    render(<MarketPage onUse={vi.fn()} onSkillsChanged={vi.fn()} />)
    await screen.findByRole('button', { name: '安装' })
    fireEvent.click(screen.getByRole('button', { name: '添加' }))
    fireEvent.click(screen.getByRole('menuitem', { name: '从 Git 仓库导入' }))
    fireEvent.change(screen.getByLabelText('插件来源'), { target: { value: 'https://github.com/example/plugin' } })
    fireEvent.change(screen.getByLabelText('插件子目录'), { target: { value: 'plugins/my-plugin' } })
    const submit = screen.getByRole('button', { name: '导入' })
    clickTwice(submit)
    await waitFor(() => expect(packageApi.import).toHaveBeenCalledTimes(1))
    expect(packageApi.import).toHaveBeenCalledWith('https://github.com/example/plugin', 'plugins/my-plugin')
    expect(screen.getByRole('button', { name: '正在导入…' })).toBeDisabled()
    fireEvent.click(screen.getByRole('button', { name: '取消' }))
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    openFeishuDetail()
    expect(await screen.findByRole('heading', { name: '飞书 CLI', level: 1 })).toBeInTheDocument()
    vi.mocked(packageApi.list).mockResolvedValue([imported])
    await act(async () => { pending.resolve(imported) })
    expect(screen.getByRole('heading', { name: '飞书 CLI', level: 1 })).toBeInTheDocument()
    expect(screen.queryByRole('heading', { name: 'Local example' })).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '插件' }))
    fireEvent.click(await screen.findByRole('tab', { name: '个人' }))
    expect(screen.getByRole('button', { name: 'Local example' })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('tab', { name: '公开' }))
    fireEvent.click(screen.getByRole('button', { name: '添加' }))
    fireEvent.click(screen.getByRole('menuitem', { name: '从 Git 仓库导入' }))
    fireEvent.change(screen.getByLabelText('插件来源'), { target: { value: 'https://github.com/example/retry' } })
    fireEvent.click(screen.getByRole('button', { name: '导入' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('queued')
    expect(screen.getByLabelText('插件来源')).toHaveValue('https://github.com/example/retry')
    expect(screen.getByRole('button', { name: '导入' })).toBeEnabled()
  })
})
