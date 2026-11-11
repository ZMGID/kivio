import { fireEvent, render, screen, within } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'
import { lifecycleSubagentProvider } from './segments'
import { ToolCallBlock } from './ToolCallBlock'
import { MessageBubble } from './MessageBubble'
import type { ToolCallRecord } from './types'

function call(overrides: Partial<ToolCallRecord> = {}): ToolCallRecord {
  return {
    id: 'oc-a', name: 'subagent', source: 'external_cli', status: 'running',
    arguments: {
      sessionUpdate: 'tool_call', toolCallId: 'oc-a', name: 'subagent',
      title: 'subagent', kind: 'think', rawInput: {
        description: 'Inspect source', agent: 'explore', prompt: 'Read the code',
      },
    },
    ...overrides,
  }
}

beforeEach(() => {
  localStorage.setItem('kivio-lang', 'zh')
  HTMLDialogElement.prototype.showModal ??= function () { this.setAttribute('open', '') }
  HTMLDialogElement.prototype.close ??= function () { this.removeAttribute('open') }
})

describe('OpenCode subagent timeline', () => {
  it.each(['Agent', 'Task', 'subagent', 'subagent_fork', 'workflow', 'ralph'])('replaces every legacy %s card even before arguments arrive', (name) => {
    const { rerender } = render(<ToolCallBlock toolCall={call({ name, arguments: {}, status: 'running' })} />)
    expect(screen.queryByText('SUBAGENT')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: /开始工作/ })).toBeVisible()
    rerender(<ToolCallBlock toolCall={call({ name, arguments: {}, status: 'success',
      structured_content: { type: 'subagent', name: 'Worker', model: 'model-a', result: 'Final report',
        usage: { inputTokens: 30, outputTokens: 10, totalTokens: 40 },
        subagentProgress: { status: 'completed', steps: ['Read source', 'Reviewed tests'] } },
    })} />)
    fireEvent.click(screen.getByRole('button', { name: 'Worker 已完成' }))
    const dialog = within(screen.getByRole('dialog'))
    expect(dialog.getByText('Final report')).toBeVisible()
    expect(dialog.getByText('Read source')).toBeVisible()
    expect(dialog.getByText('Reviewed tests')).toBeVisible()
    expect(dialog.getByText(/40.*token/)).toBeVisible()
  })

  it('recognizes partial ACP arguments from structured content and hides historical child tool rows', () => {
    const root = call()
    render(<ToolCallBlock toolCall={{ ...root, arguments: {}, structured_content: root.arguments }} />)
    expect(screen.getByRole('button', { name: 'Inspect source 开始工作' })).toBeVisible()
    render(<ToolCallBlock toolCall={call({ name: 'Inspect source: shell', arguments: {
      sessionUpdate: 'tool_call', name: 'shell', kind: 'execute', rawInput: { command: 'dir' },
      _meta: { 'opencode/child-session': { id: 'child', parentID: 'parent', depth: 1 } },
    } })} />)
    expect(screen.queryByText(/Inspect source: shell/)).not.toBeInTheDocument()
    expect(screen.queryByText(/opencode\/child-session/)).not.toBeInTheDocument()
  })
  it('recognizes V2 subagent ACP calls without confusing V1, native, MCP or child tools', () => {
    expect(lifecycleSubagentProvider(call())).toBe('opencode')
    expect(lifecycleSubagentProvider(call({ name: 'Inspect source', arguments: {
      sessionUpdate: 'tool_call_update', kind: 'think', rawInput: {
        description: 'Inspect source', subagent_type: 'explore', prompt: 'Read the code',
      },
    } }))).toBeNull()
    expect(lifecycleSubagentProvider(call({ source: 'native' }))).toBeNull()
    expect(lifecycleSubagentProvider(call({ source: 'mcp' }))).toBeNull()
    expect(lifecycleSubagentProvider(call({ arguments: { sessionUpdate: 'tool_call', kind: 'execute', rawInput: { command: 'echo subagent' } } }))).toBeNull()
    expect(lifecycleSubagentProvider(call({ arguments: {
      sessionUpdate: 'tool_call', name: 'subagent', kind: 'think', rawInput: {},
      _meta: { 'opencode/child-session': { id: 'nested', parentID: 'child', depth: 2 } },
    } }))).toBeNull()
  })

  it('keeps one lifecycle entry while arguments, progress and the final reply arrive', () => {
    const pending = call({ arguments: { sessionUpdate: 'tool_call', name: 'subagent', kind: 'think', rawInput: {} } })
    const { rerender } = render(<ToolCallBlock toolCall={pending} />)
    expect(screen.getByRole('button', { name: 'OpenCode 子代理 开始工作' })).toBeVisible()
    rerender(<ToolCallBlock toolCall={call({ result_preview: 'Reading source' })} />)
    expect(screen.queryByText('SUBAGENT')).not.toBeInTheDocument()
    const started = screen.getByRole('button', { name: 'Inspect source 开始工作' })
    expect(within(started).getByTitle('opencode')).toBeInTheDocument()
    fireEvent.click(started)
    expect(within(screen.getByRole('dialog')).getByText('Read the code')).toBeVisible()
    expect(within(screen.getByRole('dialog')).getByText('Reading source')).toBeVisible()
    rerender(<ToolCallBlock toolCall={call({ status: 'success', result_preview: '<subagent session_id="child">\nFound the cause\n</subagent>' })} />)
    expect(screen.getByRole('button', { name: 'Inspect source 已完成' })).toBeVisible()
    expect(within(screen.getByRole('dialog')).getByText('Found the cause')).toBeVisible()
    expect(screen.queryByText(/session_id=/)).not.toBeInTheDocument()
  })

  it('unwraps a V2 result and groups parallel launches in the message', () => {
    const first = call({ status: 'success', result_preview: '<subagent id="child">\nCHILD_OK\n</subagent>' })
    const child = call({ id: 'child:tool', name: 'Inspect source: shell', arguments: {
      _meta: { 'opencode/child-session': { id: 'child' } }, name: 'shell', rawInput: { command: 'ls' },
    } })
    render(<MessageBubble message={{ id: 'message', role: 'assistant', timestamp: 1, content: '',
      tool_calls: [first, child, call({ id: 'oc-b' })], segments: [
        { id: 'a', kind: 'tool', phase: 'tool_loop', order: 0, tool_call_id: 'oc-a' },
        { id: 'child', kind: 'tool', phase: 'tool_loop', order: 1, tool_call_id: 'child:tool' },
        { id: 'b', kind: 'tool', phase: 'tool_loop', order: 2, tool_call_id: 'oc-b' },
      ],
    }} />)
    fireEvent.click(screen.getByRole('button', { name: /^Worked/ }))
    expect(screen.queryByText(/Inspect source: shell|opencode\/child-session/)).not.toBeInTheDocument()
    const starts = screen.getAllByRole('button', { name: 'Inspect source 开始工作' })
    expect(starts).toHaveLength(2)
    expect(starts[0].parentElement).toBe(starts[1].parentElement)
    fireEvent.click(screen.getByRole('button', { name: 'Inspect source 已完成' }))
    expect(within(screen.getByRole('dialog')).getByText('CHILD_OK')).toBeVisible()
    expect(screen.queryByText(/<subagent/)).not.toBeInTheDocument()
  })

  it('does not show a background launch receipt as completion, and preserves failed results', () => {
    const { rerender } = render(<ToolCallBlock toolCall={call({ status: 'success',
      result_preview: '<subagent>Started in background</subagent>',
      structured_content: { rawOutput: { metadata: { status: 'running', sessionID: 'child' } } },
    })} />)
    expect(screen.queryByRole('button', { name: 'Inspect source 已完成' })).not.toBeInTheDocument()
    rerender(<ToolCallBlock toolCall={call({ status: 'cancelled',
      structured_content: { rawOutput: { metadata: { status: 'running' } } },
    })} />)
    expect(screen.getByRole('button', { name: 'Inspect source 已中断' })).toBeVisible()
    rerender(<ToolCallBlock toolCall={call({ status: 'error', error: 'Worker failed', result_preview: 'Partial findings' })} />)
    fireEvent.click(screen.getByRole('button', { name: 'Inspect source 失败' }))
    const dialog = within(screen.getByRole('dialog'))
    expect(dialog.getByText('Partial findings')).toBeVisible()
    expect(dialog.getByRole('alert')).toHaveTextContent('Worker failed')
  })
})
