import { fireEvent, render, screen } from '@testing-library/react'
import { expect, it } from 'vitest'
import { ToolCallBlock } from './ToolCallBlock'
import { mergeSubagentProgress } from './streamApply'
import { mergeToolRecord } from './conversationRuns'
import type { ToolCallRecord } from './types'

it('shows native task assignments, shared context, and solution spaces before admission', () => {
  render(<ToolCallBlock toolCall={{
    id: 'task', source: 'native', name: 'task', status: 'running',
    arguments: JSON.stringify({ context: '共享约束', tasks: [
      { name: '审查前端', agent: 'reviewer', task: '检查卡片', solutionSpace: '只读仓库' },
      { task: '检查协议', solutionSpace: '修改测试' },
    ] }),
  }} />)
  expect(screen.getByText('正在启动…')).toBeVisible()
  fireEvent.click(screen.getByRole('button', { name: /子代理/ }))
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
  expect(screen.getByText('2 个运行中')).toBeVisible()
  fireEvent.click(screen.getByRole('button', { name: /子代理/ }))
  expect(screen.getByText('read A')).toBeVisible()
  expect(screen.getByText('Checking A')).toBeVisible()
  expect(screen.getByText('read B')).toBeVisible()
  expect(screen.getByText('Checking B')).toBeVisible()
  expect(screen.getByText('任务: job-alpha')).toBeVisible()
  expect(screen.getByText('任务: job-beta')).toBeVisible()

  const completed = mergeSubagentProgress(second, { ...event, taskId: 'alpha', name: 'Alpha', status: 'completed', preview: 'Alpha result' })
  rerender(<ToolCallBlock toolCall={completed} />)
  expect(screen.getByText('1 个运行中')).toBeVisible()
  expect(screen.getByText('已完成')).toBeVisible()
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
  expect(screen.getByText('1 个运行中')).toBeVisible()
  fireEvent.click(screen.getByRole('button', { name: /子代理/ }))
  expect(screen.getByText('Worker')).toBeVisible()
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
  expect(screen.getByText('1 个等待运行')).toBeVisible()
  fireEvent.click(screen.getByRole('button', { name: /子代理/ }))
  expect(screen.getByText('等待运行')).toBeVisible()
  expect(screen.getByText('Queued worker')).toBeVisible()
})

it('distinguishes failed and aborted children from successful scheduling', () => {
  render(<ToolCallBlock toolCall={{ id: 'task', source: 'native', name: 'task', status: 'success',
    structured_content: { agents: [{ agentId: 'a', name: 'First' }, { agentId: 'b', name: 'Second' }],
      subagentProgressByTask: {
        a: { status: 'failed', preview: 'Child error' },
        b: { status: 'aborted', preview: 'Cancelled work' },
      } },
  }} />)
  expect(screen.getByText('2 个已结束')).toBeVisible()
  fireEvent.click(screen.getByRole('button', { name: /子代理/ }))
  expect(screen.getByText('失败')).toBeVisible()
  expect(screen.getByText('已中断')).toBeVisible()
  expect(screen.getByText('Child error')).toBeVisible()
  expect(screen.getByText('Cancelled work')).toBeVisible()
})

it('shows wait progress, complete structured results, and already-delivered receipts', () => {
  const call: ToolCallRecord = { id: 'wait', source: 'native', name: 'wait', status: 'running', arguments: {} }
  const { rerender } = render(<ToolCallBlock toolCall={call} />)
  expect(screen.getByText('正在等待结果')).toBeVisible()
  fireEvent.click(screen.getByRole('button', { name: /等待子代理/ }))
  rerender(<ToolCallBlock toolCall={{ ...call, status: 'success',
    result_preview: 'Delivery: already delivered or recovered.',
    structured_content: { op: 'wait', jobs: [
      { id: 'job-a', agentId: 'alpha', label: 'Research', status: 'completed', result: 'Full untruncated result' },
      { id: 'job-b', agentId: 'beta', label: 'Review', status: 'completed', result: null },
      { id: 'job-c', agentId: 'gamma', label: 'Tests', status: 'running', progress: { stage: 'Reviewing assertions', currentToolIntent: 'run tests', recentOutput: ['Still checking'] } },
    ] },
  }} />)
  expect(screen.getByText('Full untruncated result')).toBeVisible()
  expect(screen.getByText('Delivery: already delivered or recovered.')).toBeVisible()
  expect(screen.getByText('run tests')).toBeVisible()
  expect(screen.getByText('Still checking')).toBeVisible()
  expect(screen.getByText('Reviewing assertions')).toBeVisible()
  expect(screen.getByText('任务: job-c')).toBeVisible()
})

it.each(['wait', 'native__wait'])('shows an empty %s receipt and does not hide wait errors', name => {
  const { rerender } = render(<ToolCallBlock toolCall={{ id: 'wait', source: 'native', name, status: 'success', structured_content: { op: 'wait', jobs: [] }, result_preview: 'No tasks to wait for.' }} />)
  fireEvent.click(screen.getByRole('button', { name: /等待子代理/ }))
  expect(screen.getByText('No tasks to wait for.')).toBeVisible()
  rerender(<ToolCallBlock toolCall={{ id: 'wait', source: 'native', name, status: 'error', error: '无法等待' }} />)
  expect(screen.getByText('操作异常')).toBeVisible()
  expect(screen.getByRole('alert')).toHaveTextContent('无法等待')
})

it('does not repeat a complete job result already included in the wait receipt', () => {
  const { container } = render(<ToolCallBlock toolCall={{ id: 'wait', source: 'native', name: 'wait', status: 'success',
    result_preview: 'Completed:\\nFull result',
    structured_content: { jobs: [{ id: 'job', agentId: 'worker', label: 'Research', status: 'completed', result: 'Full result' }] },
  }} />)
  fireEvent.click(screen.getByRole('button', { name: /等待子代理/ }))
  expect(container.querySelectorAll('pre')).toHaveLength(1)
  expect(screen.getByText(/Full result/)).toBeVisible()
})

it('keeps a rejected task and its error inspectable without claiming it ran', () => {
  render(<ToolCallBlock toolCall={{ id: 'task', source: 'native', name: 'task', status: 'error', arguments: { tasks: [{ name: '调查', task: 'Read package.json', solutionSpace: 'Read only' }] }, error: 'Task rejected' }} />)
  expect(screen.getByText('操作异常')).toBeVisible()
  expect(screen.queryByText('已调度')).toBeNull()
  fireEvent.click(screen.getByRole('button', { name: /调查/ }))
  expect(screen.getByRole('alert')).toHaveTextContent('Task rejected')
  expect(screen.getByText('Read package.json')).toBeVisible()
})
