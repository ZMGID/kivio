import { act, fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import type { AutomationRun } from '../../api/automationContracts'
import { createBlankAutomation } from './graph'
import { RunDetails } from './RunDetails'

const getRun = vi.hoisted(() => vi.fn())
vi.mock('./api', () => ({ automationApi: { getRun } }))
const run: AutomationRun = {
  id: 'old', automationId: 'auto', origin: 'manual', status: 'error', startedAt: '2026-10-03T00:00:00Z',
  nodes: [{ nodeId: 'removed', nodeType: 'action.http', status: 'error', error: 'HTTP 503',
    input: { text: 'recorded input', json: { query: 'old' } }, output: 'response preview' }],
}
beforeEach(() => { getRun.mockReset() })

it('shows stored input, failure and preview even when the node no longer exists', async () => {
  getRun.mockResolvedValue(run)
  render(<RunDetails automation={createBlankAutomation()} runId="old" onClose={() => {}} onLocate={() => {}} />)
  await screen.findByText('HTTP 503')
  expect(screen.getByText(/recorded input/)).toBeInTheDocument()
  expect(screen.getByText('response preview')).toBeInTheDocument()
  expect(screen.getByText('此节点已从画布删除。')).toBeInTheDocument()
  expect(screen.queryByRole('button', { name: '定位到画布' })).not.toBeInTheDocument()
})

it('retries failed reads and ignores late results from a previously selected run', async () => {
  let finish!: (run: AutomationRun) => void
  getRun.mockImplementationOnce(() => new Promise<AutomationRun>((resolve) => { finish = resolve }))
  const props = { automation: createBlankAutomation(), onClose: () => {}, onLocate: () => {} }
  const view = render(<RunDetails {...props} runId="old" />)
  getRun.mockRejectedValueOnce(new Error('read failed'))
  view.rerender(<RunDetails {...props} runId="new" />)
  await screen.findByText('read failed')
  getRun.mockResolvedValue({ ...run, id: 'new', nodes: [] })
  fireEvent.click(screen.getByRole('button', { name: '重试' }))
  await screen.findByText('没有节点记录。')
  await act(async () => { finish(run) })
  expect(screen.queryByText('HTTP 503')).not.toBeInTheDocument()
})
