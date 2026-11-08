// 工作区面板分支列表的排序与命名规则，参照 T3 Code 的 listRefs / sanitizeNewRefName。
import type { GitBranchesResult } from '../../api/dockContracts'

export type BranchRef = { name: string; badge: 'current' | 'default' | 'remote' | null; current: boolean }

/** 分支名不能含空白：和 T3 一样把空白换成连字符。 */
export function sanitizeBranchName(raw: string): string {
  return raw.trim().replace(/\s+/g, '-')
}

export function orderBranchRefs(data: GitBranchesResult): BranchRef[] {
  const local = data.branches.map((branch) => ({
    name: branch.name,
    current: branch.current,
    badge: branch.current ? 'current' as const : branch.name === data.defaultBranch ? 'default' as const : null,
  }))
  const remote = data.remoteBranches.map((branch) => ({ name: branch.name, current: false, badge: 'remote' as const }))
  const rank = (ref: BranchRef) => (ref.current ? 0 : ref.badge === 'default' ? 1 : 2)
  // 后端已按最近提交排序；稳定排序只把当前、默认提到最前，远程排在本地之后。
  return [...local.sort((a, b) => rank(a) - rank(b)), ...remote]
}
