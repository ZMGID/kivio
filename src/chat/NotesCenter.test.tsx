import { act, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { NotesCenter } from './NotesCenter'
import type { Note } from '../api/tauri'

const mocks = vi.hoisted(() => ({
  update: vi.fn(),
  onMarkdown: null as null | ((ctx: unknown, md: string) => void),
  initialMarkdown: '',
  note: {
    id: 'n1', title: 'Test note', content: 'original', folder: '', origin: 'user',
    createdAt: '2026-01-01', updatedAt: '2026-01-01',
  },
  persisted: null as Note | null,
}))
vi.mock('../api/tauri', () => ({
  isTauriRuntime: () => true,
  api: {
    notesList: async () => [{ ...mocks.persisted, preview: 'original' }],
    notesFoldersList: async () => [],
    notesRead: async () => mocks.persisted,
    notesDirPath: async () => '',
    notesUpdate: mocks.update,
  },
}))
vi.mock('@milkdown/crepe', () => ({
  Crepe: class {
    constructor({ defaultValue }: { defaultValue: string }) { mocks.initialMarkdown = defaultValue }
    on(register: (listener: unknown) => void) {
      register({ markdownUpdated: (callback: typeof mocks.onMarkdown) => { mocks.onMarkdown = callback } })
    }
    create() { return Promise.resolve() }
    destroy() {}
  },
}))
vi.mock('../components/i18n', () => {
  const translations = new Proxy({}, { get: (_object, key) => String(key) })
  return { useT: () => translations, useLang: () => 'zh' }
})
vi.mock('./dock/workspaceActivity', () => ({
  workspaceActivity: { isAvailable: () => true, subscribe: () => () => {} },
}))

beforeEach(() => {
  vi.useFakeTimers()
  vi.resetAllMocks()
  mocks.persisted = { ...mocks.note }
  mocks.update.mockImplementation(async (id: string, title: string, content: string, folder: string) => {
    mocks.persisted = { ...mocks.note, id, title, content, folder }
    return mocks.persisted
  })
})
afterEach(() => vi.useRealTimers())

async function openEditor() {
  await act(async () => { render(<NotesCenter />) })
  await act(async () => { fireEvent.click(screen.getByText('Test note')) })
}
function typeContent(content: string) { act(() => mocks.onMarkdown!(null, content)) }
async function advance(ms = 800) { await act(async () => { await vi.advanceTimersByTimeAsync(ms) }) }
async function back() {
  await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'chatNotesBack' })) })
}

it('preserves typing after submission and persists it before returning to the list', async () => {
  let finish!: (note: Note) => void
  mocks.update.mockImplementationOnce(() => new Promise(resolve => { finish = resolve }))
  await openEditor()
  typeContent('draft A')
  await advance()
  expect(mocks.update).toHaveBeenCalledTimes(1)
  typeContent('draft AB')
  // Even another debounce and an explicit flush cannot create overlapping writes.
  await advance()
  await back()
  expect(mocks.update).toHaveBeenCalledTimes(1)
  expect(screen.queryByRole('button', { name: 'chatNotesBack' })).not.toBeNull()
  await act(async () => { finish({ ...mocks.note, content: 'draft A' }) })
  expect(mocks.persisted?.content).toBe('draft AB')
  expect(screen.queryByRole('button', { name: 'chatNotesBack' })).toBeNull()
  await act(async () => { fireEvent.click(screen.getByText('Test note')) })
  expect(mocks.initialMarkdown).toBe('draft AB')
})

it('keeps a failed draft editable and retries it successfully before leaving', async () => {
  mocks.update.mockRejectedValueOnce(new Error('disk full'))
  await openEditor()
  typeContent('unsaved important content')
  await back()
  expect(screen.queryByRole('button', { name: 'chatNotesBack' })).not.toBeNull()
  expect(screen.queryByText('disk full')).not.toBeNull()
  expect(mocks.persisted?.content).toBe('original')
  typeContent('repaired draft')
  await back()
  expect(mocks.persisted?.content).toBe('repaired draft')
  expect(screen.queryByText('disk full')).toBeNull()
  await act(async () => { fireEvent.click(screen.getByText('Test note')) })
  expect(mocks.initialMarkdown).toBe('repaired draft')
})

it('does not write an unchanged note', async () => {
  await openEditor()
  await back()
  expect(mocks.update).not.toHaveBeenCalled()
})

it('preserves title edits made while a content save is pending', async () => {
  let finish!: (note: Note) => void
  mocks.update.mockImplementationOnce(() => new Promise(resolve => { finish = resolve }))
  await openEditor()
  typeContent('updated content')
  await advance()
  fireEvent.change(screen.getByRole('textbox'), { target: { value: 'New title' } })
  await act(async () => { finish({ ...mocks.note, content: 'updated content' }) })
  await back()
  expect(mocks.persisted).toMatchObject({ title: 'New title', content: 'updated content' })
})
