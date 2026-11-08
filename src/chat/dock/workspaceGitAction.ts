// 工作区面板的 Git 主按钮与下拉项，参照 T3 Code 的 resolveQuickAction / buildMenuItems：
// 主按钮只给出当前最合理的一步；做不了时禁用并说明原因，而不是让用户点了再失败。
import type { GitRepoState } from '../../api/dockContracts'

export type GitAction = 'commit' | 'commit_push' | 'push'

export type GitHint = 'busy' | 'detached' | 'noRemote' | 'behind' | 'upToDate' | 'unavailable'

export type GitQuickAction =
  | { kind: 'run'; action: GitAction }
  | { kind: 'hint'; action: GitAction; hint: GitHint }

export type GitMenuState = Record<GitAction, boolean>

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
    return hasRemote ? { kind: 'run', action: 'push' } : { kind: 'hint', action: 'push', hint: 'noRemote' }
  }
  if (behind) return { kind: 'hint', action: 'push', hint: 'behind' }
  if (state.ahead > 0) return { kind: 'run', action: 'push' }
  return { kind: 'hint', action: 'push', hint: 'upToDate' }
}

export function resolveGitMenu(state: GitRepoState | null, hasRemote: boolean, busy: boolean): GitMenuState {
  if (busy || !state || state.status !== 'ready' || !hasBranch(state)) {
    return { commit: false, commit_push: false, push: false }
  }
  const dirty = state.entries.length > 0
  const pushable = hasRemote && state.behind === 0
  return {
    commit: dirty,
    commit_push: dirty && pushable,
    push: pushable && (state.ahead > 0 || !state.upstream),
  }
}
