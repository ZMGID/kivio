import { describe, expect, it } from 'vitest'
import { layoutGitGraph } from './gitGraph'
import type { GitCommitItem } from '../../api/dockContracts'

const commit = (sha: string, parents: string[] = []): GitCommitItem => ({ sha, parents, shortSha: sha, subject: sha, authorName: '', authorDate: '', refs: [] })

describe('commit graph topology', () => {
  it('connects a linear history and stops at the root', () => {
    const rows = layoutGitGraph([commit('a', ['b']), commit('b', ['c']), commit('c')])
    expect(rows.map((row) => row.lane)).toEqual([0, 0, 0])
    expect(rows[1].edges).toHaveLength(2)
    expect(rows[2].edges.every((edge) => edge.half === 'top')).toBe(true)
  })

  it('forks at merge commits and joins at the common ancestor', () => {
    const rows = layoutGitGraph([commit('merge', ['main', 'feature']), commit('feature', ['base']), commit('main', ['base']), commit('base')])
    expect(rows[0].edges.filter((edge) => edge.half === 'bottom').map((edge) => edge.to)).toEqual([0, 1])
    expect(rows[1].lane).toBe(1)
    expect(rows[2].edges.filter((edge) => edge.half === 'bottom').map((edge) => edge.to)).toEqual([0, 0])
    expect(rows[3].width).toBe(1)
  })

  it('does not invent edges between unrelated branch tips', () => {
    const rows = layoutGitGraph([commit('a', ['base']), commit('other'), commit('base')])
    expect(rows[1].lane).toBe(1)
    expect(rows[1].edges.filter((edge) => edge.half === 'bottom')).toEqual([{ from: 0, to: 0, color: 0, half: 'bottom' }])
  })

  it('keeps existing geometry and colors when another page arrives', () => {
    const first = [commit('merge', ['a', 'b']), commit('a', ['root'])]
    expect(layoutGitGraph([...first, commit('b', ['root']), commit('root')]).slice(0, 2)).toEqual(layoutGitGraph(first))
  })

  it('connects all parents of an octopus merge without negative lanes', () => {
    const rows = layoutGitGraph([commit('merge', ['a', 'b', 'c']), commit('b', ['root']), commit('a', ['root']), commit('c', ['root']), commit('root')])
    expect(rows[0].edges).toHaveLength(3)
    expect(rows.flatMap((row) => row.edges).every((edge) => edge.from >= 0 && edge.to >= 0)).toBe(true)
    for (let i = 0; i < rows.length - 1; i++) {
      const bottom = [...new Set(rows[i].edges.filter((edge) => edge.half === 'bottom').map((edge) => edge.to))].sort()
      const top = rows[i + 1].edges.filter((edge) => edge.half === 'top').map((edge) => edge.from).sort()
      expect(bottom).toEqual(top)
    }
  })
})
