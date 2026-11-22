import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ConversationTitlebarControls } from './ConversationTitlebarControls'
import { chatApi } from './api'

vi.mock('./api', () => ({ chatApi: { getBranches: vi.fn() } }))

vi.mock('./AgentModelPicker', () => ({
  AgentModelPicker: ({ agentRuntime, locked }: { agentRuntime: { kind: string }; locked: boolean }) => (
    <div data-testid="agent-model-picker" data-kind={agentRuntime.kind} data-locked={String(locked)} />
  ),
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
  it('keeps history navigation available for the Chat runtime without a workspace panel', async () => {
    vi.mocked(chatApi.getBranches).mockResolvedValue([
      { id: 'b1', label: 'Original', active: true },
      { id: 'b2', label: 'Earlier answer', active: false },
    ])
    const onSwitch = vi.fn().mockResolvedValue(true)
    const { queryByTestId } = render(<ConversationTitlebarControls {...base}
      activeAgentRuntime={{ kind: 'chat' }}
      usesExternalRuntime={false} usesChatRuntime
      historyBranches={{ disabled: false, onSwitch }} />)
    fireEvent.click(await screen.findByRole('button', { name: '历史分支' }))
    fireEvent.click(screen.getByRole('menuitemradio', { name: 'Earlier answer' }))
    expect(onSwitch).toHaveBeenCalledWith('b2')
    expect(queryByTestId('workspace-panel')).toBeNull()
  })

  it('shows the workspace panel for agent chats with or without a project', () => {
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
    expect(queryByTestId('workspace-panel')).not.toBeNull()
    rerender(<ConversationTitlebarControls {...props} conversationId={null} projectName={null} workdir="" />)
    expect(queryByTestId('workspace-panel')).not.toBeNull()
  })

  it('renders one combined agent, model and effort picker for every runtime', () => {
    const { getByTestId, rerender } = render(
      <ConversationTitlebarControls
        {...base}
        activeAgentRuntime={{ kind: 'external', externalAgentId: 'claude' }}
        usesExternalRuntime
        runtimeLocked
      />,
    )
    expect(getByTestId('agent-model-picker')).toHaveAttribute('data-kind', 'external')
    expect(getByTestId('agent-model-picker')).toHaveAttribute('data-locked', 'true')
    rerender(<ConversationTitlebarControls {...base} activeAgentRuntime={{ kind: 'builtin' }} usesExternalRuntime={false} />)
    expect(screen.getAllByTestId('agent-model-picker')).toHaveLength(1)
    expect(getByTestId('agent-model-picker')).toHaveAttribute('data-kind', 'builtin')
    expect(screen.queryByRole('button', { name: '面板' })).toBeNull()
  })

  it('keeps task and history branch entries out of the titlebar', () => {
    const { container, queryByRole } = render(
      <ConversationTitlebarControls {...base} activeAgentRuntime={{ kind: 'builtin' }} usesExternalRuntime={false} />,
    )
    expect(container.querySelector('.lucide-list-checks')).toBeNull()
    expect(queryByRole('button', { name: '历史分支' })).toBeNull()
  })
})
