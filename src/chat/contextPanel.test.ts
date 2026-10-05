import { describe, expect, it } from 'vitest'
import {
  applyLiveContextUsage,
  autoCompactPercent,
  contextBreakdown,
  keepNewerContextMeasurement,
  mergeContextMeasurement,
  mergeContextSnapshot,
  reportedContextTokens,
} from './contextPanel'
import type { Conversation, ConversationContextState } from './types'

describe('provider-only context meter', () => {
  const previous: ConversationContextState = {
    context_window_tokens: 200_000,
    estimated_input_tokens: 120_000,
    reported_context_tokens: 30_000,
    token_count_source: 'provider_context_reported',
    compression_count: 2,
    segments: [
      { id: 'tool_definitions', label: 'Tools', estimated_tokens: 24_000, chars: 2000 },
      { id: 'conversation', label: 'Messages', estimated_tokens: 100, chars: 6000 },
    ],
  }

  it('never lets a budget estimate or tool breakdown replace reported occupancy', () => {
    expect(reportedContextTokens(previous)).toBe(30_000)
    for (const token_count_source of [undefined, 'estimated', 'provider_reported_with_estimate', 'provider_reported']) {
      expect(reportedContextTokens({ ...previous, token_count_source })).toBeNull()
    }
  })

  it('accepts the first API report before any full snapshot exists', () => {
    const next = applyLiveContextUsage(null, {
      usedTokens: 33_000, tokenCountSource: 'provider_context_reported', contextWindowTokens: 200_000,
    })
    expect(reportedContextTokens(next)).toBe(33_000)
    expect(next.usage_ratio).toBe(0.165)
  })

  it('replaces the report rather than accumulating it or scaling tool definitions', () => {
    const first = applyLiveContextUsage(previous, { usedTokens: 120_000, tokenCountSource: 'provider_context_reported' })
    expect(first.segments?.[0].estimated_tokens).toBe(24_000)
    const next = applyLiveContextUsage(first, { usedTokens: 31_000, tokenCountSource: 'provider_context_reported' })
    expect(reportedContextTokens(next)).toBe(31_000)
    expect(next.usage_ratio).toBe(0.155)
    expect(next.segments?.[0].estimated_tokens).toBe(24_000)
    expect(contextBreakdown(next.segments).map(({ id, percent }) => [id, percent]))
      .toEqual([['conversation', 0.75], ['tools', 0.25]])
    expect(next.compression_count).toBe(2)
  })

  it('invalidates the meter without dropping categories the event did not replace', () => {
    const invalidated = applyLiveContextUsage(previous, { usedTokens: 0 })
    expect(reportedContextTokens(invalidated)).toBeNull()
    expect(invalidated.usage_ratio).toBeNull()
    expect(invalidated.segments).toEqual(previous.segments)
    expect(invalidated.context_window_tokens).toBe(200_000)
    const cleared = applyLiveContextUsage(previous, { usedTokens: 0, segments: [] })
    expect(cleared.segments).toEqual([])
    const refreshed = applyLiveContextUsage(cleared, { usedTokens: 4_000, tokenCountSource: 'provider_context_reported' })
    expect(reportedContextTokens(refreshed)).toBe(4_000)
    expect(refreshed.segments).toEqual([])
  })

  it('keeps unknown capacity distinct from zero usage and accepts a model-window change', () => {
    const next = applyLiveContextUsage(null, { usedTokens: 0, tokenCountSource: 'provider_context_reported' })
    expect(reportedContextTokens(next)).toBe(0)
    expect(next.usage_ratio).toBeNull()
    const switched = applyLiveContextUsage(previous, {
      usedTokens: 32_000, tokenCountSource: 'provider_context_reported', contextWindowTokens: 128_000,
    })
    expect(switched.usage_ratio).toBe(0.25)
  })

  it('keeps CLI reports but does not promote CLI estimates to measurements', () => {
    expect(reportedContextTokens({ estimated_input_tokens: 42, token_count_source: 'cli_reported' })).toBe(42)
    expect(reportedContextTokens({ estimated_input_tokens: 42, token_count_source: 'estimated' })).toBeNull()
  })
})

it('reads the backend compaction budget', () => {
  expect(autoCompactPercent({ contextWindowTokens: 200000, autoCompactThresholdTokens: 170000 })).toBe(85)
  expect(autoCompactPercent({ contextWindowTokens: 200000 })).toBeNull()
})

it('does not invent a character distribution for legacy estimates or malformed counts', () => {
  expect(contextBreakdown([
    { id: 'mcp', label: 'MCP', estimated_tokens: 100_000 },
    { id: 'skills', label: 'Skills', chars: NaN },
    { id: 'conversation', label: 'Messages', chars: -1 },
  ])).toEqual([])
})

it('merges all measured content into three categories without losing character counts', () => {
  const segments = contextBreakdown([
    { id: 'native_tools', label: '', chars: 100 },
    { id: 'tool_definitions', label: '', chars: 200 },
    { id: 'mcp', label: '', chars: 100 },
    { id: 'skills', label: '', chars: 100 },
    { id: 'system_prompt', label: '', chars: 50 },
    { id: 'assistant', label: '', chars: 50 },
    { id: 'memory_l1', label: '', chars: 50 },
    { id: 'runtime_context', label: '', chars: 50 },
    { id: 'conversation', label: '', chars: 100 },
    { id: 'summarized_conversation', label: '', chars: 100 },
    { id: 'attachments', label: '', chars: 100 },
  ])
  expect(segments.map(({ id, percent }) => [id, percent]))
    .toEqual([['tools', 0.5], ['conversation', 0.3], ['system_prompt', 0.2]])
})

describe('measurement order', () => {
  const streaming: ConversationContextState = {
    context_window_tokens: 1_000_000,
    reported_context_tokens: 53_000,
    token_count_source: 'provider_context_reported',
    measurement_seq: 5,
    segments: [{ id: 'conversation', label: '', chars: 800, estimated_tokens: 2_000 }],
    clear_boundaries: [{ id: 'clr', created_at: 2 }],
    compaction_boundaries: [{ id: 'cmp', created_at: 1 }],
  }

  it('does not let an older refresh snapshot roll a streaming report back', () => {
    const disk = mergeContextSnapshot(streaming, {
      ...streaming,
      reported_context_tokens: 7_100,
      measurement_seq: 4,
      segments: [{ id: 'tools', label: '', chars: 10, estimated_tokens: 100 }],
      clear_boundaries: [],
      compaction_boundaries: [],
    })
    expect(reportedContextTokens(disk)).toBe(53_000)
    expect(disk.measurement_seq).toBe(5)
    expect(disk.clear_boundaries).toEqual([{ id: 'clr', created_at: 2 }])
    expect(disk.compaction_boundaries).toEqual([{ id: 'cmp', created_at: 1 }])
    expect(disk).toBe(streaming)
  })

  it('applies a newer snapshot even when the reported total drops', () => {
    const compacted = mergeContextSnapshot(streaming, {
      reported_context_tokens: 7_100,
      token_count_source: 'provider_context_reported',
      measurement_seq: 6,
      context_window_tokens: 1_000_000,
      segments: [{ id: 'conversation', label: '', chars: 100, estimated_tokens: 400 }],
      compaction_boundaries: [{ id: 'cmp', created_at: 1 }, { id: 'cmp2', created_at: 3 }],
    })
    expect(reportedContextTokens(compacted)).toBe(7_100)
    expect(compacted.compaction_boundaries?.map((boundary) => boundary.id)).toEqual(['cmp', 'cmp2'])
    expect(compacted.clear_boundaries).toEqual([{ id: 'clr', created_at: 2 }])
  })

  it('applies a newer model invalidation and ignores a late older report', () => {
    const invalidated = mergeContextSnapshot(streaming, {
      reported_context_tokens: null,
      token_count_source: null,
      measurement_seq: 6,
      segments: [],
      context_window_tokens: 128_000,
    })
    expect(reportedContextTokens(invalidated)).toBeNull()
    expect(invalidated.segments).toEqual([])
    const late = applyLiveContextUsage(invalidated, {
      usedTokens: 53_000,
      tokenCountSource: 'provider_context_reported',
      measurementSeq: 5,
    })
    expect(late).toBe(invalidated)
    const preserved = applyLiveContextUsage(streaming, { usedTokens: 0, measurementSeq: 6 })
    expect(reportedContextTokens(preserved)).toBeNull()
    expect(preserved.segments).toEqual(streaming.segments)
    expect(preserved.context_window_tokens).toBe(1_000_000)
    const cleared = applyLiveContextUsage(streaming, { usedTokens: 0, measurementSeq: 6, segments: [] })
    expect(cleared.segments).toEqual([])
  })

  it('ignores a stale live report and an unsequenced snapshot once a sequence is applied', () => {
    const staleLive = applyLiveContextUsage(streaming, {
      usedTokens: 7_100,
      tokenCountSource: 'provider_context_reported',
      measurementSeq: 4,
    })
    expect(staleLive).toBe(streaming)
    const unsequenced = mergeContextSnapshot(streaming, {
      reported_context_tokens: 7_100,
      token_count_source: 'provider_context_reported',
    })
    expect(unsequenced).toBe(streaming)
    const legacy = mergeContextSnapshot(
      { reported_context_tokens: 100, token_count_source: 'provider_context_reported' },
      { reported_context_tokens: 200, token_count_source: 'provider_context_reported' },
    )
    expect(reportedContextTokens(legacy)).toBe(200)
  })

  it('keeps the fresher meter when a restored conversation revision is newer', () => {
    const current = {
      id: 'c', revision: 4, title: 'live', messages: [], provider_id: 'p', model: 'm',
      created_at: 1, updated_at: 1, context_state: streaming, contextState: streaming,
    } as Conversation
    const restored = {
      ...current,
      revision: 8,
      title: 'from disk',
      context_state: {
        reported_context_tokens: 7_100,
        token_count_source: 'provider_context_reported',
        measurement_seq: 4,
      },
      contextState: undefined,
    } as Conversation
    const kept = keepNewerContextMeasurement(restored, current)
    expect(kept.title).toBe('from disk')
    expect(kept.revision).toBe(8)
    expect(reportedContextTokens(kept.context_state)).toBe(53_000)
    expect(kept.contextState).toBe(kept.context_state)
  })

  it('shows request segments from a source-less live report while the meter stays unknown', () => {
    const current = { ...streaming, lifecycle_id: 2, cache_hit_rate: 0.8, cacheHitRate: 0.8 }
    const next = mergeContextMeasurement(current, {
      kind: 'live',
      usage: {
        usedTokens: 0,
        measurementSeq: 6,
        lifecycleId: 2,
        segments: [
          { id: 'system_prompt', label: '', chars: 200, estimatedTokens: 1000 },
          { id: 'tools', label: '', chars: 100, estimatedTokens: 400 },
          { id: 'conversation', label: '', chars: 500, estimatedTokens: 2000 },
        ],
      },
    })
    expect(reportedContextTokens(next)).toBeNull()
    expect(contextBreakdown(next.segments).map((segment) => segment.id))
      .toEqual(['conversation', 'system_prompt', 'tools'])
    expect(next.cache_hit_rate).toBeUndefined()
    const kept = mergeContextMeasurement(current, {
      kind: 'live',
      usage: {
        usedTokens: 60_000,
        tokenCountSource: 'provider_context_reported',
        measurementSeq: 6,
        lifecycleId: 2,
      },
    })
    expect(reportedContextTokens(kept)).toBe(60_000)
    expect(kept.cache_hit_rate).toBe(0.8)
    expect(kept.segments).toEqual(streaming.segments)
  })

  it('drops omitted categories when the lifecycle changes and ignores a lower sequence', () => {
    const current = { ...streaming, lifecycle_id: 2, cache_hit_rate: 0.8, cacheHitRate: 0.8 }
    const invalidated = mergeContextMeasurement(current, {
      kind: 'live',
      usage: { usedTokens: 0, measurementSeq: 6, lifecycleId: 3 },
    })
    expect(reportedContextTokens(invalidated)).toBeNull()
    expect(invalidated.segments).toEqual([])
    expect(invalidated.cache_hit_rate).toBeUndefined()
    const stale = mergeContextMeasurement(current, {
      kind: 'live',
      usage: {
        usedTokens: 1,
        tokenCountSource: 'provider_context_reported',
        measurementSeq: 4,
        lifecycleId: 9,
        segments: [],
        cacheInputTokens: 10,
        cacheReadTokens: 1,
      },
    })
    expect(stale).toBe(current)
    const cleared = mergeContextMeasurement(current, {
      kind: 'snapshot',
      state: {
        ...streaming,
        measurement_seq: 6,
        lifecycle_id: 3,
        cache_hit_rate: null,
        cacheHitRate: 0.8,
        segments: [],
      },
    })
    expect(cleared.cache_hit_rate).toBeNull()
    expect(cleared.cacheHitRate).toBeNull()
    expect(cleared.segments).toEqual([])
  })
})

it('sums independent estimates including attachments without inventing missing measurements', () => {
  const segments = contextBreakdown([
    { id: 'native_tools', label: '', chars: 100, estimated_tokens: 4000 },
    { id: 'mcp', label: '', chars: 200, estimatedTokens: 6000 },
    { id: 'conversation', label: '', chars: 700, estimated_tokens: 1000 },
    { id: 'attachments', label: '', chars: 0, estimated_tokens: 800 },
    { id: 'system_prompt', label: '', chars: 100 },
  ])
  expect(segments.find(s => s.id === 'tools')?.estimatedTokens).toBe(10000)
  expect(segments.find(s => s.id === 'conversation')?.estimatedTokens).toBe(1800)
  expect(segments.find(s => s.id === 'system_prompt')?.estimatedTokens).toBeNull()
  expect(segments.find(s => s.id === 'tools')?.percent).toBeCloseTo(300 / 1100)
})
