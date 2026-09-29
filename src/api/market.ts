import { invoke } from '@tauri-apps/api/core'
import type { PluginDetails, PluginPackage } from './pluginPackages'

/** 与 src-tauri/src/market.rs 的快照契约保持一致。 */
export type MarketManifest = {
  id: string
  name: string
  summary: string
  categoryIds: string[]
  /** data: URL（SVG） */
  icon: string
  welcome: string
  inputHint: string
  startPrompt: string
  setupSkillId: string
  mainSkillId: string
  skillIds: string[]
  checkCommand: string | null
  repository: string | null
  revision: string
}

export type MarketLocal = {
  status: 'ready' | 'failed'
  enabled: boolean
  error: string | null
}

export type MarketPlugin = { manifest: MarketManifest; local: MarketLocal | null }

export type MarketSnapshot = {
  categories: { id: string; name: string }[]
  plugins: MarketPlugin[]
}

export const MARKET_CHANGED_EVENT = 'kivio-market-changed'

export const marketApi = {
  snapshot: () => invoke<MarketSnapshot>('market_snapshot'),
  install: (id: string) => invoke<MarketSnapshot>('market_install', { id }),
  uninstall: (id: string) => invoke<MarketSnapshot>('market_uninstall', { id }),
  setEnabled: (id: string, enabled: boolean) => invoke<MarketSnapshot>('market_set_enabled', { id, enabled }),
}

export type MarketplacePlugin = {
  name: string
  displayName: string
  description: string
  version: string | null
  category: string
  unavailableReason: string | null
}
export type Marketplace = {
  id: string
  name: string
  description: string
  source: string
  plugins: MarketplacePlugin[]
}

/** User-owned marketplace sources; packageApi remains the owner of installed capabilities. */
export const marketplaceApi = {
  describe: (id: string, plugin: string) => invoke<PluginDetails>('plugin_marketplaces_describe', { id, plugin }),
  list: () => invoke<Marketplace[]>('plugin_marketplaces_list'),
  add: (source: string) => invoke<Marketplace[]>('plugin_marketplaces_add', { source }),
  refresh: (id: string) => invoke<Marketplace[]>('plugin_marketplaces_refresh', { id }),
  remove: (id: string) => invoke<Marketplace[]>('plugin_marketplaces_remove', { id }),
  install: (id: string, plugin: string) => invoke<PluginPackage>('plugin_marketplaces_install', { id, plugin }),
}
