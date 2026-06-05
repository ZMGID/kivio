import { memo, useEffect, useState } from 'react'
import { Check, ChevronDown, Copy, Trash2 } from 'lucide-react'
import { copyToClipboard } from '../utils/clipboard'
import { AssistantMessageMeta } from './AssistantMessageMeta'
import { ChatAttachments } from './ChatAttachments'
import { ChatMarkdown } from './ChatMarkdown'
import { ReasoningBlock } from './ReasoningBlock'
import { ToolCallBlock } from './ToolCallBlock'
import { ToolCallErrorBoundary } from './ToolCallErrorBoundary'
import type {
  ChatMessage,
  ChatMixerAggregatorRecord,
  ChatMixerLaneRecord,
  ChatMixerRunRecord,
  ChatMixerStatus,
  ChatToolArtifact,
} from './types'

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

function GeneratedImageArtifacts({ artifacts }: { artifacts: ChatToolArtifact[] }) {
  const imageArtifacts = artifacts.filter((artifact) => {
    const dataUrl = artifactDataUrl(artifact)
    return dataUrl.startsWith('data:image/')
  })
  if (imageArtifacts.length === 0) return null

  return (
    <div className="mt-3 space-y-3">
      {imageArtifacts.map((artifact, index) => (
        <figure key={`${artifact.name}-${index}`} className="m-0">
          <img
            src={artifactDataUrl(artifact)}
            alt={artifact.name || 'Generated image'}
            loading="lazy"
            className="max-h-[420px] max-w-full rounded-md border border-neutral-200/90 bg-white object-contain dark:border-neutral-700 dark:bg-neutral-900"
          />
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

function mixerDurationMs(record: {
  duration_ms?: number | null
  durationMs?: number | null
}): number | null {
  return record.durationMs ?? record.duration_ms ?? null
}

function formatMixerDuration(value: number | null): string {
  if (value == null) return ''
  if (value < 1000) return `${value} ms`
  return `${(value / 1000).toFixed(value < 10_000 ? 1 : 0)} s`
}

function mixerProviderName(record: ChatMixerLaneRecord | ChatMixerAggregatorRecord): string {
  return record.providerName ?? record.provider_name ?? record.providerId ?? record.provider_id ?? ''
}

function mixerStatusLabel(status: ChatMixerStatus): string {
  switch (status) {
    case 'completed':
      return '完成'
    case 'failed':
      return '失败'
    case 'cancelled':
      return '已取消'
    case 'running':
      return '运行中'
    case 'queued':
    default:
      return '等待中'
  }
}

function mixerStatusClass(status: ChatMixerStatus): string {
  switch (status) {
    case 'completed':
      return 'bg-emerald-500/10 text-emerald-700 dark:bg-emerald-400/15 dark:text-emerald-300'
    case 'failed':
      return 'bg-red-500/10 text-red-700 dark:bg-red-400/15 dark:text-red-300'
    case 'cancelled':
      return 'bg-neutral-200/70 text-neutral-600 dark:bg-neutral-800 dark:text-neutral-300'
    case 'running':
      return 'bg-blue-500/10 text-blue-700 dark:bg-blue-400/15 dark:text-blue-300'
    case 'queued':
    default:
      return 'bg-neutral-200/70 text-neutral-600 dark:bg-neutral-800 dark:text-neutral-300'
  }
}

function MixerLaneBlock({ lane }: { lane: ChatMixerLaneRecord }) {
  const providerName = mixerProviderName(lane)
  const duration = formatMixerDuration(mixerDurationMs(lane))
  const content = lane.content?.trim() ?? ''
  const reasoning = lane.reasoning?.trim() ?? ''
  const error = lane.error?.trim() ?? ''

  return (
    <div className="rounded-lg border border-neutral-200 bg-white px-3 py-2 dark:border-neutral-800 dark:bg-neutral-950/40">
      <div className="mb-1.5 flex flex-wrap items-center gap-1.5">
        <span className="min-w-0 truncate text-[12px] font-semibold text-neutral-800 dark:text-neutral-100">
          {lane.label || lane.model}
        </span>
        <span className={`rounded-full px-2 py-0.5 text-[10px] font-medium ${mixerStatusClass(lane.status)}`}>
          {mixerStatusLabel(lane.status)}
        </span>
        {duration && <span className="text-[10px] text-neutral-400 dark:text-neutral-500">{duration}</span>}
      </div>
      <div className="mb-2 truncate text-[11px] text-neutral-500 dark:text-neutral-400">
        {[providerName, lane.model].filter(Boolean).join(' / ')}
      </div>
      {reasoning && (
        <div className="mb-2 rounded-md bg-neutral-100 px-2 py-1 text-[11px] leading-4 text-neutral-600 dark:bg-neutral-800 dark:text-neutral-300">
          {reasoning}
        </div>
      )}
      {content ? (
        <div className="text-[13px] leading-relaxed">
          <ChatMarkdown content={content} />
        </div>
      ) : error ? (
        <div className="text-[12px] leading-5 text-red-600 dark:text-red-300">{error}</div>
      ) : (
        <div className="text-[12px] leading-5 text-neutral-400 dark:text-neutral-500">无输出</div>
      )}
    </div>
  )
}

function MixerAggregatorBlock({ aggregator }: { aggregator: ChatMixerAggregatorRecord }) {
  const providerName = mixerProviderName(aggregator)
  const duration = formatMixerDuration(mixerDurationMs(aggregator))
  const error = aggregator.error?.trim() ?? ''

  return (
    <div className="rounded-lg border border-neutral-200 bg-neutral-50 px-3 py-2 dark:border-neutral-800 dark:bg-neutral-900/70">
      <div className="mb-1 flex flex-wrap items-center gap-1.5">
        <span className="text-[12px] font-semibold text-neutral-800 dark:text-neutral-100">聚合模型</span>
        <span className={`rounded-full px-2 py-0.5 text-[10px] font-medium ${mixerStatusClass(aggregator.status)}`}>
          {mixerStatusLabel(aggregator.status)}
        </span>
        {duration && <span className="text-[10px] text-neutral-400 dark:text-neutral-500">{duration}</span>}
      </div>
      <div className="truncate text-[11px] text-neutral-500 dark:text-neutral-400">
        {[providerName, aggregator.model].filter(Boolean).join(' / ')}
      </div>
      {aggregator.status !== 'completed' && error && (
        <div className="mt-1 text-[12px] leading-5 text-red-600 dark:text-red-300">{error}</div>
      )}
    </div>
  )
}

function MixerRunBlock({ run }: { run: ChatMixerRunRecord }) {
  const [expanded, setExpanded] = useState(false)
  const successCount = run.lanes.filter((lane) => lane.status === 'completed').length
  const failedCount = run.lanes.filter((lane) => lane.status === 'failed').length
  const minSuccessful = run.minSuccessfulLanes ?? run.min_successful_lanes ?? 1
  const duration = formatMixerDuration(mixerDurationMs(run))
  const summary = [
    `${successCount}/${run.lanes.length} lane`,
    `min ${minSuccessful}`,
    duration,
    run.synthesized ? '已合成' : '未合成',
    failedCount > 0 ? `${failedCount} 失败` : '',
  ].filter(Boolean).join(' · ')

  return (
    <section className="mt-3 overflow-hidden rounded-xl border border-neutral-200 bg-neutral-50/70 dark:border-neutral-800 dark:bg-neutral-900/55">
      <button
        type="button"
        onClick={() => setExpanded((value) => !value)}
        className="flex w-full items-center gap-2 px-3 py-2 text-left transition-colors hover:bg-black/[0.03] dark:hover:bg-white/[0.04]"
        aria-expanded={expanded}
        data-tauri-drag-region="false"
      >
        <span className="rounded-full bg-neutral-900 px-2 py-0.5 text-[10px] font-semibold text-white dark:bg-neutral-100 dark:text-neutral-900">
          Mixer
        </span>
        <span className="min-w-0 flex-1 truncate text-[11px] text-neutral-500 dark:text-neutral-400">
          {summary}
        </span>
        <ChevronDown
          size={13}
          strokeWidth={2}
          className={`shrink-0 text-neutral-400 transition-transform duration-300 ${expanded ? 'rotate-180' : ''}`}
        />
      </button>
      <div className={`chat-motion-reveal ${expanded ? 'is-open' : ''}`}>
        <div className="space-y-2 border-t border-neutral-200 px-3 py-3 dark:border-neutral-800">
          {run.aggregator && <MixerAggregatorBlock aggregator={run.aggregator} />}
          {run.lanes.map((lane) => (
            <MixerLaneBlock key={lane.id} lane={lane} />
          ))}
        </div>
      </div>
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
  const mixerRuns = message.mixerRuns ?? message.mixer_runs ?? []
  const toolArtifacts = toolCalls.flatMap((toolCall) => toolCall.artifacts ?? [])
  const unreferencedToolArtifacts = toolArtifacts.filter(
    (artifact) => !artifactIsReferenced(message.content, artifact),
  )
  const hasAnswerContent = message.content.trim().length > 0
  const hasGeneratedImages = unreferencedToolArtifacts.length > 0
  const [isEditing, setIsEditing] = useState(false)
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
      <div className="max-w-[85%] min-w-0">
        {toolCalls.length > 0 && !isEditing && (
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
          (hasAnswerContent || hasGeneratedImages) && (
            <section aria-label="回答">
              {(toolCalls.length > 0 || message.reasoning) && (
                <div className="mb-1 text-[11px] font-medium text-neutral-400 dark:text-neutral-500">
                  回答
                </div>
              )}
              {hasAnswerContent && (
                <ChatMarkdown content={message.content} artifacts={toolArtifacts} />
              )}
              {hasGeneratedImages && (
                <GeneratedImageArtifacts artifacts={unreferencedToolArtifacts} />
              )}
            </section>
          )
        )}

        {!isEditing && mixerRuns.length > 0 && (
          <div>
            {mixerRuns.map((run) => (
              <MixerRunBlock key={run.id} run={run} />
            ))}
          </div>
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
