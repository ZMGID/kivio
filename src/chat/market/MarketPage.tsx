import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { listen } from '@tauri-apps/api/event'
import { ArrowLeft, ChevronDown, ExternalLink, FolderOpen, GitBranch, Loader2, MoreHorizontal, Plus, RefreshCw, Settings2 } from 'lucide-react'
import { api, isTauriRuntime } from '../../api/tauri'
import { MARKET_CHANGED_EVENT, type Marketplace, type MarketplacePlugin, type MarketPlugin } from '../../api/market'
import type { PluginPackage } from '../../api/pluginPackages'
import { Button, IconButton } from '../../components/Button'
import { confirmDialog } from '../../components/dialogQueue'
import { useLang } from '../../components/i18n'
import { Input, Toggle } from '../../settings/public/controls'
import { DefaultPluginIcon } from '../../settings/public/icons'
import { DockContextMenu, type DockMenuAnchor } from '../dock/DockContextMenu'
import { useWindowStore } from '../../utils/windowStore'
import { PluginImportDialog } from './PluginImportDialog'
import { MarketplaceDialog } from './MarketplaceDialog'
import { claudeMarketplaceIcon, marketDetailHash, marketHash, marketPluginIdFromHash, pluginAction } from './marketModel'
import { PluginContents } from './PluginContents'
import {
  discardMarketInventoryReads,
  entryKey,
  marketWindow,
  packageKey,
  refreshMarketInventory,
  runCatalogPlugin,
  runInstallEntry,
  runRemovePackage,
  runSetEnabled,
  runTogglePackage,
  runUninstall,
  subscribeMarketWindow,
  type ImportKind,
  type MarketIntent,
} from './marketOperations'
import './market.css'

/** 离开再回来时保留搜索词与滚动位置。 */
const viewState = { query: '', scroll: 0 }

export type MarketPageProps = {
  heading?: ReactNode
  /** 用插件的主 Skill 开一个新对话。 */
  onUse: (plugin: MarketPlugin) => Promise<void>
  /** 安装、卸载、加载切换后刷新对话里的 Skill 列表。 */
  onSkillsChanged: () => void
}

function PluginIcon({ plugin, src, size = 'md' }: { plugin?: MarketPlugin; src?: string; size?: 'md' | 'lg' }) {
  const [failedSrc, setFailedSrc] = useState<string | null>(null)
  return (
    <span className={`kv-market-icon is-${size}`} aria-hidden="true">
      {src && src !== failedSrc ? <img className="kv-market-logo" src={src} alt="" draggable={false} loading="lazy" referrerPolicy="no-referrer" onError={() => setFailedSrc(src)} />
        : plugin?.manifest.icon ? <span className="kv-market-brand-mark" style={{ maskImage: `url("${plugin.manifest.icon}")` }} /> : <DefaultPluginIcon size={size === 'lg' ? 30 : 18} strokeWidth={1.75} />}
    </span>
  )
}

export function MarketPage({ onUse, onSkillsChanged, heading }: MarketPageProps) {
  const zh = useLang() === 'zh'
  const text = (cn: string, en: string) => (zh ? cn : en)
  const [state] = useWindowStore(marketWindow)
  const { snapshot, packages, packageError, markets, marketError, loading, loadError, busyIds, actionError } = state
  const [marketDialog, setMarketDialog] = useState<'add' | 'manage' | null>(null)
  const [scope, setScope] = useState<'public' | 'personal'>('public')
  const [addMenu, setAddMenu] = useState<DockMenuAnchor | null>(null)
  const [importKind, setImportKind] = useState<ImportKind | null>(null)
  const [query, setQuery] = useState(viewState.query)
  const [selected, setSelected] = useState(marketPluginIdFromHash)
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({})
  const scroller = useRef<HTMLDivElement>(null)
  const surface = useRef({ marketDialog, importKind, onUse, onSkillsChanged })
  surface.current = { marketDialog, importKind, onUse, onSkillsChanged }

  useEffect(() => {
    const onSkills = () => { surface.current.onSkillsChanged() }
    const onIntent = (intent: MarketIntent): void | Promise<void> => {
      const current = surface.current
      if (intent.type === 'market-added') {
        if (current.marketDialog !== 'add') return
        setScope('personal')
        setQuery('')
        setMarketDialog(null)
        return
      }
      if (intent.type === 'imported') {
        if (current.importKind !== intent.kind) return
        setImportKind(null)
        setScope('personal')
        setQuery('')
        window.location.hash = marketDetailHash(packageKey(intent.plugin))
        return
      }
      if (intent.type === 'open-package') {
        const hashId = marketPluginIdFromHash()
        if (hashId === intent.entryKey || window.location.hash === marketHash()) {
          window.location.hash = marketDetailHash(packageKey({ id: intent.packageId }))
        }
        return
      }
      if (intent.type === 'leave-package') {
        if (marketPluginIdFromHash() !== intent.packageKey) return
        setScope('personal')
        window.location.hash = marketHash()
        return
      }
      if (intent.type === 'use-plugin' && marketPluginIdFromHash() === intent.plugin.manifest.id) {
        return current.onUse(intent.plugin)
      }
    }
    return subscribeMarketWindow({ onSkills, onIntent })
  }, [])

  useEffect(() => {
    void refreshMarketInventory()
    if (!isTauriRuntime()) return
    let disposed = false
    let unlisten: (() => void) | undefined
    void listen(MARKET_CHANGED_EVENT, () => { if (!disposed) void refreshMarketInventory() })
      .then((fn) => { if (disposed) fn(); else unlisten = fn })
      .catch(() => { /* 窗口聚焦时仍会刷新 */ })
    const onFocus = () => { if (!disposed) void refreshMarketInventory() }
    window.addEventListener('focus', onFocus)
    return () => {
      disposed = true
      discardMarketInventoryReads()
      unlisten?.()
      window.removeEventListener('focus', onFocus)
    }
  }, [])

  useEffect(() => {
    const onHash = () => { setSelected(marketPluginIdFromHash()); setAddMenu(null) }
    window.addEventListener('hashchange', onHash)
    return () => window.removeEventListener('hashchange', onHash)
  }, [])
  useEffect(() => { viewState.query = query }, [query])
  useEffect(() => {
    if (!selected && scroller.current) scroller.current.scrollTop = viewState.scroll
  }, [selected])

  const matches = (plugin: MarketPlugin) => {
    const q = query.trim().toLowerCase()
    return !q || `${plugin.manifest.name} ${plugin.manifest.summary}`.toLowerCase().includes(q)
  }
  const installed = snapshot.plugins.filter((p) => p.local?.status === 'ready' && matches(p))
  const sections = useMemo(() => {
    const listed = snapshot.plugins.filter(matches)
    const buckets = new Map<string, MarketPlugin[]>()
    for (const plugin of listed) {
      const id = plugin.manifest.categoryIds[0] || 'other'
      buckets.set(id, [...(buckets.get(id) ?? []), plugin])
    }
    const ordered = snapshot.categories.flatMap((category) => {
      const group = buckets.get(category.id)
      if (!group?.length) return []
      buckets.delete(category.id)
      return [{ id: category.id, name: category.name, items: group }]
    })
    for (const [id, items] of buckets) ordered.push({ id, name: id === 'other' ? text('其他', 'Other') : id, items })
    return ordered
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [snapshot, query, zh])

  const chosen = snapshot.plugins.find((p) => p.manifest.id === selected)
  const chosenPackage = packages.find(p => packageKey(p) === selected)
  const personal = packages.filter(p => `${p.name} ${p.description}`.toLowerCase().includes(query.trim().toLowerCase()))
  const entryPackage = (market: Marketplace, entry: MarketplacePlugin) => packages.find(p => p.marketplace?.source === market.source && p.marketplace?.plugin === entry.name)
  const packageIcon = (plugin: PluginPackage) => claudeMarketplaceIcon(plugin.marketplace?.source, plugin.marketplace?.plugin ?? plugin.name)
  const chosenEntry = markets.flatMap(market => market.plugins.map(entry => ({ market, entry }))).find(({ market, entry }) => entryKey(market, entry) === selected)
  const catalogSections = markets.map(market => ({ market, entries: market.plugins.filter(entry => `${entry.name} ${entry.displayName} ${entry.description} ${entry.category}`.toLowerCase().includes(query.trim().toLowerCase())) })).filter(section => section.entries.length > 0)
  const standalone = personal.filter(p => !markets.some(market => market.source === p.marketplace?.source && market.plugins.some(entry => entry.name === p.marketplace?.plugin)))
  const openPackage = (plugin: PluginPackage) => { window.location.hash = marketDetailHash(packageKey(plugin)) }
  const removePackage = async (plugin: PluginPackage) => {
    const ok = await confirmDialog({ title: text('移除插件', 'Remove plugin'),
      message: text(`移除「${plugin.name}」？导入的插件及其能力将被移除。`, `Remove "${plugin.name}" and its imported capabilities?`),
      confirmLabel: text('移除', 'Remove'), danger: true })
    if (ok) void runRemovePackage(plugin)
  }

  const actionLabel = (action: ReturnType<typeof pluginAction>) => ({
    install: text('安装', 'Install'),
    repair: text('重新配置', 'Repair'),
    use: text('使用', 'Use'),
    'enable-use': text('加载并使用', 'Load and use'),
  })[action]

  const primaryButton = (plugin: MarketPlugin, variant: 'default' | 'primary' = 'default') => {
    const busy = busyIds.has(plugin.manifest.id)
    return (
      <Button variant={variant} size="sm" disabled={busy} onClick={() => void runCatalogPlugin(plugin)}>
        {busy && <Loader2 size={14} className="animate-spin" />}
        {actionLabel(pluginAction(plugin))}
      </Button>
    )
  }

  const statusText = (plugin: MarketPlugin) => {
    if (!plugin.local) return ''
    if (plugin.local.status === 'failed') return text('需修复', 'Needs repair')
    return plugin.local.enabled ? '' : text('未加载', 'Not loaded')
  }

  const uninstall = async (plugin: MarketPlugin) => {
    const ok = await confirmDialog({
      title: text('卸载插件', 'Uninstall plugin'),
      message: text(
        `卸载「${plugin.manifest.name}」？市场安装的 Skill 会被移除，你自己的文件和外部 CLI 不受影响。`,
        `Uninstall "${plugin.manifest.name}"? Market-installed skills are removed; your own files and external CLIs are kept.`,
      ),
      confirmLabel: text('卸载', 'Uninstall'),
      danger: true,
    })
    if (ok) void runUninstall(plugin.manifest.id)
  }

  const detail = chosen && (
    <div className="kv-market-detail">
      <nav className="kv-market-crumbs" aria-label={text('位置', 'Breadcrumb')}>
        <button type="button" onClick={() => { window.location.hash = marketHash() }}>
          <ArrowLeft size={14} />{text('插件', 'Plugins')}
        </button>
        <span>/</span>
        <span>{chosen.manifest.name}</span>
      </nav>
      <PluginIcon plugin={chosen} size="lg" />
      <header className="kv-market-detail-head">
        <h1>{chosen.manifest.name}</h1>
        <div className="kv-market-detail-actions">
          {chosen.local?.status === 'ready' && (
            <label className="kv-market-load">
              <span>{chosen.local.enabled ? text('已加载', 'Loaded') : text('未加载', 'Not loaded')}</span>
              <Toggle
                checked={chosen.local.enabled}
                disabled={busyIds.has(chosen.manifest.id)}
                ariaLabel={text(`加载 ${chosen.manifest.name}`, `Load ${chosen.manifest.name}`)}
                onChange={(enabled) => void runSetEnabled(chosen.manifest.id, enabled)}
              />
            </label>
          )}
          {chosen.local && <Button size="sm" disabled={busyIds.has(chosen.manifest.id)} onClick={() => void uninstall(chosen)}>{text('卸载', 'Uninstall')}</Button>}
          {primaryButton(chosen, 'primary')}
        </div>
      </header>
      <p className="kv-market-summary">{chosen.manifest.summary}</p>
      {chosen.local?.status === 'failed' && (
        <p className="kv-market-warning">{text('插件组件缺失，点击“重新配置”修复。', 'Some components are missing. Use Repair to restore them.')}</p>
      )}

      <section className="kv-market-block">
        <h2>{text('技能', 'Skills')} <small>{1 + chosen.manifest.skillIds.length}</small></h2>
        {chosen.manifest.skillIds.map((skill) => (
          <div key={skill} className="kv-market-component">
            <strong>{skill}</strong>
            <span>{skill === chosen.manifest.mainSkillId ? chosen.manifest.welcome : text('安装后即可使用', 'Available after installation')}</span>
          </div>
        ))}
        <div className="kv-market-component">
          <strong>{chosen.manifest.setupSkillId}</strong>
          <span>{text('检查 Kivio 接入，补齐运行环境并验证登录', 'Check the Kivio integration, fill in dependencies and verify sign-in')}</span>
        </div>
      </section>

      {chosen.manifest.checkCommand && (
        <section className="kv-market-block">
          <h2>{text('命令', 'Commands')} <small>1</small></h2>
          <div className="kv-market-component">
            <code>{chosen.manifest.checkCommand}</code>
            <span>{text('检查接入、配置和实际可用性', 'Check integration, configuration and readiness')}</span>
          </div>
        </section>
      )}

      <section className="kv-market-block">
        <h2>{text('信息', 'Information')}</h2>
        <dl className="kv-market-info">
          <dt>{text('示例', 'Example')}</dt><dd>{chosen.manifest.inputHint}</dd>
          <dt>{text('类别', 'Category')}</dt>
          <dd>{chosen.manifest.categoryIds.map((id) => snapshot.categories.find((c) => c.id === id)?.name ?? id).join(' · ')}</dd>
          <dt>{text('版本', 'Version')}</dt><dd className="kv-market-mono">{chosen.manifest.revision.slice(0, 10)}</dd>
          {chosen.manifest.repository && <>
            <dt>{text('网站', 'Website')}</dt>
            <dd>
              <IconButton
                size="xs"
                variant="ghost"
                label={`github.com/${chosen.manifest.repository}`}
                onClick={() => void api.openExternal(`https://github.com/${chosen.manifest.repository}`)}
              >
                <ExternalLink size={14} />
              </IconButton>
            </dd>
          </>}
        </dl>
      </section>
    </div>
  )

  const packageDetail = chosenPackage && (
    <div className="kv-market-detail">
      <nav className="kv-market-crumbs" aria-label={text('位置', 'Breadcrumb')}>
        <button type="button" onClick={() => { window.location.hash = marketHash() }}><ArrowLeft size={14} />{text('插件市场', 'Plugin marketplace')}</button>
        <span>/</span><span>{chosenPackage.name}</span>
      </nav>
      <PluginIcon size="lg" src={packageIcon(chosenPackage)} />
      <header className="kv-market-detail-head">
        <h1>{chosenPackage.name}</h1>
        <div className="kv-market-detail-actions">
          <label className="kv-market-load">
            <span>{chosenPackage.enabled ? text('已加载', 'Loaded') : text('未加载', 'Not loaded')}</span>
            <Toggle checked={chosenPackage.enabled} ariaLabel={text(`加载 ${chosenPackage.name}`, `Load ${chosenPackage.name}`)}
              disabled={busyIds.has(packageKey(chosenPackage)) || (!chosenPackage.enabled && chosenPackage.diagnostics.length > 0)}
              onChange={enabled => void runTogglePackage(chosenPackage, enabled)} />
          </label>
          <Button size="sm" disabled={busyIds.has(packageKey(chosenPackage))} onClick={() => void removePackage(chosenPackage)}>{text('移除', 'Remove')}</Button>
        </div>
      </header>
      <p className="kv-market-summary">{chosenPackage.description}</p>
      {chosenPackage.diagnostics.length > 0 && <section className="kv-market-block">
        <h2>{text('需要配置', 'Configuration needed')}</h2>
        <p className="kv-market-muted">{text('解决以下问题后即可启用插件。', 'Resolve these issues to enable the plugin.')}</p>
        {chosenPackage.diagnostics.map((message, i) => <p className="kv-market-error" key={i}>{message}</p>)}
      </section>}
      <PluginContents key={packageKey(chosenPackage)} packageId={chosenPackage.id} version={chosenPackage.version} information={<>
          <dt>{text('来源', 'Source')}</dt><dd>{chosenPackage.marketplace?.source ?? chosenPackage.source}</dd>
          {chosenPackage.marketplace && <><dt>{text('市场', 'Marketplace')}</dt><dd>{chosenPackage.marketplace.name}</dd></>}
          <dt>{text('格式', 'Format')}</dt><dd>{chosenPackage.format}</dd>
          {chosenPackage.revision && <><dt>{text('修订', 'Revision')}</dt><dd className="kv-market-mono">{chosenPackage.revision.slice(0, 10)}</dd></>}
          <dt>{text('使用范围', 'Scope')}</dt><dd>{text('个人 · 内置 Kivio Agent', 'Personal · Built-in Kivio Agent')}</dd>
        </>}>
        <p className="kv-market-summary">{text('启用会加载插件能力并允许执行 Hook 脚本；依赖需自行安装。', 'Enabling loads capabilities and permits hook scripts. Install dependencies separately.')}</p>
      </PluginContents>
    </div>
  )

  const catalogDetail = chosenEntry && <div className="kv-market-detail">
    <nav className="kv-market-crumbs" aria-label={text('位置', 'Breadcrumb')}>
      <button type="button" onClick={() => { window.location.hash = marketHash() }}><ArrowLeft size={14} />{text('插件市场', 'Plugin marketplace')}</button>
      <span>/</span><span>{chosenEntry.entry.displayName}</span>
    </nav>
    <PluginIcon size="lg" src={claudeMarketplaceIcon(chosenEntry.market.source, chosenEntry.entry.name)} />
    <header className="kv-market-detail-head"><h1>{chosenEntry.entry.displayName}</h1>
      <Button variant="primary" disabled={!!chosenEntry.entry.unavailableReason || busyIds.has(entryKey(chosenEntry.market, chosenEntry.entry))}
        onClick={() => void runInstallEntry(chosenEntry.market, chosenEntry.entry)}>
        {busyIds.has(entryKey(chosenEntry.market, chosenEntry.entry)) ? text('安装中…', 'Installing…') : text('安装', 'Install')}
      </Button>
    </header>
    <p className="kv-market-summary">{chosenEntry.entry.description}</p>
    {chosenEntry.entry.unavailableReason && <p className="kv-market-warning">{chosenEntry.entry.unavailableReason}</p>}
    <PluginContents key={entryKey(chosenEntry.market, chosenEntry.entry)} marketplaceId={chosenEntry.market.id} plugin={chosenEntry.entry.name} version={chosenEntry.entry.version} information={<>
        <dt>{text('市场', 'Marketplace')}</dt><dd>{chosenEntry.market.name}</dd>
        <dt>{text('来源', 'Source')}</dt><dd>{chosenEntry.market.source}</dd>
        <dt>{text('使用范围', 'Scope')}</dt><dd>{text('个人 · 内置 Kivio Agent', 'Personal · Built-in Kivio Agent')}</dd>
      </>}>
      <p className="kv-market-summary">{text('安装后默认停用，可在详情中检查包含的能力并启用。依赖需自行安装。', 'Plugins start disabled. Review their capabilities and enable them after installation. Install dependencies separately.')}</p>
    </PluginContents>
  </div>

  return (
    <section className="kv-market" data-tauri-drag-region="false">
      <div
        ref={scroller}
        className="kv-market-scroll custom-scrollbar"
        onScroll={(e) => { if (!selected) viewState.scroll = e.currentTarget.scrollTop }}
      >
        {selected ? (
          packageDetail || catalogDetail || detail || (
            <div className="kv-market-empty">
              <p>{loading ? text('正在读取插件…', 'Loading plugin…') : text('找不到这个插件', 'Plugin not found')}</p>
              <Button size="sm" onClick={() => { window.location.hash = marketHash() }}>{text('返回插件', 'Back to plugins')}</Button>
            </div>
          )
        ) : (
          <>
            <header className="kv-market-heading">
              <div className="min-w-0">
                {heading ?? <h1>{text('插件市场', 'Plugin marketplace')}</h1>}
                <p>{text('用插件为 Kivio 扩展技能、命令与 MCP 能力', 'Extend Kivio with skills, commands and MCP through plugins')}</p>
              </div>
              <div className="kv-market-toolbar">
                <IconButton size="md" label={text('刷新', 'Refresh')} disabled={loading} onClick={() => void refreshMarketInventory()}>
                  <RefreshCw size={16} className={loading ? 'animate-spin' : ''} />
                </IconButton>
                <IconButton size="md" label={text('管理插件市场', 'Manage marketplaces')} disabled={!isTauriRuntime()} onClick={() => setMarketDialog('manage')}><Settings2 size={16} /></IconButton>
                <Button variant="primary" disabled={!isTauriRuntime()} aria-haspopup="menu" aria-expanded={!!addMenu}
                  onClick={e => { const rect = e.currentTarget.getBoundingClientRect(); setAddMenu({ left: rect.right - 190, top: rect.bottom + 6 }) }}>
                  {text('添加', 'Add')}<ChevronDown size={14} />
                </Button>
              </div>
            </header>

            <label className="kv-market-search">
              <Input value={query} onChange={setQuery} placeholder={text('搜索插件', 'Search plugins')} aria-label={text('搜索插件', 'Search plugins')} />
            </label>

            <section className="kv-market-installed">
              <h2>{text('已安装', 'Installed')}</h2>
              {installed.length + personal.length ? (
                <div className="kv-market-installed-row">
                  {installed.map((plugin) => (
                    <button
                      type="button"
                      key={plugin.manifest.id}
                      className="kv-market-installed-item"
                      title={plugin.manifest.name}
                      aria-label={plugin.manifest.name}
                      onClick={() => { window.location.hash = marketDetailHash(plugin.manifest.id) }}
                    >
                      <PluginIcon plugin={plugin} />
                    </button>
                  ))}
                  {personal.map(plugin => <button type="button" key={packageKey(plugin)} className="kv-market-installed-item"
                    title={plugin.name} aria-label={plugin.name} onClick={() => openPackage(plugin)}><PluginIcon src={packageIcon(plugin)} /></button>)}
                </div>
              ) : (
                <p className="kv-market-muted">{query.trim() ? text('没有找到相关插件', 'No matching plugins') : text('还没有安装插件', 'No plugins installed yet')}</p>
              )}
            </section>

            <div className="kv-market-tabs" role="tablist" aria-label={text('插件分类', 'Plugin scope')}>
              {(['public', 'personal'] as const).map((value, index) => <button type="button" role="tab" key={value}
                id={`market-tab-${value}`} aria-controls="market-list" aria-selected={scope === value} tabIndex={scope === value ? 0 : -1}
                onClick={() => setScope(value)} onKeyDown={e => {
                  if (e.key === 'ArrowLeft' || e.key === 'ArrowRight' || e.key === 'Home' || e.key === 'End') {
                    e.preventDefault()
                    const next = e.key === 'Home' ? 'public' : e.key === 'End' ? 'personal' : index === 0 ? 'personal' : 'public'
                    setScope(next)
                    document.getElementById(`market-tab-${next}`)?.focus()
                  }
                }}>{value === 'public' ? text('公开', 'Public') : text('个人', 'Personal')}</button>)}
            </div>

            {!isTauriRuntime() && <p className="kv-market-muted">{text('浏览器预览 · 请在 Kivio 桌面端安装和使用插件。', 'Browser preview · Install and use plugins in the Kivio desktop app.')}</p>}
            {(loadError || packageError || marketError) && (
              <div role="status" className="kv-market-warning">
                {text('部分插件暂时无法读取。', 'Some plugins could not be loaded.')} {loadError || packageError || marketError}
                <Button size="sm" onClick={() => void refreshMarketInventory()}>{text('重试', 'Retry')}</Button>
              </div>
            )}

            <div id="market-list" role="tabpanel" aria-labelledby={`market-tab-${scope}`}>
            {scope === 'personal' ? (
              <>
              {catalogSections.map(({ market, entries }) => <section className="kv-market-section" key={market.id}>
                <h2>{market.name}<small>{entries.length}</small></h2>
                <div className="kv-market-rows">{entries.map(entry => <article className="kv-market-row" key={entry.name}>
                  <button type="button" className="kv-market-row-info" onClick={() => {
                    const installedPackage = entryPackage(market, entry)
                    if (installedPackage) openPackage(installedPackage)
                    else window.location.hash = marketDetailHash(entryKey(market, entry))
                  }}>
                    <PluginIcon src={claudeMarketplaceIcon(market.source, entry.name)} /><span className="min-w-0"><strong>{entry.displayName}</strong><span>{entry.description || entry.unavailableReason}</span></span>
                  </button>
                  {entryPackage(market, entry) ? <IconButton variant="ghost" label={text(`${entry.displayName} 更多操作`, `More options for ${entry.displayName}`)} onClick={() => {
                    const installedPackage = entryPackage(market, entry)
                    if (installedPackage) openPackage(installedPackage)
                  }}><MoreHorizontal size={16} /></IconButton>
                    : entry.unavailableReason ? <Button size="sm" onClick={() => { window.location.hash = marketDetailHash(entryKey(market, entry)) }}>{text('查看原因', 'Details')}</Button>
                    : <Button size="sm" disabled={busyIds.has(entryKey(market, entry))} onClick={() => void runInstallEntry(market, entry)}>{busyIds.has(entryKey(market, entry)) ? text('安装中…', 'Installing…') : text('安装', 'Install')}</Button>}
                </article>)}</div>
              </section>)}
              {(standalone.length > 0 || !catalogSections.length) && <section className="kv-market-section">
                <h2>{text('个人插件', 'Personal plugins')}</h2>
                {standalone.length ? <div className="kv-market-rows">{standalone.map(plugin => (
                  <article className="kv-market-row" key={plugin.id}>
                    <button type="button" className="kv-market-row-info" onClick={() => openPackage(plugin)}>
                      <PluginIcon src={packageIcon(plugin)} /><span className="min-w-0"><strong>{plugin.name}</strong><span>{plugin.description}</span></span>
                    </button>
                    <IconButton variant="ghost" label={text(`${plugin.name} 更多操作`, `More options for ${plugin.name}`)} onClick={() => openPackage(plugin)}><MoreHorizontal size={16} /></IconButton>
                  </article>
                ))}</div> : <div className="kv-market-empty"><DefaultPluginIcon size={28} strokeWidth={1.75} />
                  <p>{query.trim() ? text('没有找到相关插件', 'No matching plugins') : text('添加你自己的插件', 'Add your own plugins')}</p>
                  {!query.trim() && <Button size="sm" onClick={() => setMarketDialog('add')}>{text('添加插件市场', 'Add marketplace')}</Button>}
                </div>}
              </section>}
              </>
            ) : loading && !snapshot.plugins.length ? (
              <div className="kv-market-rows" aria-busy="true">{[0, 1, 2, 3].map((i) => <div key={i} className="kv-skeleton kv-market-skeleton" />)}</div>
            ) : !sections.length ? (
              <div className="kv-market-empty">
                <DefaultPluginIcon size={28} strokeWidth={1.75} />
                <p>{query.trim() ? text('没有找到相关插件', 'No matching plugins') : text('暂无插件', 'No plugins yet')}</p>
                {query.trim() && <Button size="sm" onClick={() => setQuery('')}>{text('查看全部', 'Show all')}</Button>}
              </div>
            ) : sections.map((section) => (
              <section className="kv-market-section" key={section.id}>
                <h2>{section.name}</h2>
                {!collapsed[section.id] && (
                  <div className="kv-market-rows">
                    {section.items.map((plugin) => (
                      <article className="kv-market-row" key={plugin.manifest.id}>
                        <button type="button" className="kv-market-row-info" onClick={() => { window.location.hash = marketDetailHash(plugin.manifest.id) }}>
                          <PluginIcon plugin={plugin} />
                          <span className="min-w-0">
                            <strong>{plugin.manifest.name}</strong>
                            <span>{plugin.manifest.summary}</span>
                          </span>
                        </button>
                        <span className="kv-market-row-actions">
                          {statusText(plugin) && <span className="kv-market-status">{statusText(plugin)}</span>}
                          {plugin.local?.status === 'ready' ? <IconButton variant="ghost"
                            label={text(`${plugin.manifest.name} 更多操作`, `More options for ${plugin.manifest.name}`)}
                            onClick={() => { window.location.hash = marketDetailHash(plugin.manifest.id) }}><MoreHorizontal size={16} /></IconButton> : primaryButton(plugin)}
                        </span>
                      </article>
                    ))}
                  </div>
                )}
                <button
                  type="button"
                  className="kv-market-collapse"
                  aria-expanded={!collapsed[section.id]}
                  onClick={() => setCollapsed((current) => ({ ...current, [section.id]: !current[section.id] }))}
                >
                  {collapsed[section.id] ? text('展开', 'Expand') : text('收起', 'Collapse')}
                </button>
              </section>
            ))}
            </div>
          </>
        )}
        {actionError && <p className="kv-market-error" role="alert">{actionError}</p>}
      </div>
      {addMenu && <DockContextMenu anchor={addMenu} onClose={() => setAddMenu(null)} items={[
        { key: 'marketplace', label: text('添加插件市场', 'Add marketplace'), icon: <Plus size={16} />, onSelect: () => setMarketDialog('add') },
        { key: 'local', label: text('从本地目录导入', 'Import from a folder'), icon: <FolderOpen size={16} />, onSelect: () => setImportKind('local') },
        { key: 'git', label: text('从 Git 仓库导入', 'Import from Git'), icon: <GitBranch size={16} />, onSelect: () => setImportKind('git') },
      ]} />}
      {marketDialog && <MarketplaceDialog mode={marketDialog} zh={zh} onClose={() => setMarketDialog(null)} />}
      {importKind && <PluginImportDialog kind={importKind} zh={zh} onClose={() => setImportKind(null)} />}
    </section>
  )
}
