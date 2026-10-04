import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import type { ReactNode } from 'react'
import { beforeEach, expect, it, vi } from 'vitest'
import type { ReactFlowProps } from '@xyflow/react'
import type { Automation } from '../../api/automationContracts'
import type { AutomationRfNode } from './nodes/FlowNode'
import { AutomationCenter } from './AutomationCenter'
import { automationDraftStore, resetAutomationDraftStore } from './automationDraftStore'
import { createBlankAutomation } from './graph'

const kept = {
  id: 'kept-node', type: 'trigger.manual' as const, data: { label: 'Kept' }, position: { x: 72, y: 144 },
}
const fresh = {
  id: 'fresh-node', type: 'action.notify' as const, data: { label: 'Fresh' }, position: { x: 480, y: 144 },
}

function doc(overrides: Partial<Automation> = {}): Automation {
  return {
    ...createBlankAutomation(),
    id: 'auto',
    name: 'Original',
    nodes: [kept],
    edges: [],
    viewport: { x: 12, y: 36, zoom: 1 },
    updatedAt: '2026-10-04T00:00:00Z',
    ...overrides,
  }
}

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
    onAutomationRun: async () => () => {},
  },
}))
vi.mock('./api', () => ({
  automationApi: {
    save: mocks.save,
    get: mocks.get,
    validate: async () => [],
    listRuns: async () => [],
    activeRun: async () => null,
    getRun: async () => null,
  },
}))
vi.mock('@xyflow/react', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@xyflow/react')>()
  return {
    ...actual,
    ReactFlow: (props: ReactFlowProps<AutomationRfNode>) => (
      <div data-testid="canvas">{props.nodes?.map((node) => node.id).join(' ')}</div>
    ),
    ReactFlowProvider: ({ children }: { children: ReactNode }) => children,
    useReactFlow: () => ({
      screenToFlowPosition: (point: { x: number; y: number }) => point,
      fitView: async () => {},
    }),
  }
})
vi.mock('./NodeInspector', () => ({ NodeInspector: () => null }))
vi.mock('./WorkflowWorkbench', () => ({
  WorkflowWorkbench: ({ children }: { children?: ReactNode }) => children ?? null,
}))

beforeEach(() => {
  vi.resetAllMocks()
  resetAutomationDraftStore()
  mocks.changed = null
  window.location.hash = '#chat/automations/auto'
  mocks.get.mockResolvedValue(doc())
  mocks.save.mockImplementation(async (value: Automation) => value)
  mocks.reload.mockResolvedValue(undefined)
})

function mount() {
  return render(<AutomationCenter items={[]} loading={false} listError="" onReload={mocks.reload}
    onCreateByChat={() => {}} renderList={(body) => <div data-testid="list">{body}</div>} />)
}

const staleRemote = doc({
  name: 'Remote workflow',
  nodes: [{ id: 'stale-node', type: 'action.notify', data: { label: 'Stale' }, position: { x: 480, y: 144 } }],
  updatedAt: '2026-10-04T03:00:00Z',
})

async function renameAndWaitForAck(name: string) {
  mocks.save.mockImplementation(async (value: Automation) => ({ ...value, updatedAt: '2026-10-04T02:00:00Z' }))
  fireEvent.change(screen.getByRole('textbox'), { target: { value: name } })
  await waitFor(() => {
    const snap = automationDraftStore.getSnapshot()
    expect(snap.status).toBe('saved')
    expect(snap.dirty).toBe(false)
    expect(snap.draft?.name).toBe(name)
    expect(snap.draft?.updatedAt).toBe('2026-10-04T02:00:00Z')
  })
  return automationDraftStore.getSnapshot().draft
}

it('saves the remotely refreshed graph when the only local edit is the name', async () => {
  const view = mount()
  await waitFor(() => expect(screen.getByTestId('canvas')).toHaveTextContent('kept-node'))
  expect(screen.getByTestId('canvas')).not.toHaveTextContent('fresh-node')
  view.unmount()

  mocks.get.mockResolvedValue(doc({
    nodes: [kept, fresh],
    edges: [{ id: 'fresh-edge', source: 'kept-node', target: 'fresh-node' }],
    viewport: { x: 180, y: 48, zoom: 1.2 },
    updatedAt: '2026-10-04T01:00:00Z',
  }))
  mount()
  await waitFor(() => expect(screen.getByTestId('canvas')).toHaveTextContent('fresh-node'))
  expect(screen.getByRole('textbox')).toHaveValue('Original')
  mocks.save.mockClear()

  fireEvent.change(screen.getByRole('textbox'), { target: { value: 'Renamed' } })
  await waitFor(() => expect(mocks.save).toHaveBeenCalled())
  const saved = mocks.save.mock.lastCall?.[0] as Automation
  expect(saved.name).toBe('Renamed')
  expect(saved.nodes.map((node) => node.id)).toEqual(['kept-node', 'fresh-node'])
  expect(saved.nodes.find((node) => node.id === 'fresh-node')?.position).toEqual(fresh.position)
  expect(saved.edges).toEqual([
    expect.objectContaining({ id: 'fresh-edge', source: 'kept-node', target: 'fresh-node' }),
  ])
  expect(saved.viewport).toEqual({ x: 180, y: 48, zoom: 1.2 })
})

it('keeps the saved draft when a refresh started on the clean copy resolves later', async () => {
  const view = mount()
  await waitFor(() => expect(screen.getByTestId('canvas')).toHaveTextContent('kept-node'))
  view.unmount()

  let finish!: (value: Automation) => void
  mocks.get.mockImplementation(() => new Promise<Automation>((resolve) => { finish = resolve }))
  mount()
  await screen.findByRole('textbox')
  const acknowledged = await renameAndWaitForAck('Local workflow')

  await act(async () => {
    finish(staleRemote)
    await Promise.resolve()
  })

  expect(screen.getByRole('textbox')).toHaveValue('Local workflow')
  expect(screen.queryByDisplayValue('Remote workflow')).not.toBeInTheDocument()
  expect(automationDraftStore.getSnapshot().draft).toBe(acknowledged)
  expect(screen.getByTestId('canvas')).toHaveTextContent('kept-node')
  expect(screen.getByTestId('canvas')).not.toHaveTextContent('stale-node')
})

it('keeps a saved edit when a remote read that started clean resolves later', async () => {
  mount()
  await waitFor(() => expect(screen.getByTestId('canvas')).toHaveTextContent('kept-node'))
  let finish!: (value: Automation) => void
  mocks.get.mockImplementation(() => new Promise<Automation>((resolve) => { finish = resolve }))
  await act(async () => {
    mocks.changed?.({ kind: 'updated', id: 'auto', updatedAt: '2026-10-04T03:00:00Z' })
  })
  expect(finish).toEqual(expect.any(Function))
  const acknowledged = await renameAndWaitForAck('Local workflow')

  await act(async () => {
    finish(staleRemote)
    await Promise.resolve()
  })

  expect(screen.getByRole('textbox')).toHaveValue('Local workflow')
  expect(automationDraftStore.getSnapshot().draft).toBe(acknowledged)
  expect(screen.getByTestId('canvas')).not.toHaveTextContent('stale-node')
})
