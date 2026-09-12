import { act, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createEmptyStreamSnapshot, type ConversationStreamSnapshot } from './conversationRuns'
import { useStreamRenderFrame } from './hooks/useStreamRenderFrame'
import { freezeCancelledStream, isLocallyCancelledPayload } from './streamCancellation'
import { getCoarse, getSnapshot, reset, setCoarse, setSnapshot } from './streamingStore'

beforeEach(() => { vi.useFakeTimers(); reset() })
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); reset() })

describe('manual stream cancellation', () => {
  it.each([false, true])('stays stopped after queued renders and revisiting the conversation (hidden=%s)', (hidden) => {
    vi.spyOn(document, 'hidden', 'get').mockReturnValue(hidden)
    let pendingFrame: FrameRequestCallback | undefined
    vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => { pendingFrame = cb; return 1 })
    vi.stubGlobal('cancelAnimationFrame', () => { pendingFrame = undefined })
    const snapshot = { ...createEmptyStreamSnapshot(), runId: 'run', content: 'partial answer', reasoningStreaming: true }
    const applySnapshot = (next: ConversationStreamSnapshot) => {
      setSnapshot(next)
      setCoarse({ streaming: next.streaming, cancelling: false })
    }
    const { result } = renderHook(() => useStreamRenderFrame({
      currentConversationIdRef: { current: 'conversation' },
      applySnapshot,
    }))
    act(() => {
      applySnapshot(snapshot)
      result.current.showStreamSnapshotIfCurrent('conversation', snapshot)
      freezeCancelledStream(snapshot, result.current.cancelPendingFrame)
      pendingFrame?.(0)
      vi.runAllTimers()
    })
    expect(getCoarse().streaming).toBe(false)
    expect(getSnapshot()).toMatchObject({ streaming: false, reasoningStreaming: false, content: 'partial answer' })
    // Switching away and back restores the per-conversation snapshot.
    act(() => applySnapshot(snapshot))
    expect(getCoarse().streaming).toBe(false)
  })

  it.each(['run_cancelled', 'run_completed', 'run_failed'])('allows %s to settle a restored run without a pending send invoke', (type) => {
    expect(isLocallyCancelledPayload({ conversationId: 'c', runId: 'r', type }, 'c', 'r')).toBe(false)
  })

  it('still ignores late deltas from the cancelled run, including cancellation before run_started', () => {
    expect(isLocallyCancelledPayload({ conversationId: 'c', runId: 'r', type: 'text_delta' }, 'c', 'r')).toBe(true)
    expect(isLocallyCancelledPayload({ conversationId: 'c', runId: 'r', type: 'run_started' }, 'c', null)).toBe(true)
    expect(isLocallyCancelledPayload({ conversationId: 'c', runId: 'new', type: 'text_delta' }, 'c', 'r')).toBe(false)
    expect(isLocallyCancelledPayload({ conversationId: 'other', runId: 'r' }, 'c', 'r')).toBe(false)
  })
})
