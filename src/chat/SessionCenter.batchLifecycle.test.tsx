import { save } from '@tauri-apps/plugin-dialog'
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { chatApi } from './api'
import { SessionCenter } from './SessionCenter'
import { resetSessionBatchStoreForTests, sessionBatchStore, startSessionBatch } from './sessionBatchStore'
import type { ChatProject, ChatSet, ConversationSearchHit } from './types'

vi.mock('@tauri-apps/plugin-dialog', () => ({
  save: vi.fn(async () => '/tmp/history.md'),
}))

const chatSet: ChatSet = {
  id: 'set-1',
  name: '集一',
  created_at: 1,
  updated_at: 1,
}

const project: ChatProject = {
  id: 'project-1',
  name: '项目一',
  created_at: 1,
  updated_at: 1,
}

const conversation: ConversationSearchHit = {
  id: 'conversation-1',
  title: '历史对话',
  preview: '预览',
  provider_id: 'provider',
  model: 'model',
  message_count: 2,
  created_at: 1,
  updated_at: 1,
  project_id: project.id,
  folder: project.name,
}

let library: ConversationSearchHit[] = [conversation]

const openGates: Array<{ reject: (error: Error) => void }> = []

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: Error) => void
  const promise = new Promise<T>((yes, no) => {
    resolve = yes
    reject = no
  })
  const gate = { promise, resolve, reject }
  openGates.push(gate)
  return gate
}

function renderCenter(options?: {
  onConversationsChanged?: () => void
  onConversationDeleted?: (id: string) => void
}) {
  return render(
    <SessionCenter
      lang="zh"
      embedded
      currentConversationId={conversation.id}
      onSelectConversation={vi.fn()}
      onConversationsChanged={options?.onConversationsChanged}
      onConversationDeleted={options?.onConversationDeleted}
    />,
  )
}

async function selectVisible() {
  await screen.findByText(library[0]?.title ?? '历史对话')
  fireEvent.click(screen.getAllByRole('checkbox')[0])
}

function toolbar() {
  const selected = screen.getByText(/^已选 /)
  const bar = selected.parentElement
  if (!bar) throw new Error('missing selection bar')
  return bar
}

function toolbarButton(name: string) {
  return within(toolbar()).getByRole('button', { name })
}

beforeEach(() => {
  resetSessionBatchStoreForTests()
  library = [conversation]
  vi.spyOn(window, 'alert').mockImplementation(() => {})
  vi.spyOn(window, 'confirm').mockReturnValue(true)
  vi.spyOn(chatApi, 'queryConversations').mockImplementation(async () => ({
    items: library,
    total: library.length,
  }))
  vi.spyOn(chatApi, 'getProjects').mockResolvedValue([project])
  vi.spyOn(chatApi, 'getSets').mockResolvedValue([])
})

afterEach(async () => {
  resetSessionBatchStoreForTests()
  for (const gate of openGates) gate.reject(new Error('settled by test cleanup'))
  openGates.length = 0
  await act(async () => {
    await Promise.resolve()
  })
  vi.restoreAllMocks()
})

describe('SessionCenter batch lifetime', () => {
  it('dedupes ids and keeps a second archive from mutating again', async () => {
    const update = vi.spyOn(chatApi, 'bulkUpdateConversations').mockResolvedValue(2)
    const first = startSessionBatch({ kind: 'archive', archived: true, ids: ['a', 'a', 'b'] })
    const again = startSessionBatch({ kind: 'archive', archived: false, ids: ['c'] })
    expect(again).toBe(first)
    await first
    expect(update).toHaveBeenCalledTimes(1)
    expect(update).toHaveBeenCalledWith(['a', 'b'], { archived: true })
  })

  it('stays busy across navigation and refreshes the archived inventory on return', async () => {
    const gate = deferred<number>()
    const update = vi.spyOn(chatApi, 'bulkUpdateConversations').mockReturnValue(gate.promise)
    const onConversationsChanged = vi.fn()
    const onConversationDeleted = vi.fn()
    const first = renderCenter({ onConversationsChanged, onConversationDeleted })
    await selectVisible()
    const archive = toolbarButton('归档')
    await act(async () => {
      archive.click()
      archive.click()
    })
    expect(update).toHaveBeenCalledTimes(1)
    expect(toolbarButton('归档')).toBeDisabled()

    first.unmount()
    const returned = renderCenter({ onConversationsChanged, onConversationDeleted })
    expect(await screen.findByText('已选 1')).toBeInTheDocument()
    expect(toolbarButton('归档')).toBeDisabled()
    expect(screen.getByText('已选 1').parentElement).toHaveAttribute('aria-busy', 'true')
    returned.unmount()
    expect(onConversationsChanged).not.toHaveBeenCalled()

    library = [{ ...conversation, title: '归档后的对话', archived: true }]
    gate.resolve(1)
    await act(async () => {
      await gate.promise
    })
    expect(onConversationsChanged).not.toHaveBeenCalled()
    expect(onConversationDeleted).not.toHaveBeenCalled()

    renderCenter({ onConversationsChanged, onConversationDeleted })
    expect(await screen.findByText('归档后的对话')).toBeInTheDocument()
    await waitFor(() => expect(onConversationsChanged).toHaveBeenCalled())
    expect(onConversationDeleted).toHaveBeenCalledWith(conversation.id)
    expect(update).toHaveBeenCalledTimes(1)
  })

  it('keeps a failed delete selectable and retries without a second in-flight mutation', async () => {
    const gate = deferred<{ deleted: number; warnings: string[] }>()
    const remove = vi.spyOn(chatApi, 'bulkDeleteConversations').mockReturnValue(gate.promise)
    const first = renderCenter()
    await selectVisible()
    fireEvent.click(screen.getByRole('button', { name: '删除' }))
    await act(async () => {
      await Promise.resolve()
    })
    expect(remove).toHaveBeenCalledTimes(1)
    first.unmount()

    gate.reject(new Error('disk full'))
    await act(async () => {
      await gate.promise.catch(() => {})
    })

    renderCenter()
    expect(await screen.findByRole('alert')).toHaveTextContent('disk full')
    expect(window.alert).toHaveBeenCalledWith('disk full')
    expect(screen.getByRole('button', { name: '删除' })).toBeEnabled()

    remove.mockResolvedValue({ deleted: 1, warnings: ['orphan blob'] })
    library = []
    fireEvent.click(screen.getByRole('button', { name: '删除' }))
    await waitFor(() => expect(remove).toHaveBeenCalledTimes(2))
    expect(remove.mock.calls[1][0]).toEqual([conversation.id])
    await waitFor(() => expect(window.alert).toHaveBeenCalledWith(expect.stringContaining('orphan blob')))
    expect(await screen.findByText('还没有对话')).toBeInTheDocument()
  })

  it('shows delete warnings that finished while the library was closed', async () => {
    const gate = deferred<{ deleted: number; warnings: string[] }>()
    vi.spyOn(chatApi, 'bulkDeleteConversations').mockReturnValue(gate.promise)
    const onConversationDeleted = vi.fn()
    const first = renderCenter({ onConversationDeleted })
    await selectVisible()
    fireEvent.click(screen.getByRole('button', { name: '删除' }))
    await act(async () => {
      await Promise.resolve()
    })
    first.unmount()
    library = []
    gate.resolve({ deleted: 1, warnings: ['orphan blob'] })
    await act(async () => {
      await gate.promise
    })

    renderCenter({ onConversationDeleted })
    await waitFor(() => expect(window.alert).toHaveBeenCalledWith(expect.stringContaining('orphan blob')))
    expect(onConversationDeleted).toHaveBeenCalledWith(conversation.id)
    expect(await screen.findByText('还没有对话')).toBeInTheDocument()
  })
})

function openRowMenu(title = '历史对话') {
  const row = screen.getByRole('button', { name: new RegExp(title) })
  const trigger = row.querySelector('button[data-row-chrome]')
  if (!trigger) throw new Error('missing row menu')
  fireEvent.click(trigger)
}

describe('SessionCenter remaining batch lifetime', () => {
  it('drops a late archive write after reset and still accepts the next pin', async () => {
    const gate = deferred<number>()
    const update = vi.spyOn(chatApi, 'bulkUpdateConversations').mockReturnValue(gate.promise)
    const flight = startSessionBatch({ kind: 'archive', archived: true, ids: ['a'] })
    await act(async () => { await Promise.resolve() })
    expect(sessionBatchStore.getSnapshot().phase).toBe('running')
    resetSessionBatchStoreForTests()
    gate.resolve(1)
    await act(async () => { await flight })
    expect(sessionBatchStore.getSnapshot()).toMatchObject({ phase: 'idle', action: null, error: '' })
    const pin = startSessionBatch({ kind: 'pin', pinned: true, ids: ['b'] })
    await pin
    expect(update).toHaveBeenLastCalledWith(['b'], { pinned: true })
    expect(sessionBatchStore.getSnapshot().phase).toBe('success')
  })

  it('keeps a second export from opening another save dialog', async () => {
    const saveGate = deferred<string | null>()
    const update = vi.spyOn(chatApi, 'bulkUpdateConversations').mockResolvedValue(1)
    vi.mocked(save).mockReturnValue(saveGate.promise)
    const first = startSessionBatch({
      kind: 'export',
      ids: ['a'],
      targets: [{ id: 'a', title: '甲' }],
      lang: 'zh',
    })
    const second = startSessionBatch({ kind: 'pin', pinned: false, ids: ['b'] })
    expect(second).toBe(first)
    await act(async () => { await Promise.resolve() })
    expect(save).toHaveBeenCalledTimes(1)
    saveGate.resolve(null)
    await first
    expect(sessionBatchStore.getSnapshot().phase).toBe('idle')
    expect(update).not.toHaveBeenCalled()
  })

  it('keeps pin busy across navigation and retries the same ids after failure', async () => {
    const gate = deferred<number>()
    const update = vi.spyOn(chatApi, 'bulkUpdateConversations').mockReturnValue(gate.promise)
    const onConversationDeleted = vi.fn()
    const first = renderCenter({ onConversationDeleted })
    await selectVisible()
    await act(async () => {
      toolbarButton('收藏').click()
      toolbarButton('收藏').click()
      await Promise.resolve()
    })
    expect(update).toHaveBeenCalledTimes(1)
    expect(update).toHaveBeenCalledWith([conversation.id], { pinned: true })
    expect(toolbarButton('收藏')).toBeDisabled()
    first.unmount()

    gate.reject(new Error('pin failed'))
    await act(async () => { await gate.promise.catch(() => {}) })
    renderCenter({ onConversationDeleted })
    expect(await screen.findByRole('alert')).toHaveTextContent('pin failed')
    expect(window.alert).toHaveBeenCalledWith('pin failed')
    expect(onConversationDeleted).not.toHaveBeenCalled()
    expect(toolbarButton('收藏')).toBeEnabled()

    const retry = deferred<number>()
    update.mockReturnValue(retry.promise)
    fireEvent.click(toolbarButton('收藏'))
    await act(async () => { await Promise.resolve() })
    expect(update).toHaveBeenCalledTimes(2)
    expect(update.mock.calls[1]).toEqual([[conversation.id], { pinned: true }])
    library = [{ ...conversation, title: '已收藏', pinned: true }]
    retry.resolve(1)
    await waitFor(() => expect(screen.getByText('已收藏')).toBeInTheDocument())
  })

  it('does not let a finished pin replace a selection the user changed', async () => {
    const other: ConversationSearchHit = { ...conversation, id: 'conversation-2', title: '另一条' }
    library = [conversation, other]
    const gate = deferred<number>()
    const update = vi.spyOn(chatApi, 'bulkUpdateConversations').mockReturnValue(gate.promise)
    const onConversationsChanged = vi.fn()
    renderCenter({ onConversationsChanged })
    await screen.findByText('另一条')
    fireEvent.click(screen.getAllByRole('checkbox')[1])
    fireEvent.click(toolbarButton('收藏'))
    await act(async () => { await Promise.resolve() })
    expect(update).toHaveBeenCalledWith([conversation.id], { pinned: true })
    fireEvent.click(screen.getAllByRole('checkbox')[2])
    await act(async () => { gate.resolve(1) })
    await waitFor(() => expect(onConversationsChanged).toHaveBeenCalled())
    expect(screen.getAllByRole('checkbox')[1]).toBeChecked()
    expect(screen.getAllByRole('checkbox')[2]).toBeChecked()
    expect(update).toHaveBeenCalledTimes(1)
  })

  it('refreshes a project move that finished away without deleting the open chat', async () => {
    const gate = deferred<number>()
    const update = vi.spyOn(chatApi, 'bulkUpdateConversations').mockReturnValue(gate.promise)
    const onConversationDeleted = vi.fn()
    const onConversationsChanged = vi.fn()
    const first = renderCenter({ onConversationDeleted, onConversationsChanged })
    await selectVisible()
    fireEvent.click(screen.getByRole('button', { name: '移入项目' }))
    fireEvent.click(screen.getByRole('menuitem', { name: project.name }))
    await act(async () => { await Promise.resolve() })
    expect(update).toHaveBeenCalledWith([conversation.id], { projectId: project.id })
    first.unmount()
    expect(onConversationsChanged).not.toHaveBeenCalled()
    const returned = renderCenter({ onConversationDeleted, onConversationsChanged })
    expect(await screen.findByRole('button', { name: '移入项目' })).toBeDisabled()
    returned.unmount()
    library = [{ ...conversation, title: '已移入项目' }]
    gate.resolve(1)
    await act(async () => { await gate.promise })
    renderCenter({ onConversationDeleted, onConversationsChanged })
    expect(await screen.findByText('已移入项目')).toBeInTheDocument()
    await waitFor(() => expect(onConversationsChanged).toHaveBeenCalled())
    expect(onConversationDeleted).not.toHaveBeenCalled()
  })

  it('retries a failed set move from the restored selection', async () => {
    vi.spyOn(chatApi, 'getSets').mockResolvedValue([chatSet])
    const gate = deferred<number>()
    const update = vi.spyOn(chatApi, 'bulkUpdateConversations').mockReturnValue(gate.promise)
    const first = renderCenter()
    await selectVisible()
    fireEvent.click(screen.getByRole('button', { name: '移入集' }))
    fireEvent.click(screen.getByRole('menuitem', { name: chatSet.name }))
    await act(async () => { await Promise.resolve() })
    expect(update).toHaveBeenCalledWith([conversation.id], { setId: chatSet.id })
    first.unmount()
    gate.reject(new Error('set busy'))
    await act(async () => { await gate.promise.catch(() => {}) })
    renderCenter()
    expect(await screen.findByRole('alert')).toHaveTextContent('set busy')
    expect(await screen.findByRole('button', { name: chatSet.name })).toBeInTheDocument()
    update.mockResolvedValue(1)
    fireEvent.click(screen.getByRole('button', { name: '移入集' }))
    fireEvent.click(screen.getByRole('menuitem', { name: chatSet.name }))
    await waitFor(() => expect(update).toHaveBeenCalledTimes(2))
    expect(update.mock.calls[1]).toEqual([[conversation.id], { setId: chatSet.id }])
  })

  it('keeps a single-row archive on updateConversation across navigation', async () => {
    const gate = deferred<undefined>()
    const update = vi.spyOn(chatApi, 'updateConversation').mockImplementation(() => gate.promise as never)
    const bulk = vi.spyOn(chatApi, 'bulkUpdateConversations').mockResolvedValue(1)
    const onConversationDeleted = vi.fn()
    const onConversationsChanged = vi.fn()
    const first = renderCenter({ onConversationDeleted, onConversationsChanged })
    await screen.findByText('历史对话')
    openRowMenu()
    await act(async () => {
      fireEvent.click(screen.getByRole('menuitem', { name: '归档' }))
      await Promise.resolve()
    })
    expect(update).toHaveBeenCalledTimes(1)
    expect(update).toHaveBeenCalledWith(conversation.id, { archived: true })
    expect(bulk).not.toHaveBeenCalled()
    expect(toolbarButton('归档')).toBeDisabled()
    first.unmount()
    expect(onConversationDeleted).not.toHaveBeenCalled()
    library = [{ ...conversation, title: '行已归档', archived: true }]
    gate.resolve(undefined)
    await act(async () => { await gate.promise })
    renderCenter({ onConversationDeleted, onConversationsChanged })
    expect(await screen.findByText('行已归档')).toBeInTheDocument()
    await waitFor(() => expect(onConversationDeleted).toHaveBeenCalledWith(conversation.id))
    expect(onConversationsChanged).toHaveBeenCalled()
    expect(update).toHaveBeenCalledTimes(1)
  })

  it('retries a failed single-row delete with the same command', async () => {
    const gate = deferred<string[]>()
    const remove = vi.spyOn(chatApi, 'deleteConversation').mockReturnValue(gate.promise)
    const bulk = vi.spyOn(chatApi, 'bulkDeleteConversations')
    const first = renderCenter()
    await screen.findByText('历史对话')
    openRowMenu()
    await act(async () => {
      fireEvent.click(screen.getByRole('menuitem', { name: '删除' }))
      await Promise.resolve()
    })
    expect(remove).toHaveBeenCalledTimes(1)
    expect(remove).toHaveBeenCalledWith(conversation.id)
    expect(bulk).not.toHaveBeenCalled()
    first.unmount()
    gate.reject(new Error('disk full'))
    await act(async () => { await gate.promise.catch(() => {}) })
    renderCenter()
    expect(await screen.findByRole('alert')).toHaveTextContent('删除对话失败：disk full')
    expect(window.alert).toHaveBeenCalledWith('删除对话失败：disk full')
    expect(screen.getByRole('button', { name: '删除' })).toBeEnabled()
    remove.mockResolvedValue(['orphan blob'])
    library = []
    fireEvent.click(screen.getByRole('button', { name: '删除' }))
    await waitFor(() => expect(remove).toHaveBeenCalledTimes(2))
    expect(remove.mock.calls[1]).toEqual([conversation.id])
    expect(bulk).not.toHaveBeenCalled()
    await waitFor(() => expect(window.alert).toHaveBeenCalledWith(expect.stringContaining('orphan blob')))
    expect(await screen.findByText('还没有对话')).toBeInTheDocument()
  })

  it('keeps a partial export and retries the same selection after a late failure', async () => {
    const other: ConversationSearchHit = { ...conversation, id: 'conversation-2', title: '另一条' }
    library = [conversation, other]
    const paths = ['/tmp/one.md', '/tmp/two.md', '/tmp/retry-a.md', '/tmp/retry-b.md']
    vi.mocked(save).mockImplementation(async () => paths.shift() ?? '/tmp/more.md')
    const gate = deferred<void>()
    const exp = vi.spyOn(chatApi, 'exportConversationMarkdown').mockImplementation((id) => {
      if (id === conversation.id) return Promise.resolve()
      return gate.promise
    })
    const first = renderCenter()
    await screen.findByText('另一条')
    fireEvent.click(screen.getAllByRole('checkbox')[0])
    fireEvent.click(screen.getByRole('button', { name: '导出' }))
    await waitFor(() => expect(exp).toHaveBeenCalledTimes(2))
    expect(exp.mock.calls.map((call) => call[0])).toEqual([conversation.id, other.id])
    first.unmount()
    const away = renderCenter()
    expect(await screen.findByRole('button', { name: '导出' })).toBeDisabled()
    expect(screen.getByText('已选 2').parentElement).toHaveAttribute('aria-busy', 'true')
    away.unmount()
    gate.reject(new Error('export failed'))
    await act(async () => { await gate.promise.catch(() => {}) })
    renderCenter()
    const alert = await screen.findByRole('alert')
    expect(alert).toHaveTextContent('export failed')
    expect(alert).toHaveTextContent('/tmp/one.md')
    expect(screen.getByRole('button', { name: '导出' })).toBeEnabled()
    exp.mockResolvedValue(undefined)
    fireEvent.click(screen.getByRole('button', { name: '导出' }))
    await waitFor(() => expect(exp).toHaveBeenCalledTimes(4))
    expect(exp.mock.calls[2]?.[0]).toBe(conversation.id)
    expect(exp.mock.calls[3]?.[0]).toBe(other.id)
  })
})
