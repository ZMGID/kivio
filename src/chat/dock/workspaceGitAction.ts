// 工作区面板的 Git 主按钮与下拉项，参照 T3 Code 的 resolveQuickAction / buildMenuItems：
// 主按钮只给出当前最合理的一步；做不了时禁用并说明原因，而不是让用户点了再失败。
import type { GitRepoState, GitStatusEntry } from '../../api/dockContracts'

export type GitAction = 'commit' | 'commit_push' | 'push' | 'pull' | 'publish'

export type GitHint = 'busy' | 'detached' | 'noRemote' | 'diverged' | 'upToDate' | 'unavailable'

export type GitQuickAction =
  | { kind: 'run'; action: GitAction }
  | { kind: 'hint'; action: GitAction; hint: GitHint }

export type GitMenuState = Record<GitAction, boolean>

/**
 * 不弹窗提交时使用的提交信息：T3 在留空时自动生成，这里按改动文件给出确定的摘要，
 * 例如「Update a.ts, b.ts and 3 more」。新增/删除单独标注动词。
 */
export function defaultCommitMessage(entries: GitStatusEntry[]): string {
  if (entries.length === 0) return 'Update files'
  const verb = entries.every((entry) => entry.untracked || entry.indexStatus === 'A') ? 'Add'
    : entries.every((entry) => entry.indexStatus === 'D' || entry.worktreeStatus === 'D') ? 'Remove'
      : 'Update'
  const names = entries.map((entry) => entry.path.split('/').pop() || entry.path)
  const shown = names.slice(0, 2).join(', ')
  return names.length > 2 ? `${verb} ${shown} and ${names.length - 2} more` : `${verb} ${shown}`
}

export function hasBranch(state: GitRepoState): boolean {
  return Boolean(state.head) && state.head !== 'HEAD' && state.head !== '(detached)'
}

export function resolveGitQuickAction(state: GitRepoState | null, hasRemote: boolean, busy: boolean): GitQuickAction {
  if (busy) return { kind: 'hint', action: 'commit', hint: 'busy' }
  if (!state || state.status !== 'ready') return { kind: 'hint', action: 'commit', hint: 'unavailable' }
  if (!hasBranch(state)) return { kind: 'hint', action: 'commit', hint: 'detached' }
  const behind = state.behind > 0
  if (state.entries.length > 0) {
    // 落后上游时只提交，避免推送被拒；没有远程也只能提交。
    return { kind: 'run', action: behind || !hasRemote ? 'commit' : 'commit_push' }
  }
  if (!state.upstream) {
    // 参照 T3：没有远程时，干净的分支提供「发布仓库」。
    return { kind: 'run', action: hasRemote ? 'push' : 'publish' }
  }
  // 参照 T3：只落后时拉取；既领先又落后时不替用户合并。
  if (behind && state.ahead > 0) return { kind: 'hint', action: 'pull', hint: 'diverged' }
  if (behind) return { kind: 'run', action: 'pull' }
  if (state.ahead > 0) return { kind: 'run', action: 'push' }
  return { kind: 'hint', action: 'push', hint: 'upToDate' }
}

export function resolveGitMenu(state: GitRepoState | null, hasRemote: boolean, busy: boolean): GitMenuState {
  if (busy || !state || state.status !== 'ready' || !hasBranch(state)) {
    return { commit: false, commit_push: false, push: false, pull: false, publish: false }
  }
  const dirty = state.entries.length > 0
  const pushable = hasRemote && state.behind === 0
  return {
    commit: dirty,
    commit_push: dirty && pushable,
    push: pushable && (state.ahead > 0 || !state.upstream),
    pull: Boolean(state.upstream) && state.behind > 0 && state.ahead === 0 && !dirty,
    publish: !hasRemote,
  }
}
