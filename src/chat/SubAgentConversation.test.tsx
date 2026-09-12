import { fireEvent, render, screen } from '@testing-library/react'
import { expect, it } from 'vitest'
import { SubAgentConversation } from './SubAgentConversation'
import type { SubAgentRecord } from '../api/tauri'

it('replays assignments, assistant turns and tool cards without internal instructions or duplicate results', () => {
  const child: SubAgentRecord = {
    id: 'worker', name: 'Research', sequence: 1, profile: { model: 'test', agentType: 'researcher' },
    runs: [{ id: 'run', status: 'completed', prompt: 'Read files', result: 'Final findings' }], messages: [], tools: [],
    history: [
      { role: 'system', content: 'Internal system instructions' },
      { role: 'user', content: 'Read files' },
      { role: 'assistant', content: 'I will inspect the entry point.', tool_calls: [{ id: 'call', function: { name: 'read', arguments: '{}' } }] },
      { role: 'tool', tool_call_id: 'call', content: 'File contents' },
      { role: 'assistant', content: 'Final findings', reasoning_content: 'Internal reasoning' },
    ],
  }
  render(<SubAgentConversation child={child} lang="en" />)
  expect(screen.getByText('Read files')).toBeVisible()
  expect(screen.getByText('I will inspect the entry point.')).toBeVisible()
  expect(screen.getAllByText('Final findings')).toHaveLength(1)
  expect(screen.queryByText('Internal system instructions')).toBeNull()
  expect(screen.queryByText('Internal reasoning')).toBeNull()
  expect(screen.queryByText('File contents')).toBeNull()
  fireEvent.click(screen.getByRole('button', { name: /^Worked/ }))
  expect(screen.queryByRole('button', { name: '重新生成' })).toBeNull()
  expect(screen.queryByRole('textbox')).toBeNull()
})
