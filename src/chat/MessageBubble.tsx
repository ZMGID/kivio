import { memo, useEffect, useState } from 'react'
import { AlertCircle, Check, ChevronDown, Copy, Trash2 } from 'lucide-react'
import { copyToClipboard } from '../utils/clipboard'
import { AssistantMessageMeta } from './AssistantMessageMeta'
import { ChatAttachments } from './ChatAttachments'
import { ChatDotGridBackground } from './ChatDotGridBackground'
import { ChatMarkdown } from './ChatMarkdown'
import { openChatImageViewer } from './imageViewer'
import { ReasoningBlock } from './ReasoningBlock'
import { ToolCallBlock } from './ToolCallBlock'
import { ToolCallErrorBoundary } from './ToolCallErrorBoundary'
import type { ChatMessage, ChatMessageSegment, ChatToolArtifact, ToolCallRecord } from './types'
import { compareTimelineSegments, segmentToolCallId } from './segments'

const DIRECT_IMAGE_GENERATION_PENDING = '[[KIVIO_DIRECT_IMAGE_GENERATION_PENDING]]'

interface MessageBubbleProps {
  message: ChatMessage
  conversationId?: string | null
  tokensPerSec?: number
  /** 思维链正在流式写入 */
  reasoningStreaming?: boolean
  onUpdateMessage?: (messageId: string, content: string) => Promise<void>
  onRegenerateMessage?: (messageId: string) => Promise<void>
  onDeleteMessage?: (messageId: string) => Promise<void>
}

function artifactDataUrl(artifact: ChatToolArtifact): string {
  return artifact.dataUrl ?? artifact.data_url ?? ''
}

function markdownImageSources(content: string): Set<string> {
  const sources = new Set<string>()
  for (const match of content.matchAll(/!\[[^\]]*]\(([^)\s]+)(?:\s+"[^"]*")?\)/g)) {
    sources.add(match[1].trim().toLowerCase())
  }
  return sources
}

function artifactDisplayKey(name: string): string {
  try {
    return decodeURIComponent(name).trim().replace(/^\.?\//, '').replace(/\\/g, '/').toLowerCase()
  } catch {
    return name.trim().replace(/^\.?\//, '').replace(/\\/g, '/').toLowerCase()
  }
}

function artifactIsReferenced(content: string, artifact: ChatToolArtifact): boolean {
  const sources = markdownImageSources(content)
  if (sources.size === 0) return false
  const dataUrl = artifactDataUrl(artifact)
  if (dataUrl && content.includes(dataUrl)) return true
  const name = artifactDisplayKey(artifact.name)
  const basename = name.split('/').filter(Boolean).pop() ?? name
  for (const source of sources) {
    const normalizedSource = artifactDisplayKey(source)
    if (normalizedSource === name || normalizedSource === basename) {
      return true
    }
  }
  return false
}

function GeneratedImageArtifacts({
  artifacts,
  onImageClick,
}: {
  artifacts: ChatToolArtifact[]
  onImageClick?: (src: string, alt: string, name?: string) => void
}) {
  const imageArtifacts = artifacts.filter((artifact) => {
    const dataUrl = artifactDataUrl(artifact)
    return dataUrl.startsWith('data:image/')
  })
  if (imageArtifacts.length === 0) return null

  return (
    <div className="mt-3 space-y-3">
      {imageArtifacts.map((artifact, index) => (
        <figure key={`${artifact.name}-${index}`} className="m-0">
          <button
            type="button"
            className="block max-w-full cursor-zoom-in rounded-md p-0 text-left"
            onClick={() => onImageClick?.(artifactDataUrl(artifact), artifact.name || 'Generated image', artifact.name)}
            aria-label="预览图片"
          >
            <img
              src={artifactDataUrl(artifact)}
              alt={artifact.name || 'Generated image'}
              loading="lazy"
              className="max-h-[420px] max-w-full rounded-md border border-neutral-200/90 bg-white object-contain dark:border-neutral-700 dark:bg-neutral-900"
            />
          </button>
          {artifact.name && (
            <figcaption className="mt-1 text-[11px] text-neutral-400 dark:text-neutral-500">
              {artifact.name}
            </figcaption>
          )}
        </figure>
      ))}
    </div>
  )
}

function ImageGenerationPending() {
  return (
    <section aria-label="图片生成中" className="image-generation-pending">
      <div className="mb-3">
        <div className="flex items-center gap-2 text-[14px] font-medium leading-5 text-neutral-700 dark:text-neutral-300">
          <span className="image-generation-pending-indicator" aria-hidden="true" />
          <span>正在生成图片</span>
        </div>
        <div className="mt-1 pl-4 text-[12px] leading-5 text-neutral-400 dark:text-neutral-500">
          正在细化画面细节，请稍候。
        </div>
      </div>
      <div className="image-generation-pending-frame" aria-hidden="true">
        <ChatDotGridBackground />
      </div>
    </section>
  )
}

function toolRecordId(toolCall: ToolCallRecord): string {
  return toolCall.id || toolCall.toolCallId || toolCall.call_id || toolCall.callId || ''
}

function orderedSegments(segments?: ChatMessageSegment[]): ChatMessageSegment[] {
  return [...(segments ?? [])].sort(compareTimelineSegments)
}

function segmentText(segment: ChatMessageSegment): string {
  return segment.text ?? ''
}

function MissingToolSegment({ toolCallId }: { toolCallId: string }) {
  return (
    <div className="not-prose mb-2 inline-flex max-w-full items-center gap-1.5 rounded-md py-0.5 text-[11.5px] leading-5 text-neutral-400 dark:text-neutral-500">
      <AlertCircle size={12} strokeWidth={1.9} className="shrink-0" />
      <span className="truncate">工具记录缺失{toolCallId ? ` · ${toolCallId}` : ''}</span>
    </div>
  )
}

function TimelineToolSegment({
  segment,
  toolCalls,
}: {
  segment: ChatMessageSegment
  toolCalls: ToolCallRecord[]
}) {
  const toolCallId = segmentToolCallId(segment)
  const toolCall = toolCalls.find((record) => toolRecordId(record) === toolCallId)
  if (!toolCall) {
    return <MissingToolSegment toolCallId={toolCallId} />
  }
  return (
    <ToolCallErrorBoundary>
      <ToolCallBlock toolCall={toolCall} />
    </ToolCallErrorBoundary>
  )
}

function TimelineTextSegment({
  segment,
  artifacts,
}: {
  segment: ChatMessageSegment
  artifacts: ChatToolArtifact[]
}) {
  const text = segmentText(segment).trim()
  if (!text) return null
  const isProcessText = segment.phase === 'tool_loop' || segment.phase === 'auxiliary'
  return (
    <div className={isProcessText ? 'text-neutral-600 dark:text-neutral-300' : undefined}>
      <ChatMarkdown
        content={text}
        artifacts={artifacts}
        onImageClick={(src, alt, name) => openChatImageViewer({ src, alt, name })}
      />
    </div>
  )
}

function TimelineSegments({
  segments,
  toolCalls,
  artifacts,
  reasoningStreaming,
}: {
  segments: ChatMessageSegment[]
  toolCalls: ToolCallRecord[]
  artifacts: ChatToolArtifact[]
  reasoningStreaming: boolean
}) {
  return (
    <section aria-label="回答时间线" className="space-y-1.5">
      {orderedSegments(segments).map((segment) => {
        if (segment.kind === 'tool') {
          return (
            <TimelineToolSegment
              key={segment.id}
              segment={segment}
              toolCalls={toolCalls}
            />
          )
        }
        if (segment.kind === 'reasoning') {
          const reasoning = segmentText(segment)
          if (!reasoning.trim()) return null
          return (
            <ReasoningBlock
              key={segment.id}
              reasoning={reasoning}
              streaming={reasoningStreaming}
            />
          )
        }
        return (
          <TimelineTextSegment
            key={segment.id}
            segment={segment}
            artifacts={artifacts}
          />
        )
      })}
    </section>
  )
}

function MessageBubbleComponent({
  message,
  conversationId,
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
  const timelineSegments = orderedSegments(message.segments)
  const hasTimelineSegments = !isEditing && timelineSegments.length > 0
  const messageArtifacts = message.artifacts ?? []
  const toolArtifacts = toolCalls.flatMap((toolCall) => toolCall.artifacts ?? [])
  const renderArtifacts = [...messageArtifacts, ...toolArtifacts]
  const isDirectImageGenerationPending =
    !isUser && message.content.trim() === DIRECT_IMAGE_GENERATION_PENDING
  const artifactReferenceContent = [
    message.content,
    ...timelineSegments.map((segment) => segmentText(segment)),
  ].join('\n\n')
  const unreferencedImageArtifacts = renderArtifacts.filter(
    (artifact) => !artifactIsReferenced(artifactReferenceContent, artifact),
  )
  const hasAnswerContent = !isDirectImageGenerationPending && message.content.trim().length > 0
  const hasGeneratedImages = unreferencedImageArtifacts.length > 0
  const [draft, setDraft] = useState(message.content)
  const [saving, setSaving] = useState(false)
  const [copied, setCopied] = useState(false)
  const [toolsExpanded, setToolsExpanded] = useState(false)
  // 工具调用超过 4 个时默认折叠（与思考过程一致）
  const toolsCollapsible = toolCalls.length > 4

  useEffect(() => {
    setDraft(message.content)
    setIsEditing(false)
  }, [message.id, message.content])

  const handleCopy = async () => {
    const ok = await copyToClipboard(message.content)
    if (!ok) return
    setCopied(true)
    window.setTimeout(() => setCopied(false), 2000)
  }

  const bubbleActionBtn =
    'rounded p-1 text-neutral-400 transition-colors hover:bg-neutral-100 hover:text-neutral-600 disabled:cursor-not-allowed disabled:opacity-40 dark:hover:bg-neutral-800 dark:hover:text-neutral-300'

  if (isUser) {
    const hasText = message.content.trim().length > 0
    return (
      <div className="group chat-motion-fade-up flex justify-end py-2">
        <div className="flex max-w-[85%] flex-col items-end gap-1">
          {attachments.length > 0 && (
            <ChatAttachments
              attachments={attachments}
              conversationId={conversationId}
              variant="user"
            />
          )}
          {hasText && (
            <div className="rounded-[20px] bg-neutral-100 px-4 py-2.5 text-neutral-900 dark:bg-neutral-800 dark:text-neutral-100">
              <div className="whitespace-pre-wrap break-words text-[15px] leading-relaxed">
                {message.content}
              </div>
            </div>
          )}
          {hasText && (
            <div className="flex items-center gap-0.5 pr-0.5 opacity-0 transition-opacity duration-150 focus-within:opacity-100 group-hover:opacity-100">
              <button
                type="button"
                onClick={() => void handleCopy()}
                className={bubbleActionBtn}
                title={copied ? '已复制' : '复制'}
                aria-label={copied ? '已复制' : '复制'}
              >
                {copied ? <Check size={14} strokeWidth={2} /> : <Copy size={14} strokeWidth={2} />}
              </button>
              {onDeleteMessage && (
                <button
                  type="button"
                  onClick={() => void onDeleteMessage(message.id)}
                  className={bubbleActionBtn}
                  title="删除"
                  aria-label="删除"
                >
                  <Trash2 size={14} strokeWidth={2} />
                </button>
              )}
            </div>
          )}
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

  const toolList = toolCalls.map((toolCall, index) => (
    <ToolCallErrorBoundary key={toolCall.id || toolCall.call_id || toolCall.callId || index}>
      <ToolCallBlock toolCall={toolCall} />
    </ToolCallErrorBoundary>
  ))
  // 折叠时仅隐藏较早的，始终保留最新 4 个可见
  const RECENT_TOOL_COUNT = 4
  const olderTools = toolsCollapsible ? toolList.slice(0, toolList.length - RECENT_TOOL_COUNT) : []
  const recentTools = toolsCollapsible ? toolList.slice(toolList.length - RECENT_TOOL_COUNT) : toolList

  return (
    <div className="chat-motion-fade-up flex justify-start py-3">
      <div className="w-full min-w-0">
        {toolCalls.length > 0 && !isEditing && !hasTimelineSegments && (
          <section
            aria-label="工具调用"
            className={message.content.trim().length > 0 || message.reasoning ? 'mb-3' : ''}
          >
            {toolsCollapsible ? (
              <button
                type="button"
                onClick={() => setToolsExpanded((value) => !value)}
                className="mb-1 flex w-full items-center gap-1 text-left text-[11px] font-medium text-neutral-400 transition-colors hover:text-neutral-600 dark:text-neutral-500 dark:hover:text-neutral-300"
                aria-expanded={toolsExpanded}
                data-tauri-drag-region="false"
              >
                <span>
                  工具调用 · {toolCalls.length} 个
                  {!toolsExpanded ? ` · 显示最新 ${RECENT_TOOL_COUNT} 个` : ''}
                </span>
                <ChevronDown
                  size={12}
                  strokeWidth={2}
                  className={`ml-auto shrink-0 transition-transform duration-300 ${toolsExpanded ? 'rotate-180' : ''}`}
                />
              </button>
            ) : (
              <div className="mb-1 text-[11px] font-medium text-neutral-400 dark:text-neutral-500">
                工具调用
              </div>
            )}
            {toolsCollapsible && (
              <div className={`chat-motion-reveal ${toolsExpanded ? 'is-open' : ''}`}>
                <div>{olderTools}</div>
              </div>
            )}
            {recentTools}
          </section>
        )}

        {message.reasoning && !isEditing && !hasTimelineSegments && (
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
        ) : isDirectImageGenerationPending ? (
          <ImageGenerationPending />
        ) : hasTimelineSegments ? (
          <>
            <TimelineSegments
              segments={timelineSegments}
              toolCalls={toolCalls}
              artifacts={renderArtifacts}
              reasoningStreaming={reasoningStreaming}
            />
            {hasGeneratedImages && (
              <GeneratedImageArtifacts
                artifacts={unreferencedImageArtifacts}
                onImageClick={(src, alt, name) => openChatImageViewer({ src, alt, name })}
              />
            )}
          </>
        ) : (
          (hasAnswerContent || hasGeneratedImages) && (
            <section aria-label="回答">
              {(toolCalls.length > 0 || message.reasoning) && (
                <div className="mb-1 text-[11px] font-medium text-neutral-400 dark:text-neutral-500">
                  回答
                </div>
              )}
              {hasAnswerContent && (
                <ChatMarkdown
                  content={message.content}
                  artifacts={renderArtifacts}
                  onImageClick={(src, alt, name) => openChatImageViewer({ src, alt, name })}
                />
              )}
              {hasGeneratedImages && (
                <GeneratedImageArtifacts
                  artifacts={unreferencedImageArtifacts}
                  onImageClick={(src, alt, name) => openChatImageViewer({ src, alt, name })}
                />
              )}
            </section>
          )
        )}

        {!isEditing && message.content.trim().length > 0 && !isDirectImageGenerationPending && (
          <AssistantMessageMeta
            content={message.content}
            reasoning={message.reasoning}
            timestamp={message.timestamp}
            tokensPerSec={tokensPerSec}
            runEntry={message.run_entry ?? message.runEntry}
            streamOutcome={message.stream_outcome ?? message.streamOutcome}
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
          <ChatAttachments
            attachments={attachments}
            conversationId={conversationId}
            variant="assistant"
          />
        )}
      </div>
    </div>
  )
}

// memo：流式生成时历史消息 props 不变 → 跳过重渲染，避免每个 token 重新解析 Markdown
export const MessageBubble = memo(MessageBubbleComponent)
