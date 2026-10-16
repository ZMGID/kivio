import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeAll, describe, expect, it, vi } from 'vitest'
import { InputBar } from './InputBar'
import { ChatRouteKeepAlive } from './ChatRouteKeepAlive'
import { Check } from 'lucide-react'
import { getComposerDraft, setComposerDraft, subscribeComposerDraft } from './composerDraft'
import { insertTextIntoComposer } from './composerInsert'

vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }))
vi.mock('@tauri-apps/api/webview', () => ({ getCurrentWebview: () => ({ onDragDropEvent: () => Promise.resolve(() => {}) }) }))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ onFocusChanged: () => Promise.resolve(() => {}) }) }))
vi.mock('../api/tauri', () => ({ api: {}, isTauriRuntime: () => false }))
vi.mock('./api', () => ({ chatApi: { getProjects: () => Promise.resolve([]) } }))

beforeAll(() => {
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList
  Range.prototype.getBoundingClientRect = () => new DOMRect()
})
function paste(text: string, textbox = screen.getByRole('textbox')) {
  fireEvent.paste(textbox, { clipboardData: { files: [], getData: (type: string) => type === 'text/plain' ? text : '' } })
}

describe('shared InputBar reading presentation', () => {
  it('releases insertion listeners when a cached reading pane is hidden and restores them on return', () => {
    const pane = <InputBar presentation="reading" conversationId="cached-reading-activity" onSend={vi.fn()} />
    const view = render(<ChatRouteKeepAlive activeKey="study">{pane}</ChatRouteKeepAlive>)
    act(() => insertTextIntoComposer('Visible'))
    expect(getComposerDraft('cached-reading-activity')?.input).toBe('Visible')
    view.rerender(<ChatRouteKeepAlive activeKey="settings"><section>Settings</section></ChatRouteKeepAlive>)
    act(() => insertTextIntoComposer('Hidden insertion must not land'))
    expect(getComposerDraft('cached-reading-activity')?.input).toBe('Visible')
    view.rerender(<ChatRouteKeepAlive activeKey="study">{pane}</ChatRouteKeepAlive>)
    act(() => insertTextIntoComposer('Returned'))
    expect(getComposerDraft('cached-reading-activity')?.input).toBe('Visible Returned')
  })

  it('uses the real editor and slots, with slash questions literal and hidden mode shortcuts disabled', async () => {
    const send = vi.fn().mockResolvedValue(true)
    const clear = vi.fn()
    const mode = vi.fn()
    const { container } = render(<InputBar presentation="reading" conversationId="reading-literal"
      placeholder="Ask about this page" onSend={send} onClearChat={clear}
      readingContextSlot={<span>Page 4 · Explain</span>} modelSlot={<button>Vision model</button>}
      onOpenAssistantCenter={vi.fn()} onOpenTools={vi.fn()} showProjectEntry onSelectProject={vi.fn()}
      onChangeReplyModels={vi.fn()} onChangeKnowledgeBaseIds={vi.fn()} onSetWebSearchMode={vi.fn()}
      modeValue="act" modeOptions={[{ value: 'act', label: 'Act', icon: Check, tone: 'neutral' }, { value: 'plan', label: 'Plan', icon: Check, tone: 'neutral' }]} onModeChange={mode}
      enabledSkills={[{ id: 'review', name: 'Review', description: 'Review code' }]} />)
    const textbox = screen.getByRole('textbox')
    expect(textbox).toHaveAttribute('data-placeholder', 'Ask about this page')
    expect(textbox).toHaveClass('chat-composer-editor')
    expect(container.querySelector('textarea')).toBeNull()
    expect(screen.getByText('Page 4 · Explain')).toBeVisible()
    expect(screen.getByRole('button', { name: 'Vision model' })).toBeVisible()
    expect(screen.getAllByRole('button')).toHaveLength(2)
    paste('/goal explain /clear and /review literally')
    expect(textbox.querySelector('[data-command]')).toBeNull()
    expect(container.querySelector('[data-chat-slash-panel]')).toBeNull()
    fireEvent.keyDown(textbox, { key: 'Tab', shiftKey: true })
    expect(mode).not.toHaveBeenCalled()
    fireEvent.keyDown(textbox, { key: 'Enter' })
    await waitFor(() => expect(send).toHaveBeenCalledWith('/goal explain /clear and /review literally', [], expect.anything()))
    expect(clear).not.toHaveBeenCalled()
    expect(getComposerDraft('reading-literal')).toBeUndefined()
  })

  it('clears an open Chat command menu when the same composer enters reading presentation', async () => {
    const send = vi.fn().mockResolvedValue(true)
    const clear = vi.fn()
    const view = render(<InputBar onSend={send} onClearChat={clear} conversationId="switch-reading-presentation" />)
    paste('/clear')
    expect(view.container.querySelector('[data-chat-slash-panel]')).not.toBeNull()
    view.rerender(<InputBar presentation="reading" onSend={send} onClearChat={clear} conversationId="switch-reading-presentation" />)
    expect(view.container.querySelector('[data-chat-slash-panel]')).toBeNull()
    expect(screen.getByRole('textbox').querySelector('[data-command]')).toBeNull()
    fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter' })
    await waitFor(() => expect(send).toHaveBeenCalledWith('/clear', [], expect.anything()))
    expect(clear).not.toHaveBeenCalled()
  })

  it('keeps long pasted prose as editor text and ignores attachment imports', () => {
    const { container } = render(<InputBar presentation="reading" conversationId="reading-long-paste" onSend={vi.fn()} />)
    const text = 'Reading excerpt '.repeat(250)
    paste(text)
    expect(getComposerDraft('reading-long-paste')?.input).toBe(text)
    expect(getComposerDraft('reading-long-paste')?.attachments).toEqual([])
    const textbox = screen.getByRole('textbox')
    fireEvent.paste(textbox, { clipboardData: { files: [new File(['x'], 'image.png', { type: 'image/png' })], getData: () => '' } })
    expect(getComposerDraft('reading-long-paste')?.attachments).toEqual([])
    expect(container.querySelector('img')).toBeNull()
  })

  it('restores and publishes through the one keyed composer draft store', async () => {
    setComposerDraft('reading-draft-a', { input: 'Restored question', attachments: [], quotes: [] })
    const changed = vi.fn()
    const unsubscribe = subscribeComposerDraft(changed)
    const props = { presentation: 'reading' as const, onSend: vi.fn().mockResolvedValue(false) }
    const view = render(<InputBar {...props} conversationId="reading-draft-a" />)
    expect(screen.getByRole('textbox')).toHaveTextContent('Restored question')
    view.rerender(<InputBar {...props} conversationId="reading-draft-b" />)
    paste('Second source question')
    expect(changed).toHaveBeenCalledWith('reading-draft-b', expect.objectContaining({ input: 'Second source question' }))
    view.rerender(<InputBar {...props} conversationId="reading-draft-a" />)
    expect(screen.getByRole('textbox')).toHaveTextContent('Restored question')
    fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter' })
    await waitFor(() => expect(props.onSend).toHaveBeenCalledOnce())
    expect(getComposerDraft('reading-draft-a')?.input).toBe('Restored question')
    expect(getComposerDraft('reading-draft-b')?.input).toBe('Second source question')
    unsubscribe()
  })

  it('keeps generation cancel on the existing shared send slot and keyboard', () => {
    const cancel = vi.fn()
    render(<InputBar presentation="reading" conversationId="reading-cancel" onSend={vi.fn()} disabled onCancel={cancel} cancelVisible />)
    fireEvent.click(screen.getByRole('button', { name: '停止生成' }))
    fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Escape' })
    expect(cancel).toHaveBeenCalledTimes(2)
  })

  it('lets only the active mounted composer own insertion and requested focus', async () => {
    const props = { presentation: 'reading' as const, onSend: vi.fn() }
    const view = render(<><InputBar {...props} conversationId="reading-visible" active focusRequest={1} /><InputBar {...props} conversationId="reading-hidden" active={false} focusRequest={1} /></>)
    await waitFor(() => expect(document.activeElement).toBe(screen.getAllByRole('textbox')[0]))
    act(() => insertTextIntoComposer('Visible draft'))
    expect(getComposerDraft('reading-visible')?.input).toBe('Visible draft')
    expect(getComposerDraft('reading-hidden')).toBeUndefined()
    view.rerender(<><InputBar {...props} conversationId="reading-visible" active={false} focusRequest={1} /><InputBar {...props} conversationId="reading-hidden" active focusRequest={2} /></>)
    await waitFor(() => expect(document.activeElement).toBe(screen.getAllByRole('textbox')[1]))
    act(() => insertTextIntoComposer('Now active'))
    expect(getComposerDraft('reading-hidden')?.input).toBe('Now active')
    expect(getComposerDraft('reading-visible')?.input).toBe('Visible draft')
  })
})
