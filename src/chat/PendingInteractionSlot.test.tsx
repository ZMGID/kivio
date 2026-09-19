import { render } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import type { ChatSessionConsentPayload, ChatUserPromptPayload } from '../api/tauri'
import { PendingInteractionSlot } from './PendingInteractionSlot'
import type { RunInteractionSnapshot } from './runInteractionInbox'
import type { AgentRuntimeConfig } from './types'

vi.mock('./AskUserBlock', () => ({
  AskUserBlock: (props: { toolCall: unknown; onResolved: () => void }) => {
    captured.toolCalls.push(props.toolCall)
    captured.onResolved = props.onResolved
    return <div data-testid="ask-user" />
  },
}))
vi.mock('./ApprovalCard', () => ({
  ApprovalCard: (props: { title: string; actions: Array<{ label: string; onSelect: () => void }> }) => {
    captured.cards.push(props)
    return (
      <div data-testid="approval-card">
        {props.actions.map((action) => (
          <button key={action.label} type="button" onClick={action.onSelect}>{action.label}</button>
        ))}
      </div>
    )
  },
}))

const captured: {
  toolCalls: unknown[]
  onResolved: (() => void) | null
  cards: Array<{ title: string; actions: Array<{ label: string; onSelect: () => void }> }>
} = { toolCalls: [], onResolved: null, cards: [] }

const RUNTIME: AgentRuntimeConfig = { kind: 'external', externalAgentId: 'claude' }

function snapshot(overrides: Partial<RunInteractionSnapshot> = {}): RunInteractionSnapshot {
  return {
    activeConversationId: null,
    toolConfirm: null,
    toolConfirmSubmitting: false,
    toolConfirmError: '',
    sessionConsent: null,
    sessionConsentSubmitting: false,
    sessionConsentError: '',
    userPrompt: null,
    pendingToolConversationIds: [],
    ...overrides,
  }
}

function renderSlot(overrides: Partial<RunInteractionSnapshot> = {}, ports?: {
  onResolveSessionConsent?: (granted: boolean) => Promise<boolean>
  onDismissUserPrompt?: (conversationId: string, runId: string, toolCallId: string) => void
}) {
  captured.toolCalls = []
  captured.onResolved = null
  captured.cards = []
  const onResolveSessionConsent = ports?.onResolveSessionConsent ?? vi.fn(async () => true)
  const onDismissUserPrompt = ports?.onDismissUserPrompt ?? vi.fn()
  const view = render(
    <PendingInteractionSlot
      snapshot={snapshot(overrides)}
      activeAgentRuntime={RUNTIME}
      onResolveToolConfirm={vi.fn(async () => true)}
      onResolveSessionConsent={onResolveSessionConsent}
      onDismissUserPrompt={onDismissUserPrompt}
      onPersistApprovedSandbox={vi.fn(async () => {})}
    />,
  )
  return { view, onResolveSessionConsent, onDismissUserPrompt }
}

describe('PendingInteractionSlot', () => {
  it('renders nothing when the inbox is empty', () => {
    const { view } = renderSlot()
    expect(view.container.firstChild).toBeNull()
  })

  it('session consent: two buttons resolve false / true', async () => {
    const consent: ChatSessionConsentPayload = { conversationId: 'c1', runId: 'r1' }
    const { view, onResolveSessionConsent } = renderSlot({ sessionConsent: consent })
    expect(captured.cards).toHaveLength(1)
    expect(captured.cards[0].title).toContain('文件和命令工具')
    view.getByRole('button', { name: '拒绝' }).click()
    view.getByRole('button', { name: '允许本次会话' }).click()
    expect(onResolveSessionConsent).toHaveBeenNthCalledWith(1, false)
    expect(onResolveSessionConsent).toHaveBeenNthCalledWith(2, true)
  })

  it('keeps the same AskUserBlock toolCall object when the prompt payload is unchanged', () => {
    const userPrompt: ChatUserPromptPayload = {
      conversationId: 'c1',
      runId: 'r1',
      toolCallId: 't1',
      name: 'ask_user',
      source: 'native',
      prompt: { title: 'q', questions: [] },
    }
    const { view, onDismissUserPrompt } = renderSlot({ userPrompt })
    expect(captured.toolCalls).toHaveLength(1)
    const first = captured.toolCalls[0]
    view.rerender(
      <PendingInteractionSlot
        snapshot={snapshot({ userPrompt })}
        activeAgentRuntime={RUNTIME}
        onResolveToolConfirm={vi.fn(async () => true)}
        onResolveSessionConsent={vi.fn(async () => true)}
        onDismissUserPrompt={onDismissUserPrompt}
        onPersistApprovedSandbox={vi.fn(async () => {})}
      />,
    )
    expect(captured.toolCalls).toHaveLength(2)
    expect(captured.toolCalls[1]).toBe(first)
    captured.onResolved?.()
    expect(onDismissUserPrompt).toHaveBeenCalledWith('c1', 'r1', 't1')
  })
})
