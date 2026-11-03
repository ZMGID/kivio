import { expect, it } from 'vitest'
import { subAgentMessages } from './subAgentMessages'
import type { SubAgentRecord } from '../api/tauri'

it('restores persisted tool metadata and errors instead of treating every returned output as success', () => {
  const child: SubAgentRecord = {
    id: 'child', name: 'Worker', sequence: 1, profile: { model: 'test', agentType: 'research' },
    runs: [{ id: 'run', status: 'failed', prompt: 'Search' }], messages: [],
    history: [
      { role: 'assistant', tool_calls: [{ id: 'tool', function: { name: 'search', arguments: '{}' } }] },
      { role: 'tool', tool_call_id: 'tool', content: 'Permission denied' },
    ],
    tools: [{ id: 'tool', name: 'search', source: 'mcp', server_name: 'Search', status: 'returned',
      result: { content: 'Permission denied', is_error: true, structured_content: { hits: [] }, artifacts: [] } }],
  }
  expect(subAgentMessages(child)[0].tool_calls![0]).toMatchObject({ source: 'mcp', server_name: 'Search', status: 'error', structured_content: { hits: [] }, error: 'Permission denied' })
})

it('hides default-injected model input but keeps real prompts and parent messages', () => {
  const child: SubAgentRecord = {
    id: 'child', name: 'Worker', sequence: 1, profile: { model: 'test', agentType: 'research' },
    runs: [{ id: 'run', status: 'completed', prompt: 'Search', result: 'Done' }], messages: [], tools: [],
    history: [
      { role: 'system', content: 'persona' },
      { role: 'user', content: 'Search' },
      { role: 'user', content: '<kivio_runtime_context>\nCurrent runtime context:\n<available_skills>...</available_skills>\n</kivio_runtime_context>' },
      { role: 'user', content: '[context summary]\nThis session continues from earlier conversation.' },
      { role: 'user', content: 'Previously used skill "pdf": ...', _skill_reload: 'pdf' },
      { role: 'user', content: 'Earlier read result for "a.ts" ...', _compact_reminder: true },
      { role: 'user', content: 'Previously completed tool operations recovered from the durable tool ledger. Use these results: []' },
      { role: 'user', content: 'Previous execution was interrupted. These tool outcomes are unknown: []' },
      { role: 'user', content: '[Sub-agent message from parent]\nAlso check b.ts' },
      { role: 'assistant', content: 'Done' },
    ],
  }
  expect(subAgentMessages(child).map(message => [message.role, message.content])).toEqual([
    ['user', 'Search'], ['user', '[Sub-agent message from parent]\nAlso check b.ts'], ['assistant', 'Done'],
  ])
})
