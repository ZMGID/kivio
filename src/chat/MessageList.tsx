import { useEffect, useMemo, useRef } from 'react'
import type { ChatMessage, ToolCallRecord } from './types'
import { MessageBubble } from './MessageBubble'

export interface AssistantStreamStats {
  messageId: string
  tokensPerSec: number
}

interface MessageListProps {
  messages: ChatMessage[]
  streaming?: boolean
  streamingContent?: string
  streamingReasoning?: string
  reasoningStreaming?: boolean
  streamingToolCalls?: ToolCallRecord[]
  error?: string
  lastAssistantStreamStats?: AssistantStreamStats | null
  onUpdateMessage?: (messageId: string, content: string) => Promise<void>
  onRegenerateMessage?: (messageId: string) => Promise<void>
  onDeleteMessage?: (messageId: string) => Promise<void>
}

export function MessageList({
  messages,
  streaming,
  streamingContent = '',
  streamingReasoning = '',
  reasoningStreaming = false,
  streamingToolCalls = [],
  error,
  lastAssistantStreamStats = null,
  onUpdateMessage,
  onRegenerateMessage,
  onDeleteMessage,
}: MessageListProps) {
  const scrollRef = useRef<HTMLDivElement>(null)

  const lastAssistantId = useMemo(() => {
    for (let i = messages.length - 1; i >= 0; i--) {
      if (messages[i].role === 'assistant') return messages[i].id
    }
    return null
  }, [messages])

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight
    }
  }, [messages, streaming, streamingContent, streamingReasoning, reasoningStreaming, streamingToolCalls, error])

  return (
    <div ref={scrollRef} className="custom-scrollbar flex-1 overflow-y-auto">
      <div className="mx-auto w-full max-w-3xl space-y-0.5 px-6 py-4">
        {messages.map((msg) => (
          <MessageBubble
            key={msg.id}
            message={msg}
            tokensPerSec={
              msg.role === 'assistant' &&
              msg.id === lastAssistantId &&
              lastAssistantStreamStats?.messageId === msg.id
                ? lastAssistantStreamStats.tokensPerSec
                : undefined
            }
            onUpdateMessage={msg.role === 'assistant' ? onUpdateMessage : undefined}
            onRegenerateMessage={msg.role === 'assistant' ? onRegenerateMessage : undefined}
            onDeleteMessage={msg.role === 'assistant' ? onDeleteMessage : undefined}
          />
        ))}

        {streaming && (streamingContent || streamingReasoning || streamingToolCalls.length > 0) && (
          <MessageBubble
            message={{
              id: 'streaming-assistant',
              role: 'assistant',
              content: streamingContent,
              reasoning: streamingReasoning || undefined,
              tool_calls: streamingToolCalls,
              timestamp: Math.floor(Date.now() / 1000),
            }}
            reasoningStreaming={reasoningStreaming}
          />
        )}

        {streaming && !streamingContent && !streamingReasoning && streamingToolCalls.length === 0 && (
          <div className="flex justify-start py-3">
            <div className="flex items-center gap-2 text-sm text-neutral-400">
              <span className="flex gap-1">
                <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-neutral-400" />
                <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-neutral-400 [animation-delay:0.2s]" />
                <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-neutral-400 [animation-delay:0.4s]" />
              </span>
              <span>正在思考…</span>
            </div>
          </div>
        )}

        {error && (
          <div className="flex justify-start py-3">
            <p className="max-w-[85%] text-sm leading-relaxed text-red-600 dark:text-red-400">
              {error}
            </p>
          </div>
        )}
      </div>
    </div>
  )
}
