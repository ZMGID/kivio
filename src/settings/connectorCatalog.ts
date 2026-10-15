/** 插件 MCP（`plugin-<id>` / `connectorId: plugin:<id>`）。
 *  MCP 已安装列表与普通服务器一起展示，带「插件」标记，只读。 */
export function isPluginManagedServer(server: {
  id: string
  connectorId?: string | null
}): boolean {
  return server.id.startsWith('plugin-') || (server.connectorId?.startsWith('plugin:') ?? false)
}

/** MCP 页保存时钉住插件条目：开关 / 删除 / 改命令只走「扩展 → 插件」。 */
export function preservePluginManagedServers<T extends { id: string; connectorId?: string | null }>(
  previous: T[],
  next: T[],
): T[] {
  const locked = new Map(
    previous.filter(isPluginManagedServer).map((server) => [server.id, server] as const),
  )
  if (locked.size === 0) return next
  const out = next.map((server) => locked.get(server.id) ?? server)
  const present = new Set(out.map((server) => server.id))
  for (const [id, server] of locked) {
    if (!present.has(id)) out.push(server)
  }
  return out
}

/**
 * 插件 MCP 以插件页（后端 `plugins_set_enabled`）为准。
 * 设置页 keep-alive 草稿 / MCP 列表在 `refreshSettings` 之后必须换上这份条目，
 * 否则后续整份保存会把 `enabled` 盖回关闭前的值。
 */
export function adoptFreshPluginManagedServers<T extends { id: string; connectorId?: string | null }>(
  draft: T[],
  fresh: T[],
): T[] {
  const freshPlugin = new Map(
    fresh.filter(isPluginManagedServer).map((server) => [server.id, server] as const),
  )
  const out: T[] = []
  const seen = new Set<string>()
  for (const server of draft) {
    if (!isPluginManagedServer(server)) {
      out.push(server)
      continue
    }
    const next = freshPlugin.get(server.id)
    if (!next) continue
    out.push(next)
    seen.add(server.id)
  }
  for (const [id, server] of freshPlugin) {
    if (!seen.has(id)) out.push(server)
  }
  if (out.length === draft.length && out.every((server, i) => server === draft[i])) return draft
  const draftPlugin = draft.filter(isPluginManagedServer)
  const outPlugin = out.filter(isPluginManagedServer)
  const draftUsers = draft.filter((server) => !isPluginManagedServer(server))
  const outUsers = out.filter((server) => !isPluginManagedServer(server))
  if (
    draftUsers.length === outUsers.length &&
    draftUsers.every((server, i) => server === outUsers[i]) &&
    draftPlugin.length === outPlugin.length &&
    draftPlugin.every((server, i) => pluginManagedSignature(server) === pluginManagedSignature(outPlugin[i]))
  ) {
    return draft
  }
  return out
}

function pluginManagedSignature(server: {
  id: string
  connectorId?: string | null
  enabled?: boolean
  command?: string
  args?: string[]
  url?: string
  transport?: string
}): string {
  return [
    server.id,
    server.connectorId ?? '',
    server.enabled === false ? '0' : '1',
    server.command ?? '',
    (server.args ?? []).join('\0'),
    server.url ?? '',
    server.transport ?? '',
  ].join('\n')
}
