import { open } from '@tauri-apps/plugin-dialog'
import { isTauriRuntime } from '../../api/tauri'
import { marketApi, marketplaceApi, type Marketplace, type MarketplacePlugin, type MarketPlugin, type MarketSnapshot } from '../../api/market'
import { packageApi, type PluginPackage } from '../../api/pluginPackages'
import { refreshSettings } from '../../api/settingsCache'
import { createWindowStore } from '../../utils/windowStore'
import { pluginAction } from './marketModel'

const EMPTY: MarketSnapshot = { categories: [], plugins: [] }

export type ImportKind = 'local' | 'git'

type MarketDraft = {
  source: string
  busy: string
  error: string
  manageError: string
  removing: string | null
  picker: number
}

type ImportDraft = {
  source: string
  subdirectory: string
  busy: boolean
  error: string
  picker: number
}

export type MarketIntent =
  | { type: 'open-package'; entryKey: string; packageId: string }
  | { type: 'use-plugin'; plugin: MarketPlugin }
  | { type: 'leave-package'; packageKey: string }
  | { type: 'market-added' }
  | { type: 'imported'; kind: ImportKind; plugin: PluginPackage }

type MarketWindowState = {
  snapshot: MarketSnapshot
  packages: PluginPackage[]
  packageError: string
  markets: Marketplace[]
  marketError: string
  loading: boolean
  loadError: string
  busyIds: ReadonlySet<string>
  actionError: string
  skillsDirty: boolean
  market: MarketDraft
  imports: Record<ImportKind, ImportDraft>
}

function blankImport(): ImportDraft {
  return { source: '', subdirectory: '', busy: false, error: '', picker: 0 }
}

function initialWindow(): MarketWindowState {
  return {
    snapshot: EMPTY,
    packages: [],
    packageError: '',
    markets: [],
    marketError: '',
    loading: true,
    loadError: '',
    busyIds: new Set(),
    actionError: '',
    skillsDirty: false,
    market: { source: '', busy: '', error: '', manageError: '', removing: null, picker: 0 },
    imports: { local: blankImport(), git: blankImport() },
  }
}

/** 安装、更新、移除、市场来源和导入的锁、草稿与结果都留在这个窗口里。 */
export const marketWindow = createWindowStore(initialWindow())

let catalogVersion = 0
let packageVersion = 0
let marketVersion = 0
/** 只在测试重置时增加。页面离开不改它，进行中的安装仍占住同一把锁。 */
let operationEpoch = 0
const skillsListeners = new Set<() => void>()
const intentListeners = new Set<(intent: MarketIntent) => void | Promise<void>>()

function flight(epoch: number, key: string) {
  return `${epoch}:${key}`
}

function abandoned(epoch: number) {
  return epoch !== operationEpoch
}

export function resetMarketWindow() {
  operationEpoch += 1
  catalogVersion += 1
  packageVersion += 1
  marketVersion += 1
  skillsListeners.clear()
  intentListeners.clear()
  marketWindow.setState(initialWindow())
}

/** 页面挂载后接收技能刷新，以及仍停在当前对话框或详情上的导航。 */
export function subscribeMarketWindow(handlers: {
  onSkills: () => void
  onIntent: (intent: MarketIntent) => void | Promise<void>
}): () => void {
  skillsListeners.add(handlers.onSkills)
  intentListeners.add(handlers.onIntent)
  if (marketWindow.getSnapshot().skillsDirty) {
    marketWindow.setState(s => s.skillsDirty ? { ...s, skillsDirty: false } : s)
    handlers.onSkills()
  }
  return () => {
    skillsListeners.delete(handlers.onSkills)
    intentListeners.delete(handlers.onIntent)
  }
}

/** 页面不再接收清单事件后，进行中的读取不能写回窗口。 */
export function discardMarketInventoryReads() {
  catalogVersion += 1
  packageVersion += 1
  marketVersion += 1
}

function notifySkills() {
  if (skillsListeners.size === 0) {
    marketWindow.setState(s => s.skillsDirty ? s : { ...s, skillsDirty: true })
    return
  }
  for (const listener of skillsListeners) listener()
}

function publishIntent(intent: MarketIntent): Promise<void> {
  const jobs: Array<void | Promise<void>> = []
  for (const listener of intentListeners) jobs.push(listener(intent))
  return Promise.all(jobs).then(() => undefined)
}

function withBusy(busy: ReadonlySet<string>, id: string, on: boolean) {
  if (busy.has(id) === on) return busy
  const next = new Set(busy)
  if (on) next.add(id)
  else next.delete(id)
  return next
}

function actionMessage(error: unknown) {
  return String(error instanceof Error ? error.message : error)
}

/** 与忙碌锁、详情路由使用同一身份。 */
export function packageKey(plugin: { id: string }) {
  return `package:${plugin.id}`
}

export function entryKey(market: { id: string }, entry: { name: string }) {
  return `marketplace:${market.id}:${entry.name}`
}

function patchImport(kind: ImportKind, update: (draft: ImportDraft) => ImportDraft) {
  marketWindow.setState(s => {
    const next = update(s.imports[kind])
    if (next === s.imports[kind]) return s
    return { ...s, imports: { ...s.imports, [kind]: next } }
  })
}

export function setMarketSource(source: string) {
  marketWindow.setState(s => s.market.source === source ? s : { ...s, market: { ...s.market, source } })
}

export function setMarketRemoving(id: string | null) {
  marketWindow.setState(s => s.market.removing === id ? s : { ...s, market: { ...s.market, removing: id } })
}

export function setImportSource(kind: ImportKind, source: string) {
  patchImport(kind, draft => draft.source === source ? draft : { ...draft, source })
}

export function setImportSubdirectory(kind: ImportKind, subdirectory: string) {
  patchImport(kind, draft => draft.subdirectory === subdirectory ? draft : { ...draft, subdirectory })
}

export function bumpMarketplacePicker() {
  marketWindow.setState(s => ({ ...s, market: { ...s.market, picker: s.market.picker + 1 } }))
}

export function bumpImportPicker(kind: ImportKind) {
  patchImport(kind, draft => ({ ...draft, picker: draft.picker + 1 }))
}

/** 离开后再进来时以后端清单为准；进行中的写入仍用更高的代次覆盖过期读取。 */
export async function refreshMarketInventory() {
  const epoch = operationEpoch
  if (abandoned(epoch)) return
  if (!isTauriRuntime()) {
    marketWindow.setState(s => s.loading ? { ...s, loading: false } : s)
    return
  }
  const catalog = ++catalogVersion
  const packagesToken = ++packageVersion
  const marketsToken = ++marketVersion
  marketWindow.setState(s => s.loading ? s : { ...s, loading: true })
  const packagesTask = packageApi.list().then(
    (next) => {
      if (abandoned(epoch) || packagesToken !== packageVersion) return
      marketWindow.setState(s => ({ ...s, packages: next, packageError: '' }))
    },
    (error: unknown) => {
      if (abandoned(epoch) || packagesToken !== packageVersion) return
      marketWindow.setState(s => ({ ...s, packageError: String(error) }))
    },
  )
  const marketsTask = marketplaceApi.list().then(
    (next) => {
      if (abandoned(epoch) || marketsToken !== marketVersion) return
      marketWindow.setState(s => ({ ...s, markets: next, marketError: '' }))
    },
    (error: unknown) => {
      if (abandoned(epoch) || marketsToken !== marketVersion) return
      marketWindow.setState(s => ({ ...s, marketError: String(error) }))
    },
  )
  try {
    const next = await marketApi.snapshot()
    if (!abandoned(epoch) && catalog === catalogVersion) marketWindow.setState(s => ({ ...s, snapshot: next, loadError: '', loading: false }))
  } catch (error) {
    if (!abandoned(epoch) && catalog === catalogVersion) marketWindow.setState(s => ({ ...s, loadError: String(error), loading: false }))
  }
  await Promise.all([packagesTask, marketsTask])
}

function applySnapshot(epoch: number, next: MarketSnapshot) {
  if (abandoned(epoch)) return false
  catalogVersion += 1
  marketWindow.setState(s => ({ ...s, snapshot: next, loadError: '', loading: false }))
  return true
}

function acceptPackage(epoch: number, plugin: PluginPackage) {
  if (abandoned(epoch)) return false
  packageVersion += 1
  marketWindow.setState(s => ({
    ...s,
    packages: [...s.packages.filter(item => item.id !== plugin.id), plugin],
    packageError: '',
  }))
  return true
}

function acceptMarkets(epoch: number, next: Marketplace[]) {
  if (abandoned(epoch)) return false
  marketVersion += 1
  marketWindow.setState(s => ({ ...s, markets: next, marketError: '' }))
  return true
}

function runGuarded(id: string, body: (epoch: number) => Promise<void>) {
  const epoch = operationEpoch
  return marketWindow.run(flight(epoch, `plugin:${id}`), async () => {
    if (abandoned(epoch)) return
    marketWindow.setState(s => ({ ...s, busyIds: withBusy(s.busyIds, id, true), actionError: '' }))
    try {
      await body(epoch)
    } catch (error) {
      if (abandoned(epoch)) return
      marketWindow.setState(s => ({ ...s, actionError: actionMessage(error) }))
      void refreshMarketInventory()
    } finally {
      if (!abandoned(epoch)) marketWindow.setState(s => ({ ...s, busyIds: withBusy(s.busyIds, id, false) }))
    }
  })
}

export function runCatalogPlugin(plugin: MarketPlugin) {
  const id = plugin.manifest.id
  const action = pluginAction(plugin)
  return runGuarded(id, async (epoch) => {
    if (action === 'install' || action === 'repair') {
      if (!applySnapshot(epoch, await marketApi.install(id))) return
      notifySkills()
      return
    }
    if (action === 'enable-use') {
      if (!applySnapshot(epoch, await marketApi.setEnabled(id, true))) return
      notifySkills()
    }
    if (abandoned(epoch)) return
    await publishIntent({ type: 'use-plugin', plugin })
  })
}

export function runSetEnabled(id: string, enabled: boolean) {
  return runGuarded(id, async (epoch) => {
    if (!applySnapshot(epoch, await marketApi.setEnabled(id, enabled))) return
    notifySkills()
  })
}

export function runUninstall(id: string) {
  return runGuarded(id, async (epoch) => {
    if (!applySnapshot(epoch, await marketApi.uninstall(id))) return
    notifySkills()
  })
}

export function runInstallEntry(market: Marketplace, entry: MarketplacePlugin) {
  const key = entryKey(market, entry)
  return runGuarded(key, async (epoch) => {
    const plugin = await marketplaceApi.install(market.id, entry.name)
    // 已安装时后端直接返回现有副本，这里按成功写入清单，不改写成失败。
    if (!acceptPackage(epoch, plugin)) return
    notifySkills()
    if (abandoned(epoch)) return
    await publishIntent({ type: 'open-package', entryKey: key, packageId: plugin.id })
  })
}

export function runTogglePackage(plugin: PluginPackage, enabled: boolean) {
  return runGuarded(packageKey(plugin), async (epoch) => {
    const next = await packageApi.setEnabled(plugin.id, enabled)
    if (!acceptPackage(epoch, next)) return
    if (abandoned(epoch)) return
    await refreshSettings()
    if (abandoned(epoch)) return
    notifySkills()
  })
}

export function runRemovePackage(plugin: PluginPackage) {
  const key = packageKey(plugin)
  return runGuarded(key, async (epoch) => {
    await packageApi.remove(plugin.id)
    if (abandoned(epoch)) return
    packageVersion += 1
    marketWindow.setState(s => ({ ...s, packages: s.packages.filter(item => item.id !== plugin.id) }))
    if (abandoned(epoch)) return
    await refreshSettings()
    if (abandoned(epoch)) return
    notifySkills()
    if (abandoned(epoch)) return
    await publishIntent({ type: 'leave-package', packageKey: key })
  })
}

export function runAddMarketplace() {
  const epoch = operationEpoch
  return marketWindow.run(flight(epoch, 'marketplace-source'), async () => {
    if (abandoned(epoch)) return
    const source = marketWindow.getSnapshot().market.source.trim()
    if (!source) return
    marketWindow.setState(s => ({ ...s, market: { ...s.market, busy: 'add', error: '', picker: s.market.picker + 1 } }))
    try {
      const next = await marketplaceApi.add(source)
      if (!acceptMarkets(epoch, next)) return
      marketWindow.setState(s => ({ ...s, market: { ...s.market, busy: '', error: '', source: '', removing: null } }))
      if (abandoned(epoch)) return
      await publishIntent({ type: 'market-added' })
    } catch (error) {
      if (abandoned(epoch)) return
      marketWindow.setState(s => ({ ...s, market: { ...s.market, busy: '', error: String(error) } }))
    }
  })
}

export function runRefreshMarketplace(id: string) {
  const epoch = operationEpoch
  return marketWindow.run(flight(epoch, 'marketplace-source'), async () => {
    if (abandoned(epoch)) return
    marketWindow.setState(s => ({ ...s, market: { ...s.market, busy: id, manageError: '' } }))
    try {
      const next = await marketplaceApi.refresh(id)
      if (!acceptMarkets(epoch, next)) return
      marketWindow.setState(s => ({ ...s, market: { ...s.market, busy: '' } }))
    } catch (error) {
      if (abandoned(epoch)) return
      marketWindow.setState(s => ({ ...s, market: { ...s.market, busy: '', manageError: String(error) } }))
    }
  })
}

export function runRemoveMarketplace(id: string) {
  const epoch = operationEpoch
  return marketWindow.run(flight(epoch, 'marketplace-source'), async () => {
    if (abandoned(epoch)) return
    marketWindow.setState(s => ({ ...s, market: { ...s.market, busy: id, manageError: '' } }))
    try {
      const next = await marketplaceApi.remove(id)
      if (!acceptMarkets(epoch, next)) return
      marketWindow.setState(s => ({ ...s, market: { ...s.market, busy: '', removing: null } }))
    } catch (error) {
      if (abandoned(epoch)) return
      marketWindow.setState(s => ({ ...s, market: { ...s.market, busy: '', manageError: String(error) } }))
    }
  })
}

export function runPickMarketplaceFolder() {
  const epoch = operationEpoch
  const token = marketWindow.getSnapshot().market.picker
  return marketWindow.run(flight(epoch, 'marketplace-picker'), async () => {
    if (abandoned(epoch)) return
    try {
      const selected = await open({ directory: true, multiple: false })
      if (abandoned(epoch)) return
      marketWindow.setState(s => {
        if (s.market.picker !== token || typeof selected !== 'string') return s
        return { ...s, market: { ...s.market, source: selected } }
      })
    } catch (error) {
      if (abandoned(epoch)) return
      marketWindow.setState(s => s.market.picker !== token ? s : { ...s, market: { ...s.market, error: String(error) } })
    }
  })
}

export function runImportPlugin(kind: ImportKind) {
  const epoch = operationEpoch
  return marketWindow.run(flight(epoch, `plugin-import:${kind}`), async () => {
    if (abandoned(epoch)) return
    const draft = marketWindow.getSnapshot().imports[kind]
    const source = draft.source.trim()
    const subdirectory = draft.subdirectory.trim()
    if (!source) return
    patchImport(kind, current => ({ ...current, busy: true, error: '', picker: current.picker + 1 }))
    try {
      const plugin = await packageApi.import(source, subdirectory || undefined)
      if (!acceptPackage(epoch, plugin)) return
      notifySkills()
      if (abandoned(epoch)) return
      patchImport(kind, current => ({ ...current, busy: false, error: '', source: '', subdirectory: '' }))
      await publishIntent({ type: 'imported', kind, plugin })
    } catch (error) {
      if (abandoned(epoch)) return
      patchImport(kind, current => ({ ...current, busy: false, error: String(error) }))
    }
  })
}

export function runPickImportFolder(kind: ImportKind) {
  const epoch = operationEpoch
  const token = marketWindow.getSnapshot().imports[kind].picker
  return marketWindow.run(flight(epoch, `plugin-import-picker:${kind}`), async () => {
    if (abandoned(epoch)) return
    try {
      const selected = await open({ directory: true, multiple: false })
      if (abandoned(epoch)) return
      marketWindow.setState(s => {
        const current = s.imports[kind]
        if (current.picker !== token || typeof selected !== 'string') return s
        return { ...s, imports: { ...s.imports, [kind]: { ...current, source: selected } } }
      })
    } catch (error) {
      if (abandoned(epoch)) return
      marketWindow.setState(s => {
        const current = s.imports[kind]
        if (current.picker !== token) return s
        return { ...s, imports: { ...s.imports, [kind]: { ...current, error: String(error) } } }
      })
    }
  })
}
