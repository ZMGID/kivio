import { useEffect, useState } from 'react'
import { FileText, Image } from 'lucide-react'
import { AssistantMessageMeta } from './AssistantMessageMeta'
import { ChatMarkdown } from './ChatMarkdown'
import { ReasoningBlock } from './ReasoningBlock'
import { ToolCallBlock } from './ToolCallBlock'
import type { Attachment, ChatMessage } from './types'

interface MessageBubbleProps {
  message: ChatMessage
  tokensPerSec?: number
  /** 思维链正在流式写入 */
  reasoningStreaming?: boolean
  onUpdateMessage?: (messageId: string, content: string) => Promise<void>
  onRegenerateMessage?: (messageId: string) => Promise<void>
  onDeleteMessage?: (messageId: string) => Promise<void>
}

function AttachmentList({
  attachments,
  variant,
}: {
  attachments: Attachment[]
  variant: 'user' | 'assistant'
}) {
  const baseClass =
    variant === 'user'
      ? 'bg-black/5 text-neutral-700 dark:bg-white/10 dark:text-neutral-200'
      : 'border border-neutral-200/80 text-neutral-700 dark:border-neutral-700 dark:text-neutral-200'

  return (
    <div className="mt-2 space-y-2">
      {attachments.map((attachment) => {
        const Icon = attachment.type === 'image' ? Image : FileText
        return (
          <div
            key={attachment.id}
            className={`flex max-w-full items-center gap-2 rounded-lg p-2 text-sm ${baseClass}`}
            title={attachment.name}
          >
            <Icon size={15} strokeWidth={1.8} className="shrink-0 text-neutral-500" />
            <span className="min-w-0 truncate">{attachment.name}</span>
          </div>
        )
      })}
    </div>
  )
}

export function MessageBubble({
  message,
  tokensPerSec,
  reasoningStreaming = false,
  onUpdateMessage,
  onRegenerateMessage,
  onDeleteMessage,
}: MessageBubbleProps) {
  const isUser = message.role === 'user'
  const canMutate = Boolean(onUpdateMessage && onDeleteMessage && onRegenerateMessage)
  const attachments = message.attachments ?? []
  const toolCalls = message.tool_calls ?? message.toolCalls ?? []
  const [isEditing, setIsEditing] = useState(false)
  const [draft, setDraft] = useState(message.content)
  const [saving, setSaving] = useState(false)

  useEffect(() => {
    setDraft(message.content)
    setIsEditing(false)
  }, [message.id, message.content])

  if (isUser) {
    return (
      <div className="flex justify-end py-2">
        <div className="max-w-[85%] rounded-[20px] bg-neutral-100 px-4 py-2.5 text-neutral-900 dark:bg-neutral-800 dark:text-neutral-100">
          {message.content.trim().length > 0 && (
            <div className="whitespace-pre-wrap break-words text-[15px] leading-relaxed">
              {message.content}
            </div>
          )}
          {attachments.length > 0 && <AttachmentList attachments={attachments} variant="user" />}
        </div>
      </div>
    )
  }

  const handleSaveEdit = async () => {
    const trimmed = draft.trim()
    if (!trimmed || !onUpdateMessage) return
    setSaving(true)
    try {
      await onUpdateMessage(message.id, trimmed)
      setIsEditing(false)
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="flex justify-start py-3">
      <div className="max-w-[85%] min-w-0">
        {toolCalls.length > 0 && !isEditing && (
          <section
            aria-label="工具调用"
            className={message.content.trim().length > 0 || message.reasoning ? 'mb-3' : ''}
          >
            <div className="mb-1 text-[11px] font-medium text-neutral-400 dark:text-neutral-500">
              工具调用
            </div>
            {toolCalls.map((toolCall, index) => (
              <ToolCallBlock
                key={toolCall.id || toolCall.call_id || toolCall.callId || index}
                toolCall={toolCall}
              />
            ))}
          </section>
        )}

        {message.reasoning && !isEditing && (
          <ReasoningBlock reasoning={message.reasoning} streaming={reasoningStreaming} />
        )}

        {isEditing ? (
          <div className="space-y-2">
            <textarea
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              rows={6}
              disabled={saving}
              className="w-full resize-y rounded-xl border border-neutral-200/90 bg-white px-3 py-2.5 text-[15px] leading-relaxed text-neutral-900 outline-none focus:border-neutral-400 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100 dark:focus:border-neutral-500"
            />
            <div className="flex items-center gap-2">
              <button
                type="button"
                disabled={saving || !draft.trim()}
                onClick={() => void handleSaveEdit()}
                className="rounded-lg bg-neutral-900 px-3 py-1.5 text-sm font-medium text-white disabled:opacity-40 dark:bg-neutral-100 dark:text-neutral-900"
              >
                {saving ? '保存中…' : '保存'}
              </button>
              <button
                type="button"
                disabled={saving}
                onClick={() => {
                  setDraft(message.content)
                  setIsEditing(false)
                }}
                className="rounded-lg px-3 py-1.5 text-sm text-neutral-600 hover:bg-neutral-100 disabled:opacity-40 dark:text-neutral-400 dark:hover:bg-neutral-800"
              >
                取消
              </button>
            </div>
          </div>
        ) : (
          message.content.trim().length > 0 && (
            <section aria-label="回答">
              {(toolCalls.length > 0 || message.reasoning) && (
                <div className="mb-1 text-[11px] font-medium text-neutral-400 dark:text-neutral-500">
                  回答
                </div>
              )}
              <ChatMarkdown content={message.content} />
            </section>
          )
        )}

        {!isEditing && message.content.trim().length > 0 && (
          <AssistantMessageMeta
            content={message.content}
            reasoning={message.reasoning}
            timestamp={message.timestamp}
            tokensPerSec={tokensPerSec}
            onEdit={canMutate ? () => setIsEditing(true) : undefined}
            onRegenerate={
              canMutate
                ? () => {
                    void onRegenerateMessage!(message.id)
                  }
                : undefined
            }
            onDelete={
              canMutate
                ? () => {
                    void onDeleteMessage!(message.id)
                  }
                : undefined
            }
          />
        )}

        {attachments.length > 0 && (
          <AttachmentList attachments={attachments} variant="assistant" />
        )}
      </div>
    </div>
  )
}
