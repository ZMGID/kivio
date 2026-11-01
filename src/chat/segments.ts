import type { ChatMessageSegment, ToolCallRecord } from './types'
import { foldToolName } from './askUserTools'
import { normalizeToolCallStatus } from './toolStatus'
import { isImageArtifact } from './artifacts'

export function segmentToolCallId(segment: ChatMessageSegment): string {
  return segment.tool_call_id ?? segment.toolCallId ?? ''
}

export function toolRecordRawName(toolCall: ToolCallRecord): string {
  return toolCall.tool_name || toolCall.toolName || toolCall.name || ''
}

/**
 * 展示/分类用的规范工具名：小写 + 去分隔符后查别名。
 * dsh 在 Windows 上把 bash 报成 `pwsh`；claude 是 PascalCase 的 `Bash` / `Read`。
 */
const TOOL_NAME_ALIASES: Record<string, string> = {
  webfetch: 'web_fetch',
  websearch: 'web_search',
  todowrite: 'todo_write',
  multiedit: 'edit',
  notebookedit: 'edit',
  askuserquestion: 'ask_user',
  pwsh: 'bash',
  powershell: 'bash',
  cmd: 'bash',
  shell: 'bash',
  joboutput: 'run_command',
  joblist: 'run_command',
  jobkill: 'run_command',
  getgoal: 'todo_write',
  creategoal: 'todo_write',
  updategoal: 'todo_write',
  readimage: 'read',
  runcode: 'bash',
  subagent: 'agent',
  subagentfork: 'agent',
  listagents: 'agent',
  sendmessage: 'agent',
  interruptagent: 'agent',
  workflow: 'agent',
  ralph: 'agent',
  cordisdefine: 'run_command',
  cordisrun: 'run_command',
  cordisstop: 'run_command',
  cordisundefine: 'run_command',
}

function foldedToolName(toolCall: ToolCallRecord): string {
  return toolRecordRawName(toolCall).toLowerCase().replace(/[_\-\s]/g, '')
}

export function canonicalToolName(toolCall: ToolCallRecord): string {
  const folded = foldedToolName(toolCall)
  if (folded === 'strreplaceeditor') {
    const command = recordString(parsedToolArguments(toolCall)?.command)
    if (command === 'view') return 'read'
    if (command === 'create') return 'write'
    return 'edit'
  }
  if (folded.startsWith('cordisinspect')) return 'read'
  return TOOL_NAME_ALIASES[folded] ?? toolRecordRawName(toolCall).toLowerCase()
}

/** Edit/Write 类工具名（含外部 CLI 的 Edit/Write/MultiEdit/NotebookEdit 变体）。 */
function isFileWriteToolName(toolCall: ToolCallRecord): boolean {
  const name = canonicalToolName(toolCall)
  return name === 'write' || name === 'write_file' || name === 'edit' || name === 'edit_file'
}

function recordObject(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null
}

function recordString(value: unknown): string {
  return typeof value === 'string' ? value : ''
}

function parsedToolArguments(toolCall: ToolCallRecord): Record<string, unknown> | null {
  const value = toolCall.arguments ?? toolCall.args ?? toolCall.input
  if (typeof value !== 'string') return recordObject(value)
  try {
    return recordObject(JSON.parse(value))
  } catch {
    return null
  }
}

export interface DiffStats {
  additions: number
  removals: number
}

/** Edit/Write 类工具的 `+N -N`：优先读后端 structured_content 的真实 additions/removals；
 *  外部 CLI 的记录没有统计时，按参数新旧文本（old_string/new_string/edits[]/content）的
 *  行数估算。非文件写入工具、未完成的调用返回 null。
 *  ponytail: replace_all 的多处命中会被低估成一处，参数即所见，够用。 */
export function toolCallDiffStats(toolCall: ToolCallRecord): DiffStats | null {
  if (!isFileWriteToolName(toolCall)) return null
  if (normalizeToolCallStatus(toolCall.status) !== 'completed') return null

  const structured = recordObject(toolCall.structured_content ?? toolCall.structuredContent)
  if (structured && !recordObject(structured.toolDraft)) {
    const files = Array.isArray(structured.files) ? structured.files.map(recordObject) : []
    if (typeof structured.additions === 'number' || typeof structured.removals === 'number' || files.length) {
      let additions = typeof structured.additions === 'number' ? structured.additions : 0
      let removals = typeof structured.removals === 'number' ? structured.removals : 0
      if (!additions && !removals) {
        for (const file of files) {
          additions += typeof file?.additions === 'number' ? file.additions : 0
          removals += typeof file?.removals === 'number' ? file.removals : 0
        }
      }
      return { additions, removals }
    }
  }

  const args = parsedToolArguments(toolCall)
  const lines = (text: string) => (text ? text.split('\n').length : 0)
  let additions = 0
  let removals = 0
  const edits = Array.isArray(args?.edits) ? args.edits : null
  if (edits) {
    for (const edit of edits) {
      const pair = recordObject(edit)
      additions += lines(recordString(pair?.new_string))
      removals += lines(recordString(pair?.old_string))
    }
  } else if (
    recordString(args?.old_string)
    || recordString(args?.new_string)
    || recordString(args?.old_str)
    || recordString(args?.new_str)
  ) {
    additions = lines(recordString(args?.new_string) || recordString(args?.new_str))
    removals = lines(recordString(args?.old_string) || recordString(args?.old_str))
  } else {
    additions = lines(recordString(args?.content) || recordString(args?.text))
  }
  return additions || removals ? { additions, removals } : null
}

/**
 * 用户在生成中「立刻引导」插进来的那句话（后端合成的 display-only 记录，见
 * `chat/agent/steering.rs`）。渲染成一条用户小气泡而不是工具卡。
 *
 * 三条判据全要 —— 这张卡呈现的是「用户说过的话」，冒充它比冒充一张搜索卡严重：
 * 只有 native 通道、保留工具名、structured type 三者同时对上才认。
 */
export function isUserSteerToolCall(toolCall: ToolCallRecord): boolean {
  if (toolCall.source !== 'native') return false
  if (toolRecordRawName(toolCall) !== 'user_steer') return false
  const structured = toolCall.structured_content ?? toolCall.structuredContent
  if (!structured || typeof structured !== 'object') return false
  return (structured as { type?: unknown }).type === 'user_steer'
}

/** 插话卡里那句话（空串 = 不是插话卡 / 载荷缺字段）。 */
export function userSteerText(toolCall: ToolCallRecord): string {
  const structured = toolCall.structured_content ?? toolCall.structuredContent
  if (!structured || typeof structured !== 'object') return ''
  const text = (structured as { text?: unknown }).text
  return typeof text === 'string' ? text : ''
}

export function isUserFollowUpToolCall(toolCall: ToolCallRecord): boolean {
  if (toolCall.source !== 'native') return false
  if (toolRecordRawName(toolCall) !== 'user_follow_up') return false
  const structured = toolCall.structured_content ?? toolCall.structuredContent
  if (!structured || typeof structured !== 'object') return false
  return (structured as { type?: unknown }).type === 'user_follow_up'
}

export function userFollowUpText(toolCall: ToolCallRecord): string {
  const structured = toolCall.structured_content ?? toolCall.structuredContent
  if (!structured || typeof structured !== 'object') return ''
  const text = (structured as { text?: unknown }).text
  return typeof text === 'string' ? text : ''
}

function structuredStringField(
  toolCall: ToolCallRecord,
  snake: string,
  camel: string,
): string | null {
  const structured = toolCall.structured_content ?? toolCall.structuredContent
  if (!structured || typeof structured !== 'object') return null
  const value = (structured as Record<string, unknown>)[snake]
    ?? (structured as Record<string, unknown>)[camel]
  return typeof value === 'string' ? value : null
}

/** 插话卡上的前端队列 id。蛇/驼峰都认，协议层 Value 原样穿过。 */
export function userSteerId(toolCall: ToolCallRecord): string | null {
  if (!isUserSteerToolCall(toolCall)) return null
  return structuredStringField(toolCall, 'steer_id', 'steerId')
}

export function userFollowUpId(toolCall: ToolCallRecord): string | null {
  if (!isUserFollowUpToolCall(toolCall)) return null
  return structuredStringField(toolCall, 'follow_up_id', 'followUpId')
}

/** 外部 CLI 的子代理工具调用：claude 新版报 `Agent`、旧版报 `Task`；dsh 报
 *  `subagent` / `subagent_fork` / `workflow` / `ralph`（source 恒为 `external_cli`）。
 *  精确匹配整名，MCP/native 不受影响。 */
export function isExternalSubagentToolCall(toolCall: ToolCallRecord): boolean {
  if (toolCall.source !== 'external_cli') return false
  const name = foldedToolName(toolCall)
  return name === 'agent'
    || name === 'task'
    || name === 'subagent'
    || name === 'subagentfork'
    || name === 'workflow'
    || name === 'ralph'
}

const IMAGE_READ_EXT = /\.(png|jpe?g|gif|webp|bmp|tiff?|heic|heif)$/i

function toolCallArgObject(toolCall: ToolCallRecord): Record<string, unknown> | null {
  const raw = toolCall.arguments ?? toolCall.args ?? toolCall.input
  if (!raw) return null
  if (typeof raw === 'object' && !Array.isArray(raw)) return raw as Record<string, unknown>
  if (typeof raw === 'string') {
    try {
      const parsed = JSON.parse(raw) as unknown
      if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
        return parsed as Record<string, unknown>
      }
    } catch {
      return null
    }
  }
  return null
}

function toolCallPathList(toolCall: ToolCallRecord): string[] {
  const args = toolCallArgObject(toolCall)
  const paths: string[] = []
  const extra = args?.paths
  if (Array.isArray(extra)) {
    for (const item of extra) {
      if (typeof item === 'string' && item.trim()) paths.push(item.trim())
    }
  }
  const path = args?.path ?? args?.file_path ?? args?.filePath
  if (typeof path === 'string' && path.trim() && !paths.includes(path.trim())) {
    paths.unshift(path.trim())
  }
  return paths
}

function pathBasename(path: string): string {
  const slash = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'))
  return slash >= 0 ? path.slice(slash + 1) : path
}

/** `read` 正在看图片：结果带 image_read / 图片 artifact，或参数路径是图片。 */
export function isImageReadToolCall(toolCall: ToolCallRecord): boolean {
  const status = normalizeToolCallStatus(toolCall.status)
  if (status === 'error' || status === 'cancelled' || status === 'skipped') return false
  const folded = foldToolName(canonicalToolName(toolCall))
  if (folded !== 'read' && folded !== 'readfile') return false
  const structured = toolCall.structured_content ?? toolCall.structuredContent
  if (structured && typeof structured === 'object' && (structured as { type?: unknown }).type === 'image_read') {
    return true
  }
  if ((toolCall.artifacts ?? []).some(isImageArtifact)) return true
  return toolCallPathList(toolCall).some((path) => IMAGE_READ_EXT.test(path))
}

export type ImageReadItem = {
  id?: string
  path: string
  name: string
  dataUrl: string
}

export function imageReadItems(toolCall: ToolCallRecord): ImageReadItem[] {
  const fromArtifacts = (toolCall.artifacts ?? []).filter(isImageArtifact).map((artifact) => {
    const path = artifact.path ?? artifact.filePath ?? artifact.localPath ?? ''
    return {
      id: artifact.id ?? undefined,
      path,
      name: artifact.name || pathBasename(path) || 'image',
      dataUrl: artifact.dataUrl ?? artifact.data_url ?? '',
    }
  })
  if (fromArtifacts.length > 0) return fromArtifacts
  return toolCallPathList(toolCall)
    .filter((path) => IMAGE_READ_EXT.test(path))
    .map((path) => ({ path, name: pathBasename(path) || 'image', dataUrl: '' }))
}

export function imageReadCount(toolCall: ToolCallRecord): number {
  const items = imageReadItems(toolCall)
  if (items.length > 0) return items.length
  const structured = toolCall.structured_content ?? toolCall.structuredContent
  if (structured && typeof structured === 'object') {
    const count = (structured as { count?: unknown }).count
    if (typeof count === 'number' && count > 0) return count
  }
  return 1
}

export type ToolDisplayCluster =
  | { type: 'imageRead'; toolCalls: ToolCallRecord[] }
  | { type: 'tool'; toolCall: ToolCallRecord }

/** 连续的读图调用收成一组，好画成一行缩略图。 */
export function clusterToolCallsForDisplay(toolCalls: ToolCallRecord[]): ToolDisplayCluster[] {
  const items: ToolDisplayCluster[] = []
  let cluster: ToolCallRecord[] | null = null
  const flush = () => {
    if (cluster?.length) items.push({ type: 'imageRead', toolCalls: cluster })
    cluster = null
  }
  for (const toolCall of toolCalls) {
    if (isImageReadToolCall(toolCall)) {
      if (!cluster) cluster = []
      cluster.push(toolCall)
      continue
    }
    flush()
    items.push({ type: 'tool', toolCall })
  }
  flush()
  return items
}

/** tool record 的唯一 id（兼容多种字段命名）。 */
export function toolRecordId(toolCall: ToolCallRecord): string {
  return toolCall.id || toolCall.toolCallId || toolCall.call_id || toolCall.callId || ''
}

export function segmentStepNumber(segment: ChatMessageSegment): number | null | undefined {
  return segment.step_number ?? segment.stepNumber
}

function segmentDisplayRank(segment: ChatMessageSegment): number {
  if (segment.kind === 'reasoning') return 0
  if (segment.kind === 'text') return 1
  return 2
}

export function compareTimelineSegments(
  a: ChatMessageSegment,
  b: ChatMessageSegment,
): number {
  const aStepNumber = segmentStepNumber(a)
  const bStepNumber = segmentStepNumber(b)
  const sameModelStep =
    aStepNumber != null &&
    aStepNumber === bStepNumber &&
    (a.round ?? null) === (b.round ?? null) &&
    a.phase === b.phase
  if (sameModelStep) {
    const rankDelta = segmentDisplayRank(a) - segmentDisplayRank(b)
    if (rankDelta !== 0) return rankDelta
  }
  return a.order - b.order
}

/** 渲染前的「有内容」判定：reasoning/text 段空白则不渲染，也不应单独成组/打断分组。
 *  tool 段始终保留（其记录可能缺失，交由 UI 兜底）。 */
function segmentHasContent(segment: ChatMessageSegment): boolean {
  if (segment.kind === 'tool') return true
  return Boolean((segment.text ?? '').trim())
}

export type TimelineGroupItem =
  | { type: 'text'; segment: ChatMessageSegment }
  | { type: 'presentation'; segment: ChatMessageSegment }
  | { type: 'group'; segments: ChatMessageSegment[] }

/** 过程按交付位置分段展示，各段共享同一个 Work 开关。
 * 过程段在中断后仍属于过程；保留末尾答复和无法判定为过程的部分正文。
 * 这里只改变展示，不改存储正文、复制或模型回放。
 */
export function groupTimelineSegments(
  orderedSegments: ChatMessageSegment[],
  state: 'running' | 'completed' | 'stopped' = 'completed',
  isPresentation?: (segment: ChatMessageSegment) => boolean,
): TimelineGroupItem[] {
  // 交付卡属于结果，不应切分 Work，也不应成为隐藏前面正文的过程边界。
  const presentations = new Set(orderedSegments.filter(segment =>
    segment.kind === 'tool' && isPresentation?.(segment)))
  let lastProcessIndex = -1
  let lastActivityIndex = -1
  orderedSegments.forEach((segment, index) => {
    if (presentations.has(segment)) return
    if (segmentHasContent(segment) && segment.kind !== 'text') {
      lastActivityIndex = index
    }
    if (segmentHasContent(segment) && (segment.kind !== 'text'
      || segment.phase === 'tool_loop' || segment.phase === 'auxiliary')) {
      lastProcessIndex = index
    }
  })
  const hasFinalAnswer = orderedSegments.some((segment, index) =>
    index > lastProcessIndex && segment.kind === 'text' && segmentHasContent(segment)
    && !/^seg_\d+_cancelled_synthesis$/.test(segment.id)
    && (segment.phase === 'plain' || segment.phase === 'synthesis'))
  const hasCancellationNotice = orderedSegments.some(segment =>
    segment.kind === 'text' && segmentHasContent(segment)
    && /^seg_\d+_cancelled_synthesis$/.test(segment.id))
  let process: ChatMessageSegment[] = []
  const body: TimelineGroupItem[] = []
  const flushProcess = () => {
    if (!process.length) return
    body.push({ type: 'group', segments: process })
    process = []
  }
  orderedSegments.forEach((segment, index) => {
    if (!segmentHasContent(segment)) return
    if (presentations.has(segment)) {
      flushProcess()
      body.push({ type: 'presentation', segment })
      return
    }
    const foldText = state === 'running'
      // A live model reply still carries tool_loop until the round finishes.
      // Only subsequent reasoning/tools establish that it was progress text;
      // its phase alone must not pull the streaming answer above deliveries.
      ? index <= lastActivityIndex
      // Stopping does not turn explicit progress into a final answer. Keep this
      // independent of the saved outcome, which older history may not contain.
      : ((segment.phase === 'tool_loop' || segment.phase === 'auxiliary')
          && (state === 'stopped' || hasCancellationNotice || hasFinalAnswer || index <= lastActivityIndex))
        || (hasFinalAnswer && index <= lastProcessIndex)
    if (segment.kind === 'text' && !foldText) {
      flushProcess()
      body.push({ type: 'text', segment })
    } else {
      process.push(segment)
    }
  })
  flushProcess()
  return body
}

/** 后端 `started_at` 是 unix 秒；个别路径会写毫秒。 */
function timestampToMs(value: number): number {
  return value < 1e12 ? value * 1000 : value
}

function matchedGroupTools(
  segments: ChatMessageSegment[],
  toolCalls: ToolCallRecord[],
  toolCallById?: ReadonlyMap<string, ToolCallRecord>,
): ToolCallRecord[] {
  const matched: ToolCallRecord[] = []
  for (const segment of segments) {
    if (segment.kind !== 'tool') continue
    const id = segmentToolCallId(segment)
    const record = toolCallById
      ? toolCallById.get(id)
      : toolCalls.find((tool) => toolRecordId(tool) === id)
    if (record) matched.push(record)
  }
  return matched
}

/**
 * 一组过程的墙钟耗时（Codex「Worked for Xs」）。
 * 取组内工具最早 start → 最晚 complete；没有时间戳时回退思考耗时。
 */
export function groupWorkDurationMs(
  segments: ChatMessageSegment[],
  toolCalls: ToolCallRecord[],
  toolCallById?: ReadonlyMap<string, ToolCallRecord>,
  reasoningDurationMs?: number | null,
): number | null {
  let minStart: number | null = null
  let maxEnd: number | null = null
  for (const tool of matchedGroupTools(segments, toolCalls, toolCallById)) {
    const start = tool.started_at ?? tool.startedAt
    const end = tool.completed_at ?? tool.completedAt
    if (typeof start === 'number') {
      minStart = minStart == null ? start : Math.min(minStart, start)
    }
    if (typeof end === 'number') {
      maxEnd = maxEnd == null ? end : Math.max(maxEnd, end)
    }
  }
  if (minStart != null && maxEnd != null && maxEnd >= minStart) {
    const delta = timestampToMs(maxEnd) - timestampToMs(minStart)
    if (delta > 0) return delta
  }
  if (reasoningDurationMs != null && reasoningDurationMs > 0) return reasoningDurationMs
  return null
}

export function formatWorkDuration(ms: number): string {
  const totalSeconds = Math.max(1, Math.round(ms / 1000))
  if (totalSeconds < 60) return `${totalSeconds}s`
  const minutes = Math.floor(totalSeconds / 60)
  const seconds = totalSeconds % 60
  return seconds > 0 ? `${minutes}m ${seconds}s` : `${minutes}m`
}

export type ToolGroupCategory =
  | 'read'
  | 'codeSearch'
  | 'globFiles'
  | 'fileWrite'
  | 'runCommand'
  | 'webFetch'
  | 'webSearch'
  | 'listDir'
  | 'fileOps'
  | 'todo'
  | 'memory'
  | 'subAgent'
  | 'skill'
  | 'image'
  | 'notion'
  | 'mcp'
  | 'other'

/** 分组头图标用的代表类别：工具类别全集 + 纯思考组的 `'reasoning'`。 */
export type ToolGroupIcon = ToolGroupCategory | 'reasoning'

function categorizeTool(toolCall: ToolCallRecord): ToolGroupCategory {
  const raw = canonicalToolName(toolCall)
  switch (raw) {
    case 'read':
    case 'read_file':
      return 'read'
    case 'grep':
    case 'search_files':
      return 'codeSearch'
    case 'find':
    case 'glob':
    case 'glob_files':
      return 'globFiles'
    case 'write':
    case 'write_file':
    case 'edit':
    case 'edit_file':
      return 'fileWrite'
    case 'bash':
    case 'run_command':
      return 'runCommand'
    case 'web_fetch':
      return 'webFetch'
    case 'web_search':
      return 'webSearch'
    case 'ls':
    case 'list_dir':
      return 'listDir'
    case 'move':
    case 'copy':
    case 'delete':
    case 'create_dir':
    case 'stat':
    case 'stat_path':
      return 'fileOps'
    case 'todo_write':
    case 'todo_update':
      return 'todo'
    case 'memory_read':
    case 'memory_search':
    case 'memory_modify':
      return 'memory'
    case 'agent':
      return 'subAgent'
    case 'skill':
    case 'skill_activate':
      return 'skill'
    case 'mixer_vision':
    case 'mixer_generate_image':
      return 'image'
    default:
      break
  }
  const server = (toolCall.server_name || toolCall.serverName || toolCall.server_id || toolCall.serverId || '')
    .toLowerCase()
  if (server.includes('notion') || raw.toLowerCase().startsWith('notion')) {
    return 'notion'
  }
  const isMcp =
    toolCall.source === 'mcp' ||
    (Boolean(toolCall.server_name || toolCall.serverName) &&
      toolCall.source !== 'native' &&
      toolCall.source !== 'skill' &&
      toolCall.source !== 'mixer')
  if (isMcp) return 'mcp'
  return 'other'
}

/** 去重（保持首次出现顺序）并剔除 `'other'` 后的「有意义类别」集合，文案与图标共用同一判定。 */
function meaningfulCategories(categories: ToolGroupCategory[]): ToolGroupCategory[] {
  const seen = new Set<ToolGroupCategory>()
  const result: ToolGroupCategory[] = []
  for (const category of categories) {
    if (category === 'other' || seen.has(category)) continue
    seen.add(category)
    result.push(category)
  }
  return result
}

/**
 * 每个类别的「动作片段」（不带时态前缀、不带状态后缀）。
 * n = 该类别下的工具数；部分类别不带数量。
 * Codex 风格：动词 + 数量 + 宾语，由调用方加「已/正在」前缀。
 */
function categoryFragment(category: ToolGroupCategory, count: number, fileCountKnown = true): string {
  switch (category) {
    case 'read':
      return fileCountKnown ? `读取 ${count} 个文件` : `读取 ${count} 次`
    case 'fileWrite':
      return fileCountKnown ? `编辑 ${count} 个文件` : `编辑 ${count} 次`
    case 'runCommand':
      return `执行 ${count} 条命令`
    case 'webFetch':
      return `读取 ${count} 个网页`
    case 'listDir':
      return `浏览 ${count} 个目录`
    case 'fileOps':
      return `处理 ${count} 个文件`
    case 'codeSearch':
      return '搜索代码'
    case 'webSearch':
      return '搜索网络'
    case 'globFiles':
      return '查找文件'
    case 'todo':
      return '更新任务清单'
    case 'memory':
      return '检索记忆'
    case 'subAgent':
      return '调度 Subagent'
    case 'skill':
      return '运行技能'
    case 'image':
      return '处理图像'
    case 'notion':
      return '检索 Notion'
    case 'mcp':
      return '调用外部工具'
    case 'other':
    default:
      return '工具调用'
  }
}

/** 代表类别：单一有意义类别时取该类别，混合/未知时回退 `'other'`（与文案选择保持一致）。 */
function representativeCategory(categories: ToolGroupCategory[]): ToolGroupCategory {
  const meaningful = meaningfulCategories(categories)
  return meaningful.length === 1 ? meaningful[0] : 'other'
}

export interface ToolGroupSummary {
  text: string
  status: 'running' | 'error' | 'done'
  /** 失败数：折叠摘要刻意不显示（命令失败是探索常态，展开后单行仍可见）。 */
  failedCount: number
  stoppedCount: number
  /** 折叠头图标用的代表类别。 */
  icon: ToolGroupIcon
  /** 组内涉及的「有意义类别」列表（去重、保持首次出现顺序、剔除 `'other'`）。
   *  混合类别时用于在摘要后排一行各类工具图标；纯 reasoning 组为 `[]`。 */
  categories: ToolGroupIcon[]
  /** 组内所有 Edit/Write 的 `+N -N` 总计；没有文件写入（或行数全为 0）时为 null。 */
  diffStats: DiffStats | null
}

/** Generate a bounded semantic summary while retaining all source categories and outcomes. */
export function summarizeToolGroup(
  segments: ChatMessageSegment[],
  toolCalls: ToolCallRecord[],
  toolCallById?: ReadonlyMap<string, ToolCallRecord>,
  active = true,
): ToolGroupSummary {
  const toolSegments = segments.filter((segment) => segment.kind === 'tool')
  // 「步数」按工具步计；纯 reasoning 组（无工具）回退到总段数。
  const stepCount = toolSegments.length || segments.length
  const matchedTools = matchedGroupTools(segments, toolCalls, toolCallById)

  const categories = matchedTools.map((tool) => categorizeTool(tool))
  const meaningful = meaningfulCategories(categories)

  // 图标代表类别：无工具段（纯 reasoning 组）→ 'reasoning'；否则取代表类别。
  const icon: ToolGroupIcon = toolSegments.length
    ? representativeCategory(categories)
    : 'reasoning'

  const statuses = matchedTools.map((tool) => normalizeToolCallStatus(tool.status))
  const running = active && statuses.some((status) => status === 'running' || status === 'pending')
  const failed = statuses.filter((status) => status === 'error').length
  const stopped = statuses.filter((status) => status === 'cancelled' || status === 'skipped').length

  const status: ToolGroupSummary['status'] = running ? 'running' : failed > 0 ? 'error' : 'done'

  // 选出本组的「动作片段」数组（不带时态前缀）。
  const fragments = buildGroupFragments(categories, meaningful, matchedTools, toolSegments.length, stepCount)

  let text = running ? `正在${fragments.join('和')}…` : fragments.join('和')
  // 失败不进摘要：外部 CLI 的探查命令经常非零退出，「N 项失败」只是噪音。
  if (stopped > 0) text += `，${stopped} 项已停止`

  return {
    text,
    status,
    failedCount: failed,
    stoppedCount: stopped,
    icon,
    categories: meaningful,
    diffStats: matchedTools.reduce<DiffStats | null>((total, tool) => {
      const stats = toolCallDiffStats(tool)
      if (!stats) return total
      return {
        additions: (total?.additions ?? 0) + stats.additions,
        removals: (total?.removals ?? 0) + stats.removals,
      }
    }, null),
  }
}

/** Select up to two categories, counting all remaining tool actions separately. */
function buildGroupFragments(
  categories: ToolGroupCategory[],
  meaningful: ToolGroupCategory[],
  tools: ToolCallRecord[],
  toolSegmentCount: number,
  stepCount: number,
): string[] {
  if (toolSegmentCount === 0) return ['思考']
  if (meaningful.length === 0) return [`调用 ${stepCount} 次工具`]
  const selected = [
    ...(['fileWrite', 'runCommand'] as ToolGroupCategory[]).filter((category) => meaningful.includes(category)),
    ...meaningful.filter((category) => category !== 'fileWrite' && category !== 'runCommand'),
  ].slice(0, 2)
  const fragments = selected.map((category) => {
    const categoryTools = tools.filter((_, index) => categories[index] === category)
    if (category === 'read' || category === 'fileWrite') {
      const paths = new Set<string>()
      let allKnown = true
      for (const tool of categoryTools) {
        const args = toolCallArgObject(tool)
        const toolPaths = toolCallPathList(tool)
        const extraPath = args?.notebook_path ?? args?.notebookPath ?? args?.relative_path ?? args?.relativePath
        if (typeof extraPath === 'string' && extraPath.trim()) toolPaths.push(extraPath.trim())
        if (!toolPaths.length || (Array.isArray(args?.paths) && args.paths.some(
          (path: unknown) => typeof path !== 'string' || !path.trim(),
        ))) allKnown = false
        for (const path of toolPaths) paths.add(path)
      }
      return categoryFragment(category, allKnown ? paths.size : categoryTools.length, allKnown)
    }
    return categoryFragment(category, categoryTools.length)
  })
  const remaining = toolSegmentCount - categories.filter((category) => selected.includes(category)).length
  if (remaining > 0) fragments.push(`其他 ${remaining} 项操作`)
  return fragments
}
