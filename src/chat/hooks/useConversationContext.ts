import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type Dispatch,
  type MutableRefObject,
  type SetStateAction,
} from 'react'
import { api } from '../../api/tauri'
import { chatApi } from '../api'
import { latestClearBoundaryId } from '../contextClearBoundary'
import { mergeContextMeasurement } from '../contextPanel'
import type { Conversation, ConversationContextState, ContextCompactionResult } from '../types'
import { useTauriEvent } from './useTauriEvent'

const CLEAR_BOUNDARY_ANIMATION_MS = 1800

/** Navigation supplies the saved measurement; only a change to the open chat's
 * settings needs a new calculation. Live usage arrives on the protocol stream. */
export function useRefreshContextOnSettingsChange(conversation: Conversation | null, refresh: (id: string) => void) {
  const id = conversation?.id
  const provider = conversation?.provider_id
  const model = conversation?.model
  const skill = conversation?.active_skill_id ?? conversation?.activeSkillId
  const runtime = conversation?.agent_runtime ?? conversation?.agentRuntime
  const agent = runtime?.externalAgentId ?? runtime?.external_agent_id
  const externalModel = runtime?.externalModel ?? runtime?.external_model
  const previous = useRef<{ id?: string; settings: string }>()
  useEffect(() => {
    const settings = JSON.stringify([provider, model, skill, runtime?.kind, agent, externalModel])
    const last = previous.current
    previous.current = { id, settings }
    if (id && last?.id === id && last.settings !== settings) refresh(id)
  }, [id, provider, model, skill, runtime?.kind, agent, externalModel, refresh])
}

export interface UseConversationContextOptions {
  currentConversation: Conversation | null
  currentConversationIdRef: MutableRefObject<string | null>
  /** 会话对象是上下文的唯一状态源，面板与消息边界读取同一份快照。 */
  setCurrentConversation: Dispatch<SetStateAction<Conversation | null>>
  /** 压缩 / 清空会改侧栏 preview，落地后让侧栏 refetch。 */
  refreshSidebar: () => void
}

function errorMessage(err: unknown, fallback: string): string {
  return typeof err === 'string' ? err : (err as Error).message || fallback
}

/**
 * 会话上下文用量的页面级 owner：统计快照（含流式中的活数）、手动压缩 / 清空、
 * 压缩进行中集合，以及边界高亮。
 *
 * 压缩状态必须按会话记，不能用全局 boolean：压缩中切会话会把「压缩中」动画留在另一个
 * 会话上，而压缩事件按 conversationId 派发，收敛条件不能再是「是不是当前会话」（那样后台
 * 会话的 completed 会被丢掉，标志永远清不掉）。手动与自动压缩共用这一个集合。
 */
export function useConversationContext({
  currentConversation,
  currentConversationIdRef,
  setCurrentConversation,
  refreshSidebar,
}: UseConversationContextOptions) {
  const contextState = currentConversation?.context_state ?? currentConversation?.contextState ?? null
  const setContextState = useCallback((update: SetStateAction<ConversationContextState | null>) => {
    setCurrentConversation((conversation) => {
      if (!conversation) return conversation
      const previous = conversation.context_state ?? conversation.contextState ?? null
      const next = typeof update === 'function' ? update(previous) : update
      if (next === previous) return conversation
      return { ...conversation, context_state: next ?? undefined, contextState: next ?? undefined }
    })
  }, [setCurrentConversation])
  const [contextLoading, setContextLoading] = useState(false)
  const [contextError, setContextError] = useState('')
  const [compactingConversationIds, setCompactingConversationIds] = useState<ReadonlySet<string>>(
    () => new Set(),
  )
  const [animateClearBoundaryId, setAnimateClearBoundaryId] = useState<string | null>(null)

  const markConversationCompacting = useCallback((conversationId: string, compacting: boolean) => {
    setCompactingConversationIds((previous) => {
      if (previous.has(conversationId) === compacting) return previous
      const next = new Set(previous)
      if (compacting) next.add(conversationId)
      else next.delete(conversationId)
      return next
    })
  }, [])

  const flashClearBoundary = useCallback((boundaryId: string) => {
    setAnimateClearBoundaryId(boundaryId)
    window.setTimeout(() => {
      setAnimateClearBoundaryId((current) => (current === boundaryId ? null : current))
    }, CLEAR_BOUNDARY_ANIMATION_MS)
  }, [])

  /** 离开 / 删除当前会话时把面板状态归零。 */
  const resetContext = useCallback(() => {
    setContextState(null)
    setContextError('')
    setContextLoading(false)
  }, [setContextState])

  /** 合并一份完整快照。测量序号更旧时保留已应用的实报和边界。 */
  const patchContextState = useCallback((nextState: ConversationContextState) => {
    setContextState((prev) => mergeContextMeasurement(prev, { kind: 'snapshot', state: nextState }))
  }, [setContextState])

  const refreshContextStats = useCallback(async (conversationId?: string) => {
    const targetConversationId = conversationId ?? currentConversationIdRef.current
    if (!targetConversationId) {
      setContextState(null)
      setContextError('')
      return
    }
    setContextLoading(true)
    setContextError('')
    try {
      const result = await chatApi.getContextStats(targetConversationId)
      if (currentConversationIdRef.current === targetConversationId) {
        patchContextState(result.contextState)
      }
    } catch (err) {
      if (currentConversationIdRef.current === targetConversationId) {
        setContextError(errorMessage(err, '上下文统计失败'))
      }
    } finally {
      if (currentConversationIdRef.current === targetConversationId) {
        setContextLoading(false)
      }
    }
  }, [currentConversationIdRef, patchContextState, setContextState])

  useEffect(() => {
    setContextLoading(false)
    setContextError('')
  }, [currentConversation?.id])

  const refreshCurrent = useCallback(() => {
    const conversationId = currentConversationIdRef.current
    if (conversationId) void refreshContextStats(conversationId)
  }, [currentConversationIdRef, refreshContextStats])

  const compressConversation = useCallback(async (conversationId: string): Promise<ContextCompactionResult> => {
    if (compactingConversationIds.has(conversationId)) return { status: 'failed' }
    markConversationCompacting(conversationId, true)
    setContextError('')
    try {
      const result = await chatApi.compressContext(conversationId)
      if (currentConversationIdRef.current === conversationId) {
        patchContextState(result.contextState)
        refreshSidebar()
      }
      return { status: 'completed', conversation: result.conversation }
    } catch (err) {
      if (currentConversationIdRef.current === conversationId) {
        setContextError(errorMessage(err, '上下文压缩失败'))
      }
      return { status: errorMessage(err, '') === '压缩已停止' ? 'cancelled' : 'failed' }
    } finally {
      // 清零不看「我还在不在这个会话」——切走后原来那个守卫永远不成立，标志会卡死。
      markConversationCompacting(conversationId, false)
    }
  }, [
    compactingConversationIds, currentConversationIdRef,
    markConversationCompacting, patchContextState, refreshSidebar,
  ])

  const compressCurrent = useCallback(async () => {
    const id = currentConversationIdRef.current
    if (id) await compressConversation(id)
  }, [compressConversation, currentConversationIdRef])

  const clearCurrent = useCallback(async () => {
    const conversationId = currentConversationIdRef.current
    if (!conversationId) return
    setContextError('')
    try {
      const result = await chatApi.clearContext(conversationId)
      if (currentConversationIdRef.current === conversationId) {
        setCurrentConversation(result.conversation)
        const latestId = latestClearBoundaryId(result.contextState)
        if (latestId) flashClearBoundary(latestId)
        patchContextState(result.contextState)
        refreshSidebar()
      }
    } catch (err) {
      if (currentConversationIdRef.current === conversationId) {
        setContextError(errorMessage(err, '清空上下文失败'))
      }
    }
  }, [currentConversationIdRef, flashClearBoundary, patchContextState, refreshSidebar, setCurrentConversation])

  useTauriEvent(api.onChatContext, (payload) => {
    const currentConversationId = currentConversationIdRef.current
    if (!currentConversationId || payload.conversationId !== currentConversationId) {
      return
    }
    // Provider prompt reports arrive independently of the persisted snapshot.
    // A history rewrite clears the report until the next API measurement.
    if (payload.live) {
      const live = payload.live
      setContextState((prev) => mergeContextMeasurement(prev, { kind: 'live', usage: live }))
      return
    }
    if (!payload.contextState) return
    patchContextState(payload.contextState)
    setContextError('')
  }, [patchContextState, setContextState])

  useTauriEvent(api.onChatCompaction, (payload) => {
    const conversationId = payload.conversationId
    if (!conversationId) return
    // 压缩状态按事件里的会话记，不看是不是当前会话：后台会话的 started/completed
    // 都要收进集合，否则切走再切回来会漏掉开始、或者永远等不到结束。
    if (payload.trigger !== 'manual') {
      markConversationCompacting(conversationId, payload.phase === 'started' || payload.phase === 'retrying')
    }
    if (payload.phase === 'started' || payload.phase === 'retrying') return
    // 下面这些改的是当前会话的展示状态，仍要按当前会话过滤。
    if (conversationId !== currentConversationIdRef.current) return
    const boundary = payload.boundary
    if (boundary && payload.phase === 'completed') {
      setCurrentConversation((conversation) => {
        if (!conversation) return conversation
        const prevState = conversation.context_state ?? conversation.contextState
        const existing = prevState?.compaction_boundaries ?? prevState?.compactionBoundaries ?? []
        if (existing.some((item) => item.id === boundary.id)) return conversation
        const nextBoundaries = [...existing, boundary]
        const nextState = {
          ...(prevState ?? {}),
          compaction_boundaries: nextBoundaries,
          compactionBoundaries: nextBoundaries,
        }
        return { ...conversation, context_state: nextState, contextState: nextState }
      })
    }
  }, [markConversationCompacting, setCurrentConversation])

  return {
    contextState,
    contextLoading,
    contextError,
    resetContext,
    compactingConversationIds,
    markConversationCompacting,
    animateClearBoundaryId,
    patchContextState,
    refreshContextStats,
    refreshCurrent,
    compressCurrent,
    compressConversation,
    clearCurrent,
  }
}
