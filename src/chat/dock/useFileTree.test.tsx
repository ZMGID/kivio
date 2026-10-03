import { act, cleanup, renderHook, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { DockFsListResult } from '../../api/dockContracts'
import { flattenTreeRows } from './fileTreeModel'
import { useFileTree } from './useFileTree'

const mocks = vi.hoisted(() => ({ fsList: vi.fn(), fsSearch: vi.fn() }))
vi.mock('./api', () => ({ dockApi: mocks }))
vi.mock('./workspaceActivity', () => ({
  workspaceActivity: { isAvailable: () => true, subscribe: () => () => {} },
}))

const expandedPaths = new Set(['samples', 'samples/group-a'])
function listing(path: string, leaf = 'case-001.md'): DockFsListResult {
  return {
    entries: path === '' ? [{ path: 'samples', kind: 'dir', hidden: false }]
      : path === 'samples' ? [{ path: 'samples/group-a', kind: 'dir', hidden: false }]
        : [{ path: `samples/group-a/${leaf}`, kind: 'file', hidden: false }],
    hasMore: false,
  }
}

beforeEach(() => {
  vi.resetAllMocks()
  mocks.fsList.mockImplementation(async (_workdir: string, path: string) => listing(path))
})
afterEach(cleanup)

describe('restored file tree expansion', () => {
  it('loads nested remembered directories when the panel opens, without requiring another click', async () => {
    const { result, rerender } = renderHook(({ active }) => useFileTree({
      workdir: '/test', active, showHidden: false, expandedPaths,
    }), { initialProps: { active: false } })
    expect(mocks.fsList).not.toHaveBeenCalled()
    rerender({ active: true })
    await waitFor(() => expect(flattenTreeRows(result.current.nodes, expandedPaths).map(row => row.path))
      .toContain('samples/group-a/case-001.md'))
    rerender({ active: false })
    rerender({ active: true })
    expect(mocks.fsList).toHaveBeenCalledTimes(3)
  })

  it('keeps failed restoration visible for explicit retry instead of repeatedly requesting it', async () => {
    mocks.fsList.mockImplementation(async (_workdir: string, path: string) => {
      if (path === 'samples/group-a') throw new Error('temporary read failure')
      return listing(path)
    })
    const { result, rerender } = renderHook(() => useFileTree({
      workdir: '/test', active: true, showHidden: false, expandedPaths,
    }))
    await waitFor(() => expect(result.current.nodes['samples/group-a']?.error).toBe('temporary read failure'))
    expect(flattenTreeRows(result.current.nodes, expandedPaths)).toContainEqual({
      type: 'error', path: 'samples/group-a', depth: 2,
    })
    rerender()
    expect(mocks.fsList).toHaveBeenCalledTimes(3)
    mocks.fsList.mockImplementation(async (_workdir: string, path: string) => listing(path))
    await act(async () => result.current.loadChildren('samples/group-a', { force: true }))
    expect(result.current.nodes['samples/group-a/case-001.md']).toBeDefined()
  })

  it('does not restore a remembered descendant while its parent is collapsed', async () => {
    const { result, rerender } = renderHook(({ expanded }) => useFileTree({
      workdir: '/test', active: true, showHidden: false, expandedPaths: expanded,
    }), { initialProps: { expanded: new Set(['samples']) } })
    await waitFor(() => expect(result.current.nodes['samples/group-a']).toBeDefined())
    rerender({ expanded: new Set(['samples/group-a']) })
    expect(flattenTreeRows(result.current.nodes, new Set(['samples/group-a'])).map(row => row.path))
      .toEqual(['samples'])
    expect(mocks.fsList).toHaveBeenCalledTimes(2)
  })

  it('discards a late directory listing after switching workdirs and restores the new tree', async () => {
    let resolveOld!: (value: DockFsListResult) => void
    mocks.fsList.mockImplementation(async (workdir: string, path: string) => {
      if (workdir === '/old' && path === 'samples/group-a') {
        return new Promise<DockFsListResult>(resolve => { resolveOld = resolve })
      }
      return listing(path, 'new.md')
    })
    const { result, rerender } = renderHook(({ workdir }) => useFileTree({
      workdir, active: true, showHidden: false, expandedPaths,
    }), { initialProps: { workdir: '/old' } })
    await waitFor(() => expect(result.current.nodes['samples/group-a']?.loading).toBe(true))
    rerender({ workdir: '/new' })
    await waitFor(() => expect(result.current.nodes['samples/group-a/new.md']).toBeDefined())
    await act(async () => resolveOld(listing('samples/group-a', 'old.md')))
    expect(result.current.nodes['samples/group-a/old.md']).toBeUndefined()
    expect(result.current.nodes['samples/group-a/new.md']).toBeDefined()
  })

  it.each(['hidden filter', 'workdir round trip'])('isolates pending requests across a %s reset', async (reset) => {
    const pending: Array<(value: DockFsListResult) => void> = []
    mocks.fsList.mockImplementation(async (workdir: string, path: string) => {
      if (workdir === '/other') return { entries: [], hasMore: false }
      if (path === 'samples/group-a') {
        return new Promise<DockFsListResult>(resolve => { pending.push(resolve) })
      }
      return listing(path)
    })
    const { result, rerender } = renderHook(({ workdir, showHidden }) => useFileTree({
      workdir, showHidden, active: true, expandedPaths,
    }), { initialProps: { workdir: '/test', showHidden: true } })
    await waitFor(() => expect(pending).toHaveLength(1))
    if (reset === 'hidden filter') rerender({ workdir: '/test', showHidden: false })
    else {
      rerender({ workdir: '/other', showHidden: true })
      rerender({ workdir: '/test', showHidden: true })
    }
    await waitFor(() => expect(pending).toHaveLength(2))
    await act(async () => pending[0]({
      entries: [{ path: 'samples/group-a/.stale', kind: 'file', hidden: true }], hasMore: false,
    }))
    expect(result.current.nodes['samples/group-a/.stale']).toBeUndefined()
    // The stale finally must not unlock the newer request for the same directory.
    act(() => { void result.current.loadChildren('samples/group-a', { force: true }) })
    expect(pending).toHaveLength(2)
    await act(async () => pending[1](listing('samples/group-a', 'current.md')))
    expect(result.current.nodes['samples/group-a/current.md']).toBeDefined()
    expect(result.current.nodes['samples/group-a/.stale']).toBeUndefined()
  })
})


describe('file search request ownership', () => {
  afterEach(() => vi.useRealTimers())

  it.each(['clear', 'new query', 'hidden filter', 'workdir round trip'])(
    'discards a late result after %s', async (change) => {
      vi.useFakeTimers()
      const pending: Array<(value: { entries: Array<{ path: string; kind: string; hidden: boolean }>; truncated: boolean }) => void> = []
      mocks.fsSearch.mockImplementation(() => new Promise(resolve => { pending.push(resolve) }))
      const { result, rerender } = renderHook(({ workdir, showHidden }) => useFileTree({
        workdir, showHidden, active: true, expandedPaths: new Set<string>(),
      }), { initialProps: { workdir: '/test', showHidden: false } })
      act(() => result.current.setSearchQuery('old'))
      await act(async () => { await vi.advanceTimersByTimeAsync(180) })
      expect(pending).toHaveLength(1)
      if (change === 'clear') act(() => result.current.setSearchQuery(''))
      if (change === 'new query') act(() => result.current.setSearchQuery('new'))
      if (change === 'hidden filter') rerender({ workdir: '/test', showHidden: true })
      if (change === 'workdir round trip') {
        rerender({ workdir: '/other', showHidden: false })
        rerender({ workdir: '/test', showHidden: false })
      }
      await act(async () => pending[0]({ entries: [{ path: 'old.txt', kind: 'file', hidden: false }], truncated: false }))
      expect(result.current.searchResults).toBeNull()
      if (change === 'new query' || change === 'hidden filter') {
        await act(async () => { await vi.advanceTimersByTimeAsync(180) })
        expect(pending).toHaveLength(2)
        await act(async () => pending[1]({ entries: [{ path: 'current.txt', kind: 'file', hidden: false }], truncated: false }))
        expect(result.current.searchResults?.[0].path).toBe('current.txt')
      }
    },
  )
})
