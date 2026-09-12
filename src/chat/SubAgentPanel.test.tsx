import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { api } from '../api/tauri'
import { SubAgentPanel, SubAgentIndicator } from './SubAgentPanel'
import { useState } from 'react'

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

it('keeps the full task prompt collapsed when opening a child result', async () => {
  render(<SubAgentPanel conversationId="conv_a" />)
  fireEvent.click(await screen.findByText('Research'))
  await screen.findByText('Full result')
  expect(screen.getByText('Inspect', { exact: true })).not.toBeVisible()
})

it('opens agent management from a compact count without mounting details in the composer', async () => {
  const stopMain = vi.fn()
  function Harness() {
    const [open, setOpen] = useState(false)
    return <><div data-testid="composer"><SubAgentIndicator conversationId="conv_a" onOpen={() => setOpen(true)} /></div>{open && <aside><SubAgentPanel conversationId="conv_a" mainAgent={{ model: 'test', running: true, onStop: stopMain }} /></aside>}</>
  }
  render(<Harness />)
  const indicator = await screen.findByRole('button', { name: '子代理 1 · 打开任务' })
  expect(screen.queryByText('Research')).toBeNull()
  fireEvent.click(indicator)
  await screen.findByText('Research')
  expect(screen.getByTestId('composer')).not.toHaveTextContent('Research')
  expect(screen.getByText('主代理')).toBeVisible()
  fireEvent.click(screen.getByRole('button', { name: '停止整个协作' }))
  expect(stopMain).toHaveBeenCalledOnce()
  expect(vi.mocked(api.chatSubagentControl).mock.calls.filter(([, args]) => args.operation === 'list')).toHaveLength(1)
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
