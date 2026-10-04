import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { AssistantCenter } from './AssistantCenter'
import { resetAssistantDraftStore } from './assistantDraftStore'
import type { ChatAssistant } from './types'

function assistant(id: string, name: string): ChatAssistant {
  return {
    id,
    name,
    description: '',
    icon: 'bot',
    color: '#6A8FBD',
    source: 'user',
    system_prompt: '',
    provider_id: '',
    model: '',
    mcp_server_ids: [],
    skill_ids: [],
    enabled: true,
    installed: true,
    archived: false,
    built_in: false,
    created_at: 1,
    updated_at: 1,
  }
}

const mocks = vi.hoisted(() => ({
  assistants: [] as ChatAssistant[],
  update: vi.fn(),
  create: vi.fn(),
  remove: vi.fn(),
  duplicate: vi.fn(),
}))

vi.mock('./api', () => ({
  chatApi: {
    getAssistants: async () => mocks.assistants,
    updateAssistant: mocks.update,
    createAssistant: mocks.create,
    deleteAssistant: mocks.remove,
    duplicateAssistant: mocks.duplicate,
  },
}))
vi.mock('../api/tauri', () => ({
  api: { onChatAssistantsChanged: async () => () => {} },
}))
vi.mock('../api/settingsCache', () => ({
  getSettingsCached: async () => ({ providers: [], chatTools: { servers: [] } }),
}))
vi.mock('../components/i18n', () => {
  const translations = new Proxy({}, { get: (_object, key) => String(key) })
  return { useT: () => translations, useLang: () => 'zh' }
})

beforeEach(() => {
  vi.resetAllMocks()
  resetAssistantDraftStore()
  mocks.assistants = [assistant('a1', 'Helper'), assistant('b1', 'Other')]
  mocks.update.mockImplementation(async (value: ChatAssistant) => value)
  mocks.create.mockImplementation(async (value: ChatAssistant) => value)
  mocks.remove.mockResolvedValue(undefined)
  mocks.duplicate.mockResolvedValue(assistant('copy', 'Helper copy'))
  window.confirm = () => true
})

function mount() {
  return render(<AssistantCenter skills={[]} onStartAssistantChat={() => {}} />)
}

async function editNamed(name: string) {
  fireEvent.click(await screen.findByText(name))
  fireEvent.click(screen.getByRole('button', { name: 'chatAssistantEdit' }))
  return screen.getByLabelText('chatAssistantName')
}

async function save(action = 'save') {
  await act(async () => { fireEvent.click(screen.getByRole('button', { name: action })) })
}

it('keeps an unsaved draft across navigation without autosaving, and drops it after back', async () => {
  let view = mount()
  const name = await editNamed('Helper')
  fireEvent.change(name, { target: { value: 'Helper draft' } })
  expect(mocks.update).not.toHaveBeenCalled()
  view.unmount()
  view = mount()
  expect(screen.getByLabelText('chatAssistantName')).toHaveValue('Helper draft')
  expect(mocks.update).not.toHaveBeenCalled()
  fireEvent.click(screen.getByRole('button', { name: 'chatAssistantBack' }))
  view.unmount()
  view = mount()
  await screen.findByText('Helper')
  expect(screen.queryByDisplayValue('Helper draft')).not.toBeInTheDocument()
  view.unmount()
})

it('keeps a saving draft when the page is left and shows the error for a later retry', async () => {
  let rejectUpdate!: (error: Error) => void
  mocks.update.mockImplementationOnce(() => new Promise<ChatAssistant>((_resolve, reject) => { rejectUpdate = reject }))
  const view = mount()
  const name = await editNamed('Helper')
  fireEvent.change(name, { target: { value: 'Helper draft' } })
  await save()
  expect(screen.getByRole('button', { name: 'save' })).toBeDisabled()
  view.unmount()
  mount()
  expect(screen.getByLabelText('chatAssistantName')).toHaveValue('Helper draft')
  expect(screen.getByRole('button', { name: 'save' })).toBeDisabled()
  await act(async () => { rejectUpdate(new Error('disk full')) })
  expect(screen.getByText('disk full')).toBeInTheDocument()
  expect(screen.getByLabelText('chatAssistantName')).toHaveValue('Helper draft')
  await save()
  await act(async () => { await Promise.resolve() })
  expect(mocks.update.mock.lastCall?.[0].name).toBe('Helper draft')
  expect(screen.queryByText('disk full')).not.toBeInTheDocument()
})

it.each(['save', 'chatAssistantStartChat'])('keeps a returned editor open when its retired %s action finishes', async (action) => {
  let finish!: (value: ChatAssistant) => void
  mocks.update.mockImplementationOnce(() => new Promise<ChatAssistant>((resolve) => { finish = resolve }))
  const onStartAssistantChat = vi.fn()
  const first = render(<AssistantCenter skills={[]} onStartAssistantChat={onStartAssistantChat} />)
  fireEvent.change(await editNamed('Helper'), { target: { value: 'Saved draft' } })
  await save(action)
  first.unmount()
  render(<AssistantCenter skills={[]} onStartAssistantChat={onStartAssistantChat} />)
  expect(screen.getByLabelText('chatAssistantName')).toHaveValue('Saved draft')
  await act(async () => { finish(assistant('a1', 'Saved draft')) })
  expect(screen.getByLabelText('chatAssistantName')).toHaveValue('Saved draft')
  expect(screen.getByRole('button', { name: 'save' })).toBeEnabled()
  expect(onStartAssistantChat).not.toHaveBeenCalled()
})

it('serializes an edit made while saving and keeps the newest draft', async () => {
  let finish!: (value: ChatAssistant) => void
  mocks.update.mockImplementationOnce(() => new Promise<ChatAssistant>((resolve) => { finish = resolve }))
  mount()
  const name = await editNamed('Helper')
  fireEvent.change(name, { target: { value: 'First' } })
  await save()
  fireEvent.change(screen.getByLabelText('chatAssistantName'), { target: { value: 'Final' } })
  await act(async () => { finish({ ...assistant('a1', 'First') }) })
  expect(mocks.update.mock.lastCall?.[0].name).toBe('Final')
})

it('does not apply a late save onto a different assistant', async () => {
  let finish!: (value: ChatAssistant) => void
  mocks.update.mockImplementationOnce(() => new Promise<ChatAssistant>((resolve) => { finish = resolve }))
  mount()
  const name = await editNamed('Helper')
  fireEvent.change(name, { target: { value: 'Helper draft' } })
  await save()
  fireEvent.click(screen.getByRole('button', { name: 'chatAssistantBack' }))
  fireEvent.click(screen.getByRole('button', { name: 'chatAssistantBackToList' }))
  fireEvent.click(await screen.findByText('Other'))
  fireEvent.click(screen.getByRole('button', { name: 'chatAssistantEdit' }))
  expect(screen.getByLabelText('chatAssistantName')).toHaveValue('Other')
  await act(async () => { finish({ ...assistant('a1', 'STALE') }) })
  expect(screen.getByLabelText('chatAssistantName')).toHaveValue('Other')
  expect(mocks.update.mock.calls.every((call) => (call[0] as ChatAssistant).id !== 'b1')).toBe(true)
})

it('does not resurrect a draft after delete once a late save settles', async () => {
  let finish!: (value: ChatAssistant) => void
  mocks.update.mockImplementationOnce(() => new Promise<ChatAssistant>((resolve) => { finish = resolve }))
  const view = mount()
  const name = await editNamed('Helper')
  fireEvent.change(name, { target: { value: 'Doomed' } })
  await save()
  fireEvent.click(screen.getByRole('button', { name: 'chatAssistantDeleteTitle' }))
  await act(async () => { finish({ ...assistant('a1', 'Doomed') }) })
  await screen.findByText('Helper')
  expect(screen.queryByDisplayValue('Doomed')).not.toBeInTheDocument()
  expect(mocks.remove).toHaveBeenCalledWith('a1')
  view.unmount()
  mount()
  await screen.findByText('Helper')
  expect(screen.queryByDisplayValue('Doomed')).not.toBeInTheDocument()
  expect(screen.queryByLabelText('chatAssistantName')).not.toBeInTheDocument()
})

it('keeps the next draft when a retired delete finishes', async () => {
  let finishDelete!: () => void
  mocks.remove.mockImplementation(() => new Promise<void>((resolve) => { finishDelete = resolve }))
  mount()
  await editNamed('Helper')
  fireEvent.click(screen.getByRole('button', { name: 'chatAssistantDeleteTitle' }))
  fireEvent.click(screen.getByRole('button', { name: 'chatAssistantBack' }))
  fireEvent.click(screen.getByRole('button', { name: 'chatAssistantBackToList' }))
  fireEvent.click(await screen.findByText('Other'))
  fireEvent.click(screen.getByRole('button', { name: 'chatAssistantEdit' }))
  fireEvent.change(screen.getByLabelText('chatAssistantName'), { target: { value: 'Draft B' } })
  await act(async () => { finishDelete() })
  expect(screen.getByLabelText('chatAssistantName')).toHaveValue('Draft B')
  expect(mocks.remove).toHaveBeenCalledWith('a1')
})

it('keeps the next draft when a retired delete of an unsaved assistant finishes', async () => {
  let finishCreate!: (value: ChatAssistant) => void
  mocks.create.mockImplementation(() => new Promise<ChatAssistant>((resolve) => { finishCreate = resolve }))
  let finishDelete!: () => void
  mocks.remove.mockImplementation(() => new Promise<void>((resolve) => { finishDelete = resolve }))
  mount()
  await screen.findByText('Helper')
  fireEvent.click(screen.getByRole('button', { name: 'chatAssistantCreate' }))
  await save()
  const createdId = (mocks.create.mock.calls[0][0] as ChatAssistant).id
  fireEvent.click(screen.getByRole('button', { name: 'chatAssistantDeleteTitle' }))
  fireEvent.click(screen.getByRole('button', { name: 'chatAssistantBack' }))
  fireEvent.click(await screen.findByText('Other'))
  fireEvent.click(screen.getByRole('button', { name: 'chatAssistantEdit' }))
  fireEvent.change(screen.getByLabelText('chatAssistantName'), { target: { value: 'Draft B' } })
  await act(async () => { finishCreate(assistant(createdId, 'Draft A')) })
  expect(screen.getByLabelText('chatAssistantName')).toHaveValue('Draft B')
  await act(async () => { finishDelete() })
  expect(screen.getByLabelText('chatAssistantName')).toHaveValue('Draft B')
  expect(mocks.remove).toHaveBeenCalledWith(createdId)
})

it('keeps the open draft when a delete finishes after remount', async () => {
  let finishDelete!: () => void
  mocks.remove.mockImplementation(() => new Promise<void>((resolve) => { finishDelete = resolve }))
  const first = mount()
  fireEvent.change(await editNamed('Helper'), { target: { value: 'Draft A' } })
  fireEvent.click(screen.getByRole('button', { name: 'chatAssistantDeleteTitle' }))
  await waitFor(() => expect(mocks.remove).toHaveBeenCalledWith('a1'))
  first.unmount()
  mount()
  expect(screen.getByLabelText('chatAssistantName')).toHaveValue('Draft A')
  await act(async () => { finishDelete() })
  expect(screen.getByLabelText('chatAssistantName')).toHaveValue('Draft A')
  expect(mocks.remove).toHaveBeenCalledWith('a1')
})

it('keeps the next draft when a retired duplicate finishes', async () => {
  let finishDuplicate!: (value: ChatAssistant) => void
  mocks.duplicate.mockImplementation(() => new Promise<ChatAssistant>((resolve) => { finishDuplicate = resolve }))
  mount()
  fireEvent.click(await screen.findByText('Helper'))
  fireEvent.click(screen.getByRole('button', { name: 'chatAssistantDuplicate' }))
  fireEvent.click(screen.getByRole('button', { name: 'chatAssistantBackToList' }))
  fireEvent.click(await screen.findByText('Other'))
  fireEvent.click(screen.getByRole('button', { name: 'chatAssistantEdit' }))
  fireEvent.change(screen.getByLabelText('chatAssistantName'), { target: { value: 'Draft B' } })
  await act(async () => { finishDuplicate(assistant('copy', 'Helper copy')) })
  expect(screen.getByLabelText('chatAssistantName')).toHaveValue('Draft B')
  expect(mocks.duplicate).toHaveBeenCalledWith('a1')
})

it('does not open a duplicate after the detail selection changes', async () => {
  let finishDuplicate!: (value: ChatAssistant) => void
  mocks.duplicate.mockImplementation(() => new Promise<ChatAssistant>((resolve) => { finishDuplicate = resolve }))
  mount()
  fireEvent.click(await screen.findByText('Helper'))
  fireEvent.click(screen.getByRole('button', { name: 'chatAssistantDuplicate' }))
  fireEvent.click(screen.getByRole('button', { name: 'chatAssistantBackToList' }))
  fireEvent.click(await screen.findByText('Other'))
  await act(async () => { finishDuplicate(assistant('copy', 'Helper copy')) })
  expect(screen.getByRole('heading', { level: 2, name: 'Other' })).toBeInTheDocument()
  expect(screen.getByRole('button', { name: 'chatAssistantBackToList' })).toBeInTheDocument()
  expect(screen.queryByLabelText('chatAssistantName')).not.toBeInTheDocument()
  expect(mocks.duplicate).toHaveBeenCalledWith('a1')
})

it('does not open a duplicate after leaving its detail', async () => {
  let finishDuplicate!: (value: ChatAssistant) => void
  mocks.duplicate.mockImplementation(() => new Promise<ChatAssistant>((resolve) => { finishDuplicate = resolve }))
  mount()
  fireEvent.click(await screen.findByText('Helper'))
  fireEvent.click(screen.getByRole('button', { name: 'chatAssistantDuplicate' }))
  fireEvent.click(screen.getByRole('button', { name: 'chatAssistantBackToList' }))
  await act(async () => { finishDuplicate(assistant('copy', 'Helper copy')) })
  expect(screen.queryByLabelText('chatAssistantName')).not.toBeInTheDocument()
  expect(screen.getByText('Helper')).toBeInTheDocument()
  expect(mocks.duplicate).toHaveBeenCalledWith('a1')
})

it('does not open a duplicate after the center remounts', async () => {
  let finishDuplicate!: (value: ChatAssistant) => void
  mocks.duplicate.mockImplementation(() => new Promise<ChatAssistant>((resolve) => { finishDuplicate = resolve }))
  const first = mount()
  fireEvent.click(await screen.findByText('Helper'))
  fireEvent.click(screen.getByRole('button', { name: 'chatAssistantDuplicate' }))
  first.unmount()
  mount()
  await screen.findByText('Helper')
  await act(async () => { finishDuplicate(assistant('copy', 'Helper copy')) })
  expect(screen.queryByLabelText('chatAssistantName')).not.toBeInTheDocument()
  expect(mocks.duplicate).toHaveBeenCalledWith('a1')
})

it('opens a duplicate while its detail stays current', async () => {
  mount()
  fireEvent.click(await screen.findByText('Helper'))
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: 'chatAssistantDuplicate' }))
  })
  expect(screen.getByLabelText('chatAssistantName')).toHaveValue('Helper copy')
})
