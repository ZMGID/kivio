import { describe, expect, it } from 'vitest'
import cases from '../../tests/fixtures/message-body.json'
import { messageBodySegments, messageBodyText } from './messageBody'
import type { ChatMessageSegment } from './types'
import type { ChatStreamPayload } from '../api/tauri'
import { createEmptyStreamSnapshot } from './conversationRuns'
import { applyStreamDeltaToSnapshot } from './streamApply'

describe('message body compatibility', () => {
  it('does not append the raw delta mirror during a real multi-step stream', () => {
    const snapshot = createEmptyStreamSnapshot()
    for (const [order, text] of ['Question?\n', 'Answer.'].entries()) {
      applyStreamDeltaToSnapshot(snapshot, { type: 'text_delta', delta: text } as ChatStreamPayload,
        { id: `step-${order}`, kind: 'text', phase: 'tool_loop', order })
    }
    expect(messageBodyText({ ...snapshot, role: 'assistant' })).toBe('Question?\n\nAnswer.')
    expect(messageBodySegments(snapshot).filter(s => s.kind === 'text')).toHaveLength(2)
    applyStreamDeltaToSnapshot(snapshot, { type: 'text_delta', delta: ' More.' } as ChatStreamPayload,
      { id: 'step-1', kind: 'text', phase: 'tool_loop', order: 1 })
    expect(messageBodyText({ ...snapshot, role: 'assistant' })).toBe('Question?\n\nAnswer. More.')
  })
  for (const fixture of cases) {
    it(fixture.name, () => {
      const message = { role: 'assistant' as const, content: fixture.content, segments: fixture.segments as ChatMessageSegment[] }
      const before = JSON.stringify(message)
      expect(messageBodyText(message)).toBe(fixture.expected)
      const visible = messageBodySegments(message).filter(s => s.kind === 'text').map(s => s.text?.trim()).filter(Boolean).join('\n\n')
      expect(visible || message.content).toBe(fixture.expected)
      expect(JSON.stringify(message)).toBe(before)
    })
  }
})
