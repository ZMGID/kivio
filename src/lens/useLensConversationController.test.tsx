import { act, renderHook } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { useLensConversationController } from './useLensConversationController'

describe('useLensConversationController', () => {
  it('opens and hides with one session transition, without retaining old answer or draft', () => {
    const { result } = renderHook(() => useLensConversationController())
    act(() => {
      result.current.editInput('draft')
      result.current.selectText('selection')
      result.current.beginAnswer([
        { role: 'user', content: 'question' },
        { role: 'assistant', content: '' },
      ])
      result.current.applyStream({ delta: 'old answer' })
    })
    expect(result.current.view.messages[1].content).toBe('old answer')

    act(() => result.current.hide())
    expect(result.current.view).toMatchObject({
      stage: 'select', input: '', selectionText: '', messages: [], streaming: false, copied: false,
    })
    act(() => result.current.open('translateText'))
    expect(result.current.view.stage).toBe('translating')
    expect(result.current.view.messages).toEqual([])
  })

  it('keeps streamed content over a late final response and displays final errors', () => {
    const { result } = renderHook(() => useLensConversationController())
    act(() => result.current.beginAnswer([
      { role: 'user', content: 'question' },
      { role: 'assistant', content: '' },
    ]))
    act(() => result.current.applyStream({ reasoningDelta: 'thinking', delta: 'streamed' }))
    act(() => result.current.applyFinal({ response: 'fallback' }))
    expect(result.current.view.messages[1]).toMatchObject({ content: 'streamed', reasoning: 'thinking' })
    act(() => result.current.applyFinal({ error: 'Failed' }))
    expect(result.current.view.messages[1].content).toBe('Failed')
  })

  it('restores history as one snapshot and clears current input and selection', () => {
    const { result } = renderHook(() => useLensConversationController())
    act(() => {
      result.current.editInput('unsent')
      result.current.selectText('old selection')
      result.current.restoreHistory('App', [{ role: 'assistant', content: 'history' }])
    })
    expect(result.current.view).toMatchObject({
      stage: 'answering', appLabel: 'App', input: '', selectionText: '', streaming: false,
      messages: [{ role: 'assistant', content: 'history' }],
    })
  })
})
