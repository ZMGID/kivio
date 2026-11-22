import { act, fireEvent, render, screen } from '@testing-library/react'
import { useState } from 'react'
import { afterEach, expect, it, vi } from 'vitest'
import { api, type ChatContextPayload } from '../../api/tauri'
import { chatApi, normalizeAgentRuntime } from '../api'
import type { Conversation } from '../types'
import { usePopoutComposer } from './usePopoutComposer'

afterEach(() => vi.restoreAllMocks())

it.each([
  { earlySeq: 7, earlyTokens: 92000, expectedPercent: '8' },
  { earlySeq: 9, earlyTokens: 6000, expectedPercent: '6' },
])('opens the popout with the newest CLI measurement when sequence $earlySeq arrives before load', async ({
  earlySeq, earlyTokens, expectedPercent,
}) => {
  const stats = vi.spyOn(chatApi, 'getContextStats').mockImplementation(() => new Promise(() => {}))
  let publish!: (event: ChatContextPayload) => void
  vi.spyOn(api, 'onChatContext').mockImplementation(async listener => {
    publish = listener
    return () => {}
  })
  const saved: Conversation = {
    id: 'pi-chat', revision: 2, title: '', provider_id: '', model: '', messages: [], created_at: 1, updated_at: 2,
    context_state: { context_source: 'external_cli', token_count_source: 'cli_reported',
      estimated_input_tokens: 8000, context_window_tokens: 100000, measurement_seq: 8 },
  }
  let load!: (conversation: Conversation) => void
  function Window() {
    const [conversation, setConversation] = useState<Conversation | null>(null)
    load = setConversation
    const { contextSlot } = usePopoutComposer({
      conversation, setConversation, conversationId: saved.id, lang: 'en', displayMessages: [],
      streaming: false, usesChatRuntime: false, usesExternalRuntime: true,
      runtime: normalizeAgentRuntime({ agent_runtime: { kind: 'external', external_agent_id: 'pi' } }),
      detectedExternalAgents: [], onSend: vi.fn(), onCancel: vi.fn(), cancelVisible: false,
      cancelling: false, disabled: false, workdir: '',
    })
    return <>{contextSlot}</>
  }
  render(<Window />)
  expect(stats).not.toHaveBeenCalled()
  fireEvent.click(screen.getByRole('button', { name: 'Context' }))
  await act(async () => {
    publish({ conversationId: saved.id, live: { usedTokens: earlyTokens, tokenCountSource: 'cli_reported', measurementSeq: earlySeq } })
  })
  await act(async () => load(saved))
  expect(stats).not.toHaveBeenCalled()
  expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', expectedPercent)
  await act(async () => publish({ conversationId: saved.id, live: { usedTokens: 91000,
    tokenCountSource: 'cli_reported', contextWindowTokens: 100000, measurementSeq: 7 } }))
  expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', expectedPercent)
})
