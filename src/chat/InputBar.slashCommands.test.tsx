import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeAll, describe, expect, it, vi } from 'vitest'
import { InputBar } from './InputBar'
import { getComposerDraft } from './composerDraft'

vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }))
vi.mock('@tauri-apps/api/webview', () => ({ getCurrentWebview: () => ({ onDragDropEvent: () => Promise.resolve(() => {}) }) }))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ onFocusChanged: () => Promise.resolve(() => {}) }) }))
vi.mock('../api/tauri', () => ({ api: {}, isTauriRuntime: () => false }))
vi.mock('./api', () => ({ chatApi: {
  getProjects: () => Promise.resolve([]),
  listExternalCliSlashCommands: () => Promise.resolve({ commands: [{ name: 'compact', slash: '/compact' }] }),
} }))

beforeAll(() => {
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList
  Range.prototype.getBoundingClientRect = () => new DOMRect()
})
function paste(text: string) {
  fireEvent.paste(screen.getByRole('textbox'), { clipboardData: { files: [], getData: (type: string) => type === 'text/plain' ? text : '' } })
}

describe('InputBar with real command editor', () => {
  it('opens slash search after Chinese, inserts the chosen skill and sends the whole task', async () => {
    const send = vi.fn().mockResolvedValue(true)
    render(<InputBar onSend={send} conversationId="inline-skill" enabledSkills={[{ id: 'review', name: 'Review', description: 'Review code' }]} />)
    paste('请使用/rev')
    fireEvent.click(await screen.findByText('/review'))
    expect(screen.getByRole('textbox').querySelector('[data-command="skill:review"]')).not.toBeNull()
    paste('检查代码')
    fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter' })
    await waitFor(() => expect(send).toHaveBeenCalledWith('请使用/review 检查代码', [], expect.anything()))
  })

  it('stages actions as chips, then executes the action while retaining surrounding text', async () => {
    const settings = vi.fn()
    const send = vi.fn()
    render(<InputBar onSend={send} onOpenSettings={settings} conversationId="inline-settings" />)
    paste('前文 /sett')
    fireEvent.click(await screen.findByText('/settings'))
    expect(screen.getByRole('textbox').querySelector('[data-command="settings"]')).not.toBeNull()
    expect(settings).not.toHaveBeenCalled()
    paste('后文')
    fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter' })
    expect(settings).toHaveBeenCalledOnce()
    expect(send).not.toHaveBeenCalled()
    expect(getComposerDraft('inline-settings')?.input).toBe('前文  后文')
  })

  it('restores command chips when returning to a draft without leaking undo history', async () => {
    const props = { onSend: vi.fn() }
    const { rerender } = render(<InputBar {...props} conversationId="inline-draft-a" />)
    paste('请先 /plan 再写任务')
    rerender(<InputBar {...props} conversationId="inline-draft-b" />)
    expect(screen.getByRole('textbox')).toHaveTextContent('')
    await act(async () => fireEvent.keyDown(screen.getByRole('textbox'), { key: 'z', ctrlKey: true }))
    expect(getComposerDraft('inline-draft-b')).toBeUndefined()
    rerender(<InputBar {...props} conversationId="inline-draft-a" />)
    expect(screen.getByRole('textbox').querySelector('[data-command="plan"]')).not.toBeNull()
    expect(getComposerDraft('inline-draft-a')?.input).toBe('请先 /plan 再写任务')
  })

  it('does not send or select a command while confirming Chinese composition', () => {
    const send = vi.fn()
    render(<InputBar onSend={send} conversationId="inline-ime" />)
    paste('请用/pl')
    fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter', isComposing: true, keyCode: 229 })
    expect(send).not.toHaveBeenCalled()
    expect(screen.getByRole('textbox').querySelector('[data-command]')).toBeNull()
  })

  it('uses the external agent logo and sends the original inline text', async () => {
    const send = vi.fn().mockResolvedValue(true)
    render(<InputBar onSend={send} conversationId="inline-cli" usesExternalRuntime externalAgentName="claude" />)
    paste('请执行/comp')
    fireEvent.click(await screen.findByText('/compact'))
    const chip = screen.getByRole('textbox').querySelector('[data-command="cli:claude:compact"]')
    expect(chip?.querySelector('img')).toHaveAttribute('src', '/agent-icons/claude.svg')
    fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter' })
    await waitFor(() => expect(send).toHaveBeenCalledWith('请执行/compact', [], expect.anything()))
  })

  it('restores a failed action to its original draft even after navigation', async () => {
    let reject!: (error: Error) => void
    const compact = vi.fn(() => new Promise<void>((_resolve, fail) => { reject = fail }))
    const props = { onSend: vi.fn(), onCompactContext: compact }
    const { rerender } = render(<InputBar {...props} conversationId="action-origin" />)
    paste('前文 /compact 后文')
    fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter' })
    expect(compact).toHaveBeenCalledOnce()
    rerender(<InputBar {...props} conversationId="action-other" />)
    await act(async () => reject(new Error('compaction failed')))
    expect(getComposerDraft('action-origin')?.input).toBe('前文 /compact 后文')
    expect(getComposerDraft('action-other')).toBeUndefined()
  })
})
