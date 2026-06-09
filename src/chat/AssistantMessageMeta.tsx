import { useState } from 'react'
import { Check, Copy, Gauge, Pencil, RotateCcw, Trash2 } from 'lucide-react'
import { copyToClipboard } from '../utils/clipboard'
import { estimateTokens } from '../utils/tokens'
import { formatAssistantMessageTime } from './messageFormat'

interface AssistantMessageMetaProps {
  content: string
  reasoning?: string
  timestamp: number
  tokensPerSec?: number
  runEntry?: string | null
  streamOutcome?: string | null
  onEdit?: () => void
  onRegenerate?: () => void
  onDelete?: () => void
}

export function AssistantMessageMeta({
  content,
  reasoning,
  timestamp,
  tokensPerSec,
  runEntry,
  streamOutcome,
  onEdit,
  onRegenerate,
  onDelete,
}: AssistantMessageMetaProps) {
  const [copied, setCopied] = useState(false)
  const tokenCount = estimateTokens(`${content}${reasoning ? `\n${reasoning}` : ''}`)
  const speed =
    tokensPerSec != null && Number.isFinite(tokensPerSec)
      ? Math.max(1, Math.round(tokensPerSec))
      : null

  const handleCopy = async () => {
    const ok = await copyToClipboard(content)
    if (!ok) return
    setCopied(true)
    window.setTimeout(() => setCopied(false), 2000)
  }

  const iconBtn =
    'rounded p-1 text-neutral-400 transition-colors hover:bg-neutral-100 hover:text-neutral-600 disabled:cursor-not-allowed disabled:opacity-40 dark:hover:bg-neutral-800 dark:hover:text-neutral-300'

  const runEntryLabel = runEntry === 'regenerate' ? '已重新生成' : null
  const streamOutcomeLabel =
    streamOutcome === 'cancelled'
      ? '已停止后继续'
      : streamOutcome === 'error'
        ? '生成异常结束'
        : null

  return (
    <div className="mt-2.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-neutral-400 dark:text-neutral-500">
      <span className="shrink-0">{formatAssistantMessageTime(timestamp)}</span>
      {runEntryLabel && <span className="shrink-0">{runEntryLabel}</span>}
      {streamOutcomeLabel && <span className="shrink-0">{streamOutcomeLabel}</span>}

      <div className="flex items-center gap-0.5">
        <button
          type="button"
          onClick={() => void handleCopy()}
          className={iconBtn}
          title={copied ? '已复制' : '复制'}
          aria-label={copied ? '已复制' : '复制'}
        >
          {copied ? <Check size={14} strokeWidth={2} /> : <Copy size={14} strokeWidth={2} />}
        </button>
        <button
          type="button"
          onClick={onEdit}
          disabled={!onEdit}
          className={iconBtn}
          title="编辑"
          aria-label="编辑"
        >
          <Pencil size={14} strokeWidth={2} />
        </button>
        <button
          type="button"
          onClick={onRegenerate}
          disabled={!onRegenerate}
          className={iconBtn}
          title="重新生成"
          aria-label="重新生成"
        >
          <RotateCcw size={14} strokeWidth={2} />
        </button>
        <button
          type="button"
          onClick={onDelete}
          disabled={!onDelete}
          className={iconBtn}
          title="删除"
          aria-label="删除"
        >
          <Trash2 size={14} strokeWidth={2} />
        </button>
      </div>

      {speed != null && (
        <span className="inline-flex items-center gap-1">
          <Gauge size={13} strokeWidth={2} />
          <span>{speed} tokens/sec</span>
        </span>
      )}

      <span className="text-neutral-400 dark:text-neutral-500">({tokenCount} tokens)</span>
    </div>
  )
}
