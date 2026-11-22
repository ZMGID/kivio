import { describe, expect, it } from 'vitest'
import { collectCompactionRecords, resolveCompactionBoundaries } from './compactionBoundary'
import type { ChatMessage, ConversationContextState } from './types'

const messages: ChatMessage[] = [
  { id: 'm1', role: 'user', content: 'hello', timestamp: 1 },
  { id: 'm2', role: 'assistant', content: 'hi', timestamp: 2 },
  { id: 'm3', role: 'user', content: 'more', timestamp: 3 },
]

describe('compactionBoundary', () => {
  it('anchors the divider at the trigger-time message (display_after_message_id)', () => {
    const contextState: ConversationContextState = {
      compaction_boundaries: [{
        id: 'b1',
        source_until_message_id: 'm1', // context split point (older)
        display_after_message_id: 'm2', // where compaction was triggered
        token_estimate_before: 42000,
        token_estimate_after: 3200,
        summary_content: 'summary text',
        trigger: 'manual',
        created_at: 10,
      }],
    }
    const views = resolveCompactionBoundaries(messages, contextState)
    expect(views).toHaveLength(1)
    expect(views[0]?.afterIndex).toBe(1) // m2, not the split point m1
    expect(views[0]?.record.summary_content).toBe('summary text')
  })

  it('falls back to source_until_message_id for records without a display anchor', () => {
    const contextState: ConversationContextState = {
      compaction_boundaries: [{
        id: 'b1',
        source_until_message_id: 'm2',
        token_estimate_before: 42000,
        token_estimate_after: 3200,
        summary_content: 'summary text',
        trigger: 'manual',
        created_at: 10,
      }],
    }
    const views = resolveCompactionBoundaries(messages, contextState)
    expect(views).toHaveLength(1)
    expect(views[0]?.afterIndex).toBe(1)
  })

  it('degrades to the split point when the display anchor was truncated away', () => {
    // Regenerate/delete can remove the trigger-time anchor (usually the latest
    // assistant message) — the divider must not vanish from the timeline.
    const contextState: ConversationContextState = {
      compaction_boundaries: [{
        id: 'b1',
        source_until_message_id: 'm1',
        display_after_message_id: 'm-deleted',
        token_estimate_before: 42000,
        token_estimate_after: 3200,
        summary_content: 'summary text',
        trigger: 'manual',
        created_at: 10,
      }],
    }
    const views = resolveCompactionBoundaries(messages, contextState)
    expect(views).toHaveLength(1)
    expect(views[0]?.afterIndex).toBe(0) // m1 (split point)
  })

  it('falls back to legacy summary when boundaries array is empty', () => {
    const contextState: ConversationContextState = {
      summary: {
        id: 's1',
        content: 'legacy summary',
        source_until_message_id: 'm1',
        token_estimate_before: 1000,
        token_estimate_after: 200,
        stale: false,
      },
    }
    const records = collectCompactionRecords(contextState)
    expect(records).toHaveLength(1)
    expect(records[0]?.trigger).toBe('auto')
    expect(resolveCompactionBoundaries(messages, contextState)[0]?.afterIndex).toBe(0)
  })

  it('skips stale legacy summary', () => {
    const contextState: ConversationContextState = {
      summary: {
        id: 's1',
        content: 'stale',
        source_until_message_id: 'm1',
        stale: true,
      },
    }
    expect(collectCompactionRecords(contextState)).toHaveLength(0)
  })
})
