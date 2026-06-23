import { describe, expect, it } from 'vitest'
import type { ChatMessageSegment, ToolCallRecord } from './types'
import {
  compareTimelineSegments,
  groupTimelineSegments,
  segmentToolCallId,
  summarizeToolGroup,
} from './segments'

function segment(partial: Partial<ChatMessageSegment> & Pick<ChatMessageSegment, 'id' | 'kind' | 'order'>): ChatMessageSegment {
  return {
    phase: 'plain',
    ...partial,
  }
}

describe('segmentToolCallId', () => {
  it('prefers snake_case tool_call_id', () => {
    expect(segmentToolCallId({ tool_call_id: 'a', toolCallId: 'b' } as ChatMessageSegment)).toBe('a')
  })

  it('falls back to camelCase toolCallId', () => {
    expect(segmentToolCallId({ toolCallId: 'b' } as ChatMessageSegment)).toBe('b')
  })
})

describe('compareTimelineSegments', () => {
  it('orders reasoning before text within the same model step', () => {
    const reasoning = segment({
      id: 'r',
      kind: 'reasoning',
      order: 2,
      step_number: 1,
      round: 0,
      phase: 'tool_loop',
    })
    const text = segment({
      id: 't',
      kind: 'text',
      order: 1,
      step_number: 1,
      round: 0,
      phase: 'tool_loop',
    })
    expect(compareTimelineSegments(reasoning, text)).toBeLessThan(0)
    expect(compareTimelineSegments(text, reasoning)).toBeGreaterThan(0)
  })

  it('falls back to order when model steps differ', () => {
    const earlier = segment({ id: 'a', kind: 'text', order: 1, step_number: 1 })
    const later = segment({ id: 'b', kind: 'reasoning', order: 2, step_number: 2 })
    expect(compareTimelineSegments(earlier, later)).toBeLessThan(0)
  })
})

function toolSegment(id: string, order: number, toolCallId: string): ChatMessageSegment {
  return segment({ id, kind: 'tool', order, tool_call_id: toolCallId })
}

function tool(partial: Partial<ToolCallRecord> & Pick<ToolCallRecord, 'id'>): ToolCallRecord {
  return { status: 'completed', ...partial }
}

describe('groupTimelineSegments', () => {
  it('aggregates consecutive reasoning + tool into one group', () => {
    const items = groupTimelineSegments([
      segment({ id: 'r', kind: 'reasoning', order: 1, text: 'think' }),
      toolSegment('t1', 2, 'call-1'),
      toolSegment('t2', 3, 'call-2'),
    ])
    expect(items).toHaveLength(1)
    expect(items[0].type).toBe('group')
    expect(items[0].type === 'group' && items[0].segments.map((s) => s.id)).toEqual(['r', 't1', 't2'])
  })

  it('splits into two groups when a text segment interrupts (tool → text → tool)', () => {
    const items = groupTimelineSegments([
      toolSegment('t1', 1, 'call-1'),
      segment({ id: 'txt', kind: 'text', order: 2, text: 'between' }),
      toolSegment('t2', 3, 'call-2'),
    ])
    expect(items.map((item) => item.type)).toEqual(['group', 'text', 'group'])
    expect(items[1].type === 'text' && items[1].segment.id).toBe('txt')
  })

  it('groups a pure reasoning run', () => {
    const items = groupTimelineSegments([
      segment({ id: 'r1', kind: 'reasoning', order: 1, text: 'a' }),
      segment({ id: 'r2', kind: 'reasoning', order: 2, text: 'b' }),
    ])
    expect(items).toHaveLength(1)
    expect(items[0].type).toBe('group')
  })

  it('filters out empty reasoning/text segments (no stray groups or splits)', () => {
    const items = groupTimelineSegments([
      segment({ id: 'r-empty', kind: 'reasoning', order: 1, text: '   ' }),
      toolSegment('t1', 2, 'call-1'),
      segment({ id: 'txt-empty', kind: 'text', order: 3, text: '' }),
      toolSegment('t2', 4, 'call-2'),
    ])
    // empty reasoning skipped, empty text does not interrupt → single group of two tools
    expect(items).toHaveLength(1)
    expect(items[0].type === 'group' && items[0].segments.map((s) => s.id)).toEqual(['t1', 't2'])
  })
})

describe('summarizeToolGroup', () => {
  it('summarizes a single category with step count and completed suffix', () => {
    const segments = [toolSegment('t1', 1, 'c1'), toolSegment('t2', 2, 'c2')]
    const toolCalls = [
      tool({ id: 'c1', name: 'read_file' }),
      tool({ id: 'c2', name: 'read' }),
    ]
    const summary = summarizeToolGroup(segments, toolCalls)
    expect(summary.text).toBe('读取文件 · 2 步 · 已完成')
    expect(summary.icon).toBe('read')
    // 单类组：去重后仅一个类别
    expect(summary.categories).toEqual(['read'])
  })

  it('joins two categories', () => {
    const segments = [toolSegment('t1', 1, 'c1'), toolSegment('t2', 2, 'c2')]
    const toolCalls = [
      tool({ id: 'c1', name: 'search_files' }),
      tool({ id: 'c2', name: 'read_file' }),
    ]
    const summary = summarizeToolGroup(segments, toolCalls)
    expect(summary.text).toBe('代码搜索与读取文件 · 2 步 · 已完成')
    // 混合类别 → 通用兜底图标
    expect(summary.icon).toBe('other')
    // 两个去重类别，保持首次出现顺序
    expect(summary.categories).toEqual(['codeSearch', 'read'])
  })

  it('dedupes repeated tools and drops other-category tools from categories', () => {
    const segments = [
      toolSegment('t1', 1, 'c1'),
      toolSegment('t2', 2, 'c2'),
      toolSegment('t3', 3, 'c3'),
    ]
    const toolCalls = [
      tool({ id: 'c1', name: 'read_file' }),
      tool({ id: 'c2', name: 'totally_unknown_tool', source: 'native' }),
      tool({ id: 'c3', name: 'read' }),
    ]
    const summary = summarizeToolGroup(segments, toolCalls)
    // 重复 read 去重，未知工具(other)被剔除
    expect(summary.categories).toEqual(['read'])
  })

  it('falls back to generic label when category is unknown', () => {
    const segments = [toolSegment('t1', 1, 'c1')]
    const toolCalls = [tool({ id: 'c1', name: 'totally_unknown_tool', source: 'native' })]
    const summary = summarizeToolGroup(segments, toolCalls)
    expect(summary.text).toBe('工具调用 · 1 步 · 已完成')
    expect(summary.icon).toBe('other')
    // 全是 other → categories 为空
    expect(summary.categories).toEqual([])
  })

  it('appends failure count and error status', () => {
    const segments = [toolSegment('t1', 1, 'c1'), toolSegment('t2', 2, 'c2')]
    const toolCalls = [
      tool({ id: 'c1', name: 'run_command', status: 'error' }),
      tool({ id: 'c2', name: 'run_command', status: 'completed' }),
    ]
    const summary = summarizeToolGroup(segments, toolCalls)
    expect(summary.text).toBe('执行命令 · 2 步 · 1 失败')
    expect(summary.status).toBe('error')
  })

  it('reports running suffix when any tool is running', () => {
    const segments = [toolSegment('t1', 1, 'c1')]
    const toolCalls = [tool({ id: 'c1', name: 'web_fetch', status: 'running' })]
    const summary = summarizeToolGroup(segments, toolCalls)
    expect(summary.text).toBe('网页读取 · 1 步 · 进行中…')
    expect(summary.status).toBe('running')
  })

  it('categorizes notion mcp tools as Notion search & read', () => {
    const segments = [toolSegment('t1', 1, 'c1')]
    const toolCalls = [tool({ id: 'c1', name: 'search', source: 'mcp', server_name: 'notion-mcp' })]
    const summary = summarizeToolGroup(segments, toolCalls)
    expect(summary.text).toBe('Notion 搜索与读取 · 1 步 · 已完成')
    expect(summary.icon).toBe('notion')
  })

  it('uses reasoning icon for a pure thinking group (no tool segments)', () => {
    const segments = [
      segment({ id: 'r1', kind: 'reasoning', order: 1, text: 'a' }),
      segment({ id: 'r2', kind: 'reasoning', order: 2, text: 'b' }),
    ]
    const summary = summarizeToolGroup(segments, [])
    expect(summary.text).toBe('思考过程 · 2 步 · 已完成')
    expect(summary.icon).toBe('reasoning')
    // 纯思考组不进图标排
    expect(summary.categories).toEqual([])
  })
})
