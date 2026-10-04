import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import type { Automation } from '../../api/automationContracts'
import { AutomationCenter } from './AutomationCenter'
import { resetAutomationDraftStore, showAutomation } from './automationDraftStore'
import { createBlankAutomation } from './graph'

const mocks = vi.hoisted(() => ({
  save: vi.fn(),
  get: vi.fn(),
  reload: vi.fn(),
  changed: null as null | ((event: { kind: string; id: string; updatedAt?: string | null }) => void),
}))
vi.mock('../../api/tauri', () => ({
  isTauriRuntime: () => true,
  api: {
    onAutomationChanged: async (cb: (event: { kind: string; id: string; updatedAt?: string | null }) => void) => {
      mocks.changed = cb
      return () => {}
    },
  },
}))
vi.mock('./api', () => ({ automationApi: { save: mocks.save, get: mocks.get } }))
vi.mock('./AutomationEditor', () => ({ AutomationEditor: ({ automation, onChange, onBack }: {
  automation: Automation; onChange: (value: Automation) => void; onBack: () => void
}) => <div>
  <input aria-label="name" value={automation.name} onChange={(event) => onChange({ ...automation, name: event.target.value })} />
  <button onClick={onBack}>Back</button>
</div> }))

beforeEach(() => {
  vi.resetAllMocks()
  resetAutomationDraftStore()
  mocks.changed = null
  window.location.hash = '#chat/automations/auto'
  mocks.get.mockResolvedValue({ ...createBlankAutomation(), id: 'auto', name: 'Original' })
  mocks.save.mockImplementation(async (value: Automation) => value)
  mocks.reload.mockResolvedValue(undefined)
})

function mount() {
  return render(<AutomationCenter items={[]} loading={false} listError="" onReload={mocks.reload}
    onCreateByChat={() => {}} renderList={(body) => <div data-testid="list">{body}</div>} />)
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

it('keeps the draft and saving state when the page is left and reopened', async () => {
  let finish!: (value: Automation) => void
  mocks.save.mockImplementationOnce(() => new Promise<Automation>((resolve) => { finish = resolve }))
  const view = mount()
  await screen.findByDisplayValue('Original')
  fireEvent.change(screen.getByLabelText('name'), { target: { value: 'My workflow' } })
  await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1))
  const sent = mocks.save.mock.calls[0][0] as Automation
  view.unmount()
  mount()
  expect(screen.getByDisplayValue('My workflow')).toBeInTheDocument()
  expect(screen.getByText('正在保存…')).toBeInTheDocument()
  await act(async () => { finish(sent) })
  expect(screen.getByDisplayValue('My workflow')).toBeInTheDocument()
  expect(mocks.save).toHaveBeenCalledTimes(1)
})

it('restores the draft and error when a save fails while the page is gone, then retries it', async () => {
  let rejectSave!: (error: Error) => void
  mocks.save.mockImplementationOnce(() => new Promise<Automation>((_resolve, reject) => { rejectSave = reject }))
  const view = mount()
  await screen.findByDisplayValue('Original')
  fireEvent.change(screen.getByLabelText('name'), { target: { value: 'My workflow' } })
  await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1))
  view.unmount()
  await act(async () => { rejectSave(new Error('disk full')) })
  mount()
  expect(screen.getByDisplayValue('My workflow')).toBeInTheDocument()
  expect(screen.getByText(/disk full/)).toBeInTheDocument()
  fireEvent.click(screen.getByText('重试保存'))
  await waitFor(() => expect(mocks.save.mock.lastCall?.[0].name).toBe('My workflow'))
  expect(mocks.save).toHaveBeenCalledTimes(2)
})

it('keeps a newer edit made during the flight after the page is gone', async () => {
  let finish!: (value: Automation) => void
  mocks.save.mockImplementationOnce(() => new Promise<Automation>((resolve) => { finish = resolve }))
  const view = mount()
  await screen.findByDisplayValue('Original')
  fireEvent.change(screen.getByLabelText('name'), { target: { value: 'First' } })
  await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1))
  const first = mocks.save.mock.calls[0][0] as Automation
  fireEvent.change(screen.getByLabelText('name'), { target: { value: 'Final' } })
  view.unmount()
  await act(async () => { finish(first) })
  expect(mocks.save.mock.lastCall?.[0].name).toBe('Final')
  mount()
  expect(screen.getByDisplayValue('Final')).toBeInTheDocument()
})

it('does not apply a late save onto a different automation', async () => {
  let finish!: (value: Automation) => void
  mocks.save.mockImplementationOnce(() => new Promise<Automation>((resolve) => { finish = resolve }))
  mount()
  await screen.findByDisplayValue('Original')
  fireEvent.change(screen.getByLabelText('name'), { target: { value: 'First' } })
  await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1))
  const first = mocks.save.mock.calls[0][0] as Automation
  showAutomation({ ...createBlankAutomation(), id: 'other', name: 'Other doc' })
  await act(async () => { finish({ ...first, name: 'SERVER' }) })
  expect(screen.getByDisplayValue('Other doc')).toBeInTheDocument()
  expect(screen.queryByDisplayValue('SERVER')).not.toBeInTheDocument()
  expect(mocks.save.mock.calls.every((call) => (call[0] as Automation).id !== 'other')).toBe(true)
})

it('does not restore a draft after the automation is deleted while a save is in flight', async () => {
  let finish!: (value: Automation) => void
  mocks.save.mockImplementationOnce(() => new Promise<Automation>((resolve) => { finish = resolve }))
  const view = mount()
  await screen.findByDisplayValue('Original')
  fireEvent.change(screen.getByLabelText('name'), { target: { value: 'Doomed' } })
  await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1))
  const sent = mocks.save.mock.calls[0][0] as Automation
  await act(async () => { mocks.changed?.({ kind: 'deleted', id: 'auto' }) })
  expect(screen.getByTestId('list')).toBeInTheDocument()
  await act(async () => { finish({ ...sent, name: 'SERVER' }) })
  expect(screen.queryByDisplayValue('SERVER')).not.toBeInTheDocument()
  expect(screen.queryByDisplayValue('Doomed')).not.toBeInTheDocument()
  view.unmount()
  window.location.hash = '#chat/automations/auto'
  mount()
  await screen.findByDisplayValue('Original')
  expect(screen.queryByDisplayValue('Doomed')).not.toBeInTheDocument()
})

it('restores the retained editor from the Tasks root without treating return as Back', async () => {
  let finish!: (value: Automation) => void
  mocks.save.mockImplementationOnce(() => new Promise<Automation>((resolve) => { finish = resolve }))
  const view = mount()
  await screen.findByDisplayValue('Original')
  fireEvent.change(screen.getByLabelText('name'), { target: { value: 'Retained workflow' } })
  view.unmount()
  await waitFor(() => expect(mocks.save).toHaveBeenCalledOnce())
  const sent = mocks.save.mock.calls[0][0] as Automation
  window.location.hash = '#chat/automations'
  mount()
  expect(screen.getByDisplayValue('Retained workflow')).toBeInTheDocument()
  await act(async () => { finish(sent) })
  expect(screen.getByDisplayValue('Retained workflow')).toBeInTheDocument()
  expect(window.location.hash).toBe('#chat/automations/auto')
  fireEvent.click(screen.getByText('Back'))
  await screen.findByTestId('list')
  expect(screen.queryByLabelText('name')).not.toBeInTheDocument()
})

it('refreshes a clean retained automation after a remote update while away', async () => {
  const view = mount()
  await screen.findByDisplayValue('Original')
  view.unmount()
  mocks.get.mockResolvedValue({
    ...createBlankAutomation(), id: 'auto', name: 'Remote workflow', updatedAt: '2026-10-04T01:00:00Z',
  })
  mount()
  await screen.findByDisplayValue('Remote workflow')
})

it('does not overwrite an edit typed while the return refresh is pending', async () => {
  const view = mount()
  await screen.findByDisplayValue('Original')
  view.unmount()
  let finish!: (value: Automation) => void
  mocks.get.mockImplementation(() => new Promise<Automation>((resolve) => { finish = resolve }))
  mount()
  fireEvent.change(screen.getByLabelText('name'), { target: { value: 'Local workflow' } })
  await act(async () => {
    await Promise.resolve()
    finish({ ...createBlankAutomation(), id: 'auto', name: 'Remote workflow', updatedAt: '2026-10-04T01:00:00Z' })
  })
  expect(screen.getByDisplayValue('Local workflow')).toBeInTheDocument()
  expect(screen.queryByDisplayValue('Remote workflow')).not.toBeInTheDocument()
})
