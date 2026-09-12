import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { api } from '../api/tauri'
import { SubAgentPanel } from './SubAgentPanel'

vi.mock('../api/tauri', () => ({ api: { chatSubagentControl: vi.fn() } }))
const record = {
  id: 'child', name: 'Research', conversationId: 'conv_a', sequence: 1,
  profile: { model: 'test', agentType: 'research' },
  runs: [{ id: 'run-1', status: 'completed', prompt: 'Inspect', result: 'Full result' }],
  messages: [], history: [], tools: [],
}
beforeEach(() => {
  vi.mocked(api.chatSubagentControl).mockReset().mockImplementation(async (_conversation, args) => {
    if (args.operation === 'wait') return new Promise(() => {})
    if (args.operation === 'list') return { sequence: 1, agents: [record] }
    return record
  })
})
it('idle information is a message; explicit continuation starts work', async () => {
  render(<SubAgentPanel conversationId="conv_a" />)
  fireEvent.click(await screen.findByText('Research'))
  await screen.findByText('Full result')
  fireEvent.change(screen.getByLabelText('补充信息或后续任务'), { target: { value: 'Inspect another case' } })
  fireEvent.click(screen.getByRole('button', { name: '留言' }))
  await waitFor(() => expect(api.chatSubagentControl).toHaveBeenCalledWith('conv_a', expect.objectContaining({ operation: 'message', id: 'child', message: 'Inspect another case' })))
  fireEvent.change(screen.getByLabelText('补充信息或后续任务'), { target: { value: 'Continue research' } })
  fireEvent.click(screen.getByRole('button', { name: '继续' }))
  await waitFor(() => expect(api.chatSubagentControl).toHaveBeenCalledWith('conv_a', expect.objectContaining({ operation: 'continue' })))
})
it('unmounting the panel does not stop its worker', async () => {
  const view = render(<SubAgentPanel conversationId="conv_a" />)
  await screen.findByText('Research')
  view.unmount()
  expect(vi.mocked(api.chatSubagentControl).mock.calls.some(([, args]) => args.operation === 'stop')).toBe(false)
})

it('a failed connection can reload the durable snapshot without starting work', async () => {
  vi.mocked(api.chatSubagentControl).mockRejectedValueOnce(new Error('Disconnected'))
  render(<SubAgentPanel conversationId="conv_a" lang="en" />)
  await screen.findByRole('alert')
  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }))
  fireEvent.click(await screen.findByText('Research'))
  await screen.findByText('Full result')
  expect(vi.mocked(api.chatSubagentControl).mock.calls.every(([, args]) => ['list', 'get', 'wait'].includes(args.operation))).toBe(true)
})
