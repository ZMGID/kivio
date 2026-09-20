import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { InputBar } from './InputBar'

const clipboard = vi.hoisted(() => ({ read: vi.fn(), readText: vi.fn(), writeText: vi.fn() }))
const api = vi.hoisted(() => ({
  chatReadClipboardFiles: vi.fn(), chatSavePastedImage: vi.fn(),
  chatReadClipboard: vi.fn(), chatWriteClipboardText: vi.fn(),
  chatInspectAttachmentPaths: vi.fn(),
}))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }))
vi.mock('@tauri-apps/api/webview', () => ({ getCurrentWebview: () => ({ onDragDropEvent: () => Promise.resolve(() => {}) }) }))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ onFocusChanged: () => Promise.resolve(() => {}) }) }))
vi.mock('../api/tauri', () => ({ api, isTauriRuntime: () => true }))
vi.mock('./utils', async original => ({ ...await original<typeof import('./utils')>(), isTauriRuntime: () => true }))
vi.mock('./api', () => ({ chatApi: { getProjects: () => Promise.resolve([]), listExternalCliSlashCommands: () => Promise.resolve({ commands: [] }) } }))
vi.mock('./ChatAttachments', () => ({ ChatAttachments: ({ attachments }: { attachments: { name: string }[] }) => <div>{attachments.map(a => <span key={a.name}>{a.name}</span>)}</div> }))

class TestTransfer {
  files: File[] = []
  text = ''
  items = { add: (file: File) => { this.files.push(file) } }
  setData(_type: string, value: string) { this.text = value }
  getData() { return this.text }
}
beforeEach(() => {
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
