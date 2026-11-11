import { describe, expect, it } from 'vitest'
import { findSubagentToolIndex, mergeSubagentProgress } from './streamApply'
import type { ChatSubagentPayload } from '../api/tauri'
import type { ToolCallRecord } from './types'

const event: ChatSubagentPayload = {
  parentConversationId: 'conversation', parentRunId: 'run', parentToolCallId: '', taskId: 'child-a',
  name: 'A', depth: 1, status: 'running', externalSessionId: 'actor',
}
const task = (id: string, childId: string): ToolCallRecord => ({
  id, name: 'subagent', source: 'external_cli', status: 'running',
  structured_content: { externalSessionId: 'actor', backgroundTaskId: childId },
})

describe('external subagent ownership', () => {
  it('keeps the final report when delayed progress arrives', () => {
    const completed: ToolCallRecord = { ...task('tool-a', 'child-a'), status: 'success', result_preview: 'Final report' }
    expect(mergeSubagentProgress(completed, { ...event, preview: 'old progress' })).toBe(completed)
  })
  it('does not attach a previous turn to the only running child in this turn', () => {
    expect(findSubagentToolIndex([task('tool-b', 'child-b')], event)).toBe(-1)
    expect(findSubagentToolIndex([task('tool-a', 'child-a')], event)).toBe(0)
    expect(findSubagentToolIndex([task('tool-a', 'child-a')], { ...event, externalSessionId: 'retired' })).toBe(-1)
  })
})
