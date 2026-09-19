import {
  useCallback,
  useState,
  type Dispatch,
  type MutableRefObject,
  type SetStateAction,
} from 'react'
import { api } from '../../api/tauri'
import { chatApi } from '../api'
import { latestCompactionBoundaryId, mergeCompactionContextState } from '../compactionBoundary'
import { latestClearBoundaryId, mergeClearContextState } from '../contextClearBoundary'
import { applyLiveContextUsage } from '../contextPanel'
import type { Conversation, ConversationContextState } from '../types'
import { useTauriEvent } from './useTauriEvent'

/** 边界（压缩 / 清空）落地后高亮的时长，与 CompactionDivider 的入场动画对齐。 */
const BOUNDARY_ANIMATION_MS = 1800
/** 手动压缩成功后按住「压缩中」的最短时长，让分隔线动画有机会播完。 */
const COMPRESS_SETTLE_MS = 360

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
  const [animateCompactionBoundaryId, setAnimateCompactionBoundaryId] = useState<string | null>(null)
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

  const flashCompactionBoundary = useCallback((boundaryId: string) => {
    setAnimateCompactionBoundaryId(boundaryId)
    window.setTimeout(() => {
      setAnimateCompactionBoundaryId((current) => (current === boundaryId ? null : current))
    }, BOUNDARY_ANIMATION_MS)
  }, [])

  const flashClearBoundary = useCallback((boundaryId: string) => {
    setAnimateClearBoundaryId(boundaryId)
    window.setTimeout(() => {
      setAnimateClearBoundaryId((current) => (current === boundaryId ? null : current))
    }, BOUNDARY_ANIMATION_MS)
  }, [])

  /** 离开 / 删除当前会话时把面板状态归零。 */
  const resetContext = useCallback(() => {
    setContextState(null)
    setContextError('')
    setContextLoading(false)
  }, [setContextState])

  /** 合并一份完整的权威上下文快照（保留本地已知的压缩 / 清空边界）。 */
  const patchContextState = useCallback((nextState: ConversationContextState) => {
    setContextState((prev) => mergeClearContextState(prev, mergeCompactionContextState(prev, nextState)))
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

  const refreshCurrent = useCallback(() => {
    const conversationId = currentConversationIdRef.current
    if (conversationId) void refreshContextStats(conversationId)
  }, [currentConversationIdRef, refreshContextStats])

  const compressCurrent = useCallback(async () => {
    const conversationId = currentConversationIdRef.current
    if (!conversationId || compactingConversationIds.has(conversationId)) return
    markConversationCompacting(conversationId, true)
    setContextError('')
    try {
      const result = await chatApi.compressContext(conversationId)
      if (currentConversationIdRef.current === conversationId) {
        const latestId = latestCompactionBoundaryId(result.contextState)
        if (latestId) flashCompactionBoundary(latestId)
        patchContextState(result.contextState)
        refreshSidebar()
        await new Promise<void>((resolve) => {
          window.setTimeout(resolve, COMPRESS_SETTLE_MS)
        })
      }
    } catch (err) {
      if (currentConversationIdRef.current === conversationId) {
        setContextError(errorMessage(err, '上下文压缩失败'))
      }
    } finally {
      // 清零不看「我还在不在这个会话」——切走后原来那个守卫永远不成立，标志会卡死。
      markConversationCompacting(conversationId, false)
    }
  }, [
    compactingConversationIds, currentConversationIdRef, flashCompactionBoundary,
    markConversationCompacting, patchContextState, refreshSidebar,
  ])

  const clearCurrent = useCallback(async () => {
    const conversationId = currentConversationIdRef.current
    if (!conversationId) return
    setContextError('')
    try {
      const result = await chatApi.clearContext(conversationId)
      if (currentConversationIdRef.current === conversationId) {
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
  }, [currentConversationIdRef, flashClearBoundary, patchContextState, refreshSidebar])

  useTauriEvent(api.onChatContext, (payload) => {
    const currentConversationId = currentConversationIdRef.current
    if (!currentConversationId || payload.conversationId !== currentConversationId) {
      return
    }
    // 生成过程中的活数：只有分子 + 分母，就地补进现有状态（分段/压缩计数/来源标签留给
    // 轮末的权威快照）。不能走 patchContextState —— 那条要求一份完整的上下文状态对象。
    if (payload.live) {
      const live = payload.live
      setContextState((prev) => applyLiveContextUsage(prev, live) ?? prev)
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
      markConversationCompacting(conversationId, payload.phase === 'started')
    }
    if (payload.phase === 'started') return
    // 下面这些改的是当前会话的展示状态（边界动画 / currentConversation），仍要按当前会话过滤。
    if (conversationId !== currentConversationIdRef.current) return
    const boundary = payload.boundary
    if (boundary?.id) flashCompactionBoundary(boundary.id)
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
  }, [flashCompactionBoundary, markConversationCompacting, setCurrentConversation])

  return {
    contextState,
    contextLoading,
    setContextLoading,
    contextError,
    resetContext,
    compactingConversationIds,
    markConversationCompacting,
    animateCompactionBoundaryId,
    animateClearBoundaryId,
    patchContextState,
    refreshContextStats,
    refreshCurrent,
    compressCurrent,
    clearCurrent,
  }
}
