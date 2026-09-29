import type { MarketPlugin } from '../../api/market'
import { decodeChatRouteId, encodeChatRouteId } from '../routeCodec'
import { hashPath } from '../browserRoute'
import claudeIcons from './claude-market-icons.json'

/** Supplemental HTTPS logos verified against ZCode's Claude catalog, 2026-09-29.
 * Match the actual repository source so another market with the same names keeps its own identity.
 */
export function claudeMarketplaceIcon(source: string | undefined, name: string): string | undefined {
  if (!source) return undefined
  try {
    const url = new URL(source)
    const repository = url.pathname.replace(/\/$/, '').replace(/\.git$/, '')
    if (url.protocol !== 'https:' || url.hostname !== 'github.com' || url.username || url.password
      || repository !== '/anthropics/claude-plugins-official') return undefined
    return Object.prototype.hasOwnProperty.call(claudeIcons, name) ? (claudeIcons as Record<string, string>)[name] : undefined
  } catch { return undefined }
}

export type PluginAction = 'install' | 'repair' | 'use' | 'enable-use'

/** 插件卡片的主按钮：未安装→安装；组件缺失→重新配置；已安装→使用（未加载时先加载）。 */
export function pluginAction(plugin: MarketPlugin): PluginAction {
  const local = plugin.local
  if (!local) return 'install'
  if (local.status !== 'ready') return 'repair'
  return local.enabled ? 'use' : 'enable-use'
}

export const MARKET_ROUTE = 'chat/plugins'

export function marketHash(): string {
  return `#${MARKET_ROUTE}`
}

export function marketDetailHash(id: string): string {
  return `#${encodeChatRouteId(`${MARKET_ROUTE}/`, id)}`
}

/** `#chat/plugins/{id}` 的插件 id；列表页返回 null。 */
export function marketPluginIdFromHash(): string | null {
  return decodeChatRouteId(`${MARKET_ROUTE}/`, hashPath())
}

/** “使用”时发出的第一条消息。 */
export function marketUsePrompt(plugin: MarketPlugin): string {
  return plugin.manifest.startPrompt.trim() || `使用${plugin.manifest.name}，告诉我可以做什么。`
}
