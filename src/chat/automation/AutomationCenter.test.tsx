import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import type { Automation } from '../../api/automationContracts'
import { AutomationCenter } from './AutomationCenter'
import { createBlankAutomation } from './graph'

const mocks = vi.hoisted(() => ({ save: vi.fn(), get: vi.fn(), reload: vi.fn() }))
vi.mock('../../api/tauri', () => ({ isTauriRuntime: () => true, api: { onAutomationChanged: async () => () => {} } }))
vi.mock('./api', () => ({ automationApi: { save: mocks.save, get: mocks.get } }))
vi.mock('./AutomationEditor', () => ({ AutomationEditor: ({ automation, onChange, onBack }: {
  automation: Automation; onChange: (value: Automation) => void; onBack: () => void
}) => <div>
  <input aria-label="name" value={automation.name} onChange={(event) => onChange({ ...automation, name: event.target.value })} />
  <button onClick={onBack}>Back</button>
</div> }))

beforeEach(() => {
  vi.resetAllMocks()
  window.location.hash = '#chat/automations/auto'
  mocks.get.mockResolvedValue({ ...createBlankAutomation(), id: 'auto', name: 'Original' })
  mocks.save.mockImplementation(async (value: Automation) => value)
  mocks.reload.mockResolvedValue(undefined)
})

function mount() {
  return render(<AutomationCenter items={[]} loading={false} listError="" onReload={mocks.reload}
    renderList={(body) => <div data-testid="list">{body}</div>} />)
}

it('keeps the draft open on save failure and allows leaving after a successful retry', async () => {
  mount()
  await screen.findByDisplayValue('Original')
  mocks.save.mockRejectedValueOnce(new Error('disk full'))
  fireEvent.change(screen.getByLabelText('name'), { target: { value: 'My workflow' } })
  fireEvent.click(screen.getByText('Back'))
  await screen.findByText(/disk full/)
  expect(screen.getByDisplayValue('My workflow')).toBeInTheDocument()
  expect(screen.queryByTestId('list')).not.toBeInTheDocument()
  fireEvent.click(screen.getByText('Back'))
  await screen.findByTestId('list')
  expect(mocks.save.mock.lastCall?.[0].name).toBe('My workflow')
})

it('serializes autosave and leaving so an older write cannot replace the final draft', async () => {
  let finish!: (value: Automation) => void
  mocks.save.mockImplementationOnce(() => new Promise<Automation>((resolve) => { finish = resolve }))
  mount()
  await screen.findByDisplayValue('Original')
  fireEvent.change(screen.getByLabelText('name'), { target: { value: 'First' } })
  await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1))
  const first = mocks.save.mock.calls[0][0] as Automation
  fireEvent.change(screen.getByLabelText('name'), { target: { value: 'Final' } })
  fireEvent.click(screen.getByText('Back'))
  await act(async () => { await Promise.resolve() })
  expect(mocks.save).toHaveBeenCalledTimes(1)
  await act(async () => { finish(first) })
  await screen.findByTestId('list')
  expect(mocks.save.mock.lastCall?.[0].name).toBe('Final')
})
