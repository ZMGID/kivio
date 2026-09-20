import { useCallback, useEffect, useRef } from 'react'
import { api, type ChatExternalSendRequest } from '../../api/tauri'
import type { Conversation, PendingAttachment } from '../types'

let ownerSequence = 0
function nextOwnerId(): string {
  ownerSequence += 1
  return `chat-external-${Date.now()}-${ownerSequence}-${Math.random().toString(36).slice(2)}`
}

interface UseExternalSendQueueParams {
  /** 取到消息后先切回会话视图。 */
  onEnterConversationView: () => void
  /** 历史预置分支：把整段多轮历史搬成新会话，不发消息。 */
  onImportConversation: (
    messages: NonNullable<ChatExternalSendRequest['messages']>,
    attachmentPaths: string[],
  ) => Promise<boolean>
  /** 发送一条外部消息；返回 false 表示当前发不出去（如正在生成），需重排。 */
  onSendMessage: (
    content: string,
    attachments: PendingAttachment[],
    options: {
      forceNewConversation: true
      conversationOverride?: Conversation
      onPartialConversation: (conversation: Conversation) => void
    },
  ) => Promise<boolean>
  onError: (message: string) => void
}

/**
 * 外部发送队列（如 Lens 交接过来的消息）。
 *
 * 三个 ref 只服务这一件事，故整体搬出：
 *   - queue：已取走但尚未发出的请求
 *   - processing：单飞标志，避免并发 drain
 *   - requested：drain 期间又被触发时的重排标志
 *
 * 返回的 drainExternalSends 身份稳定（依赖数组为空），调用方的 effect 不会因它重订阅 ——
 * 这是搬迁前就有的性质，靠参数回调经 ref 间接调用来保持。
 */
export function useExternalSendQueue({
  onEnterConversationView,
  onImportConversation,
  onSendMessage,
  onError,
}: UseExternalSendQueueParams) {
  const queueRef = useRef<ChatExternalSendRequest[]>([])
  const ownerIdRef = useRef<string | null>(null)
  if (ownerIdRef.current === null) ownerIdRef.current = nextOwnerId()
  const mountEpochRef = useRef(0)
  const deliveredRef = useRef<Set<string>>(new Set())
  const processingRef = useRef(false)
  const requestedRef = useRef(false)
  const partialByRequestRef = useRef(new Map<string, Conversation>())
  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const heartbeatRef = useRef<ReturnType<typeof setInterval> | null>(null)
  const retryDelayRef = useRef(100)
  const disposedRef = useRef(false)

  // 参数回调每次渲染都是新身份；经 ref 读取以保持 drain 本身稳定。
  const callbacksRef = useRef({ onEnterConversationView, onImportConversation, onSendMessage, onError })
  callbacksRef.current = { onEnterConversationView, onImportConversation, onSendMessage, onError }

  const drainExternalSends = useCallback(async () => {
    const ownerId = ownerIdRef.current
    const mountEpoch = mountEpochRef.current
    const stale = () => disposedRef.current || mountEpochRef.current !== mountEpoch
    if (!ownerId || stale()) return
    if (retryTimerRef.current !== null) {
      window.clearTimeout(retryTimerRef.current)
      retryTimerRef.current = null
    }
    if (processingRef.current) {
      requestedRef.current = true
      return
    }

    processingRef.current = true
    try {
      do {
        requestedRef.current = false

        const result = await api.chatTakeExternalSends(ownerId)
        if (stale()) return
        if (!result.success) {
          const error = 'error' in result && typeof result.error === 'string'
            ? result.error
            : ''
          throw new Error(error || 'Failed to take external Chat messages')
        }
        const requests = result.requests ?? []
        if (requests.length > 0) {
          const knownIds = new Set(queueRef.current.map((request) => request.id))
          queueRef.current.push(...requests.filter((request) => !knownIds.has(request.id)))
          if (queueRef.current.length > 0 && heartbeatRef.current === null) {
            heartbeatRef.current = window.setInterval(() => {
              if (stale()) return
              void api.chatRenewExternalSends(ownerId).catch((error) => {
                if (!stale()) console.error('Failed to renew external Chat messages:', error)
              })
            }, 10_000)
          }
        }

        const request = queueRef.current[0]
        if (!request) {
          if (result.pendingLeased) {
            requestedRef.current = true
            break
          }
          continue
        }
        if (deliveredRef.current.has(request.id)) {
          const acknowledged = await api.chatAckExternalSend(ownerId, request.id)
          if (stale()) return
          if (!acknowledged.success) {
            // A later owner already claimed it, or it was acknowledged elsewhere.
            queueRef.current.shift()
            deliveredRef.current.delete(request.id)
            partialByRequestRef.current.delete(request.id)
            continue
          }
          queueRef.current.shift()
          deliveredRef.current.delete(request.id)
          partialByRequestRef.current.delete(request.id)
          retryDelayRef.current = 100
          continue
        }
        callbacksRef.current.onEnterConversationView()
        const attachmentPaths = (request.attachments ?? [])
          .map((attachment) => attachment.path)
          .filter((path): path is string => !!path)

        // 历史预置分支：把 Lens 完整多轮历史 + 截图搬成一个新会话（不发消息、不触发回复），落地末尾可续聊。
        if (request.messages && request.messages.length > 0) {
          const imported = await callbacksRef.current.onImportConversation(request.messages, attachmentPaths)
          if (stale()) return
          if (!imported) {
            requestedRef.current = true
            break
          }
          deliveredRef.current.add(request.id)
          continue
        }

        const attachments = (request.attachments ?? [])
          .filter((attachment) => attachment.path)
          .map<PendingAttachment>((attachment, index) => ({
            id: attachment.id || `external-${request.id}-${index}`,
            type: attachment.type === 'video' || attachment.type === 'file'
              ? attachment.type
              : 'image',
            name: attachment.name || (attachment.type === 'image' ? 'Image' : 'Attachment'),
            path: attachment.path,
          }))
        const accepted = await callbacksRef.current.onSendMessage(
          request.content ?? '',
          attachments,
          {
            forceNewConversation: true,
            conversationOverride: partialByRequestRef.current.get(request.id),
            onPartialConversation: (conversation) => {
              if (!stale()) partialByRequestRef.current.set(request.id, conversation)
            },
          },
        )
        if (stale()) return
        if (accepted) {
          deliveredRef.current.add(request.id)
        } else {
          requestedRef.current = true
          break
        }
      } while (requestedRef.current || queueRef.current.length > 0)
    } catch (err) {
      if (stale()) return
      console.error('Failed to process external Chat message:', err)
      requestedRef.current = true
      callbacksRef.current.onError(
        typeof err === 'string' ? err : (err as Error).message || '外部消息发送失败',
      )
    } finally {
      if (!stale()) {
        processingRef.current = false
        if (queueRef.current.length === 0 && heartbeatRef.current !== null) {
          window.clearInterval(heartbeatRef.current)
          heartbeatRef.current = null
        }
        if (requestedRef.current) {
          const delay = retryDelayRef.current
          retryDelayRef.current = Math.min(delay * 2, 5000)
          retryTimerRef.current = window.setTimeout(() => {
            retryTimerRef.current = null
            void drainExternalSends()
          }, delay)
        }
      }
    }
  }, [])

  /** A completed run can retry immediately; timer remains the bounded fallback. */
  const wakeAfterRun = useCallback(async () => {
    if (requestedRef.current || queueRef.current.length > 0) {
      await drainExternalSends()
    }
  }, [drainExternalSends])

  useEffect(() => {
    if (ownerIdRef.current === null) ownerIdRef.current = nextOwnerId()
    const ownerId = ownerIdRef.current
    const delivered = deliveredRef.current
    const partial = partialByRequestRef.current
    mountEpochRef.current += 1
    disposedRef.current = false
    return () => {
      mountEpochRef.current += 1
      disposedRef.current = true
      ownerIdRef.current = null
      queueRef.current = []
      delivered.clear()
      partial.clear()
      processingRef.current = false
      requestedRef.current = false
      retryDelayRef.current = 100
      if (retryTimerRef.current !== null) {
        window.clearTimeout(retryTimerRef.current)
        retryTimerRef.current = null
      }
      if (heartbeatRef.current !== null) {
        window.clearInterval(heartbeatRef.current)
        heartbeatRef.current = null
      }
      void api.chatReleaseExternalSends(ownerId).catch((error) => {
        console.error('Failed to release external Chat messages:', error)
      })
    }
  }, [])

  /** 流式结束后调用方据此判断要不要补一次 drain（搬迁前是直接读 ref）。 */
  const hasPendingDrainRequest = useCallback(() => requestedRef.current, [])

  return { drainExternalSends, wakeAfterRun, hasPendingDrainRequest }
}
