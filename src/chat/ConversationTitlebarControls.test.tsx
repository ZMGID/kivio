import { render } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ConversationTitlebarControls } from './ConversationTitlebarControls'

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
vi.mock('./BackgroundJobsIndicator', () => ({
  BackgroundJobsIndicator: () => <div data-testid="jobs" />,
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
  onOpenPopout: vi.fn(),
  onOpenDockTasks: vi.fn(),
  onToggleDock: vi.fn(),
}

describe('ConversationTitlebarControls', () => {
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
})
