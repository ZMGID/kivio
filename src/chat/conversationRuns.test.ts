import { describe, expect, it } from 'vitest'
import { createEmptyStreamSnapshot, mergeToolRecord, acceptStreamRun } from './conversationRuns'

describe('createEmptyStreamSnapshot', () => {
  it('creates a streaming snapshot with empty content', () => {
    const snapshot = createEmptyStreamSnapshot()
    expect(snapshot.streaming).toBe(true)
    expect(snapshot.content).toBe('')
    expect(snapshot.toolCalls).toEqual([])
    expect(snapshot.startedAt).toBeTypeOf('number')
    expect(snapshot.reasoningStartedAtBySegmentId).toEqual({})
    expect(snapshot.reasoningDurationMsBySegmentId).toEqual({})
  })
})

describe('stream run ownership', () => {

  it('preserves the current protocol behavior for events without a run id', () => {
    const snapshot = createEmptyStreamSnapshot()
    snapshot.runId = 'run-a'
    expect(acceptStreamRun(snapshot, null)).toBe(true)
    expect(snapshot.runId).toBe('run-a')
  })
})

describe('mergeToolRecord', () => {
  it('keeps the existing structured content when the update carries none', () => {
    // 问用户答完之后，claude 回的 tool_result 那条更新不带 structured_content。
    // 直接展开会把「问了什么 + 选了什么」抹掉，消息流里只剩一行灰字。
    const answered = {
      id: 'tc1',
      structured_content: { askUser: { phase: 'answered', questions: [], answers: {} } },
      structuredContent: { askUser: { phase: 'answered', questions: [], answers: {} } },
    } as never
    // 注意 `structured_content: undefined` 是**显式**的：协议记录是按字段拼出来的
    // （`toolEventToRecord`），键总在、值可能是 undefined。对象展开不会用「缺失的键」
    // 覆盖已有值，但会用「值为 undefined 的键」覆盖 —— 这才是抹掉载荷的那一下。
    const toolResult = {
      id: 'tc1',
      status: 'success',
      result_preview: 'ok',
      structured_content: undefined,
      structuredContent: undefined,
    } as never
    const merged = mergeToolRecord(answered, toolResult)
    expect(merged.status).toBe('success')
    expect(merged.result_preview).toBe('ok')
    expect(merged.structured_content).toEqual({
      askUser: { phase: 'answered', questions: [], answers: {} },
    })
    expect(merged.structuredContent).toEqual(merged.structured_content)
  })

  it('lets a real structured payload win over the old one', () => {
    const previous = { id: 'tc1', structured_content: { askUser: { phase: 'awaiting' } } } as never
    const next = { id: 'tc1', structured_content: { askUser: { phase: 'answered' } } } as never
    expect(mergeToolRecord(previous, next).structured_content)
      .toEqual({ askUser: { phase: 'answered' } })
  })
})
