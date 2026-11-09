import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ConversationTitlebarControls } from './ConversationTitlebarControls'
import { chatApi } from './api'

vi.mock('./api', () => ({ chatApi: { getBranches: vi.fn() } }))

vi.mock('./RuntimePicker', () => ({
  RuntimePicker: () => <div data-testid="runtime-picker" />,
  ExternalModelSelector: () => <div data-testid="external-model" />,
}))
vi.mock('./ModelSelector', () => ({
  ModelSelector: () => <div data-testid="model-selector" />,
}))
vi.mock('./ThinkingLevelSelector', () => ({
  ThinkingLevelSelector: () => <div data-testid="thinking" />,
}))
vi.mock('./PermissionPicker', () => ({
  PermissionPicker: () => <div data-testid="permission" />,
}))
vi.mock('./dock/WorkspaceActionsMenu', () => ({
  WorkspaceActionsMenu: () => <div data-testid="workspace-panel" />,
}))
const base = {
  conversationId: 'c1',
  runtimeLocked: false,
  usesChatRuntime: false,
  activeProviderId: 'p',
  activeModel: 'm',
  thinkingLevel: null,
  approvalPolicy: 'always_ask',
  dockOpen: false,
  uiLang: 'zh' as const,
  onRuntimeChange: vi.fn(),
  onExternalModelChange: vi.fn(),
  onModelChange: vi.fn(),
  onThinkingLevelChange: vi.fn(),
  onApprovalPolicyChange: vi.fn(),
  onSandboxChange: vi.fn(),
  busy: false,
  onOpenPopout: vi.fn(),
  onToggleDock: vi.fn(),
  workdir: '',
  projectName: null,
  onOpenGitPanel: vi.fn(),
  onOpenTasks: vi.fn(),
}

describe('ConversationTitlebarControls', () => {
  it.each([false, true])('keeps history navigation available without project Git controls (Chat runtime: %s)', async (usesChatRuntime) => {
    vi.mocked(chatApi.getBranches).mockResolvedValue([
      { id: 'b1', label: 'Original', active: true },
      { id: 'b2', label: 'Earlier answer', active: false },
    ])
    const onSwitch = vi.fn().mockResolvedValue(true)
    const { queryByTestId } = render(<ConversationTitlebarControls {...base}
      activeAgentRuntime={{ kind: usesChatRuntime ? 'chat' : 'builtin' }}
      usesExternalRuntime={false} usesChatRuntime={usesChatRuntime}
      historyBranches={{ disabled: false, onSwitch }} />)
    fireEvent.click(await screen.findByRole('button', { name: '历史分支' }))
    fireEvent.click(screen.getByRole('menuitemradio', { name: 'Earlier answer' }))
    expect(onSwitch).toHaveBeenCalledWith('b2')
    expect(queryByTestId('workspace-panel')).toBeNull()
  })

  it('hides the workspace panel when switching to a conversation without a project, even with a working directory', () => {
    const props = {
      ...base,
      activeAgentRuntime: { kind: 'builtin' },
      usesExternalRuntime: false,
      workdir: '/repo',
    } satisfies Parameters<typeof ConversationTitlebarControls>[0]
    const { queryByTestId, rerender } = render(
      <ConversationTitlebarControls {...props} projectName="Repo" />,
    )
    expect(queryByTestId('workspace-panel')).not.toBeNull()

    rerender(<ConversationTitlebarControls {...props} conversationId="no-project" projectName={null} />)
    expect(queryByTestId('workspace-panel')).toBeNull()
  })

  it('renders the external model picker for a CLI runtime', () => {
    const { getByTestId, queryByTestId } = render(
      <ConversationTitlebarControls
        {...base}
        activeAgentRuntime={{ kind: 'external', externalAgentId: 'claude' }}
        usesExternalRuntime
      />,
    )
    expect(getByTestId('external-model')).toBeTruthy()
    expect(queryByTestId('model-selector')).toBeNull()
    expect(queryByTestId('thinking')).toBeNull()
  })

  it('renders the built-in model + thinking selectors otherwise', () => {
    const { getByTestId, queryByTestId } = render(
      <ConversationTitlebarControls
        {...base}
        activeAgentRuntime={{ kind: 'builtin' }}
        usesExternalRuntime={false}
      />,
    )
    expect(getByTestId('model-selector')).toBeTruthy()
    expect(getByTestId('thinking')).toBeTruthy()
    expect(queryByTestId('external-model')).toBeNull()
  })

  it('keeps task and history branch entries out of the titlebar', () => {
    const { container, queryByRole } = render(
      <ConversationTitlebarControls {...base} activeAgentRuntime={{ kind: 'builtin' }} usesExternalRuntime={false} />,
    )
    expect(container.querySelector('.lucide-list-checks')).toBeNull()
    expect(queryByRole('button', { name: '历史分支' })).toBeNull()
  })
})
