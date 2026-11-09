// 工作区面板可隐藏的项（参照 T3 的 threadDetailsItems）。id 存在本地设置里，不要改名。
// 用量和上下文窗口与 T3 一样默认隐藏，从右键菜单打开。
export const WORKSPACE_ITEMS = [
  'project', 'environment', 'openIn', 'branch', 'gitActions', 'changes', 'history', 'tasks', 'subagents', 'usage', 'context',
] as const
export type WorkspaceItemId = (typeof WORKSPACE_ITEMS)[number]

const HIDDEN_BY_DEFAULT: ReadonlySet<WorkspaceItemId> = new Set(['usage', 'context'])

export function workspaceItemDefault(id: WorkspaceItemId): boolean {
  return !HIDDEN_BY_DEFAULT.has(id)
}

export function resolveWorkspaceItems(overrides: Record<string, boolean>): Record<WorkspaceItemId, boolean> {
  return Object.fromEntries(WORKSPACE_ITEMS.map((id) => [id, overrides[id] ?? workspaceItemDefault(id)])) as Record<WorkspaceItemId, boolean>
}

/** 只保留与默认不同的选择，默认值以后调整时能覆盖到没改过的人。 */
export function toggleWorkspaceItem(overrides: Record<string, boolean>, id: WorkspaceItemId, visible: boolean) {
  const { [id]: _previous, ...rest } = overrides
  void _previous
  return visible === workspaceItemDefault(id) ? rest : { ...rest, [id]: visible }
}
