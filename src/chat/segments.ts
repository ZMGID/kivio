import type { ChatMessageSegment, ToolCallRecord } from './types'
import { normalizeToolCallStatus } from './toolStatus'

export function segmentToolCallId(segment: ChatMessageSegment): string {
  return segment.tool_call_id ?? segment.toolCallId ?? ''
}

export function toolRecordRawName(toolCall: ToolCallRecord): string {
  return toolCall.tool_name || toolCall.toolName || toolCall.name || ''
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
  | { type: 'group'; segments: ChatMessageSegment[] }

/**
 * 以正文(text)段为分隔，把两条正文之间连续的非 text 段（reasoning + tool）聚成一个组。
 * - 纯函数：输入有序 segments → 输出渲染项数组，便于单测。
 * - text 段单独成项（原样渲染正文），永远打断分组。
 * - `tool → text → tool` ⇒ 两个组。
 * - 空白 reasoning/text 段先过滤，避免产生空组或多余分隔。
 */
export function groupTimelineSegments(orderedSegments: ChatMessageSegment[]): TimelineGroupItem[] {
  const items: TimelineGroupItem[] = []
  let current: ChatMessageSegment[] | null = null
  for (const segment of orderedSegments) {
    if (!segmentHasContent(segment)) continue
    if (segment.kind === 'text') {
      current = null
      items.push({ type: 'text', segment })
      continue
    }
    if (!current) {
      current = []
      items.push({ type: 'group', segments: current })
    }
    current.push(segment)
  }
  return items
}

export type ToolGroupCategory =
  | 'read'
  | 'codeSearch'
  | 'globFiles'
  | 'fileWrite'
  | 'runCommand'
  | 'webFetch'
  | 'webSearch'
  | 'runPython'
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

const CATEGORY_LABELS: Record<ToolGroupCategory, string> = {
  read: '读取文件',
  codeSearch: '代码搜索',
  globFiles: '查找文件',
  fileWrite: '编辑文件',
  runCommand: '执行命令',
  webFetch: '网页读取',
  webSearch: '网络搜索',
  runPython: '运行代码',
  listDir: '浏览目录',
  fileOps: '文件操作',
  todo: '更新任务清单',
  memory: '记忆检索',
  subAgent: '子 Agent 协作',
  skill: '运行技能',
  image: '图像处理',
  notion: 'Notion 搜索与读取',
  mcp: '外部工具调用',
  other: '工具调用',
}

function categorizeTool(toolCall: ToolCallRecord): ToolGroupCategory {
  const raw = toolRecordRawName(toolCall)
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
    case 'run_python':
      return 'runPython'
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
    case 'skill_activate':
    case 'skill_read_file':
    case 'skill_run_script':
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

function describeCategories(categories: ToolGroupCategory[]): string {
  // 全部归到「other」（无法分类）→ 回退通用文案
  const meaningful = meaningfulCategories(categories)
  if (meaningful.length === 0) return CATEGORY_LABELS.other
  if (meaningful.length === 1) return CATEGORY_LABELS[meaningful[0]]
  if (meaningful.length === 2) {
    return `${CATEGORY_LABELS[meaningful[0]]}与${CATEGORY_LABELS[meaningful[1]]}`
  }
  return '多类工具调用'
}

/** 代表类别：单一有意义类别时取该类别，混合/未知时回退 `'other'`（与文案选择保持一致）。 */
function representativeCategory(categories: ToolGroupCategory[]): ToolGroupCategory {
  const meaningful = meaningfulCategories(categories)
  return meaningful.length === 1 ? meaningful[0] : 'other'
}

export interface ToolGroupSummary {
  text: string
  status: 'running' | 'error' | 'done'
  /** 折叠头图标用的代表类别。 */
  icon: ToolGroupIcon
  /** 组内涉及的「有意义类别」列表（去重、保持首次出现顺序、剔除 `'other'`）。
   *  混合类别时用于在摘要后排一行各类工具图标；纯 reasoning 组为 `[]`。 */
  categories: ToolGroupIcon[]
}

/**
 * 为一个分组生成自然语言摘要：按组内工具 rawName 归类 + ` · N 步` + 状态后缀。
 * 状态后缀：进行中…（任一工具 running）/ N 失败（有失败）/ 已完成。
 * 分类不出明确类别时回退 `工具调用 · N 步`。
 */
export function summarizeToolGroup(
  segments: ChatMessageSegment[],
  toolCalls: ToolCallRecord[],
): ToolGroupSummary {
  const toolSegments = segments.filter((segment) => segment.kind === 'tool')
  // 「步数」按工具步计；纯 reasoning 组（无工具）回退到总段数。
  const stepCount = toolSegments.length || segments.length
  const matchedTools: ToolCallRecord[] = []
  for (const segment of toolSegments) {
    const id = segmentToolCallId(segment)
    const record = toolCalls.find((tool) => toolRecordId(tool) === id)
    if (record) matchedTools.push(record)
  }

  const categories = matchedTools.map((tool) => categorizeTool(tool))
  const meaningful = meaningfulCategories(categories)
  const label = matchedTools.length
    ? describeCategories(categories)
    : toolSegments.length
      ? CATEGORY_LABELS.other
      : '思考过程'

  // 图标代表类别：无工具段（纯 reasoning 组）→ 'reasoning'；否则取代表类别。
  const icon: ToolGroupIcon = toolSegments.length
    ? representativeCategory(categories)
    : 'reasoning'

  const running = matchedTools.some((tool) => normalizeToolCallStatus(tool.status) === 'running')
  const failed = matchedTools.filter((tool) => normalizeToolCallStatus(tool.status) === 'error').length

  let suffix: string
  let status: ToolGroupSummary['status']
  if (running) {
    suffix = '进行中…'
    status = 'running'
  } else if (failed > 0) {
    suffix = `${failed} 失败`
    status = 'error'
  } else {
    suffix = '已完成'
    status = 'done'
  }

  return {
    text: `${label} · ${stepCount} 步 · ${suffix}`,
    status,
    icon,
    categories: meaningful,
  }
}

/** tool record 的唯一 id（兼容多种字段命名）。 */
function toolRecordId(toolCall: ToolCallRecord): string {
  return toolCall.id || toolCall.toolCallId || toolCall.call_id || toolCall.callId || ''
}
