import { useEffect, useRef } from 'react'
import type { ChatMessage } from './types'
import { MessageBubble } from './MessageBubble'

interface MessageListProps {
  messages: ChatMessage[]
  streaming?: boolean
  streamingContent?: string
  streamingReasoning?: string
  error?: string
}

export function MessageList({
  messages,
  streaming,
  streamingContent = '',
  streamingReasoning = '',
  error,
}: MessageListProps) {
  const scrollRef = useRef<HTMLDivElement>(null)

  // 自动滚动到底部
  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight
    }
  }, [messages, streaming, streamingContent, streamingReasoning, error])

  if (messages.length === 0 && !streaming && !error) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <div className="text-center">
          <h2 className="text-3xl font-medium text-neutral-900 dark:text-neutral-100 mb-8">
            今天我能为您做些什么？
          </h2>
        </div>
      </div>
    )
  }

  return (
    <div ref={scrollRef} className="flex-1 overflow-y-auto px-6 py-4">
      {messages.map((msg) => (
        <MessageBubble key={msg.id} message={msg} />
      ))}

      {/* 流式加载指示器 */}
      {streaming && (streamingContent || streamingReasoning) && (
        <MessageBubble
          message={{
            id: 'streaming-assistant',
            role: 'assistant',
            content: streamingContent,
            reasoning: streamingReasoning || undefined,
            timestamp: Math.floor(Date.now() / 1000),
          }}
        />
      )}

      {streaming && !streamingContent && !streamingReasoning && (
        <div className="flex justify-start mb-4">
          <div className="bg-neutral-100 dark:bg-neutral-800 rounded-2xl px-4 py-3">
            <div className="flex items-center gap-2">
              <span className="flex gap-1">
                <span className="w-2 h-2 rounded-full bg-neutral-400 animate-pulse" />
                <span className="w-2 h-2 rounded-full bg-neutral-400 animate-pulse [animation-delay:0.2s]" />
                <span className="w-2 h-2 rounded-full bg-neutral-400 animate-pulse [animation-delay:0.4s]" />
              </span>
              <span className="text-sm text-neutral-500">正在思考...</span>
            </div>
          </div>
        </div>
      )}

      {error && (
        <div className="flex justify-start mb-4">
          <div className="max-w-[70%] rounded-2xl px-4 py-3 bg-red-50 dark:bg-red-950/30 text-red-700 dark:text-red-300 border border-red-100 dark:border-red-900/50">
            <div className="text-sm leading-relaxed whitespace-pre-wrap break-words">{error}</div>
          </div>
        </div>
      )}
    </div>
  )
}
