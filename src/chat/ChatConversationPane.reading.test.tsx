import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeAll, describe, expect, it, vi } from 'vitest'
import { ChatConversationPane, type ChatConversationPaneProps } from './ChatConversationPane'

vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }))
vi.mock('@tauri-apps/api/webview', () => ({ getCurrentWebview: () => ({ onDragDropEvent: () => Promise.resolve(() => {}) }) }))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ onFocusChanged: () => Promise.resolve(() => {}) }) }))
vi.mock('../api/tauri', () => ({ api: {}, isTauriRuntime: () => false }))
vi.mock('./api', () => ({ chatApi: { getProjects: () => Promise.resolve([]) } }))
vi.mock('./MessageList', () => ({ MessageList: ({ presentation }: { presentation?: string }) => <div data-testid="shared-message-list">{presentation}</div> }))

beforeAll(() => {
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList
  Range.prototype.getBoundingClientRect = () => new DOMRect()
})
const props = (): ChatConversationPaneProps => ({
  titlebarControls: <span>Ordinary Chat controls</span>, usesNativeTitlebar: true, sidebarCollapsed: false,
  titlebarRowClass: '', titlebarMacInsetClass: '', onToggleSidebar: vi.fn(), onNewConversation: vi.fn(),
  protocolVersionMismatch: false, showEmptyHero: true, currentAssistantName: null,
  selectedProjectName: null, selectedSetName: null,
  inputBarProps: { onSend: vi.fn().mockResolvedValue(false), conversationId: 'reading-shared-pane', modelSlot: <button>Model choice</button> },
  messageListProps: { conversationId: 'reading-shared-pane', messages: [] },
  hookWarning: null, currentConversationId: 'reading-shared-pane', onDismissHookWarning: vi.fn(),
  forkOrigin: null, onSelectConversation: vi.fn(), importedHistoryStale: false, pendingSlot: null,
  queuedMessages: [], canSteerQueuedMessages: false, onSteerQueuedMessage: vi.fn(), onRemoveQueuedMessage: vi.fn(),
  onRestoreQueuedMessage: vi.fn(), lang: 'en', imageViewerItem: null, onCloseImageViewer: vi.fn(), onRender: vi.fn(),
})

describe('shared ChatConversationPane reading layout', () => {
  it('keeps one real composer mounted from empty to populated reading history', async () => {
    const initial = props()
    const view = render(<ChatConversationPane {...initial} presentation="reading" emptyStateSlot={<span>Ask about the selected page</span>} />)
    const editor = screen.getByRole('textbox')
    expect(screen.getByText('Ask about the selected page')).toBeVisible()
    expect(screen.queryByText('Ordinary Chat controls')).toBeNull()
    expect(screen.getByRole('button', { name: 'Model choice' })).toBeVisible()
    expect(await screen.findByTestId('shared-message-list')).toHaveTextContent('reading')
    fireEvent.paste(editor, { clipboardData: { files: [], getData: (kind: string) => kind === 'text/plain' ? 'Preserved question' : '' } })
    view.rerender(<ChatConversationPane {...initial} presentation="reading" showEmptyHero={false}
      messageListProps={{ ...initial.messageListProps, messages: [{ id: 'question', role: 'user', timestamp: 1, content: 'Earlier question' }] }} />)
    expect(screen.getAllByRole('textbox')).toHaveLength(1)
    expect(screen.getByRole('textbox')).toBe(editor)
    expect(editor).toHaveTextContent('Preserved question')
    expect(screen.queryByText('Ask about the selected page')).toBeNull()
  })

  it('keeps ordinary Chat titlebar and the shared chat presentation by default', async () => {
    const { container } = render(<ChatConversationPane {...props()} showEmptyHero={false} />)
    expect(screen.getByText('Ordinary Chat controls')).toBeVisible()
    await waitFor(() => expect(screen.getByTestId('shared-message-list')).toHaveTextContent('chat'))
    expect(container.querySelector('[data-composer-presentation="chat"]')).not.toBeNull()
    expect(screen.queryByRole('button', { name: 'Model choice' })).toBeNull()
    expect(screen.getAllByRole('button').length).toBeGreaterThan(1)
  })
})
