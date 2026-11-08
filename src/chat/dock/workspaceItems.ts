// 工作区面板可隐藏的项（参照 T3 的 threadDetailsItems）。id 存在本地设置里，不要改名。
export const WORKSPACE_ITEMS = ['project', 'openIn', 'branch', 'gitActions', 'changes', 'history', 'tasks', 'subagents'] as const
export type WorkspaceItemId = (typeof WORKSPACE_ITEMS)[number]
