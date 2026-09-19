import { useCallback, useReducer } from 'react'
import type { ExplainMessage, LensWebSearchPayload, LensWebSearchResult } from '../api/tauri'
import type { Mode, Stage } from './types'

type View = {
  stage: Stage
  appLabel: string
  input: string
  selectionText: string
  messages: ExplainMessage[]
  streaming: boolean
  copied: boolean
}

type Action =
  | { type: 'open'; mode: Mode }
  | { type: 'hide' }
  | { type: 'restore'; appLabel: string; messages: ExplainMessage[] }
  | { type: 'stage'; stage: Stage }
  | { type: 'capture'; appLabel: string; stage: Stage }
  | { type: 'input'; value: string }
  | { type: 'selection'; value: string }
  | { type: 'answer'; messages: ExplainMessage[] }
  | { type: 'stream'; delta?: string; reasoningDelta?: string }
  | { type: 'webSearch'; payload: LensWebSearchPayload }
  | { type: 'final'; response?: string; error?: string; results?: LensWebSearchResult[] }
  | { type: 'busy'; value: boolean }
  | { type: 'handoffFailed' }
  | { type: 'copied'; value: boolean }

const emptyView = (): View => ({
  stage: 'select', appLabel: '', input: '', selectionText: '', messages: [], streaming: false, copied: false,
})

function updateAssistant(view: View, update: (message: ExplainMessage) => ExplainMessage): View {
  const last = view.messages.at(-1)
  if (!last || last.role !== 'assistant') return view
  return { ...view, messages: [...view.messages.slice(0, -1), update(last)] }
}

function transition(view: View, action: Action): View {
  switch (action.type) {
    case 'open': return { ...emptyView(), stage: action.mode === 'translateText' ? 'translating' : 'select' }
    case 'hide': return emptyView()
    case 'restore': return { ...view, stage: 'answering', appLabel: action.appLabel, input: '', selectionText: '', messages: action.messages, streaming: false }
    case 'stage': return { ...view, stage: action.stage }
    case 'capture': return { ...view, appLabel: action.appLabel, stage: action.stage }
    case 'input': return { ...view, input: action.value }
    case 'selection': return { ...view, selectionText: action.value }
    case 'answer': return { ...view, messages: action.messages, stage: 'answering', streaming: true }
    case 'stream': return updateAssistant(view, last => ({
      ...last,
      content: last.content + (action.delta ?? ''),
      reasoning: action.reasoningDelta ? (last.reasoning ?? '') + action.reasoningDelta : last.reasoning,
    }))
    case 'webSearch': return updateAssistant(view, last => ({ ...last, webSearch: {
      status: action.payload.status,
      query: action.payload.query,
      reason: action.payload.reason,
      results: action.payload.results,
      error: action.payload.error,
    } }))
    case 'final': {
      const withAnswer = updateAssistant(view, last => action.error
        ? { role: 'assistant', content: action.error! }
        : action.response && !last.content ? { ...last, content: action.response } : last)
      if (!action.results?.length) return withAnswer
      return updateAssistant(withAnswer, last => last.webSearch?.results?.length
        ? last : { ...last, webSearch: { status: 'done', results: action.results } })
    }
    case 'busy': return { ...view, streaming: action.value }
    case 'handoffFailed': return { ...view, streaming: false, stage: 'ready' }
    case 'copied': return { ...view, copied: action.value }
  }
}

/** Session-visible conversation state; opening, hide and history restore are atomic transitions. */
export function useLensConversationController() {
  const [view, dispatch] = useReducer(transition, undefined, emptyView)
  const open = useCallback((mode: Mode) => dispatch({ type: 'open', mode }), [])
  const hide = useCallback(() => dispatch({ type: 'hide' }), [])
  const restoreHistory = useCallback((appLabel: string, messages: ExplainMessage[]) => dispatch({ type: 'restore', appLabel, messages }), [])
  const showStage = useCallback((stage: Stage) => dispatch({ type: 'stage', stage }), [])
  const capture = useCallback((appLabel: string, stage: Stage) => dispatch({ type: 'capture', appLabel, stage }), [])
  const editInput = useCallback((value: string) => dispatch({ type: 'input', value }), [])
  const selectText = useCallback((value: string) => dispatch({ type: 'selection', value }), [])
  const beginAnswer = useCallback((messages: ExplainMessage[]) => dispatch({ type: 'answer', messages }), [])
  const applyStream = useCallback((part: { delta?: string; reasoningDelta?: string }) => dispatch({ type: 'stream', ...part }), [])
  const applyWebSearch = useCallback((payload: LensWebSearchPayload) => dispatch({ type: 'webSearch', payload }), [])
  const applyFinal = useCallback((part: { response?: string; error?: string; results?: LensWebSearchResult[] }) => dispatch({ type: 'final', ...part }), [])
  const setBusy = useCallback((value: boolean) => dispatch({ type: 'busy', value }), [])
  const handoffFailed = useCallback(() => dispatch({ type: 'handoffFailed' }), [])
  const showCopied = useCallback((value: boolean) => dispatch({ type: 'copied', value }), [])
  return { view, open, hide, restoreHistory, showStage, capture, editInput, selectText, beginAnswer, applyStream, applyWebSearch, applyFinal, setBusy, handoffFailed, showCopied }
}
