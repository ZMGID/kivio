import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { useState } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { chatApi } from './api'
import { CliImportDialog } from './CliImportDialog'
import { resetCliImportStoreForTests, startCliImport } from './cliImportStore'
import type { ChatProject, CliImportResult, ImportableCliSession } from './types'

const project: ChatProject = {
  id: 'project-1',
  name: '项目',
  root_path: '/repo',
  created_at: 1,
  updated_at: 1,
}

const session: ImportableCliSession = {
  agentId: 'claude',
  sessionId: 'sess-1',
  title: '原生会话',
  cwd: '/repo',
  updatedAt: 1_700_000_000_000,
  messageCount: 2,
  alreadyImported: false,
}

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

function Harness({
  onImported,
  onOpenChange,
}: {
  onImported: (ids: string[]) => void
  onOpenChange?: (open: boolean) => void
}) {
  const [open, setOpen] = useState(true)
  const set = (next: boolean) => {
    setOpen(next)
    onOpenChange?.(next)
  }
  return (
    <>
      <button type="button" onClick={() => set(true)}>重新打开</button>
      {open && (
        <CliImportDialog
          project={project}
          onClose={() => set(false)}
          onImported={onImported}
          onOpenConversation={vi.fn()}
        />
      )}
    </>
  )
}

async function chooseSession() {
  fireEvent.click(await screen.findByRole('button', { name: /原生会话/ }))
}

async function closeDialog() {
  const dialog = screen.getByRole('dialog')
  fireEvent.mouseDown(dialog.parentElement as HTMLElement)
  fireEvent.animationEnd(dialog)
  await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
}

beforeEach(() => {
  resetCliImportStoreForTests()
  vi.spyOn(chatApi, 'listImportableCliSessions').mockResolvedValue([session])
})

afterEach(async () => {
  resetCliImportStoreForTests()
  for (const gate of openGates) gate.reject(new Error('settled by test cleanup'))
  openGates.length = 0
  await act(async () => {
    await Promise.resolve()
  })
  vi.restoreAllMocks()
})

describe('CLI session import lifetime', () => {
  it('dedupes session identities and ignores a second start while one is in flight', async () => {
    const gate = deferred<CliImportResult>()
    const importSessions = vi.spyOn(chatApi, 'importCliSessions').mockReturnValue(gate.promise)
    const first = startCliImport(project.id, [
      { agentId: 'claude', sessionId: 'sess-1' },
      { agentId: 'claude', sessionId: 'sess-1' },
    ])
    const second = startCliImport(project.id, [{ agentId: 'claude', sessionId: 'sess-2' }])
    expect(second).toBe(first)
    await act(async () => {
      await Promise.resolve()
    })
    expect(importSessions).toHaveBeenCalledTimes(1)
    expect(importSessions).toHaveBeenCalledWith(project.id, [{ agentId: 'claude', sessionId: 'sess-1' }])
    gate.resolve({ success: true, imported: [], failures: [] })
    await first
  })

  it('keeps importing after the dialog closes and applies the new conversation on return', async () => {
    const gate = deferred<CliImportResult>()
    const importSessions = vi.spyOn(chatApi, 'importCliSessions').mockReturnValue(gate.promise)
    const onImported = vi.fn()
    render(<Harness onImported={onImported} />)
    await chooseSession()
    const button = screen.getByRole('button', { name: '导入 1' })
    await act(async () => {
      button.click()
      button.click()
    })
    expect(importSessions).toHaveBeenCalledTimes(1)
    expect(screen.getByRole('dialog')).toHaveAttribute('aria-busy', 'true')

    await closeDialog()
    expect(onImported).not.toHaveBeenCalled()

    fireEvent.click(screen.getByRole('button', { name: '重新打开' }))
    expect(screen.getByRole('dialog')).toHaveAttribute('aria-busy', 'true')
    expect(screen.getByRole('button', { name: '导入 1' })).toBeDisabled()
    await closeDialog()

    gate.resolve({
      success: true,
      imported: [{ agentId: 'claude', sessionId: 'sess-1', conversationId: 'conv-new' }],
      failures: [],
    })
    await act(async () => {
      await gate.promise
    })
    expect(onImported).not.toHaveBeenCalled()

    fireEvent.click(screen.getByRole('button', { name: '重新打开' }))
    await waitFor(() => expect(onImported).toHaveBeenCalledTimes(1))
    expect(onImported).toHaveBeenCalledWith(['conv-new'])
    expect(importSessions).toHaveBeenCalledTimes(1)
    fireEvent.animationEnd(screen.getByRole('dialog'))
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
  })

  it('restores a failed import for retry when the dialog is opened again', async () => {
    const gate = deferred<CliImportResult>()
    const importSessions = vi.spyOn(chatApi, 'importCliSessions').mockReturnValue(gate.promise)
    const onImported = vi.fn()
    const view = render(<Harness onImported={onImported} />)
    await chooseSession()
    fireEvent.click(screen.getByRole('button', { name: '导入 1' }))
    await act(async () => {
      await Promise.resolve()
    })
    view.unmount()

    gate.reject(new Error('disk full'))
    await act(async () => {
      await gate.promise.catch(() => {})
    })

    render(<Harness onImported={onImported} />)
    expect(await screen.findByText('disk full')).toBeInTheDocument()
    expect(screen.getByRole('dialog')).toHaveAttribute('aria-busy', 'false')
    expect(onImported).not.toHaveBeenCalled()

    importSessions.mockResolvedValue({
      success: false,
      imported: [],
      failures: [{ agentId: 'claude', sessionId: 'sess-1', error: 'still broken' }],
    })
    fireEvent.click(screen.getByRole('button', { name: '导入 1' }))
    await waitFor(() => expect(importSessions).toHaveBeenCalledTimes(2))
    expect(importSessions.mock.calls[1][1]).toEqual([{ agentId: 'claude', sessionId: 'sess-1' }])
    expect(await screen.findByText('1 条导入失败：still broken')).toBeInTheDocument()
  })

  it('reports a partial import that finished while the dialog was closed', async () => {
    const gate = deferred<CliImportResult>()
    vi.spyOn(chatApi, 'importCliSessions').mockReturnValue(gate.promise)
    const onImported = vi.fn()
    const view = render(<Harness onImported={onImported} />)
    await chooseSession()
    fireEvent.click(screen.getByRole('button', { name: '导入 1' }))
    await act(async () => {
      await Promise.resolve()
    })
    view.unmount()
    gate.resolve({
      success: true,
      imported: [{ agentId: 'claude', sessionId: 'sess-1', conversationId: 'conv-part' }],
      failures: [{ agentId: 'claude', sessionId: 'sess-2', error: 'missing' }],
    })
    await act(async () => {
      await gate.promise
    })

    render(<Harness onImported={onImported} />)
    expect(await screen.findByText('1 条导入失败：missing')).toBeInTheDocument()
    await waitFor(() => expect(onImported).toHaveBeenCalledWith(['conv-part']))
  })
  it('blocks retry until partial import reconciliation and retries only failed sessions', async () => {
    const second = { ...session, sessionId: 'sess-2', title: 'Second session' }
    const gate = deferred<CliImportResult>()
    const scan = deferred<ImportableCliSession[]>()
    vi.mocked(chatApi.listImportableCliSessions)
      .mockResolvedValueOnce([session, second])
      .mockReturnValueOnce(scan.promise)
    const importSessions = vi.spyOn(chatApi, 'importCliSessions')
      .mockReturnValueOnce(gate.promise)
      .mockResolvedValue({ success: true, imported: [], failures: [] })
    render(<Harness onImported={vi.fn()} />)
    await chooseSession()
    fireEvent.click(screen.getByRole('button', { name: /Second session/ }))
    fireEvent.click(screen.getByRole('button', { name: '导入 2' }))
    await act(async () => {
      gate.resolve({
        success: false,
        imported: [{ agentId: 'claude', sessionId: 'sess-1', conversationId: 'conv-part' }],
        failures: [{ agentId: 'claude', sessionId: 'sess-2', error: 'missing' }],
      })
      await gate.promise
    })
    const retry = screen.getByRole('button', { name: '导入 1' })
    expect(retry).toBeDisabled()
    fireEvent.click(retry)
    expect(importSessions).toHaveBeenCalledTimes(1)
    await act(async () => {
      scan.resolve([{ ...session, alreadyImported: true }, second])
      await scan.promise
    })
    await waitFor(() => expect(retry).toBeEnabled())
    fireEvent.click(retry)
    await waitFor(() => expect(importSessions).toHaveBeenCalledTimes(2))
    expect(importSessions.mock.calls[1][1]).toEqual([{ agentId: 'claude', sessionId: 'sess-2' }])
  })

  it('ignores a scan captured before import bindings when a newer scan already settled', async () => {
    const second = { ...session, sessionId: 'sess-2', title: 'Second session' }
    const gate = deferred<CliImportResult>()
    const oldScan = deferred<ImportableCliSession[]>()
    const currentScan = deferred<ImportableCliSession[]>()
    vi.mocked(chatApi.listImportableCliSessions)
      .mockResolvedValueOnce([session, second])
      .mockReturnValueOnce(oldScan.promise)
      .mockReturnValueOnce(currentScan.promise)
    vi.spyOn(chatApi, 'importCliSessions').mockReturnValue(gate.promise)
    const view = render(<Harness onImported={vi.fn()} />)
    await chooseSession()
    fireEvent.click(screen.getByRole('button', { name: /Second session/ }))
    fireEvent.click(screen.getByRole('button', { name: '导入 2' }))
    await act(async () => { await Promise.resolve() })
    view.unmount()
    render(<Harness onImported={vi.fn()} />)
    await act(async () => {
      gate.resolve({
        success: false,
        imported: [{ agentId: 'claude', sessionId: 'sess-1', conversationId: 'conv-part' }],
        failures: [{ agentId: 'claude', sessionId: 'sess-2', error: 'missing' }],
      })
      await gate.promise
    })
    await act(async () => {
      currentScan.resolve([{ ...session, alreadyImported: true }, second])
      await currentScan.promise
    })
    await act(async () => { oldScan.resolve([session, second]); await oldScan.promise })
    fireEvent.click(screen.getByRole('button', { name: /原生会话/ }))
    expect(screen.getByRole('button', { name: '导入 1' })).toBeEnabled()
    expect(screen.queryByRole('button', { name: '导入 2' })).not.toBeInTheDocument()
  })
})
