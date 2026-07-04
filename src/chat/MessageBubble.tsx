import { memo, useEffect, useRef, useState } from 'react'
import {
  AlertCircle,
  Bot,
  Brain,
  Check,
  ChevronDown,
  Copy,
  FileCode2,
  FilePen,
  FileSearch,
  FileText,
  FolderInput,
  FolderOpen,
  Globe,
  ImagePlus,
  ListChecks,
  Pencil,
  Play,
  Plug,
  ScrollText,
  Search,
  SquareTerminal,
  Trash2,
} from 'lucide-react'
import type { LucideIcon } from 'lucide-react'
import { copyToClipboard } from '../utils/clipboard'
import { AssistantMessageMeta } from './AssistantMessageMeta'
import { ChatAttachments } from './ChatAttachments'
import { ChatDotGridBackground } from './ChatDotGridBackground'
import { ChatMarkdown } from './ChatMarkdown'
import { GeneratedFileArtifacts } from './GeneratedFileArtifacts'
import { isExecutableAgentPlanText } from './agentPlan'
import { isImageArtifact } from './artifacts'
import { loadArtifactDataUrl } from './attachmentPreview'
import { openChatImageViewer } from './imageViewer'
import { ReasoningBlock } from './ReasoningBlock'
import { ModelIcon } from './ModelIcon'
import { ToolCallBlock } from './ToolCallBlock'
import { ToolCallErrorBoundary } from './ToolCallErrorBoundary'
import type { AgentPlanState, ChatMessage, ChatMessageSegment, ChatToolArtifact, ToolCallRecord } from './types'
import { knowledgeSearchHits, type KbHitView } from './knowledgeBaseHits'
import { compareTimelineSegments, groupTimelineSegments, segmentToolCallId, summarizeToolGroup, toolRecordId } from './segments'
import type { TimelineGroupItem, ToolGroupIcon } from './segments'

const DIRECT_IMAGE_GENERATION_PENDING = '[[KIVIO_DIRECT_IMAGE_GENERATION_PENDING]]'

// 模組級穩定引用：內聯箭頭每次渲染新建會打穿 ChatMarkdown 的 memo（導致公式重渲）。
const handleChatImageClick = (src: string, alt: string, name?: string) =>
  openChatImageViewer({ src, alt, name })

interface MessageBubbleProps {
  message: ChatMessage
  conversationId?: string | null
  tokensPerSec?: number
  reasoningDurationMs?: number | null
  reasoningDurationMsBySegmentId?: Record<string, number>
  /** 思維鏈正在流式寫入 */
  reasoningStreaming?: boolean
  /** 這條訊息整體是否在流式生成中（僅 streaming-assistant bubble 為 true） */
  messageStreaming?: boolean
  /** R8（多模型一問多答）：本條 user 訊息這一問發給了哪些模型；多模型時渲染在氣泡頂部。 */
  sentModels?: { providerId: string | null; model: string | null }[]
  onUpdateMessage?: (messageId: string, content: string) => Promise<void>
  onRegenerateMessage?: (messageId: string, newContent?: string) => Promise<void>
  onDeleteMessage?: (messageId: string) => Promise<void>
  agentPlanOverride?: AgentPlanState | null
  onExecuteAgentPlan?: (messageId: string) => Promise<void> | void
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

function ArtifactImage({
  artifact,
  conversationId,
}: {
  artifact: ChatToolArtifact
  conversationId?: string | null
}) {
  const inline = artifactDataUrl(artifact)
  const [src, setSrc] = useState<string>(inline)

  // 正常情況下 data_url 是內聯縮圖(秒顯);僅當縮圖生成失敗為空時,用 path 懶載入原圖兜底。
  useEffect(() => {
    if (inline) {
      setSrc(inline)
      return
    }
    if (!artifact.path) return
    let cancelled = false
    void loadArtifactDataUrl(artifact, conversationId).then((loaded) => {
      if (!cancelled && loaded) setSrc(loaded)
    })
    return () => {
      cancelled = true
    }
  }, [inline, artifact, conversationId])

  if (!src) return null
  const name = artifact.name || 'Generated image'
  return (
    <figure className="m-0">
      <button
        type="button"
        className="block max-w-full cursor-zoom-in rounded-md p-0 text-left"
        onClick={() =>
          openChatImageViewer({
            src,
            alt: name,
            name: artifact.name,
            path: artifact.path,
            conversationId,
          })
        }
        aria-label="預覽圖片"
      >
        <img
          src={src}
          alt={name}
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
  )
}

function GeneratedImageArtifacts({
  artifacts,
  conversationId,
}: {
  artifacts: ChatToolArtifact[]
  conversationId?: string | null
}) {
  const imageArtifacts = artifacts.filter(isImageArtifact)
  if (imageArtifacts.length === 0) return null

  return (
    <div className="mt-3 space-y-3">
      {imageArtifacts.map((artifact, index) => (
        <ArtifactImage
          key={`${artifact.name}-${index}`}
          artifact={artifact}
          conversationId={conversationId}
        />
      ))}
    </div>
  )
}

function ImageGenerationPending() {
  return (
    <section aria-label="圖片生成中" className="image-generation-pending">
      <div className="mb-3">
        <div className="flex items-center gap-2 text-[14px] font-medium leading-5 text-neutral-700 dark:text-neutral-300">
          <span className="image-generation-pending-indicator" aria-hidden="true" />
          <span>正在生成圖片</span>
        </div>
        <div className="mt-1 pl-4 text-[12px] leading-5 text-neutral-400 dark:text-neutral-500">
          正在細化畫面細節，請稍候。
        </div>
      </div>
      <div className="image-generation-pending-frame" aria-hidden="true">
        <ChatDotGridBackground />
      </div>
    </section>
  )
}

function AgentPlanAction({
  messageId,
  planState,
  disabled,
  onExecute,
}: {
  messageId: string
  planState?: AgentPlanState | null
  disabled?: boolean
  onExecute?: (messageId: string) => Promise<void> | void
}) {
  const plan = planState?.plan?.trim() ?? ''
  if (!isExecutableAgentPlanText(plan)) return null

  const approved = (planState?.status ?? 'draft') === 'approved'
  return (
    <div className="not-prose mt-3 flex max-w-full items-center gap-2 border-l-2 border-emerald-400/70 pl-3 text-[12px] leading-5 text-neutral-500 dark:border-emerald-500/60 dark:text-neutral-400">
      <ListChecks size={14} strokeWidth={2} className="shrink-0 text-emerald-600 dark:text-emerald-400" />
      <span className="min-w-0 flex-1 truncate">{approved ? '已按這條計畫執行' : '計畫草案'}</span>
      {!approved && onExecute && (
        <button
          type="button"
          onClick={() => void onExecute(messageId)}
          disabled={disabled}
          className="inline-flex h-7 shrink-0 items-center gap-1.5 rounded-full bg-neutral-900 px-2.5 text-[12px] font-medium text-white transition-colors hover:bg-neutral-700 disabled:bg-neutral-200 disabled:text-neutral-400 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200 dark:disabled:bg-neutral-700 dark:disabled:text-neutral-500"
          title="執行這條計畫"
          aria-label="執行這條計畫"
        >
          <Play size={12} strokeWidth={2.2} fill="currentColor" />
          執行這條計畫
        </button>
      )}
    </div>
  )
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
      <span className="truncate">工具記錄缺失{toolCallId ? ` · ${toolCallId}` : ''}</span>
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
  citations,
}: {
  segment: ChatMessageSegment
  artifacts: ChatToolArtifact[]
  citations?: Map<number, KbHitView>
}) {
  const text = segmentText(segment).trim()
  if (!text) return null
  const isProcessText = segment.phase === 'tool_loop' || segment.phase === 'auxiliary'
  return (
    <div className={isProcessText ? 'text-neutral-600 dark:text-neutral-300' : undefined}>
      <ChatMarkdown
        content={text}
        artifacts={artifacts}
        citations={citations}
        onImageClick={handleChatImageClick}
      />
    </div>
  )
}

function TimelineSegmentNode({
  segment,
  index,
  segmentCount,
  toolCalls,
  artifacts,
  citations,
  reasoningStreaming,
  reasoningDurationMs,
  reasoningDurationMsBySegmentId,
  reasoningSegmentCount,
}: {
  segment: ChatMessageSegment
  index: number
  segmentCount: number
  toolCalls: ToolCallRecord[]
  artifacts: ChatToolArtifact[]
  citations?: Map<number, KbHitView>
  reasoningStreaming: boolean
  reasoningDurationMs?: number | null
  reasoningDurationMsBySegmentId?: Record<string, number>
  reasoningSegmentCount: number
}) {
  if (segment.kind === 'tool') {
    return <TimelineToolSegment segment={segment} toolCalls={toolCalls} />
  }
  if (segment.kind === 'reasoning') {
    const reasoning = segmentText(segment)
    if (!reasoning.trim()) return null
    return (
      <ReasoningBlock
        reasoning={reasoning}
        streaming={reasoningStreaming && index === segmentCount - 1}
        durationMs={
          reasoningDurationMsBySegmentId?.[segment.id]
            ?? (reasoningSegmentCount === 1 ? reasoningDurationMs : null)
        }
      />
    )
  }
  if (!segmentText(segment).trim()) return null
  return <TimelineTextSegment segment={segment} artifacts={artifacts} citations={citations} />
}

function TimelineStepsIcon({ size = 16, className }: { size?: number; className?: string }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.8}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      aria-hidden="true"
    >
      <circle cx="3.5" cy="4" r="1.6" />
      <circle cx="3.5" cy="12" r="1.6" />
      <path d="M3.5 5.6v4.8" />
      <path d="M8 4h5" />
      <path d="M8 12h3.5" />
    </svg>
  )
}

/** macOS 經典放射狀短線 spinner：8 根短線繞中心放射、透明度階梯遞增，整體步進旋轉。 */
function TimelineSpinner({ size = 16, className }: { size?: number; className?: string }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      className={className}
      aria-hidden="true"
    >
      <g className="kv-tick-spinner" stroke="currentColor" strokeWidth={1.8} strokeLinecap="round">
        {Array.from({ length: 8 }).map((_, i) => (
          <line
            key={i}
            x1="12"
            y1="3.5"
            x2="12"
            y2="7"
            transform={`rotate(${i * 45} 12 12)`}
            opacity={(i + 1) / 8}
          />
        ))}
      </g>
    </svg>
  )
}

/**
 * 分組頭摺疊態圖示：按摘要代表類別選 lucide 圖示，與 ToolCallBlock 的單工具圖示觀感一致。
 * `other`（通用/混合兜底）保留自繪 TimelineStepsIcon。
 */
const GROUP_ICON_BY_CATEGORY: Record<
  ToolGroupIcon,
  LucideIcon | typeof TimelineStepsIcon
> = {
  read: FileText,
  codeSearch: Search,
  globFiles: FileSearch,
  fileWrite: FilePen,
  runCommand: SquareTerminal,
  webFetch: Globe,
  webSearch: Search,
  runPython: FileCode2,
  listDir: FolderOpen,
  fileOps: FolderInput,
  todo: ListChecks,
  memory: Brain,
  subAgent: Bot,
  skill: ScrollText,
  image: ImagePlus,
  notion: Plug,
  mcp: Plug,
  reasoning: Brain,
  other: TimelineStepsIcon,
}

/**
 * 一組「連續的 thinking + tool 段」= 單一可摺疊單元。
 * - 「生成中」= 這條訊息還在流式生成、且這是末組（messageStreaming && isLastGroup）：
 *   始終保持展開，不受工具間隙/reasoning 是否在流影響，避免抖動。
 * - 後面出現正文/別的塊（非末組）或訊息流式結束（含歷史訊息）→ 摺疊成一行摘要。
 * - 使用者手動點過開關後以使用者操作為準（userToggledRef，參考 ReasoningBlock）。
 * - 歷史摺疊態只保留摘要 header，不掛載組內 ReasoningBlock / ToolCallBlock；
 *   展開後再原樣平鋪，避免重歷史訊息預設掛滿工具/Markdown/Diff 子樹。
 */
function TimelineGroupBlock({
  segments,
  toolCalls,
  artifacts,
  citations,
  isLastGroup,
  messageStreaming,
  reasoningStreaming,
  reasoningDurationMs,
  reasoningDurationMsBySegmentId,
  reasoningSegmentCount,
}: {
  segments: ChatMessageSegment[]
  toolCalls: ToolCallRecord[]
  artifacts: ChatToolArtifact[]
  citations?: Map<number, KbHitView>
  isLastGroup: boolean
  messageStreaming: boolean
  reasoningStreaming: boolean
  reasoningDurationMs?: number | null
  reasoningDurationMsBySegmentId?: Record<string, number>
  reasoningSegmentCount: number
}) {
  const generating = messageStreaming && isLastGroup
  const summary = summarizeToolGroup(segments, toolCalls)
  const SummaryIcon = GROUP_ICON_BY_CATEGORY[summary.icon]
  const [open, setOpen] = useState(generating)
  const userToggledRef = useRef(false)

  // 生成中預設展開、完成自動摺疊；使用者手動操作後不再覆蓋。
  useEffect(() => {
    if (userToggledRef.current) return
    setOpen(generating)
  }, [generating])

  const handleToggle = () => {
    userToggledRef.current = true
    setOpen((value) => !value)
  }

  return (
    <section aria-label="過程分組" className="not-prose">
      <button
        type="button"
        onClick={handleToggle}
        aria-expanded={open}
        data-tauri-drag-region="false"
        className="mb-1 flex w-full items-center gap-1.5 text-left text-[15px] leading-relaxed font-medium text-neutral-400 transition-colors hover:text-neutral-600 dark:text-neutral-500 dark:hover:text-neutral-300"
      >
        {generating ? (
          <TimelineSpinner size={16} className="shrink-0 text-neutral-400 dark:text-neutral-500" />
        ) : (
          <SummaryIcon size={16} className="shrink-0" />
        )}
        <div className="flex min-w-0 items-center gap-1.5">
          <span
            className={`min-w-0 truncate ${
              generating ? 'chat-motion-tool-shimmer' : ''
            }`}
          >
            {summary.text}
          </span>
          {summary.categories.length > 1 && (
            <span className="flex shrink-0 items-center gap-1" aria-hidden="true">
              {summary.categories.map((category) => {
                const CategoryIcon = GROUP_ICON_BY_CATEGORY[category]
                return <CategoryIcon key={category} size={14} />
              })}
            </span>
          )}
        </div>
        <ChevronDown
          size={16}
          strokeWidth={2}
          className={`shrink-0 transition-transform duration-300 ${open ? 'rotate-180' : ''}`}
        />
      </button>
      {open && (
        <div className="chat-motion-reveal is-open" aria-hidden={false}>
          <div className="space-y-1.5">
            {segments.map((segment, index) => (
              <div key={segment.id} className="chat-motion-fade">
                <TimelineSegmentNode
                  segment={segment}
                  index={index}
                  segmentCount={segments.length}
                  toolCalls={toolCalls}
                  artifacts={artifacts}
                  citations={citations}
                  reasoningStreaming={reasoningStreaming && isLastGroup}
                  reasoningDurationMs={reasoningDurationMs}
                  reasoningDurationMsBySegmentId={reasoningDurationMsBySegmentId}
                  reasoningSegmentCount={reasoningSegmentCount}
                />
              </div>
            ))}
          </div>
        </div>
      )}
    </section>
  )
}

/** 彙總本條訊息所有 knowledge_search 命中，按 n 建索引，供答案裡的 `[n]` 角標查源。
 *  多次檢索 n 會重疊 —— 後寫覆蓋（罕見，且隻影響彈窗預覽內容）。 */
function buildCitationMap(toolCalls: ToolCallRecord[]): Map<number, KbHitView> {
  const map = new Map<number, KbHitView>()
  for (const toolCall of toolCalls) {
    for (const hit of knowledgeSearchHits(toolCall) ?? []) {
      map.set(hit.n, hit)
    }
  }
  return map
}

function TimelineSegments({
  segments,
  toolCalls,
  artifacts,
  messageStreaming,
  reasoningStreaming,
  reasoningDurationMs,
  reasoningDurationMsBySegmentId,
}: {
  segments: ChatMessageSegment[]
  toolCalls: ToolCallRecord[]
  artifacts: ChatToolArtifact[]
  messageStreaming: boolean
  reasoningStreaming: boolean
  reasoningDurationMs?: number | null
  reasoningDurationMsBySegmentId?: Record<string, number>
}) {
  const ordered = orderedSegments(segments)
  const citations = buildCitationMap(toolCalls)
  const reasoningSegmentCount = ordered.filter((segment) => segment.kind === 'reasoning').length
  const referencedToolIds = new Set(
    ordered
      .filter((segment) => segment.kind === 'tool')
      .map((segment) => segmentToolCallId(segment))
      .filter(Boolean),
  )
  const orphanTools = toolCalls
    .filter((toolCall) => {
      const id = toolRecordId(toolCall)
      return id && !referencedToolIds.has(id)
    })
    .sort((left, right) => {
      const leftStarted = left.startedAt ?? left.started_at ?? 0
      const rightStarted = right.startedAt ?? right.started_at ?? 0
      return leftStarted - rightStarted
    })

  const groupItems = groupTimelineSegments(ordered)
  const lastGroupIndex = groupItems.reduce(
    (last, item, index) => (item.type === 'group' ? index : last),
    -1,
  )

  return (
    <section aria-label="回答時間線" className="space-y-1.5">
      {groupItems.map((item: TimelineGroupItem, index) => {
        if (item.type === 'text') {
          if (!segmentText(item.segment).trim()) return null
          // 每個時間線分段單獨淡入：流式中新分段順次出現而非"啪"地彈出。
          return (
            <div key={item.segment.id} className="chat-motion-fade">
              <TimelineTextSegment segment={item.segment} artifacts={artifacts} citations={citations} />
            </div>
          )
        }
        const groupKey = item.segments[0]?.id ?? `group-${index}`
        return (
          <div key={groupKey} className="chat-motion-fade">
            <TimelineGroupBlock
              segments={item.segments}
              toolCalls={toolCalls}
              artifacts={artifacts}
              citations={citations}
              isLastGroup={index === lastGroupIndex}
              messageStreaming={messageStreaming}
              reasoningStreaming={reasoningStreaming}
              reasoningDurationMs={reasoningDurationMs}
              reasoningDurationMsBySegmentId={reasoningDurationMsBySegmentId}
              reasoningSegmentCount={reasoningSegmentCount}
            />
          </div>
        )
      })}
      {orphanTools.map((toolCall, index) => (
        <div key={toolRecordId(toolCall) || `orphan-tool-${index}`} className="chat-motion-fade">
          <ToolCallErrorBoundary>
            <ToolCallBlock toolCall={toolCall} />
          </ToolCallErrorBoundary>
        </div>
      ))}
    </section>
  )
}

function MessageBubbleComponent({
  message,
  conversationId,
  tokensPerSec,
  reasoningDurationMs,
  reasoningDurationMsBySegmentId,
  reasoningStreaming = false,
  messageStreaming = false,
  sentModels,
  onUpdateMessage,
  onRegenerateMessage,
  onDeleteMessage,
  agentPlanOverride = null,
  onExecuteAgentPlan,
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
    (artifact) => isImageArtifact(artifact) && !artifactIsReferenced(artifactReferenceContent, artifact),
  )
  const generatedFileArtifacts = renderArtifacts.filter((artifact) => !isImageArtifact(artifact))
  const hasAnswerContent = !isDirectImageGenerationPending && message.content.trim().length > 0
  const hasGeneratedImages = unreferencedImageArtifacts.length > 0
  const hasGeneratedFiles = generatedFileArtifacts.length > 0
  const [draft, setDraft] = useState(message.content)
  const [saving, setSaving] = useState(false)
  const [copied, setCopied] = useState(false)
  const [toolsExpanded, setToolsExpanded] = useState(false)
  // 工具呼叫超過 4 個時預設摺疊（與思考過程一致）
  const toolsCollapsible = toolCalls.length > 4
  const agentPlan = message.agent_plan ?? message.agentPlan ?? agentPlanOverride
  const isAgentPlanMessage = isExecutableAgentPlanText(agentPlan?.plan)

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
    'rounded p-1 text-neutral-400 transition duration-[var(--kv-dur-instant)] hover:bg-neutral-100 hover:text-neutral-600 active:scale-90 disabled:cursor-not-allowed disabled:opacity-40 dark:hover:bg-neutral-800 dark:hover:text-neutral-300'

  if (isUser) {
    const hasText = message.content.trim().length > 0
    const canEditUser = Boolean(onRegenerateMessage)
    const handleEditAndRegenerate = () => {
      const trimmed = draft.trim()
      if (!trimmed || !onRegenerateMessage) return
      // regenerate 的 Promise 要等整個生成結束才 resolve（agent run 可能數分鐘），編輯框不陪跑：
      // 樂觀關閉編輯態（handleRegenerateMessage 的樂觀截斷會同步重渲本氣泡）。失敗時它內部
      // 會 reload 會話恢復原文，並線上程裡展示錯誤 + 重試。
      // 內容沒變 → 純重新生成；變了 → 編輯並重新生成（後端原子替換+截斷）。
      setIsEditing(false)
      void onRegenerateMessage(message.id, trimmed === message.content ? undefined : trimmed)
    }
    // R8（多模型一問多答）：本問發給 ≥2 個模型時，在 user 氣泡頂部渲染模型標籤行（如 @deepseek @qwen）。
    // 單模型不顯示這行（sentModels 預設或 <2）。
    const replyModelTags = (sentModels ?? []).filter((m) => (m.model ?? '').trim().length > 0)
    const showModelTags = replyModelTags.length >= 2
    return (
      <div className="group chat-motion-fade-up flex justify-end py-2">
        <div className={`flex min-w-0 flex-col items-end gap-1 ${isEditing ? 'w-full max-w-full' : 'max-w-[85%]'}`}>
          {showModelTags && (
            <div className="flex flex-wrap items-center justify-end gap-1.5 pr-0.5">
              {replyModelTags.map((tag, index) => (
                <span
                  key={`${tag.model}-${index}`}
                  className="inline-flex items-center gap-1 rounded-full bg-neutral-100 px-2 py-0.5 text-[11px] font-medium text-neutral-500 dark:bg-neutral-800 dark:text-neutral-400"
                  title={tag.providerId ? `${tag.model} | ${tag.providerId}` : (tag.model ?? '')}
                >
                  {tag.model && <ModelIcon model={tag.model} size={12} />}
                  <span className="max-w-[140px] truncate">@{tag.model}</span>
                </span>
              ))}
            </div>
          )}
          {attachments.length > 0 && (
            <ChatAttachments
              attachments={attachments}
              conversationId={conversationId}
              variant="user"
            />
          )}
          {isEditing ? (
            <div className="w-full space-y-2">
              <textarea
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                rows={4}
                autoFocus
                className="w-full resize-y rounded-[20px] border border-neutral-200/90 bg-neutral-50 px-4 py-2.5 text-[15px] leading-relaxed text-neutral-900 outline-none focus:border-neutral-400 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100 dark:focus:border-neutral-500"
              />
              <div className="flex items-center justify-end gap-2">
                <button
                  type="button"
                  onClick={() => {
                    setDraft(message.content)
                    setIsEditing(false)
                  }}
                  className="rounded-lg px-3 py-1.5 text-sm text-neutral-600 hover:bg-neutral-100 disabled:opacity-40 dark:text-neutral-400 dark:hover:bg-neutral-800"
                >
                  取消
                </button>
                <button
                  type="button"
                  // 編輯中若本會話起了新 run（如輸入欄又發了一條），回撥被 MessageList 收走
                  // （canEditUser 變 false）→ 停用儲存避免靜默 no-op；取消仍可退出。
                  disabled={!draft.trim() || !canEditUser}
                  onClick={handleEditAndRegenerate}
                  className="rounded-lg bg-neutral-900 px-3 py-1.5 text-sm font-medium text-white disabled:opacity-40 dark:bg-neutral-100 dark:text-neutral-900"
                  title="替換這條提問，並丟棄其後的回覆重新生成"
                >
                  儲存並重新生成
                </button>
              </div>
            </div>
          ) : (
            hasText && (
              <div className="rounded-[20px] bg-neutral-100 px-4 py-2.5 text-neutral-900 dark:bg-neutral-800 dark:text-neutral-100">
                <div className="whitespace-pre-wrap [overflow-wrap:anywhere] text-[15px] leading-relaxed">
                  {message.content}
                </div>
              </div>
            )
          )}
          {hasText && !isEditing && (
            <div className="flex items-center gap-0.5 pr-0.5 opacity-0 transition-opacity duration-150 focus-within:opacity-100 group-hover:opacity-100">
              <button
                type="button"
                onClick={() => void handleCopy()}
                className={bubbleActionBtn}
                title={copied ? '已複製' : '複製'}
                aria-label={copied ? '已複製' : '複製'}
              >
                {copied ? <Check size={14} strokeWidth={2} className="chat-motion-pop" /> : <Copy size={14} strokeWidth={2} />}
              </button>
              {canEditUser && (
                <button
                  type="button"
                  onClick={() => setIsEditing(true)}
                  className={bubbleActionBtn}
                  title="編輯並重新生成"
                  aria-label="編輯並重新生成"
                >
                  <Pencil size={14} strokeWidth={2} />
                </button>
              )}
              {onDeleteMessage && (
                <button
                  type="button"
                  onClick={() => void onDeleteMessage(message.id)}
                  className={bubbleActionBtn}
                  title="刪除"
                  aria-label="刪除"
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

  // 摺疊時僅隱藏較早的，始終保留最新 4 個可見
  const RECENT_TOOL_COUNT = 4
  const olderToolCalls = toolsCollapsible ? toolCalls.slice(0, toolCalls.length - RECENT_TOOL_COUNT) : []
  const recentToolCalls = toolsCollapsible ? toolCalls.slice(toolCalls.length - RECENT_TOOL_COUNT) : toolCalls
  const renderToolCall = (toolCall: ToolCallRecord, index: number) => (
    <ToolCallErrorBoundary key={toolCall.id || toolCall.call_id || toolCall.callId || index}>
      <ToolCallBlock toolCall={toolCall} />
    </ToolCallErrorBoundary>
  )

  return (
    <div className="chat-motion-fade-up flex justify-start py-3">
      <div className="w-full min-w-0">
        {toolCalls.length > 0 && !isEditing && !hasTimelineSegments && (
          <section
            aria-label="工具呼叫"
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
                  工具呼叫 · {toolCalls.length} 個
                  {!toolsExpanded ? ` · 顯示最新 ${RECENT_TOOL_COUNT} 個` : ''}
                </span>
                <ChevronDown
                  size={12}
                  strokeWidth={2}
                  className={`ml-auto shrink-0 transition-transform duration-300 ${toolsExpanded ? 'rotate-180' : ''}`}
                />
              </button>
            ) : (
              <div className="mb-1 text-[11px] font-medium text-neutral-400 dark:text-neutral-500">
                工具呼叫
              </div>
            )}
            {toolsCollapsible && toolsExpanded && (
              <div className="chat-motion-reveal is-open">
                <div>{olderToolCalls.map((toolCall, index) => renderToolCall(toolCall, index))}</div>
              </div>
            )}
            {recentToolCalls.map((toolCall, index) =>
              renderToolCall(toolCall, olderToolCalls.length + index),
            )}
          </section>
        )}

        {message.reasoning && !isEditing && !hasTimelineSegments && (
          <ReasoningBlock
            reasoning={message.reasoning}
            streaming={reasoningStreaming}
            durationMs={reasoningDurationMs}
          />
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
                {saving ? '儲存中…' : '儲存'}
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
              messageStreaming={messageStreaming}
              reasoningStreaming={reasoningStreaming}
              reasoningDurationMs={reasoningDurationMs}
              reasoningDurationMsBySegmentId={reasoningDurationMsBySegmentId}
            />
            {hasGeneratedImages && (
              <GeneratedImageArtifacts
                artifacts={unreferencedImageArtifacts}
                conversationId={conversationId}
              />
            )}
            {hasGeneratedFiles && <GeneratedFileArtifacts artifacts={generatedFileArtifacts} />}
          </>
        ) : (
          (hasAnswerContent || hasGeneratedImages || hasGeneratedFiles) && (
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
                  onImageClick={handleChatImageClick}
                />
              )}
              {hasGeneratedImages && (
                <GeneratedImageArtifacts
                  artifacts={unreferencedImageArtifacts}
                  conversationId={conversationId}
                />
              )}
              {hasGeneratedFiles && <GeneratedFileArtifacts artifacts={generatedFileArtifacts} />}
            </section>
          )
        )}

        {!isEditing && isAgentPlanMessage && !isDirectImageGenerationPending && (
          <AgentPlanAction
            messageId={message.id}
            planState={agentPlan}
            disabled={messageStreaming}
            onExecute={onExecuteAgentPlan}
          />
        )}

        {!isEditing && message.content.trim().length > 0 && !isDirectImageGenerationPending && (
          <AssistantMessageMeta
            content={message.content}
            reasoning={message.reasoning}
            timestamp={message.timestamp}
            tokensPerSec={tokensPerSec}
            runEntry={message.run_entry ?? message.runEntry}
            streamOutcome={message.stream_outcome ?? message.streamOutcome}
            usage={message.usage}
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

// memo：流式生成時歷史訊息 props 不變 → 跳過重渲染，避免每個 token 重新解析 Markdown
export const MessageBubble = memo(MessageBubbleComponent)
