import type { GitCommitItem } from '../../api/dockContracts'

export const GRAPH_COLORS = ['#a855f7', '#f59e0b', '#ec4899', '#14b8a6', '#3b82f6', '#f97316']
type Lane = { sha: string; color: number }
export type GraphEdge = { from: number; to: number; color: number; half: 'top' | 'bottom' }
export type GraphRow = { lane: number; color: number; width: number; edges: GraphEdge[] }

/** Pending parent lanes survive page boundaries; every edge follows an actual parent SHA. */
export function layoutGitGraph(commits: GitCommitItem[]): GraphRow[] {
  const lanes: Lane[] = []
  let nextColor = 0
  return commits.map((commit) => {
    const before = [...lanes]
    let lane = lanes.findIndex((item) => item.sha === commit.sha)
    if (lane < 0) {
      lane = lanes.length
      lanes.push({ sha: commit.sha, color: nextColor++ })
    }
    const current = lanes[lane]
    const middle = [...lanes]
    lanes.splice(lane, 1)
    const parents = [...new Set(commit.parents ?? [])]
    parents.forEach((sha, index) => {
      if (!lanes.some((item) => item.sha === sha)) {
        lanes.splice(Math.min(lane + index, lanes.length), 0, {
          sha, color: index === 0 ? current.color : nextColor++,
        })
      }
    })
    const edges: GraphEdge[] = before.map((item, from) => ({
      from, to: middle.findIndex((next) => next.sha === item.sha), color: item.color, half: 'top',
    }))
    middle.forEach((item, from) => {
      if (item.sha === commit.sha) return
      edges.push({ from, to: lanes.findIndex((next) => next.sha === item.sha), color: item.color, half: 'bottom' })
    })
    parents.forEach((sha) => {
      const to = lanes.findIndex((item) => item.sha === sha)
      edges.push({ from: lane, to, color: lanes[to].color, half: 'bottom' })
    })
    return { lane, color: current.color, width: Math.max(before.length, middle.length, lanes.length), edges }
  })
}
