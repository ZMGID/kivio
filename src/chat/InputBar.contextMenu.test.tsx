import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { StrictMode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { InputBar } from './InputBar'
import { draftKey, getComposerDraft, migrateNewChatDraft, setComposerDraft } from './composerDraft'

const clipboard = vi.hoisted(() => ({ read: vi.fn(), readText: vi.fn(), writeText: vi.fn() }))
const openDialog = vi.hoisted(() => vi.fn())
const dragEvents = vi.hoisted(() => ({ handler: null as null | ((event: { payload: { type: string; paths: string[] } }) => void) }))
const api = vi.hoisted(() => ({
  chatReadClipboardFiles: vi.fn(), chatSavePastedImage: vi.fn(),
  chatReadClipboard: vi.fn(), chatWriteClipboardText: vi.fn(),
  chatInspectAttachmentPaths: vi.fn(),
}))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: openDialog }))
vi.mock('@tauri-apps/api/webview', () => ({ getCurrentWebview: () => ({ onDragDropEvent: (handler: typeof dragEvents.handler) => { dragEvents.handler = handler; return Promise.resolve(() => { dragEvents.handler = null }) } }) }))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ onFocusChanged: () => Promise.resolve(() => {}) }) }))
vi.mock('../api/tauri', () => ({ api, isTauriRuntime: () => true }))
vi.mock('./utils', async original => ({ ...await original<typeof import('./utils')>(), isTauriRuntime: () => true }))
vi.mock('./api', () => ({ chatApi: { getProjects: () => Promise.resolve([]), listExternalCliSlashCommands: () => Promise.resolve({ commands: [] }) } }))
vi.mock('./ChatAttachments', () => ({ ChatAttachments: ({ attachments, onRemove }: { attachments: { id: string; name: string }[]; onRemove?: (id: string) => void }) => <div>{attachments.map(a => <div key={a.id}><span>{a.name}</span><button type="button" aria-label={`remove ${a.name}`} onClick={() => onRemove?.(a.id)}>×</button></div>)}</div> }))

class TestTransfer {
  files: File[] = []
  text = ''
  html = ''
  items = { add: (file: File) => { this.files.push(file) } }
  setData(type: string, value: string) { if (type === 'text/html') this.html = value; else this.text = value }
  getData(type: string) { return type === 'text/html' ? this.html : this.text }
}
beforeEach(() => {
  dragEvents.handler = null
  openDialog.mockReset()
  vi.stubGlobal('DataTransfer', TestTransfer)
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: clipboard })
  clipboard.read.mockReset().mockRejectedValue(new Error('Web clipboard must not be used'))
  clipboard.readText.mockReset().mockRejectedValue(new Error('Web clipboard must not be used'))
  clipboard.writeText.mockReset().mockRejectedValue(new Error('Web clipboard must not be used'))
  api.chatReadClipboard.mockReset().mockResolvedValue({ kind: 'text', text: '插入' })
  api.chatWriteClipboardText.mockReset().mockResolvedValue(undefined)
  api.chatReadClipboardFiles.mockReset().mockResolvedValue({ success: true, files: [] })
  api.chatInspectAttachmentPaths.mockReset().mockImplementation(async (paths: string[]) =>
    paths.map((path) => ({
      path,
      name: path.replace(/\\/g, '/').split('/').filter(Boolean).pop() ?? path,
      type: 'file' as const,
    })),
  )
  api.chatSavePastedImage.mockReset().mockResolvedValue({ success: true, path: '/tmp/pasted.png', name: 'pasted.png' })
})
afterEach(() => {
  expect(clipboard.read).not.toHaveBeenCalled()
  expect(clipboard.readText).not.toHaveBeenCalled()
  expect(clipboard.writeText).not.toHaveBeenCalled()
  vi.unstubAllGlobals(); vi.restoreAllMocks()
})

function openMenu(value = '前面选中后面', start = 2, end = 4) {
  const textarea = screen.getByRole('textbox') as HTMLTextAreaElement
  fireEvent.change(textarea, { target: { value } })
  textarea.setSelectionRange(start, end)
  fireEvent.contextMenu(textarea, { clientX: 100, clientY: 100 })
  return textarea
}

describe('composer custom editing menu', () => {
  it('keeps a pending new-chat attachment when navigating to an existing empty conversation', async () => {
    setComposerDraft(draftKey(undefined), { input: '', quotes: [], attachments: [] })
    let resolve!: (value: { success: true; files: { path: string }[] }) => void
    api.chatReadClipboardFiles.mockReturnValue(new Promise(r => { resolve = r }))
    const view = render(<InputBar onSend={() => {}} />)
    fireEvent.paste(screen.getByRole('textbox'), { clipboardData: new TestTransfer() })
    view.rerender(<InputBar onSend={() => {}} conversationId="existing-empty" />)
    await act(async () => resolve({ success: true, files: [{ path: '/tmp/new-only.csv' }] }))
    expect(screen.queryByText('new-only.csv')).toBeNull()
    expect(getComposerDraft('existing-empty')).toBeUndefined()
    expect(getComposerDraft(draftKey(undefined))?.attachments[0]?.name).toBe('new-only.csv')
    setComposerDraft(draftKey(undefined), { input: '', quotes: [], attachments: [] })
  })
  it('shows an async attachment result while mounted under StrictMode', async () => {
    let resolve!: (paths: string[]) => void
    openDialog.mockReturnValue(new Promise(r => { resolve = r }))
    render(<StrictMode><InputBar onSend={() => {}} conversationId="strict-attachment" /></StrictMode>)
    fireEvent.click(screen.getByRole('button', { name: '添加' }))
    fireEvent.click(screen.getByRole('button', { name: '添加附件' }))
    await act(async () => resolve(['/tmp/strict.csv']))
    expect(await screen.findByText('strict.csv')).toBeInTheDocument()
  })

  it('keeps a delayed dropped file with its original draft', async () => {
    let resolve!: (items: { path: string; name: string; type: 'file' }[]) => void
    api.chatInspectAttachmentPaths.mockReturnValue(new Promise(r => { resolve = r }))
    const view = render(<InputBar onSend={() => {}} conversationId="drop-origin-a" />)
    await waitFor(() => expect(dragEvents.handler).not.toBeNull())
    act(() => dragEvents.handler?.({ payload: { type: 'drop', paths: ['/tmp/dropped.csv'] } }))
    view.rerender(<InputBar onSend={() => {}} conversationId="drop-origin-b" />)
    await act(async () => resolve([{ path: '/tmp/dropped.csv', name: 'dropped.csv', type: 'file' }]))
    expect(getComposerDraft('drop-origin-a')?.attachments[0]?.name).toBe('dropped.csv')
    expect(screen.queryByText('dropped.csv')).toBeNull()
  })

  it('keeps a delayed file picker result with its original draft', async () => {
    let resolve!: (paths: string[]) => void
    openDialog.mockReturnValue(new Promise(r => { resolve = r }))
    const view = render(<InputBar onSend={() => {}} conversationId="picker-origin-a" />)
    fireEvent.click(screen.getByRole('button', { name: '添加' }))
    fireEvent.click(screen.getByRole('button', { name: '添加附件' }))
    await waitFor(() => expect(openDialog).toHaveBeenCalled())
    view.rerender(<InputBar onSend={() => {}} conversationId="picker-origin-b" />)
    await act(async () => resolve(['/tmp/picked.csv']))
    expect(getComposerDraft('picker-origin-a')?.attachments[0]?.name).toBe('picked.csv')
    expect(screen.queryByText('picked.csv')).toBeNull()
  })

  it('keeps a delayed pasted attachment with the draft where paste started', async () => {
    let resolve!: (value: { success: true; files: { path: string }[] }) => void
    api.chatReadClipboardFiles.mockReturnValue(new Promise(r => { resolve = r }))
    const view = render(<InputBar onSend={() => {}} conversationId="paste-origin-a" />)
    fireEvent.paste(screen.getByRole('textbox'), { clipboardData: new TestTransfer() })
    await waitFor(() => expect(api.chatReadClipboardFiles).toHaveBeenCalled())
    view.rerender(<InputBar onSend={() => {}} conversationId="paste-origin-b" />)
    await act(async () => resolve({ success: true, files: [{ path: '/tmp/late.csv' }] }))
    await waitFor(() => expect(getComposerDraft('paste-origin-a')?.attachments[0]?.name).toBe('late.csv'))
    expect(getComposerDraft('paste-origin-b')?.attachments).toEqual(undefined)
    expect(screen.queryByText('late.csv')).toBeNull()
    view.rerender(<InputBar onSend={() => {}} conversationId="paste-origin-a" />)
    expect(await screen.findByText('late.csv')).toBeInTheDocument()
  })

  it('does not restore an attachment removed while an older paste of its path is pending', async () => {
    setComposerDraft('paste-removed-late', { input: '', quotes: [], attachments: [
      { id: 'old-attachment', name: 'same.csv', path: '/tmp/same.csv', type: 'file' },
    ] })
    let resolve!: (value: { success: true; files: { path: string }[] }) => void
    api.chatReadClipboardFiles.mockReturnValue(new Promise(r => { resolve = r }))
    render(<InputBar onSend={() => {}} conversationId="paste-removed-late" />)
    fireEvent.paste(screen.getByRole('textbox'), { clipboardData: new TestTransfer() })
    await waitFor(() => expect(api.chatReadClipboardFiles).toHaveBeenCalled())
    fireEvent.click(screen.getByRole('button', { name: 'remove same.csv' }))
    await act(async () => resolve({ success: true, files: [{ path: '/tmp/same.csv' }] }))
    expect(getComposerDraft('paste-removed-late')?.attachments).toEqual(undefined)
    expect(screen.queryByText('same.csv')).toBeNull()
  })

  it('migrates a pending attachment operation with a new conversation draft', async () => {
    setComposerDraft(draftKey(undefined), { input: '', quotes: [], attachments: [] })
    let resolve!: (value: { success: true; files: { path: string }[] }) => void
    api.chatReadClipboardFiles.mockReturnValue(new Promise(r => { resolve = r }))
    const view = render(<InputBar onSend={() => {}} />)
    fireEvent.paste(screen.getByRole('textbox'), { clipboardData: new TestTransfer() })
    await waitFor(() => expect(api.chatReadClipboardFiles).toHaveBeenCalled())
    migrateNewChatDraft(draftKey(undefined), 'paste-new-created')
    view.rerender(<InputBar onSend={() => {}} conversationId="paste-new-created" />)
    await act(async () => resolve({ success: true, files: [{ path: '/tmp/created.csv' }] }))
    expect(await screen.findByText('created.csv')).toBeInTheDocument()
    expect(getComposerDraft('paste-new-created')?.attachments[0]?.name).toBe('created.csv')
    expect(getComposerDraft(draftKey(undefined))).toBeUndefined()
  })

  it('shows a migrated attachment that finishes after the welcome composer remounts', async () => {
    setComposerDraft(draftKey(undefined), { input: '', quotes: [], attachments: [] })
    let resolve!: (value: { success: true; files: { path: string }[] }) => void
    api.chatReadClipboardFiles.mockReturnValue(new Promise(r => { resolve = r }))
    const welcome = render(<InputBar onSend={() => {}} layout="inline" />)
    fireEvent.paste(screen.getByRole('textbox'), { clipboardData: new TestTransfer() })
    act(() => { migrateNewChatDraft(draftKey(undefined), 'paste-remounted') })
    welcome.unmount()
    render(<InputBar onSend={() => {}} conversationId="paste-remounted" />)
    await act(async () => resolve({ success: true, files: [{ path: '/tmp/remounted.csv' }] }))
    expect(await screen.findByText('remounted.csv')).toBeInTheDocument()
    expect(getComposerDraft('paste-remounted')?.attachments[0]?.name).toBe('remounted.csv')
  })

  it('rejects a removed attachment after draft migration and composer remount', async () => {
    setComposerDraft(draftKey(undefined), { input: '', quotes: [], attachments: [
      { id: 'old-remounted', name: 'same-remounted.csv', path: '/tmp/same-remounted.csv', type: 'file' },
    ] })
    let resolve!: (value: { success: true; files: { path: string }[] }) => void
    api.chatReadClipboardFiles.mockReturnValue(new Promise(r => { resolve = r }))
    const welcome = render(<InputBar onSend={() => {}} layout="inline" />)
    fireEvent.paste(screen.getByRole('textbox'), { clipboardData: new TestTransfer() })
    await waitFor(() => expect(api.chatReadClipboardFiles).toHaveBeenCalled())
    act(() => { migrateNewChatDraft(draftKey(undefined), 'removed-after-remount') })
    welcome.unmount()
    render(<InputBar onSend={() => {}} conversationId="removed-after-remount" />)
    fireEvent.click(screen.getByRole('button', { name: 'remove same-remounted.csv' }))
    expect(getComposerDraft('removed-after-remount')?.attachments ?? []).toEqual([])
    await act(async () => resolve({ success: true, files: [{ path: '/tmp/same-remounted.csv' }] }))
    expect(getComposerDraft('removed-after-remount')?.attachments ?? []).toEqual([])
    expect(screen.queryByText('same-remounted.csv')).toBeNull()
    // A new explicit paste after removal is a new intent, not an old result.
    fireEvent.paste(screen.getByRole('textbox'), { clipboardData: new TestTransfer() })
    expect(await screen.findByText('same-remounted.csv')).toBeInTheDocument()
  })

  it('prefers spreadsheet cells over a clipboard image', async () => {
    const view = render(<InputBar onSend={() => {}} conversationId="paste-sheet" />)
    const transfer = new TestTransfer()
    transfer.text = 'A\tB\n1\t2'
    transfer.files.push(new File(['png'], 'sheet.png', { type: 'image/png' }))
    fireEvent.paste(screen.getByRole('textbox'), { clipboardData: transfer })
    await act(async () => {})
    expect(api.chatReadClipboardFiles).not.toHaveBeenCalled()
    expect(api.chatSavePastedImage).not.toHaveBeenCalled()
    expect(getComposerDraft('paste-sheet')?.attachments).toEqual(undefined)
    view.unmount()
  })

  it.each([
    ['A\tB', '', 'beA\tBre'],
    ['A', '<table><tr><td>A</td></tr></table>', 'beAre'],
  ])('prefers native menu spreadsheet text %s over its image', async (text, html, expected) => {
    api.chatReadClipboard.mockResolvedValue({ kind: 'image', dataBase64: 'aW1hZ2U=', text, html })
    render(<InputBar onSend={() => {}} conversationId="menu-sheet" />)
    const textarea = openMenu('before')
    fireEvent.click(screen.getByRole('menuitem', { name: '粘贴' }))
    await waitFor(() => expect(textarea).toHaveValue(expected))
    expect(api.chatSavePastedImage).not.toHaveBeenCalled()
  })

  it('keeps an image when HTML only contains a table-like word', async () => {
    api.chatReadClipboard.mockResolvedValue({ kind: 'image', dataBase64: 'aW1hZ2U=', text: 'A', html: '<tableau>' })
    render(<InputBar onSend={() => {}} conversationId="menu-non-table" />)
    const textarea = openMenu('before')
    fireEvent.click(screen.getByRole('menuitem', { name: '粘贴' }))
    await screen.findByText('pasted.png')
    expect(textarea).toHaveValue('before')
  })

  it('copies only the selection and cuts it after clipboard success', async () => {
    render(<InputBar onSend={() => {}} conversationId="menu-cut" />)
    const textarea = openMenu()
    fireEvent.click(screen.getByRole('menuitem', { name: '复制' }))
    await waitFor(() => expect(api.chatWriteClipboardText).toHaveBeenCalledWith('选中'))
    expect(textarea).toHaveValue('前面选中后面')
    openMenu()
    fireEvent.click(screen.getByRole('menuitem', { name: '剪切' }))
    await waitFor(() => expect(textarea).toHaveValue('前面后面'))
    expect(textarea.selectionStart).toBe(2)
  })
  it('replaces the selected text on paste and restores the caret', async () => {
    render(<InputBar onSend={() => {}} conversationId="menu-paste" />)
    const textarea = openMenu()
    fireEvent.click(screen.getByRole('menuitem', { name: '粘贴' }))
    await waitFor(() => expect(textarea).toHaveValue('前面插入后面'))
    expect(textarea.selectionStart).toBe(4)
  })
  it('does not delete text if copying for cut fails', async () => {
    api.chatWriteClipboardText.mockRejectedValue(new Error('denied'))
    render(<InputBar onSend={() => {}} conversationId="menu-cut-failed" />)
    const textarea = openMenu()
    fireEvent.click(screen.getByRole('menuitem', { name: '剪切' }))
    await screen.findByText('无法写入剪贴板，请重试。')
    expect(textarea).toHaveValue('前面选中后面')
  })
  it('selects all and closes on Escape without cancelling generation', () => {
    const cancel = vi.fn()
    render(<InputBar onSend={() => {}} onCancel={cancel} conversationId="menu-select" />)
    const textarea = openMenu()
    fireEvent.click(screen.getByRole('menuitem', { name: '全选' }))
    expect(textarea.selectionStart).toBe(0)
    expect(textarea.selectionEnd).toBe(textarea.value.length)
    openMenu()
    fireEvent.keyDown(screen.getByRole('menu'), { key: 'Escape' })
    expect(screen.queryByRole('menu')).toBeNull()
    expect(cancel).not.toHaveBeenCalled()
    expect(textarea).toHaveFocus()
  })
  it('disables editing while locked but keeps copying available', () => {
    render(<InputBar onSend={() => {}} conversationId="menu-locked" disabled />)
    openMenu()
    expect(screen.getByRole('menuitem', { name: '剪切' })).toBeDisabled()
    expect(screen.getByRole('menuitem', { name: '粘贴' })).toBeDisabled()
    expect(screen.getByRole('menuitem', { name: '复制' })).toBeEnabled()
  })
  it('does not paste into another conversation after async clipboard reading', async () => {
    let resolve!: (content: { kind: 'text'; text: string }) => void
    api.chatReadClipboard.mockReturnValue(new Promise<{ kind: 'text'; text: string }>(r => { resolve = r }))
    const view = render(<InputBar onSend={() => {}} conversationId="menu-scope-first" />)
    openMenu()
    fireEvent.click(screen.getByRole('menuitem', { name: '粘贴' }))
    await waitFor(() => expect(api.chatReadClipboard).toHaveBeenCalled())
    view.rerender(<InputBar onSend={() => {}} conversationId="menu-scope-second" />)
    await act(async () => resolve({ kind: 'text', text: '迟到的内容' }))
    expect(screen.getByRole('textbox')).toHaveValue('')
  })
  it('returns a delayed menu file to its original draft', async () => {
    let resolve!: (content: { kind: 'files'; paths: string[] }) => void
    api.chatReadClipboard.mockReturnValueOnce(new Promise(r => { resolve = r }))
    const view = render(<InputBar onSend={() => {}} conversationId="menu-late-a" />)
    openMenu()
    fireEvent.click(screen.getByRole('menuitem', { name: '粘贴' }))
    await waitFor(() => expect(api.chatReadClipboard).toHaveBeenCalled())
    view.rerender(<InputBar onSend={() => {}} conversationId="menu-late-b" />)
    await act(async () => resolve({ kind: 'files', paths: ['/tmp/menu-late.csv'] }))
    expect(getComposerDraft('menu-late-a')?.attachments[0]?.name).toBe('menu-late.csv')
    expect(screen.queryByText('menu-late.csv')).toBeNull()
  })
  it('adds copied system files as attachments without inserting filenames', async () => {
    api.chatReadClipboard.mockResolvedValue({ kind: 'files', paths: ['/tmp/report.csv'] })
    render(<InputBar onSend={() => {}} conversationId="menu-files" />)
    const textarea = openMenu()
    fireEvent.click(screen.getByRole('menuitem', { name: '粘贴' }))
    await screen.findByText('report.csv')
    expect(textarea).toHaveValue('前面选中后面')
    expect(clipboard.readText).not.toHaveBeenCalled()
  })
  it('turns long pasted text into an attachment', async () => {
    api.chatReadClipboard.mockResolvedValue({ kind: 'text', text: '字'.repeat(3001) })
    render(<InputBar onSend={() => {}} conversationId="menu-long-text" />)
    const textarea = openMenu()
    fireEvent.click(screen.getByRole('menuitem', { name: '粘贴' }))
    await screen.findByText(/\.txt$/)
    expect(textarea).toHaveValue('前面选中后面')
  })
  it('saves pasted screenshots through the existing image attachment path', async () => {
    api.chatReadClipboard.mockResolvedValue({ kind: 'image', dataBase64: 'aW1hZ2U=' })
    render(<InputBar onSend={() => {}} conversationId="menu-image" />)
    const textarea = openMenu()
    fireEvent.click(screen.getByRole('menuitem', { name: '粘贴' }))
    await screen.findByText('pasted.png')
    expect(api.chatSavePastedImage).toHaveBeenCalledWith('pasted-image.png', 'image/png', expect.any(String))
    expect(textarea).toHaveValue('前面选中后面')
  })
  it('does not fall back to browser permission requests if OS clipboard access fails', async () => {
    api.chatReadClipboard.mockRejectedValue(new Error('Clipboard is busy'))
    render(<InputBar onSend={() => {}} conversationId="menu-read-failed" />)
    const textarea = openMenu()
    fireEvent.click(screen.getByRole('menuitem', { name: '粘贴' }))
    await screen.findByText('无法读取剪贴板，请重试或使用 Ctrl+V。')
    expect(textarea).toHaveValue('前面选中后面')
  })
})
