import { type ComponentType, memo, useEffect, useMemo, useRef, useState } from 'react'
import {
  AlertCircle,
  Bot,
  Brain,
  CheckCircle2,
  ChevronDown,
  CircleSlash,
  Copy,
  Download,
  Eye,
  FileCode2,
  FilePen,
  FilePlus2,
  FileSearch,
  FileText,
  FolderInput,
  FolderOpen,
  FolderPlus,
  ImagePlus,
  ListChecks,
  Loader2,
  MessageCircleQuestion,
  Plug,
  Save,
  ScrollText,
  Search,
  SquareTerminal,
  Trash2,
  Wrench,
  XCircle,
} from 'lucide-react'
import type { LucideIcon } from 'lucide-react'
import type { AgentTodoItem, AgentTodoState, AgentTodoStatus, ToolCallRecord, ToolCallStatus } from './types'
import { normalizeToolCallStatus } from './toolStatus'
import { formatToolResultPreview } from './toolResultPreview'
import { knowledgeSearchHits, type KbHitView } from './knowledgeBaseHits'
import { AskUserBlock } from './AskUserBlock'
import { ChatMarkdown } from './ChatMarkdown'
import { WebSearchIcon } from '../settings/NavIcons'

export interface ToolCallBlockLabels {
  pending: string
  running: string
  success: string
  completed: string
  error: string
  skipped: string
  cancelled: string
  arguments: string
  result: string
  source: string
  tool: string
}

export interface ToolCallBlockProps {
  toolCall: ToolCallRecord
  defaultOpen?: boolean
  labels?: Partial<ToolCallBlockLabels>
}

const defaultLabels: ToolCallBlockLabels = {
  pending: '準備呼叫',
  running: '呼叫中',
  success: '已完成',
  completed: '已完成',
  error: '呼叫失敗',
  skipped: '已跳過',
  cancelled: '已取消',
  arguments: '引數',
  result: '結果',
  source: '來源',
  tool: '工具',
}

interface FileMutationFile {
  path: string
  operation: string
  bytesWritten?: number
  bytes_written?: number
  additions: number
  removals: number
  diff?: string
}

interface FileMutationStructuredContent {
  ok?: boolean
  operation: string
  targetTouched?: boolean
  target_touched?: boolean
  resolvedPath?: string | null
  resolved_path?: string | null
  files?: FileMutationFile[]
  bytesWritten?: number
  bytes_written?: number
  additions?: number
  removals?: number
  diff?: string
  warnings?: string[]
  diagnostics?: unknown[]
}

function compactText(text: string, max = 220): string {
  const cleaned = text.replace(/\s+/g, ' ').trim()
  if (cleaned.length <= max) return cleaned
  return `${cleaned.slice(0, max).trimEnd()}...`
}

function previewValue(value: unknown, max = 220): string {
  if (value == null) return ''
  if (typeof value === 'string') return compactText(value, max)
  if (typeof value === 'number' || typeof value === 'boolean') return String(value)
  try {
    return compactText(JSON.stringify(value, null, 2), max)
  } catch {
    return compactText(String(value), max)
  }
}

function parsedArguments(toolCall: ToolCallRecord): Record<string, unknown> | null {
  const value = toolCall.arguments ?? toolCall.args ?? toolCall.input
  if (!value) return null
  if (typeof value === 'object' && !Array.isArray(value)) return value as Record<string, unknown>
  if (typeof value !== 'string') return null
  try {
    const parsed = JSON.parse(value)
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed)
      ? parsed as Record<string, unknown>
      : null
  } catch {
    return null
  }
}

function toolRawName(toolCall: ToolCallRecord): string {
  return toolCall.tool_name || toolCall.toolName || toolCall.name || ''
}

function toolGlyph(toolCall: ToolCallRecord): LucideIcon | ComponentType<{ size?: number; strokeWidth?: number; className?: string }> {
  const raw = toolRawName(toolCall)
  switch (raw) {
    case 'read':
    case 'read_file':
      return FileText
    case 'write':
    case 'write_file':
      return FilePlus2
    case 'edit':
    case 'edit_file':
      return FilePen
    case 'bash':
    case 'run_command':
      return SquareTerminal
    case 'grep':
    case 'search_files':
      return Search
    case 'find':
    case 'glob':
    case 'glob_files':
      return FileSearch
    case 'ls':
    case 'list_dir':
      return FolderOpen
    case 'delete':
      return Trash2
    case 'move':
      return FolderInput
    case 'copy':
      return Copy
    case 'create_dir':
      return FolderPlus
    case 'stat':
    case 'stat_path':
      return FileSearch
    case 'run_python':
      return FileCode2
    case 'web_search':
      return WebSearchIcon
    case 'web_fetch':
      return Download
    case 'skill_activate':
    case 'skill_read_file':
    case 'skill_run_script':
      return ScrollText
    case 'todo_write':
    case 'todo_update':
      return ListChecks
    case 'memory_read':
    case 'memory_search':
    case 'memory_modify':
      return Brain
    case 'save_assistant':
      return Save
    case 'mixer_vision':
      return Eye
    case 'mixer_generate_image':
      return ImagePlus
    case 'agent':
      return Bot
    case 'ask_user':
      return MessageCircleQuestion
    default:
      break
  }
  const isMcp =
    toolCall.source === 'mcp' ||
    ((toolCall.server_name || toolCall.serverName) &&
      toolCall.source !== 'native' &&
      toolCall.source !== 'skill' &&
      toolCall.source !== 'mixer')
  if (isMcp) return Plug
  return Wrench
}

function isAskUserTool(toolCall: ToolCallRecord): boolean {
  return toolRawName(toolCall) === 'ask_user'
}

function objectValue(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null
}

function todoStatusLabel(status?: string): string {
  switch (status) {
    case 'completed':
      return '已完成'
    case 'in_progress':
      return '進行中'
    case 'pending':
      return '待處理'
    default:
      return status ? compactText(status, 24) : ''
  }
}

function normalizeTodoItem(value: unknown): AgentTodoItem | null {
  const item = objectValue(value)
  if (!item) return null
  const id = typeof item.id === 'string' ? item.id.trim() : ''
  const content = typeof item.content === 'string' ? item.content.trim() : ''
  const status = typeof item.status === 'string' ? item.status : ''
  if (!id && !content) return null
  return {
    id,
    content,
    status: (status === 'completed' || status === 'in_progress' || status === 'pending'
      ? status
      : 'pending') as AgentTodoStatus,
  }
}

function normalizeTodoItems(value: unknown): AgentTodoItem[] {
  if (!Array.isArray(value)) return []
  return value
    .map((item) => normalizeTodoItem(item))
    .filter((item): item is AgentTodoItem => Boolean(item))
}

function todoCounts(items?: AgentTodoItem[]): { completed: number; total: number } | null {
  if (!items?.length) return null
  return {
    completed: items.filter((item) => item.status === 'completed').length,
    total: items.length,
  }
}

function formatTodoCounts(items?: AgentTodoItem[]): string {
  const counts = todoCounts(items)
  return counts ? `${counts.completed}/${counts.total}` : ''
}

function structuredTodoState(toolCall: ToolCallRecord): AgentTodoState | null {
  const structured = objectValue(toolCall.structured_content ?? toolCall.structuredContent)
  const todoState = objectValue(structured?.todoState)
  if (!todoState) return null
  return {
    items: normalizeTodoItems(todoState.items),
    updated_at: typeof todoState.updated_at === 'number' ? todoState.updated_at : undefined,
    updatedAt: typeof todoState.updatedAt === 'number' ? todoState.updatedAt : undefined,
  }
}

function stringArrayValue(value: unknown): string[] {
  if (!Array.isArray(value)) return []
  return value.filter((item): item is string => typeof item === 'string' && item.trim().length > 0)
}

function stringValue(value: unknown): string {
  return typeof value === 'string' ? value : ''
}

interface SubagentView {
  name: string
  agentType?: string
  depth: number
  status: string
  result?: string
  error?: string
  preview?: string
  steps: string[]
  usage?: { inputTokens?: number; outputTokens?: number; totalTokens?: number }
}

/** Parse the optional token usage from a final sub-agent structured result.
 *  Live `subagentProgress` has no usage; only the completed `{type:"subagent"}`
 *  payload carries it. */
function subagentUsage(value: unknown): SubagentView['usage'] {
  const usage = objectValue(value)
  if (!usage) return undefined
  const input = typeof usage.inputTokens === 'number' ? usage.inputTokens : undefined
  const output = typeof usage.outputTokens === 'number' ? usage.outputTokens : undefined
  const total = typeof usage.totalTokens === 'number' ? usage.totalTokens : undefined
  if (input == null && output == null && total == null) return undefined
  return { inputTokens: input, outputTokens: output, totalTokens: total }
}

/** Compact token count, e.g. 1234 → "1.2k", 999 → "999". */
function formatTokenCount(value?: number): string {
  if (value == null || !Number.isFinite(value) || value < 0) return ''
  if (value < 1000) return String(Math.round(value))
  const thousands = value / 1000
  return `${thousands >= 100 ? Math.round(thousands) : thousands.toFixed(1)}k`
}

/** One-line token summary like `↑1.2k ↓340 · 1.5k tokens`. Empty when no usage. */
function subagentUsageLine(usage: SubagentView['usage']): string {
  if (!usage) return ''
  const parts: string[] = []
  const input = formatTokenCount(usage.inputTokens)
  const output = formatTokenCount(usage.outputTokens)
  if (input) parts.push(`↑${input}`)
  if (output) parts.push(`↓${output}`)
  const total = formatTokenCount(usage.totalTokens)
  const head = parts.join(' ')
  if (head && total) return `${head} · ${total} tokens`
  if (head) return head
  if (total) return `${total} tokens`
  return ''
}

/** Parse sub-agent state (P3) from a tool record's structured content: either
 *  the final `{ type: "subagent", ... }` result or the live `subagentProgress`
 *  merged in from `chat-subagent` events. */
function structuredSubagent(toolCall: ToolCallRecord): SubagentView | null {
  const structured = objectValue(toolCall.structured_content ?? toolCall.structuredContent)
  if (!structured) return null
  const isFinal = structured.type === 'subagent'
  const progress = objectValue(structured.subagentProgress)
  if (!isFinal && !progress) return null
  return {
    name: stringValue(progress?.name) || stringValue(structured.name) || 'subagent',
    agentType: stringValue(structured.agentType) || undefined,
    depth: numberValue(progress?.depth ?? structured.depth),
    status: stringValue(progress?.status) || stringValue(structured.status) || 'running',
    result: stringValue(structured.result) || undefined,
    error: stringValue(structured.error) || undefined,
    preview: stringValue(progress?.preview) || undefined,
    steps: stringArrayValue(progress?.steps),
    usage: subagentUsage(structured.usage),
  }
}

function isSubAgentRecord(toolCall: ToolCallRecord): boolean {
  if (structuredSubagent(toolCall)) return true
  return toolCall.source === 'native' && toolRawName(toolCall) === 'agent'
}

/** Sub-agent type / display name fall back to the spawn arguments while the run
 *  is live (structured content only carries agentType in the final result). */
function subagentAgentType(view: SubagentView | null, args: Record<string, unknown> | null): string {
  return view?.agentType || stringValue(args?.subagent_type) || ''
}

function subagentName(view: SubagentView | null, args: Record<string, unknown> | null): string {
  return (
    view?.name ||
    stringValue(args?.name) ||
    stringValue(args?.subagent_type) ||
    'subagent'
  )
}

function subagentPrompt(args: Record<string, unknown> | null): string {
  return stringValue(args?.prompt)
}

function subagentTitle(agentType: string, name: string): string {
  const parts = ['Subagent']
  if (agentType) parts.push(agentType)
  if (name && name !== agentType) parts.push(name)
  return parts.join(' · ')
}

function subagentStatusLine(view: SubagentView | null, status: ToolCallStatus): string {
  if (status === 'completed') return '已完成'
  if (status === 'error') return view?.error ? compactText(view.error, 160) : '執行失敗'
  if (status === 'cancelled') return '已取消'
  if (status === 'running') {
    const lastStep = view?.steps?.length ? view.steps[view.steps.length - 1] : ''
    if (lastStep) return compactText(lastStep, 160)
    if (view?.preview) return compactText(view.preview, 160)
    return '執行中…'
  }
  return '準備執行…'
}

function SubAgentCard({ toolCall, defaultOpen = false }: ToolCallBlockProps) {
  const status = normalizeToolCallStatus(toolCall.status)
  const [open, setOpen] = useState(defaultOpen)
  const view = useMemo(() => structuredSubagent(toolCall), [toolCall])
  const args = useMemo(() => parsedArguments(toolCall), [toolCall])

  const agentType = subagentAgentType(view, args)
  const name = subagentName(view, args)
  const title = subagentTitle(agentType, name)
  const duration = formatDuration(getDuration(toolCall))
  const statusLine = subagentStatusLine(view, status)
  const prompt = subagentPrompt(args)
  const result = view?.result || ''
  const error = view?.error || (toolCall.error ? compactToolError(toolCall.error) : '')
  const steps = view?.steps ?? []
  const preview = view?.preview || ''
  const usageLine = status !== 'running' ? subagentUsageLine(view?.usage) : ''

  const hasDetails = Boolean(prompt || steps.length || preview || result || error)

  return (
    <div className="not-prose mb-2 text-[12.5px] leading-5 text-neutral-500 dark:text-neutral-400">
      <button
        type="button"
        onClick={() => {
          if (hasDetails) setOpen((value) => !value)
        }}
        aria-expanded={hasDetails ? open : undefined}
        className={`max-w-full min-w-0 inline-flex items-center gap-1.5 rounded-md py-0.5 transition-colors ${
          hasDetails
            ? 'hover:text-neutral-700 dark:hover:text-neutral-200'
            : 'cursor-default'
        }`}
      >
        <span
          className={`shrink-0 text-[13px] leading-none text-violet-500 dark:text-violet-300 ${
            status === 'running' ? 'subagent-sparkle is-running' : 'subagent-sparkle'
          }`}
          aria-hidden="true"
        />
        <span className="shrink-0 font-medium text-neutral-700 dark:text-neutral-200">
          {title}
        </span>
        <span className="shrink-0">
          <StatusIcon status={status} />
        </span>
        {duration && (
          <span className="shrink-0 tabular-nums text-neutral-400 dark:text-neutral-500">
            · {duration}
          </span>
        )}
        {usageLine && (
          <span className="shrink-0 tabular-nums text-neutral-400 dark:text-neutral-500">
            · {usageLine}
          </span>
        )}
        {statusLine && (
          <span
            className={`min-w-0 truncate ${
              status === 'error'
                ? 'text-neutral-400 dark:text-neutral-500'
                : status === 'running'
                  ? 'chat-motion-subagent-shimmer'
                  : 'text-neutral-400 dark:text-neutral-500'
            }`}
          >
            · {statusLine}
          </span>
        )}
        {hasDetails && (
          <ChevronDown
            size={12}
            strokeWidth={2}
            className={`shrink-0 transition-transform duration-300 ${open ? 'rotate-180' : ''}`}
          />
        )}
      </button>

      {hasDetails && (
        <div className={`chat-motion-reveal ${open ? 'is-open' : ''}`} aria-hidden={!open}>
          <div className="mt-1.5 ml-1.5 space-y-1.5 border-l-2 border-violet-400/50 pl-2.5 dark:border-violet-400/40">
            {prompt && (
              <div>
                <div className="text-[10.5px] font-medium text-neutral-400 dark:text-neutral-500">
                  任務
                </div>
                <div className="whitespace-pre-wrap break-words text-neutral-500 dark:text-neutral-400">
                  {compactText(prompt, 600)}
                </div>
              </div>
            )}
            {status === 'running' && (steps.length > 0 || preview) && (
              <div className="rounded-md border-l-2 border-violet-400/60 bg-violet-500/[0.04] py-1 pl-2.5 pr-1.5 dark:bg-violet-400/[0.06]">
                {steps.length > 0 && (
                  <div className="space-y-0.5 text-[10.5px] text-neutral-500 dark:text-neutral-400">
                    {steps.map((step, index) => (
                      <div key={`${index}-${step}`} className="truncate">
                        · {step}
                      </div>
                    ))}
                  </div>
                )}
                {preview && (
                  <div className="mt-0.5 whitespace-pre-wrap break-words text-neutral-500 dark:text-neutral-400">
                    {preview}
                  </div>
                )}
              </div>
            )}
            {status !== 'running' && result && (
              <div>
                <div className="text-[10.5px] font-medium text-neutral-400 dark:text-neutral-500">
                  結果
                </div>
                <ChatMarkdown content={result} />
              </div>
            )}
            {error && (
              <div className="whitespace-pre-wrap break-words text-neutral-500 dark:text-neutral-400">
                {error}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  )
}

function numberValue(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : 0
}

function normalizeFileMutationFile(value: unknown): FileMutationFile | null {
  const file = objectValue(value)
  if (!file) return null
  const path = typeof file.path === 'string' ? file.path.trim() : ''
  if (!path) return null
  return {
    path,
    operation: typeof file.operation === 'string' ? file.operation : 'edit',
    bytesWritten: numberValue(file.bytesWritten),
    bytes_written: numberValue(file.bytes_written),
    additions: numberValue(file.additions),
    removals: numberValue(file.removals),
    diff: typeof file.diff === 'string' ? file.diff : '',
  }
}

// `write`/`edit` are the current Pi-style names. `write_file`/`edit_file` are
// legacy aliases from before the Pi-style rename: persisted conversations still
// carry their ToolCallRecords and must keep rendering.
function isFileMutationTool(rawName: string): boolean {
  return ['write', 'edit', 'write_file', 'edit_file'].includes(rawName)
}

function KnowledgeHits({ hits }: { hits: KbHitView[] }) {
  return (
    <div className="space-y-1.5">
      {hits.map((hit, idx) => (
        <div
          key={`${hit.n}-${idx}`}
          className="rounded-md border border-black/[0.08] bg-black/[0.02] p-2 dark:border-white/[0.1] dark:bg-white/[0.03]"
        >
          <div className="flex items-center gap-1.5 text-[10.5px] font-medium text-neutral-500 dark:text-neutral-400">
            <span className="shrink-0 rounded bg-indigo-500/15 px-1 text-indigo-500">[{hit.n}]</span>
            <span className="min-w-0 truncate">
              {hit.docName}
              {hit.headingPath ? ` · ${hit.headingPath}` : ''}
            </span>
            <span className="ml-auto shrink-0 tabular-nums text-neutral-400 dark:text-neutral-500">
              {hit.score.toFixed(2)}
            </span>
          </div>
          <div className="mt-1 line-clamp-4 whitespace-pre-wrap break-words text-neutral-500 dark:text-neutral-400">
            {hit.text}
          </div>
        </div>
      ))}
    </div>
  )
}

function structuredFileMutation(toolCall: ToolCallRecord): FileMutationStructuredContent | null {
  const rawName = toolRawName(toolCall)
  if (!isFileMutationTool(rawName)) return null

  const structured = objectValue(toolCall.structured_content ?? toolCall.structuredContent)
  if (!structured) return null
  if (objectValue(structured.toolDraft)) return null
  const operation = typeof structured.operation === 'string' ? structured.operation : rawName
  const files = Array.isArray(structured.files)
    ? structured.files
      .map((file) => normalizeFileMutationFile(file))
      .filter((file): file is FileMutationFile => Boolean(file))
    : []
  const resolvedPath = typeof structured.resolvedPath === 'string'
    ? structured.resolvedPath
    : typeof structured.resolved_path === 'string'
      ? structured.resolved_path
      : null

  const hasMutationShape = Boolean(
    typeof structured.ok === 'boolean' ||
    typeof structured.operation === 'string' ||
    typeof structured.targetTouched === 'boolean' ||
    typeof structured.target_touched === 'boolean' ||
    resolvedPath ||
    files.length > 0 ||
    typeof structured.diff === 'string' ||
    typeof structured.additions === 'number' ||
    typeof structured.removals === 'number' ||
    Array.isArray(structured.warnings) ||
    Array.isArray(structured.diagnostics),
  )
  if (!hasMutationShape) return null
  return {
    ok: typeof structured.ok === 'boolean' ? structured.ok : true,
    operation,
    targetTouched: typeof structured.targetTouched === 'boolean' ? structured.targetTouched : undefined,
    target_touched: typeof structured.target_touched === 'boolean' ? structured.target_touched : undefined,
    resolvedPath,
    resolved_path: resolvedPath,
    files,
    bytesWritten: numberValue(structured.bytesWritten),
    bytes_written: numberValue(structured.bytes_written),
    additions: numberValue(structured.additions),
    removals: numberValue(structured.removals),
    diff: typeof structured.diff === 'string' ? structured.diff : '',
    warnings: stringArrayValue(structured.warnings),
    diagnostics: Array.isArray(structured.diagnostics) ? structured.diagnostics : [],
  }
}

function fileMutationStats(mutation: FileMutationStructuredContent): string {
  return `+${mutation.additions ?? 0} -${mutation.removals ?? 0}`
}

function fileMutationTarget(mutation: FileMutationStructuredContent): string {
  if (mutation.files?.length === 1) return mutation.files[0]?.path || ''
  if (mutation.files?.length) return `${mutation.files.length} 個檔案`
  return mutation.resolvedPath || mutation.resolved_path || ''
}

function fileMutationPreview(mutation: FileMutationStructuredContent): string {
  const target = fileMutationTarget(mutation)
  const stats = mutation.files?.length ? fileMutationStats(mutation) : ''
  return [target, stats].filter(Boolean).join(' · ')
}

function FileMutationDetails({ mutation }: { mutation: FileMutationStructuredContent }) {
  const files = mutation.files ?? []
  const warnings = mutation.warnings ?? []
  const diagnostics = mutation.diagnostics ?? []
  const diff = (mutation.diff || files.map((file) => file.diff).filter(Boolean).join('\n')).trim()

  return (
    <div className="space-y-1.5">
      {files.length > 0 && (
        <div>
          <div className="text-[10.5px] font-medium text-neutral-400 dark:text-neutral-500">
            檔案變更
          </div>
          <div className="space-y-0.5 text-neutral-500 dark:text-neutral-400">
            {files.map((file, index) => (
              <div key={`${file.path}-${index}`} className="flex min-w-0 items-center gap-1.5">
                <span className="shrink-0 text-neutral-400 dark:text-neutral-500">
                  {fileOperationLabel(file.operation)}
                </span>
                <span className="min-w-0 truncate">{file.path}</span>
                <span className="shrink-0 tabular-nums text-[#C56646] dark:text-[#E39A78]">
                  +{file.additions}
                </span>
                <span className="shrink-0 tabular-nums text-red-500/80">
                  -{file.removals}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}
      {warnings.length > 0 && (
        <div>
          <div className="text-[10.5px] font-medium text-neutral-400 dark:text-neutral-500">
            警告
          </div>
          <div className="whitespace-pre-wrap break-words text-amber-600 dark:text-amber-300">
            {warnings.join('\n')}
          </div>
        </div>
      )}
      {diagnostics.length > 0 && (
        <div>
          <div className="text-[10.5px] font-medium text-neutral-400 dark:text-neutral-500">
            診斷
          </div>
          <div className="whitespace-pre-wrap break-words text-neutral-500 dark:text-neutral-400">
            {previewValue(diagnostics, 900)}
          </div>
        </div>
      )}
      {diff && (
        <div>
          <div className="text-[10.5px] font-medium text-neutral-400 dark:text-neutral-500">
            Diff
          </div>
          <pre className="custom-scrollbar max-h-72 overflow-auto whitespace-pre-wrap break-words rounded-md bg-black/[0.035] px-2 py-1.5 font-mono text-[10.5px] leading-4 text-neutral-600 dark:bg-white/[0.055] dark:text-neutral-300">
            {diff}
          </pre>
        </div>
      )}
    </div>
  )
}

function fileOperationLabel(operation: string): string {
  switch (operation) {
    case 'create':
      return '新增'
    case 'overwrite':
      return '覆蓋'
    case 'edit':
      return '修改'
    case 'delete':
      return '刪除'
    case 'noop':
      return '無變更'
    default:
      return operation || '變更'
  }
}

function fileToolArgumentPreview(toolCall: ToolCallRecord, args: Record<string, unknown> | null): string {
  const rawName = toolRawName(toolCall)
  const path = typeof args?.path === 'string' ? args.path.trim() : ''
  const query = typeof args?.query === 'string'
    ? args.query.trim()
    : typeof args?.pattern === 'string'
      ? args.pattern.trim()
      : ''
  const glob = typeof args?.glob === 'string' ? args.glob.trim() : ''
  if (rawName === 'write' || rawName === 'write_file') {
    return path ? path : '寫入檔案'
  }
  if (rawName === 'edit' || rawName === 'edit_file') {
    const edits = Array.isArray(args?.edits) ? args.edits : null
    if (edits) {
      const label = edits.length === 1 ? '1 處編輯' : `${edits.length} 處編輯`
      return [path, label].filter(Boolean).join(' · ')
    }
    // Legacy single-edit records (old_string/new_string) from persisted conversations.
    const oldString = typeof args?.old_string === 'string' ? compactText(args.old_string, 80) : ''
    return [path, oldString ? `替換 ${oldString}` : ''].filter(Boolean).join(' · ')
  }
  if (rawName === 'grep' || rawName === 'search_files') {
    const scope = glob
      ? [path || '.', glob].join(' + ')
      : path
    if (!query && !scope) return ''
    const scopeLabel = scope ? compactText(scope, 120) : ''
    const queryLabel = query ? `搜尋 ${compactText(query, 80)}` : ''
    return [queryLabel, scopeLabel].filter(Boolean).join(' · ')
  }
  return ''
}

function formatDuration(ms?: number): string {
  if (ms == null || !Number.isFinite(ms) || ms < 0) return ''
  if (ms < 1000) return `${Math.round(ms)}ms`
  if (ms < 10_000) return `${(ms / 1000).toFixed(1)}s`
  return `${Math.round(ms / 1000)}s`
}

function getDuration(toolCall: ToolCallRecord): number | undefined {
  if (toolCall.duration_ms != null) return toolCall.duration_ms
  if (toolCall.durationMs != null) return toolCall.durationMs

  const startedAt = toolCall.started_at ?? toolCall.startedAt
  const completedAt = toolCall.completed_at ?? toolCall.completedAt
  if (startedAt == null || completedAt == null) return undefined

  const delta = completedAt - startedAt
  return delta > 0 && delta < 10_000 ? delta * 1000 : delta
}

function getToolName(toolCall: ToolCallRecord): string {
  const raw = toolRawName(toolCall) || 'Tool'
  const args = parsedArguments(toolCall)
  const relativePath = typeof args?.relative_path === 'string'
    ? args.relative_path
    : typeof args?.relativePath === 'string'
      ? args.relativePath
      : ''

  if (raw === 'skill_activate') return '啟用 Skill'
  if (raw === 'skill_read_file') return '讀取 Skill 檔案'
  if (
    raw === 'skill_run_script' &&
    (relativePath.endsWith('pdf_text_digest.py') || relativePath.endsWith('pdf_extract_digest.py'))
  ) {
    return '生成 PDF 摘要上下文'
  }
  if (raw === 'skill_run_script') return '執行 Skill 指令碼'
  // 使用繁體中文動詞呈現工具列；目標（檔名 + 行號 / 指令 / pattern）由 getToolTarget 追加。
  switch (raw) {
    case 'read':
    case 'read_file':
      return '讀取'
    case 'write':
    case 'write_file':
      return '寫入'
    case 'edit':
    case 'edit_file':
      return '編輯'
    case 'bash':
    case 'run_command':
      return '執行'
    case 'grep':
    case 'search_files':
      return '搜尋'
    case 'glob':
    case 'glob_files':
    case 'find':
      return '比對'
    case 'ls':
    case 'list_dir':
      return '列出'
    case 'stat':
    case 'stat_path':
      return '檢查'
    case 'delete':
      return '刪除'
    case 'move':
      return '移動'
    case 'copy':
      return '複製'
    case 'create_dir':
      return '建立目錄'
    default:
      break
  }
  if (raw === 'run_python') return 'Python'
  if (raw === 'web_search') {
    // 結果裡帶了實際使用的搜尋服務名稱（後端 structured_content.provider），顯示為「網路搜尋 · Ollama」。
    const structured = objectValue(toolCall.structured_content ?? toolCall.structuredContent) ?? {}
    const provider = stringValue(structured.provider)
    return provider ? `網路搜尋 · ${provider}` : '網路搜尋'
  }
  if (raw === 'web_fetch') return '網頁抓取'
  if (raw === 'knowledge_search') return '知識庫檢索'
  if (raw === 'mixer_vision') return '混音器視覺分析'
  if (raw === 'mixer_generate_image') return '混音器生圖'
  if (raw === 'todo_write' || raw === 'todo_update') return '更新 Todo'
  return raw
}

function firstString(...values: unknown[]): string {
  for (const value of values) {
    if (typeof value === 'string' && value.trim()) return value.trim()
  }
  return ''
}

/** 只取路径最后一段（Cursor 行内用文件名而非全路径）：`src/a/Lens.tsx` → `Lens.tsx`。 */
function basename(path: string): string {
  const trimmed = path.trim().replace(/[\\/]+$/, '')
  const idx = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'))
  return idx >= 0 ? trimmed.slice(idx + 1) : trimmed
}

/** read 的行号范围 `L起-止`：优先用结果里的真实 start/end（structured_content 保留了
 *  ReadFileResult），流式未出结果时回退到请求窗口 offset/limit；整文件读取则为空。 */
function readLineLabel(toolCall: ToolCallRecord, args: Record<string, unknown> | null): string {
  const structured = objectValue(toolCall.structured_content ?? toolCall.structuredContent)
  const start = numberValue(structured?.start_line ?? structured?.startLine)
  const end = numberValue(structured?.end_line ?? structured?.endLine)
  if (start > 0 && end > 0) return `L${start}-${end}`
  const offset = numberValue(args?.offset)
  const limit = numberValue(args?.limit)
  if (offset > 0) return limit > 0 ? `L${offset}-${offset + limit - 1}` : `L${offset}+`
  return ''
}

/** 折叠行的「目标」：以输入参数为主（文件名 / 命令 / pattern / url），不含动词、不含结果。
 *  Cursor 风格 —— 行内只呈现「动词 + 目标」，其余细节（结果、diff、错误）放展开区。 */
function getToolTarget(toolCall: ToolCallRecord): string {
  const raw = toolRawName(toolCall)
  const args = parsedArguments(toolCall)
  const path = firstString(args?.path, args?.relative_path, args?.relativePath)
  const primary = ((): string => {
    switch (raw) {
      case 'read':
      case 'read_file':
        return [basename(path), readLineLabel(toolCall, args)].filter(Boolean).join(' ')
      case 'write':
      case 'write_file':
      case 'edit':
      case 'edit_file': {
        const mutation = structuredFileMutation(toolCall)
        if (mutation && (mutation.files?.length ?? 0) > 1) {
          return `${mutation.files!.length} files`
        }
        const target = (mutation && fileMutationTarget(mutation)) || path
        return target.includes('/') || target.includes('\\') ? basename(target) : target
      }
      case 'bash':
      case 'run_command':
        return compactText(firstString(args?.description, args?.command), 160)
      case 'grep':
      case 'search_files':
        return compactText(firstString(args?.query, args?.pattern), 140)
      case 'glob':
      case 'glob_files':
      case 'find': {
        const pattern = compactText(firstString(args?.pattern, args?.glob), 140)
        const dir = firstString(args?.path)
        return dir && dir !== '.' ? `${pattern} 於 ${basename(dir)}` : pattern
      }
      case 'ls':
      case 'list_dir':
      case 'stat':
      case 'stat_path':
      case 'delete':
      case 'create_dir':
        return path
      case 'move':
      case 'copy': {
        const from = firstString(args?.source, args?.from, args?.src, args?.path)
        const to = firstString(args?.destination, args?.to, args?.dest, args?.target)
        return [from, to].filter(Boolean).join(' → ')
      }
      case 'web_fetch':
        return compactText(firstString(args?.url), 140)
      case 'web_search':
      case 'knowledge_search':
        return compactText(firstString(args?.query), 140)
      case 'todo_write': {
        const counts = formatTodoCounts(normalizeTodoItems(args?.todos))
        return counts
      }
      case 'todo_update':
        return compactText(firstString(args?.content, args?.id), 120)
      case 'mixer_vision': {
        const count = numberValue(args?.images)
        return count > 0 ? `${count} image${count > 1 ? 's' : ''}` : ''
      }
      case 'mixer_generate_image':
        return compactText(firstString(args?.prompt), 140)
      default:
        return ''
    }
  })()
  if (primary) return primary
  // run_python 只显示动词「Python」，不追加目标；其余工具（含参数尚未解析出的情况）
  // 回退到已有的输入参数摘要（todo / mixer / skill / MCP 及流式占位 argumentPreview）。
  if (raw === 'run_python') return ''
  return getArgumentPreview(toolCall)
}

function getArgumentPreview(toolCall: ToolCallRecord): string {
  const rawName = toolRawName(toolCall)
  const args = parsedArguments(toolCall)
  if (rawName === 'todo_write') {
    const todos = normalizeTodoItems(args?.todos)
    const counts = formatTodoCounts(todos)
    return counts ? `清單 ${counts}` : todos.length ? `清單 ${todos.length} 項` : '替換 Todo 清單'
  }
  if (rawName === 'todo_update') {
    const content = typeof args?.content === 'string' ? compactText(args.content, 120) : ''
    const status = typeof args?.status === 'string' ? todoStatusLabel(args.status) : ''
    const id = typeof args?.id === 'string' ? compactText(args.id, 80) : ''
    const target = content || id
    return ['更新條目', status, target].filter(Boolean).join(' · ')
  }
  const fileMutation = structuredFileMutation(toolCall)
  if (fileMutation) {
    return fileMutationPreview(fileMutation)
  }
  const fileArgsPreview = fileToolArgumentPreview(toolCall, args)
  if (fileArgsPreview) return fileArgsPreview
  if (rawName === 'mixer_vision') {
    const imageCount = typeof args?.images === 'number' ? args.images : null
    const provider = typeof args?.provider === 'string' ? args.provider : ''
    const model = typeof args?.model === 'string' ? args.model : ''
    const imageLabel = imageCount == null
      ? '圖片'
      : `圖片 ${imageCount} 張`
    const modelLabel = [provider, model].filter(Boolean).join(' / ')
    return modelLabel ? `${imageLabel} · ${modelLabel}` : imageLabel
  }
  if (rawName === 'mixer_generate_image') {
    const prompt = typeof args?.prompt === 'string' ? compactText(args.prompt, 140) : ''
    const size = typeof args?.size === 'string' && args.size ? args.size : ''
    const quality = typeof args?.quality === 'string' && args.quality ? args.quality : ''
    const count = typeof args?.n === 'number' && Number.isFinite(args.n) ? `${args.n} 張` : ''
    return [prompt, size, quality, count].filter(Boolean).join(' · ')
  }
  return (
    toolCall.argument_preview ||
    toolCall.argumentPreview ||
    toolCall.argumentsPreview ||
    previewValue(toolCall.arguments ?? toolCall.args ?? toolCall.input)
  )
}

function getResultPreview(toolCall: ToolCallRecord): string {
  const rawName = toolRawName(toolCall)
  const args = parsedArguments(toolCall)
  const relativePath = typeof args?.relative_path === 'string'
    ? args.relative_path
    : typeof args?.relativePath === 'string'
      ? args.relativePath
      : ''
  if (rawName === 'skill_run_script' && relativePath.endsWith('pdf_extract_digest.py')) {
    return '已提取 PDF 文字並生成摘要上下文'
  }
  if (rawName === 'todo_write' || rawName === 'todo_update') {
    if (normalizeToolCallStatus(toolCall.status) !== 'completed') return ''
    const counts = formatTodoCounts(structuredTodoState(toolCall)?.items)
    return counts ? `已同步 ${counts}` : '已同步'
  }
  const fileMutation = structuredFileMutation(toolCall)
  if (fileMutation) {
    if (fileMutation.ok === false) {
      return `未完成 ${fileMutationPreview(fileMutation)}`
    }
    return `已應用 ${fileMutationPreview(fileMutation)}`
  }
  const raw =
    toolCall.result_preview ||
    toolCall.resultPreview ||
    previewValue(toolCall.result ?? toolCall.output)
  if (!raw) return ''
  return formatToolResultPreview(raw)
}

function stripPythonFailurePrefix(message: string): string {
  return message
    .replace(/^Python\s*(?:執行失敗|語法錯誤|執行超時|沙盒呼叫失敗)(?:（[^）]+）)?[：:]\s*/i, '')
    .trim()
}

function cleanPythonExceptionSnippet(message: string): string {
  const normalized = stripPythonFailurePrefix(message).replace(/\s+/g, ' ').trim()
  const stackBoundary = normalized.search(
    /\s+(?=Traceback \(most recent call last\):|File\s+"|File\s+'|await CodeRunner\(|coroutine =|new_error@|[0-9]+@wasm-function|\^+)/,
  )
  const clipped = stackBoundary >= 0 ? normalized.slice(0, stackBoundary) : normalized
  return compactText(clipped, 260)
}

function extractPythonException(message: string): string {
  const cleaned = message
    .replace(/\bstderr:\s*/gi, '\n')
    .replace(/\bstdout:\s*/gi, '\n')
  const stackNoise = /(pyodide\.asm\.js|wasm-function|new_error@|_pyodide)/i
  const exceptionName = /^[A-Za-z_][\w.]*(?:Error|Exception|Warning|Interrupt|Exit|Fault|Found|Denied|Timeout)\b/
  const lines = cleaned
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
  const tracebackLine = [...lines]
    .reverse()
    .find((line) => exceptionName.test(line) && !stackNoise.test(line) && !line.startsWith('PythonError: Traceback'))
  if (tracebackLine) return cleanPythonExceptionSnippet(tracebackLine)

  const inlineMatches = [
    ...cleaned.matchAll(
      /\b([A-Za-z_][\w.]*(?:Error|Exception|Warning|Interrupt|Exit|Fault|Found|Denied|Timeout)\b(?::\s*[^。\r\n]+)?)/g,
    ),
  ]
    .map((match) => cleanPythonExceptionSnippet(match[1] || ''))
    .filter((value) => value && !stackNoise.test(value) && !value.startsWith('PythonError: Traceback'))
  const inline = inlineMatches.reverse()[0]
  return inline || ''
}

function compactToolError(error: string): string {
  const lower = error.toLowerCase()
  if (
    lower.includes('pyodide.asm.js') ||
    lower.includes('wasm-function') ||
    lower.includes('traceback (most recent call last)') ||
    lower.includes('pythonerror: traceback') ||
    lower.includes('_pyodide/')
  ) {
    const exception = extractPythonException(error)
    if (exception) return `Python 執行失敗：${exception}`
    return 'Python 執行失敗。詳情已隱藏，請檢視最終回答。'
  }
  return compactText(error, 260)
}

function StatusIcon({ status }: { status: ToolCallStatus }) {
  // 僅在「即時」由非完成態切到完成態時讓 ✓ 彈入；歷史訊息（掛載即完成態）不 pop，
  // 避免切換會話時大量歷史工具的 ✓ 同時彈動造成視覺噪聲。
  const prevStatusRef = useRef(status)
  const isDone = status === 'completed' || status === 'success'
  const wasDone = prevStatusRef.current === 'completed' || prevStatusRef.current === 'success'
  const justCompleted = isDone && !wasDone
  useEffect(() => {
    prevStatusRef.current = status
  }, [status])
  if (status === 'running') {
    return <Loader2 className="shrink-0 animate-spin" size={14} />
  }
  if (isDone) {
    return (
      <CheckCircle2
        className={`shrink-0 text-[#C56646] dark:text-[#E39A78]${justCompleted ? ' chat-motion-pop' : ''}`}
        size={14}
        strokeWidth={1.9}
      />
    )
  }
  if (status === 'error') {
    return <AlertCircle className="shrink-0 text-neutral-400 dark:text-neutral-500" size={14} strokeWidth={1.9} />
  }
  if (status === 'skipped') {
    return <CircleSlash className="shrink-0" size={14} strokeWidth={1.9} />
  }
  if (status === 'cancelled') {
    return <XCircle className="shrink-0" size={14} strokeWidth={1.9} />
  }
  return <Wrench className="shrink-0" size={14} strokeWidth={1.85} />
}

function ToolTypeIcon({ toolCall, status }: { toolCall: ToolCallRecord; status: ToolCallStatus }) {
  // 行首按工具型別顯示圖示；執行/完成/待執行用工具圖示本身，出錯/跳過/取消保留專用狀態圖示。
  const prevStatusRef = useRef(status)
  const isDone = status === 'completed' || status === 'success'
  const wasDone = prevStatusRef.current === 'completed' || prevStatusRef.current === 'success'
  const justCompleted = isDone && !wasDone
  useEffect(() => {
    prevStatusRef.current = status
  }, [status])
  if (status === 'error') {
    return <AlertCircle className="shrink-0 text-neutral-400 dark:text-neutral-500" size={14} strokeWidth={1.9} />
  }
  if (status === 'skipped') {
    return <CircleSlash className="shrink-0" size={14} strokeWidth={1.9} />
  }
  if (status === 'cancelled') {
    return <XCircle className="shrink-0" size={14} strokeWidth={1.9} />
  }
  const Glyph = toolGlyph(toolCall)
  if (status === 'running') {
    return (
      <Glyph
        className="shrink-0 text-neutral-400 dark:text-neutral-500 animate-pulse"
        size={14}
        strokeWidth={1.9}
      />
    )
  }
  if (isDone) {
    return (
      <Glyph
        className={`shrink-0 text-neutral-400 dark:text-neutral-500${justCompleted ? ' chat-motion-pop' : ''}`}
        size={14}
        strokeWidth={1.9}
      />
    )
  }
  return (
    <Glyph
      className="shrink-0 text-neutral-400 dark:text-neutral-500"
      size={14}
      strokeWidth={1.9}
    />
  )
}

function DefaultToolCallBlock({
  toolCall,
  defaultOpen = false,
  labels,
}: ToolCallBlockProps) {
  const mergedLabels = { ...defaultLabels, ...labels }
  const status = normalizeToolCallStatus(toolCall.status)
  const [open, setOpen] = useState(defaultOpen)

  const toolName = getToolName(toolCall)
  const target = useMemo(() => getToolTarget(toolCall), [toolCall])
  const fileMutation = useMemo(() => structuredFileMutation(toolCall), [toolCall])
  const knowledgeHits = useMemo(() => knowledgeSearchHits(toolCall), [toolCall])
  const argumentPreview = useMemo(() => getArgumentPreview(toolCall), [toolCall])
  const resultPreview = useMemo(() => getResultPreview(toolCall), [toolCall])
  const error = toolCall.error ? compactToolError(toolCall.error) : ''
  const hasFileMutationDetails = Boolean(
    fileMutation && (
      fileMutation.files?.length ||
      fileMutation.diff ||
      fileMutation.warnings?.length ||
      fileMutation.diagnostics?.length
    ),
  )
  const hasDetails = Boolean(argumentPreview || resultPreview || error || hasFileMutationDetails || knowledgeHits)

  return (
    <div className="not-prose mb-1 text-[12.5px] leading-5 text-neutral-500 dark:text-neutral-400">
      <button
        type="button"
        onClick={() => {
          if (hasDetails) setOpen((value) => !value)
        }}
        aria-expanded={hasDetails ? open : undefined}
        className={`max-w-full min-w-0 inline-flex items-center gap-1.5 rounded-md py-0 transition-colors ${
          hasDetails
            ? 'hover:text-neutral-700 dark:hover:text-neutral-200'
            : 'cursor-default'
        }`}
      >
        <ToolTypeIcon toolCall={toolCall} status={status} />
        <span
          className={`shrink-0 font-medium text-neutral-700 dark:text-neutral-200${
            status === 'running' ? ' chat-motion-tool-shimmer' : ''
          }`}
        >
          {toolName || mergedLabels.tool}
        </span>
        {target && (
          <span className="min-w-0 truncate text-neutral-400 dark:text-neutral-500">
            {target}
          </span>
        )}
        {hasDetails && (
          <ChevronDown
            size={12}
            strokeWidth={2}
            className={`shrink-0 transition-transform duration-300 ${open ? 'rotate-180' : ''}`}
          />
        )}
      </button>

      {hasDetails && (
        <div className={`chat-motion-reveal ${open ? 'is-open' : ''}`} aria-hidden={!open}>
          <div className="mt-1.5 ml-1.5 space-y-1.5 border-l border-black/[0.08] pl-2.5 dark:border-white/[0.1]">
            {argumentPreview && (
              <div>
                <div className="text-[10.5px] font-medium text-neutral-400 dark:text-neutral-500">
                  {mergedLabels.arguments}
                </div>
                <div className="whitespace-pre-wrap break-words text-neutral-500 dark:text-neutral-400">
                  {argumentPreview}
                </div>
              </div>
            )}
            {open && resultPreview && !knowledgeHits && (
              <div>
                <div className="text-[10.5px] font-medium text-neutral-400 dark:text-neutral-500">
                  {mergedLabels.result}
                </div>
                <div className="whitespace-pre-wrap break-words text-neutral-500 dark:text-neutral-400">
                  {resultPreview}
                </div>
              </div>
            )}
            {open && knowledgeHits && <KnowledgeHits hits={knowledgeHits} />}
            {fileMutation && hasFileMutationDetails && (
              <FileMutationDetails mutation={fileMutation} />
            )}
            {error && (
              <div className="whitespace-pre-wrap break-words text-neutral-500 dark:text-neutral-400">
                {error}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  )
}

function ToolCallBlockComponent(props: ToolCallBlockProps) {
  if (isAskUserTool(props.toolCall)) {
    return <AskUserBlock toolCall={props.toolCall} />
  }
  if (isSubAgentRecord(props.toolCall)) {
    return <SubAgentCard {...props} />
  }
  return <DefaultToolCallBlock {...props} />
}

// memo：摺疊某塊 / 流式更新時，props 不變的工具塊跳過重渲染（尤其 knowledge_search 的大段源卡片）
export const ToolCallBlock = memo(ToolCallBlockComponent)
