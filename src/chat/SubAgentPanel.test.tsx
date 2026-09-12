import { fireEvent, render, screen } from '@testing-library/react'
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
it('opens a read-only conversation without message or editing controls', async () => {
  render(<SubAgentPanel conversationId="conv_a" />)
  fireEvent.click(await screen.findByText('Research'))
  await screen.findByText('Full result')
  expect(screen.getByText('Inspect', { exact: true })).toBeVisible()
  expect(screen.queryByRole('textbox')).toBeNull()
  expect(screen.queryByRole('button', { name: '继续' })).toBeNull()
  expect(screen.queryByText('完整历史与工具记录')).toBeNull()
  expect(api.chatSubagentControl).not.toHaveBeenCalledWith('conv_a', expect.objectContaining({ operation: 'message' }))
})
it('unmounting the panel does not stop its worker', async () => {
  const view = render(<SubAgentPanel conversationId="conv_a" />)
  await screen.findByText('Research')
  view.unmount()
  expect(vi.mocked(api.chatSubagentControl).mock.calls.some(([, args]) => args.operation === 'stop')).toBe(false)
})

it('shows task instructions as an assignment bubble', async () => {
  render(<SubAgentPanel conversationId="conv_a" />)
  fireEvent.click(await screen.findByText('Research'))
  await screen.findByText('Full result')
  expect(screen.getByText('Inspect', { exact: true })).toBeVisible()
})

it('opens agent management from a compact count without mounting details in the composer', async () => {

  function Harness() {
    const [open, setOpen] = useState(false)
    return <><div data-testid="composer"><SubAgentIndicator conversationId="conv_a" onOpen={() => setOpen(true)} /></div>{open && <aside><SubAgentPanel conversationId="conv_a" /></aside>}</>
  }
  render(<Harness />)
  const indicator = await screen.findByRole('button', { name: '子代理 1 · 打开任务' })
  expect(screen.queryByText('Research')).toBeNull()
  fireEvent.click(indicator)
  await screen.findByText('Research')
  expect(screen.getByTestId('composer')).not.toHaveTextContent('Research')
  expect(screen.queryByText('主代理')).toBeNull()
  expect(screen.getByRole('heading', { name: '正在运行 · 0' })).toBeVisible()
  expect(screen.getByRole('heading', { name: '已关闭 · 1' })).toBeVisible()
  expect(screen.queryByRole('textbox')).toBeNull()

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
