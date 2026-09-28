vi.mock('./ComposerEditor', () => import('./ComposerEditor.testSupport'))
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { InputBar } from './InputBar'
import { draftKey, setComposerDraft } from './composerDraft'

vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }))
vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({ onDragDropEvent: () => Promise.resolve(() => {}) }),
}))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ onFocusChanged: () => Promise.resolve(() => {}) }),
}))
vi.mock('../api/tauri', () => ({ api: {}, isTauriRuntime: () => false }))
vi.mock('./api', () => ({ chatApi: { getProjects: () => Promise.resolve([]) } }))

function videoDraft(id: string) {
  setComposerDraft(draftKey(id), {
    input: '看看这个', quotes: [],
    attachments: [{ id: 'v1', type: 'video', name: 'clip.mp4', path: 'C:/clip.mp4' }],
  })
}

describe('video attachments use the ordinary composer', () => {
  it('does not ask the user to select analysis or add a command', async () => {
    videoDraft('video-normal')
    const onSend = vi.fn()
    render(<InputBar conversationId="video-normal" onSend={onSend} gitLang="zh" />)
    expect(screen.queryByRole('button', { name: /分析.*视频|Analyze.*video/ })).toBeNull()
    expect(screen.queryByText(/混音器分析|随下一条消息|14 MiB/)).toBeNull()
    expect(onSend).not.toHaveBeenCalled()
    fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter' })
    await waitFor(() => expect(onSend).toHaveBeenCalled())
    expect(onSend.mock.calls[0][0]).toBe('看看这个')
    expect(onSend.mock.calls[0][1][0].type).toBe('video')
  })
})
