import { act, renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { dockApi } from '../dock/api'
import {
  requestDockDiffPreview,
  requestDockMarkdownPreview,
  requestDockPreview,
  requestDockSubAgent,
} from '../dock/dockPreview'
import { insertTextIntoComposer } from '../composerInsert'
import { useRightDock } from './useRightDock'

vi.mock('../dock/api', () => ({
  dockApi: { resolveCwd: vi.fn() },
}))
vi.mock('../composerInsert', () => ({
  insertTextIntoComposer: vi.fn(),
}))

const mockResolveCwd = vi.mocked(dockApi.resolveCwd)

function setup(overrides: Partial<Parameters<typeof useRightDock>[0]> = {}) {
  const currentConversationIdRef = { current: overrides.conversationId ?? 'c1' }
  return renderHook(
    (props: Parameters<typeof useRightDock>[0]) => useRightDock(props),
    {
      initialProps: {
        conversationId: 'c1',
        projectId: null,
        agentRuntimeKind: 'builtin',
        currentConversationIdRef,
        ...overrides,
      },
    },
  )
}

beforeEach(() => {
  window.localStorage.clear()
  mockResolveCwd.mockReset()
  mockResolveCwd.mockResolvedValue('C:/work/proj')
})

describe('useRightDock: workdir', () => {
  it('resolves the workdir from the conversation and clears it when nothing is selected', async () => {
    const { result, rerender } = setup()
    await act(async () => {})
    expect(mockResolveCwd).toHaveBeenCalledWith('c1', null)
    expect(result.current.workdir).toBe('C:/work/proj')

    rerender({
      conversationId: null, projectId: null, agentRuntimeKind: 'builtin',
      currentConversationIdRef: { current: null },
    })
    expect(result.current.workdir).toBe('')
  })

  it('re-resolves when the agent runtime kind changes (built-in vs external write dirs differ)', async () => {
    const { rerender } = setup()
    await act(async () => {})
    mockResolveCwd.mockResolvedValue('C:/work/external')
    rerender({
      conversationId: 'c1', projectId: null, agentRuntimeKind: 'external',
      currentConversationIdRef: { current: 'c1' },
    })
    await act(async () => {})
    expect(mockResolveCwd).toHaveBeenCalledTimes(2)
  })

  it('ignores a late resolve for a superseded conversation', async () => {
    let resolveFirst: (cwd: string) => void = () => {}
    mockResolveCwd.mockImplementationOnce(() => new Promise((resolve) => { resolveFirst = resolve }))
    const { result, rerender } = setup()
    mockResolveCwd.mockResolvedValueOnce('C:/second')
    rerender({
      conversationId: 'c2', projectId: null, agentRuntimeKind: 'builtin',
      currentConversationIdRef: { current: 'c2' },
    })
    await act(async () => {})
    expect(result.current.workdir).toBe('C:/second')
    await act(async () => { resolveFirst('C:/first-late') })
    expect(result.current.workdir).toBe('C:/second')
  })
})

describe('useRightDock: persisted UI state', () => {
  it('toggles / closes and remembers the open state', () => {
    const { result } = setup()
    const initial = result.current.open
    act(() => result.current.toggle())
    expect(result.current.open).toBe(!initial)
    act(() => result.current.close())
    expect(result.current.open).toBe(false)
    const { result: fresh } = setup()
    expect(fresh.current.open).toBe(false)
  })

  it('openGit / openTasks switch tab and force the dock open', () => {
    const { result } = setup()
    act(() => result.current.close())
    act(() => result.current.openGit())
    expect(result.current).toMatchObject({ open: true, tab: 'git' })
    act(() => result.current.openTasks())
    expect(result.current).toMatchObject({ open: true, tab: 'tasks' })
  })

  it('remembers width and tab across mounts', () => {
    const { result } = setup()
    act(() => {
      result.current.setWidth(420)
      result.current.setTab('terminal')
    })
    const { result: fresh } = setup()
    expect(fresh.current.width).toBe(420)
    expect(fresh.current.tab).toBe('terminal')
  })

  it('persists tree expansion per workdir and reloads it when the workdir changes back', async () => {
    const { result, rerender } = setup()
    await act(async () => {})
    act(() => result.current.setTreeExpanded(['src', 'src/chat']))
    expect(result.current.treeExpanded).toEqual(['src', 'src/chat'])

    mockResolveCwd.mockResolvedValue('C:/other')
    rerender({
      conversationId: 'c2', projectId: null, agentRuntimeKind: 'builtin',
      currentConversationIdRef: { current: 'c2' },
    })
    await act(async () => {})
    expect(result.current.treeExpanded).toEqual([])

    mockResolveCwd.mockResolvedValue('C:/work/proj')
    rerender({
      conversationId: 'c1', projectId: null, agentRuntimeKind: 'builtin',
      currentConversationIdRef: { current: 'c1' },
    })
    await act(async () => {})
    expect(result.current.treeExpanded).toEqual(['src', 'src/chat'])
  })
})

describe('useRightDock: preview channels', () => {
  it('file preview inside the workdir opens files tab, reveals and previews', async () => {
    const { result } = setup()
    await act(async () => {})
    act(() => result.current.close())
    act(() => requestDockPreview('src/a.ts'))
    expect(result.current).toMatchObject({ open: true, tab: 'files' })
    expect(result.current.reveal).toMatchObject({ path: 'src/a.ts', nonce: 1 })
    expect(result.current.preview).toMatchObject({ kind: 'file', workdir: 'C:/work/proj', path: 'src/a.ts', nonce: 1 })
  })

  it('file preview outside the workdir previews without revealing', async () => {
    const { result } = setup()
    await act(async () => {})
    act(() => requestDockPreview('D:/desk/out.md'))
    expect(result.current.reveal).toBeNull()
    expect(result.current.preview).toMatchObject({ kind: 'file', workdir: 'D:/desk', path: 'out.md' })
  })

  it('diff and markdown previews bump the nonce so identical payloads still re-render', () => {
    const { result } = setup()
    act(() => requestDockDiffPreview({ title: 't', patch: 'p' }))
    act(() => requestDockDiffPreview({ title: 't', patch: 'p' }))
    expect(result.current.preview).toMatchObject({ kind: 'diff', title: 't', patch: 'p', nonce: 2 })
    act(() => requestDockMarkdownPreview({ title: '计划', text: '# plan' }))
    expect(result.current.preview).toMatchObject({ kind: 'markdown', title: '计划', text: '# plan', nonce: 3 })
  })

  it('sub-agent requests only apply to the current conversation and open the tasks tab', () => {
    const currentConversationIdRef = { current: 'c1' }
    const { result } = setup({ currentConversationIdRef })
    act(() => requestDockSubAgent({ conversationId: 'other', agentId: 'a' }))
    expect(result.current.subAgentRequest).toBeNull()
    act(() => requestDockSubAgent({ conversationId: 'c1', agentId: 'a' }))
    expect(result.current.subAgentRequest).toMatchObject({ conversationId: 'c1', agentId: 'a', nonce: 1 })
    expect(result.current.tab).toBe('tasks')
  })

  it('revealInTree and insertMention plumb to the tree / composer', () => {
    const { result } = setup()
    act(() => result.current.revealInTree('src/x.ts'))
    expect(result.current).toMatchObject({ open: true, tab: 'files' })
    expect(result.current.reveal).toMatchObject({ path: 'src/x.ts' })
    result.current.insertMention('src/x.ts')
    expect(insertTextIntoComposer).toHaveBeenCalledWith('@src/x.ts ')
  })
})
