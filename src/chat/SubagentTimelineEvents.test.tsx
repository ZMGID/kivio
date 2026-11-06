import { fireEvent, render, screen, within } from '@testing-library/react'
import { beforeEach, expect, it } from 'vitest'
import { ToolCallBlock } from './ToolCallBlock'
import { mergeSubagentProgress } from './streamApply'
import { mergeToolRecord } from './conversationRuns'
import type { ToolCallRecord } from './types'

beforeEach(() => {
  HTMLDialogElement.prototype.showModal ??= function () { this.setAttribute('open', '') }
  HTMLDialogElement.prototype.close ??= function () { this.removeAttribute('open') }
})
const receiptDialog = () => within(screen.getByRole('dialog'))

it('shows native task assignments, shared context, and solution spaces before admission', () => {
  render(<ToolCallBlock toolCall={{
    id: 'task', source: 'native', name: 'task', status: 'running',
    arguments: JSON.stringify({ context: '共享约束', tasks: [
      { name: '审查前端', agent: 'reviewer', task: '检查卡片', solutionSpace: '只读仓库' },
      { task: '检查协议', solutionSpace: '修改测试' },
    ] }),
  }} />)
  expect(screen.getByRole('button', { name: '子代理 正在派工' })).toBeVisible()
  fireEvent.click(screen.getByRole('button', { name: '子代理 正在派工' }))
  expect(screen.getByText('共享约束')).toBeVisible()
  expect(screen.getByText('审查前端')).toBeVisible()
  expect(screen.getByText('reviewer')).toBeVisible()
  expect(screen.getByText('检查卡片')).toBeVisible()
  expect(screen.getByText(/只读仓库/)).toBeVisible()
  expect(screen.getByText('任务 2')).toBeVisible()
  expect(screen.getByText('检查协议')).toBeVisible()
  expect(screen.getByText(/修改测试/)).toBeVisible()
})

it('keeps both agents live after asynchronous task admission and displays their results', () => {
  const task: ToolCallRecord = {
    id: 'call', source: 'native', name: 'task', status: 'success',
    arguments: { context: 'Shared', tasks: [
      { name: 'Alpha', task: 'Inspect A', solutionSpace: 'A' },
      { name: 'Beta', task: 'Inspect B', solutionSpace: 'B' },
    ] },
    result_preview: 'Two agents scheduled',
    structured_content: { agents: [
      { agentId: 'alpha', jobId: 'job-alpha', agent: 'reviewer', name: 'Alpha' },
      { agentId: 'beta', jobId: 'job-beta', agent: 'reviewer', name: 'Beta' },
    ] },
  }
  const event = { parentConversationId: 'conv', parentRunId: 'run', parentToolCallId: 'call', depth: 1 }
  const first = mergeSubagentProgress(task, { ...event, taskId: 'alpha', name: 'Alpha', status: 'running', steps: ['read A'], preview: 'Checking A' })
  const second = mergeSubagentProgress(first, { ...event, taskId: 'beta', name: 'Beta', status: 'running', steps: ['read B'], preview: 'Checking B' })
  expect(second.status).toBe('success')
  expect(second.result_preview).toBe('Two agents scheduled')
  const { rerender } = render(<ToolCallBlock toolCall={second} />)
  expect(screen.getByRole('button', { name: 'Alpha 开始工作' })).toBeVisible()
  expect(screen.getByRole('button', { name: 'Beta 开始工作' })).toBeVisible()
  fireEvent.click(screen.getByRole('button', { name: 'Alpha 开始工作' }))
  expect(screen.getByText('read A')).toBeVisible()
  expect(screen.getByText('Checking A')).toBeVisible()
  expect(screen.getByText('read B')).toBeVisible()
  expect(screen.getByText('Checking B')).toBeVisible()
  expect(screen.getByText('任务: job-alpha')).toBeVisible()
  expect(screen.getByText('任务: job-beta')).toBeVisible()

  const completed = mergeSubagentProgress(second, { ...event, taskId: 'alpha', name: 'Alpha', status: 'completed', preview: 'Alpha result' })
  rerender(<ToolCallBlock toolCall={completed} />)
  expect(screen.getByRole('button', { name: 'Beta 开始工作' })).toBeVisible()
  expect(receiptDialog().getByText('已完成')).toBeVisible()
  expect(screen.getByText('Alpha result')).toBeVisible()
  expect(screen.getByText('Checking B')).toBeVisible()
})

it('keeps progress visible if it arrives before the task receipt', () => {
  const pending: ToolCallRecord = { id: 'call', source: 'native', name: 'task', status: 'running' }
  const progress = mergeSubagentProgress(pending, {
    parentConversationId: 'conv', parentRunId: 'run', parentToolCallId: 'call',
    taskId: 'worker', name: 'Worker', depth: 1, status: 'running', preview: 'Inspecting',
  })
  const { rerender } = render(<ToolCallBlock toolCall={progress} />)
  expect(screen.getByRole('button', { name: 'Worker 开始工作' })).toBeVisible()
  fireEvent.click(screen.getByRole('button', { name: 'Worker 开始工作' }))
  expect(receiptDialog().getByText('Worker')).toBeVisible()
  expect(screen.getByText('Inspecting')).toBeVisible()
  const admitted = mergeToolRecord(progress, { ...pending, status: 'success', result_preview: 'Spawned',
    structured_content: { agents: [{ agentId: 'worker', jobId: 'job-worker', name: 'Worker' }] } })
  rerender(<ToolCallBlock toolCall={admitted} />)
  expect(screen.getByText('Inspecting')).toBeVisible()
  expect(screen.getByText('任务: job-worker')).toBeVisible()
})

it('recognizes queued progress for a native wire task name', () => {
  const call: ToolCallRecord = { id: 'call', source: 'native', name: 'native__task', status: 'success' }
  const progress = mergeSubagentProgress(call, {
    parentConversationId: 'conv', parentRunId: 'run', parentToolCallId: 'call',
    taskId: 'queued', name: 'Queued worker', depth: 1, status: 'pending',
  })
  render(<ToolCallBlock toolCall={progress} />)
  expect(screen.getByRole('button', { name: 'Queued worker 等待运行' })).toBeVisible()
  fireEvent.click(screen.getByRole('button', { name: 'Queued worker 等待运行' }))
  expect(receiptDialog().getByText('等待运行')).toBeVisible()
  expect(receiptDialog().getByText('Queued worker')).toBeVisible()
})

it('distinguishes failed and aborted children from successful scheduling', () => {
  render(<ToolCallBlock toolCall={{ id: 'task', source: 'native', name: 'task', status: 'success',
    structured_content: { agents: [{ agentId: 'a', name: 'First' }, { agentId: 'b', name: 'Second' }],
      subagentProgressByTask: {
        a: { status: 'failed', preview: 'Child error' },
        b: { status: 'aborted', preview: 'Cancelled work' },
      } },
  }} />)
  expect(screen.getByRole('button', { name: 'First 失败' })).toBeVisible()
  expect(screen.getByRole('button', { name: 'Second 已中断' })).toBeVisible()
  fireEvent.click(screen.getByRole('button', { name: 'First 失败' }))
  expect(receiptDialog().getByText('失败')).toBeVisible()
  expect(receiptDialog().getByText('已中断')).toBeVisible()
  expect(receiptDialog().getByText('Child error')).toBeVisible()
  expect(receiptDialog().getByText('Cancelled work')).toBeVisible()
})

it('restores mixed inline and background results after reload without regressing on late events', () => {
  const persisted: ToolCallRecord = {
    id: 'call', source: 'native', name: 'task', status: 'success',
    arguments: { tasks: [{ task: 'Inline work' }, { task: 'Background work' }, { task: 'Cancelled work' }] },
    structured_content: {
      agents: [
        { agentId: 'inline', name: 'Inline' },
        { agentId: 'background', name: 'Background', jobId: 'job-background' },
        { agentId: 'cancelled', name: 'Cancelled', jobId: 'job-cancelled' },
      ],
      subagentProgressByTask: {
        inline: { taskId: 'inline', name: 'Inline', status: 'completed', preview: 'Inline final output' },
        background: { taskId: 'background', name: 'Background', status: 'failed', preview: 'Background failure' },
        cancelled: { taskId: 'cancelled', name: 'Cancelled', status: 'aborted', preview: 'Cancelled by caller' },
      },
    },
  }
  const reloaded = JSON.parse(JSON.stringify(persisted)) as ToolCallRecord
  const { rerender } = render(<ToolCallBlock toolCall={reloaded} />)
  expect(screen.getByRole('button', { name: 'Inline 已完成' })).toBeVisible()
  const disclosure = screen.getByRole('button', { name: 'Inline 已完成' })
  expect(disclosure).toHaveAttribute('aria-haspopup', 'dialog')
  fireEvent.click(disclosure)
  expect(screen.getByRole('dialog')).toBeVisible()
  expect(disclosure).not.toHaveAttribute('aria-expanded')
  expect(receiptDialog().getByText('已完成')).toBeVisible()
  expect(receiptDialog().getByText('失败')).toBeVisible()
  expect(receiptDialog().getByText('已中断')).toBeVisible()
  expect(screen.getByText('Inline final output')).toBeVisible()
  expect(receiptDialog().getByText('Background failure')).toBeVisible()
  expect(receiptDialog().getByText('Cancelled by caller')).toBeVisible()
  expect(screen.getByText('任务: job-background')).toBeVisible()
  expect(screen.getByText('任务: job-cancelled')).toBeVisible()

  const stale = mergeSubagentProgress(reloaded, {
    parentConversationId: 'conv', parentRunId: 'run', parentToolCallId: 'call',
    taskId: 'inline', name: 'Inline', depth: 1, status: 'running', preview: 'Stale output',
  })
  rerender(<ToolCallBlock toolCall={stale} />)
  expect(screen.getByRole('button', { name: 'Inline 已完成' })).toBeVisible()
  expect(screen.getByText('Inline final output')).toBeVisible()
  expect(screen.queryByText('Stale output')).toBeNull()
  expect(receiptDialog().getByText('Background failure')).toBeVisible()
})

it('merges receipt snapshots per child while retaining newer live progress and terminal results', () => {
  const call: ToolCallRecord = { id: 'call', source: 'native', name: 'task', status: 'running' }
  const event = { parentConversationId: 'conv', parentRunId: 'run', parentToolCallId: 'call', depth: 1 }
  const first = mergeSubagentProgress(call, { ...event, taskId: 'a', name: 'A', status: 'completed', preview: 'A final' })
  const both = mergeSubagentProgress(first, { ...event, taskId: 'b', name: 'B', status: 'running', preview: 'B working' })
  const receipt = mergeToolRecord(both, {
    ...call, status: 'success',
    structured_content: {
      agents: [{ agentId: 'a', name: 'A' }, { agentId: 'b', name: 'B', jobId: 'job-b' }],
      subagentProgressByTask: {
        a: { status: 'pending', preview: 'A stale' },
        b: { status: 'pending', preview: 'B stale' },
      },
    },
  })
  const { rerender } = render(<ToolCallBlock toolCall={receipt} />)
  expect(screen.getByRole('button', { name: 'B 开始工作' })).toBeVisible()
  fireEvent.click(screen.getByRole('button', { name: 'A 已完成' }))
  expect(receiptDialog().getByText('已完成')).toBeVisible()
  expect(screen.getByText('A final')).toBeVisible()
  expect(screen.getByText('B working')).toBeVisible()
  expect(screen.queryByText('A stale')).toBeNull()
  expect(screen.queryByText('B stale')).toBeNull()

  const finalReceipt = mergeToolRecord(receipt, {
    ...call, status: 'success',
    structured_content: {
      agents: [{ agentId: 'a', name: 'A' }, { agentId: 'b', name: 'B', jobId: 'job-b' }],
      subagentProgressByTask: {
        a: { status: 'completed', preview: 'A stale terminal snapshot' },
        b: { status: 'completed', preview: 'B final' },
      },
    },
  })
  rerender(<ToolCallBlock toolCall={finalReceipt} />)
  expect(screen.getByRole('button', { name: 'A 已完成' })).toBeVisible()
  expect(screen.getByRole('button', { name: 'B 已完成' })).toBeVisible()
  expect(screen.getByText('A final')).toBeVisible()
  expect(screen.queryByText('A stale terminal snapshot')).toBeNull()
  expect(screen.getByText('B final')).toBeVisible()
})

it('keeps a rejected task and its error inspectable without claiming it ran', () => {
  render(<ToolCallBlock toolCall={{ id: 'task', source: 'native', name: 'task', status: 'error', arguments: { tasks: [{ name: '调查', task: 'Read package.json', solutionSpace: 'Read only' }] }, error: 'Task rejected' }} />)
  expect(screen.getByRole('button', { name: '调查 派工失败' })).toBeVisible()
  expect(screen.queryByRole('button', { name: '调查 开始工作' })).toBeNull()
  fireEvent.click(screen.getByRole('button', { name: /调查/ }))
  expect(screen.getByRole('alert')).toHaveTextContent('Task rejected')
  expect(screen.getByText('Read package.json')).toBeVisible()
})

it('renders mixed inline results once while keeping rich asynchronous progress and current context separate', () => {
  render(<ToolCallBlock toolCall={{ id: 'mixed', source: 'native', name: 'task', status: 'success', structured_content: {
    agents: [{ agentId: 'inline', name: 'Inline', jobId: 'job-inline' }, { agentId: 'async', name: 'Background', jobId: 'job-async' }],
    results: [{ id: 'job-inline', agentId: 'inline', status: 'completed', result: 'Complete inline result' }],
    subagentProgressByTask: { inline: { status: 'completed', preview: 'Duplicate preview' }, async: { status: 'running', model: 'provider/model', requests: 3, toolCount: 7, tokens: 10000, contextTokens: 2000, contextWindow: 32000, durationMs: 1500, stage: 'Inspecting source', currentToolIntent: 'Read registry' } },
  } }} />)
  expect(screen.getByRole('button', { name: 'Background 开始工作' })).toBeVisible()
  fireEvent.click(screen.getByRole('button', { name: 'Inline 已完成' }))
  expect(screen.getAllByText('Complete inline result')).toHaveLength(1)
  expect(screen.queryByText('Duplicate preview')).not.toBeInTheDocument()
  expect(screen.getByText(/10,000 累计 token · 上下文 2,000 \/ 32,000/)).toBeVisible()
  expect(receiptDialog().getByText('Read registry')).toBeVisible()
})

it('keeps every agent visible and in dispatch order as statuses change', () => {
  const agents = Array.from({ length: 6 }, (_, index) => ({ agentId: `a${index}`, name: `Agent ${index}` }))
  const final = Object.fromEntries(agents.map((agent, index) => [agent.agentId, { status: index === 5 ? 'failed' : 'completed', preview: `Output ${index}`, durationMs: index * 100 }]))
  const view = render(<ToolCallBlock toolCall={{ id: 'task', source: 'native', name: 'task', status: 'success', structured_content: { agents, subagentProgressByTask: final } }} />)
  expect(screen.getAllByRole('button').map(button => button.textContent)).toEqual([
    'Agent 0 已完成', 'Agent 1 已完成', 'Agent 2 已完成', 'Agent 3 已完成', 'Agent 4 已完成', 'Agent 5 失败',
  ])
  view.rerender(<ToolCallBlock toolCall={{ id: 'task', source: 'native', name: 'task', status: 'success', structured_content: { agents, subagentProgressByTask: Object.fromEntries(agents.map(agent => [agent.agentId, { status: 'running' }])) } }} />)
  expect(screen.getAllByRole('button').map(button => button.textContent)).toEqual(agents.map(agent => `${agent.name} 开始工作`))
})

it('isolates generations and admitted child runs while retaining richer metrics through receipt merges', () => {
  const tool: ToolCallRecord = { id: 'task', source: 'native', name: 'task', status: 'success' }
  const event = { parentConversationId: 'root', parentRunId: 'parent', parentToolCallId: 'task', taskId: 'worker', name: 'Worker', depth: 1, generation: 1, childRunId: 'first' }
  const first = mergeSubagentProgress(tool, { ...event, status: 'completed', preview: 'First output', progress: { requests: 2, contextTokens: 100 } })
  const next = mergeSubagentProgress(first, { ...event, generation: 2, childRunId: 'second', status: 'running', preview: 'Current output', progress: { requests: 1, contextTokens: 50 } })
  expect(mergeSubagentProgress(next, { ...event, status: 'completed', preview: 'Late old output' })).toBe(next)
  expect(mergeSubagentProgress(next, { ...event, generation: 2, childRunId: 'other', status: 'running', preview: 'Wrong run' })).toBe(next)
  const receipt = mergeToolRecord(next, { ...tool, structured_content: { agents: [{ agentId: 'worker', name: 'Worker' }], subagentProgressByTask: { worker: { generation: 1, status: 'completed', preview: 'Old receipt' } } } })
  render(<ToolCallBlock toolCall={receipt} />)
  fireEvent.click(screen.getByRole('button', { name: 'Worker 开始工作' }))
  expect(screen.getByText('Current output')).toBeVisible()
  expect(screen.getByText(/1 请求 · 上下文 50/)).toBeVisible()
  expect(screen.queryByText('First output')).not.toBeInTheDocument()
  expect(screen.queryByText('Old receipt')).not.toBeInTheDocument()
})
