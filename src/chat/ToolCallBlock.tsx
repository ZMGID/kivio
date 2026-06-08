import { useMemo, useState } from 'react'
import {
  AlertCircle,
  CheckCircle2,
  ChevronDown,
  CircleSlash,
  Loader2,
  Wrench,
  XCircle,
} from 'lucide-react'
import type { AgentTodoItem, AgentTodoState, AgentTodoStatus, ToolCallRecord, ToolCallStatus } from './types'
import { formatToolResultPreview } from './toolResultPreview'
import { AskUserBlock } from './AskUserBlock'

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
  pending: '等待调用',
  running: '调用中',
  success: '已完成',
  completed: '已完成',
  error: '调用失败',
  skipped: '已跳过',
  cancelled: '已取消',
  arguments: '参数',
  result: '结果',
  source: '来源',
  tool: '工具',
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

function isTodoTool(toolCall: ToolCallRecord): boolean {
  const rawName = toolRawName(toolCall)
  return rawName === 'todo_write' || rawName === 'todo_update'
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
      return '进行中'
    case 'pending':
      return '待处理'
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

function normalizeStatus(status?: string): ToolCallStatus {
  switch (status) {
    case 'running':
    case 'in_progress':
    case 'calling':
    case 'executing':
      return 'running'
    case 'completed':
    case 'success':
    case 'succeeded':
      return 'completed'
    case 'error':
    case 'failed':
      return 'error'
    case 'skipped':
      return 'skipped'
    case 'cancelled':
    case 'canceled':
      return 'cancelled'
    case 'pending':
    case 'queued':
    default:
      return 'pending'
  }
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
  const command = typeof args?.command === 'string' ? args.command : ''
  const relativePath = typeof args?.relative_path === 'string'
    ? args.relative_path
    : typeof args?.relativePath === 'string'
      ? args.relativePath
      : ''
  const hasOffset = args?.offset != null

  if (raw === 'skill_activate') return '激活 Skill'
  if (raw === 'skill_read_file') return '读取 Skill 文件'
  if (
    raw === 'skill_run_script' &&
    (relativePath.endsWith('pdf_text_digest.py') || relativePath.endsWith('pdf_extract_digest.py'))
  ) {
    return '生成 PDF 摘要上下文'
  }
  if (raw === 'skill_run_script') return '执行 Skill 脚本'
  if (raw === 'read_file') return hasOffset ? '读取文件片段' : '读取文件'
  if (raw === 'write_file') return '写入文件'
  if (raw === 'edit_file') return '编辑文件'
  if (raw === 'run_command' && /\bpdftotext\b/.test(command)) return '提取 PDF 文本'
  if (raw === 'run_command') return '终端命令'
  if (raw === 'run_python') return 'Python'
  if (raw === 'web_search') return '联网搜索'
  if (raw === 'web_fetch') return '网页抓取'
  if (raw === 'mixer_vision') return '混音器视觉分析'
  if (raw === 'mixer_generate_image') return '混音器生图'
  if (raw === 'todo_write' || raw === 'todo_update') return '更新 Todo'
  return raw
}

function getSource(toolCall: ToolCallRecord): string {
  if (isTodoTool(toolCall)) return ''
  if (toolCall.source === 'skill') return 'Skill'
  if (toolCall.source === 'native') return 'Kivio'
  if (toolCall.source === 'mixer') {
    const model = toolCall.server_id || toolCall.serverId || ''
    return model ? `混音器 · ${model}` : '混音器'
  }
  return (
    toolCall.server_name ||
    toolCall.serverName ||
    toolCall.source ||
    toolCall.server_id ||
    toolCall.serverId ||
    ''
  )
}

function getArgumentPreview(toolCall: ToolCallRecord): string {
  const rawName = toolRawName(toolCall)
  const args = parsedArguments(toolCall)
  if (rawName === 'todo_write') {
    const todos = normalizeTodoItems(args?.todos)
    const counts = formatTodoCounts(todos)
    return counts ? `清单 ${counts}` : todos.length ? `清单 ${todos.length} 项` : '替换 Todo 清单'
  }
  if (rawName === 'todo_update') {
    const content = typeof args?.content === 'string' ? compactText(args.content, 120) : ''
    const status = typeof args?.status === 'string' ? todoStatusLabel(args.status) : ''
    const id = typeof args?.id === 'string' ? compactText(args.id, 80) : ''
    const target = content || id
    return ['更新条目', status, target].filter(Boolean).join(' · ')
  }
  if (rawName === 'mixer_vision') {
    const imageCount = typeof args?.images === 'number' ? args.images : null
    const provider = typeof args?.provider === 'string' ? args.provider : ''
    const model = typeof args?.model === 'string' ? args.model : ''
    const imageLabel = imageCount == null
      ? '图片'
      : `图片 ${imageCount} 张`
    const modelLabel = [provider, model].filter(Boolean).join(' / ')
    return modelLabel ? `${imageLabel} · ${modelLabel}` : imageLabel
  }
  if (rawName === 'mixer_generate_image') {
    const prompt = typeof args?.prompt === 'string' ? compactText(args.prompt, 140) : ''
    const size = typeof args?.size === 'string' && args.size ? args.size : ''
    const quality = typeof args?.quality === 'string' && args.quality ? args.quality : ''
    const count = typeof args?.n === 'number' && Number.isFinite(args.n) ? `${args.n} 张` : ''
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
    return '已提取 PDF 文本并生成摘要上下文'
  }
  if (rawName === 'todo_write' || rawName === 'todo_update') {
    if (normalizeStatus(toolCall.status) !== 'completed') return ''
    const counts = formatTodoCounts(structuredTodoState(toolCall)?.items)
    return counts ? `已同步 ${counts}` : '已同步'
  }
  const raw =
    toolCall.result_preview ||
    toolCall.resultPreview ||
    previewValue(toolCall.result ?? toolCall.output)
  if (!raw) return ''
  return formatToolResultPreview(raw)
}

function getRunningPreview(toolCall: ToolCallRecord): string {
  const raw = toolRawName(toolCall)
  if (raw === 'todo_write' || raw === 'todo_update') {
    return '正在同步 Todo…'
  }
  if (raw === 'run_python') {
    return '正在加载 Python 环境…'
  }
  if (raw === 'mixer_vision') {
    return '正在分析图片并提取视觉信息…'
  }
  if (raw === 'mixer_generate_image') {
    return '正在生成图片…'
  }
  return ''
}

function stripPythonFailurePrefix(message: string): string {
  return message
    .replace(/^Python\s*(?:执行失败|语法错误|执行超时|沙盒调用失败)(?:（[^）]+）)?[：:]\s*/i, '')
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
    if (exception) return `Python 执行失败：${exception}`
    return 'Python 执行失败。详情已隐藏，请查看最终回答。'
  }
  return compactText(error, 260)
}

function StatusIcon({ status }: { status: ToolCallStatus }) {
  if (status === 'running') {
    return <Loader2 className="shrink-0 animate-spin" size={12} />
  }
  if (status === 'completed') {
    return (
      <CheckCircle2
        className="shrink-0 text-[#C56646] dark:text-[#E39A78]"
        size={12}
        strokeWidth={1.9}
      />
    )
  }
  if (status === 'error') {
    return <AlertCircle className="shrink-0 text-red-500" size={12} strokeWidth={1.9} />
  }
  if (status === 'skipped') {
    return <CircleSlash className="shrink-0" size={12} strokeWidth={1.9} />
  }
  if (status === 'cancelled') {
    return <XCircle className="shrink-0" size={12} strokeWidth={1.9} />
  }
  return <Wrench className="shrink-0" size={12} strokeWidth={1.85} />
}

function DefaultToolCallBlock({
  toolCall,
  defaultOpen = false,
  labels,
}: ToolCallBlockProps) {
  const mergedLabels = { ...defaultLabels, ...labels }
  const status = normalizeStatus(toolCall.status)
  const [open, setOpen] = useState(defaultOpen)

  const toolName = getToolName(toolCall)
  const source = getSource(toolCall)
  const duration = formatDuration(getDuration(toolCall))
  const argumentPreview = useMemo(() => getArgumentPreview(toolCall), [toolCall])
  const resultPreview = useMemo(() => getResultPreview(toolCall), [toolCall])
  const error = toolCall.error ? compactToolError(toolCall.error) : ''
  const rowPreview = error || resultPreview || (status === 'running' ? getRunningPreview(toolCall) : '') || argumentPreview
  const hasDetails = Boolean(argumentPreview || resultPreview || error)

  return (
    <div className="not-prose mb-2 text-[11.5px] leading-5 text-neutral-500 dark:text-neutral-400">
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
        } ${status === 'running' ? 'chat-motion-soft-pulse px-1 text-neutral-600 dark:text-neutral-300' : ''}`}
      >
        <StatusIcon status={status} />
        <span className="shrink-0 font-medium text-neutral-700 dark:text-neutral-200">
          {toolName || mergedLabels.tool}
        </span>
        {source && (
          <span className="min-w-0 truncate text-neutral-400 dark:text-neutral-500">
            · {source}
          </span>
        )}
        <span className="shrink-0 text-neutral-400 dark:text-neutral-500">
          · {mergedLabels[status]}
        </span>
        {duration && (
          <span className="shrink-0 tabular-nums text-neutral-400 dark:text-neutral-500">
            · {duration}
          </span>
        )}
        {rowPreview && (
          <span
            className={`min-w-0 truncate ${
              error ? 'text-red-500' : 'text-neutral-400 dark:text-neutral-500'
            }`}
          >
            · {rowPreview}
          </span>
        )}
        {hasDetails && (
          <ChevronDown
            size={11}
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
            {resultPreview && (
              <div>
                <div className="text-[10.5px] font-medium text-neutral-400 dark:text-neutral-500">
                  {mergedLabels.result}
                </div>
                <div className="whitespace-pre-wrap break-words text-neutral-500 dark:text-neutral-400">
                  {resultPreview}
                </div>
              </div>
            )}
            {error && (
              <div className="whitespace-pre-wrap break-words text-red-500">
                {error}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  )
}

export function ToolCallBlock(props: ToolCallBlockProps) {
  if (isAskUserTool(props.toolCall)) {
    return <AskUserBlock toolCall={props.toolCall} />
  }
  return <DefaultToolCallBlock {...props} />
}
