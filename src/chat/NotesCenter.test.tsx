import { act, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { NotesCenter } from './NotesCenter'
import type { Note } from '../api/tauri'
import { NOTE_SAVE_DEBOUNCE_MS, resetNoteDraftStore, showNote } from './notesDraftStore'

const mocks = vi.hoisted(() => ({
  update: vi.fn(),
  remove: vi.fn(),
  onMarkdown: null as null | ((ctx: unknown, md: string) => void),
  initialMarkdown: '',
  editor: null as null | { markdown: string },
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
    notesDelete: mocks.remove,
  },
}))
vi.mock('@milkdown/crepe', () => ({
  Crepe: class {
    markdown: string
    constructor({ defaultValue }: { defaultValue: string }) {
      this.markdown = defaultValue
      mocks.initialMarkdown = defaultValue
      mocks.editor = this
    }
    on(register: (listener: unknown) => void) {
      register({ markdownUpdated: (callback: typeof mocks.onMarkdown) => {
        mocks.onMarkdown = (ctx, markdown) => {
          this.markdown = markdown
          callback?.(ctx, markdown)
        }
      } })
    }
    create() { return Promise.resolve() }
    getMarkdown() { return this.markdown }
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
  resetNoteDraftStore()
  mocks.remove.mockResolvedValue(undefined)
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
async function advance(ms = NOTE_SAVE_DEBOUNCE_MS) { await act(async () => { await vi.advanceTimersByTimeAsync(ms) }) }
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

it('keeps the latest draft and saving state when the page is left and reopened', async () => {
  let finish!: (note: Note) => void
  mocks.update.mockImplementationOnce(() => new Promise(resolve => { finish = resolve }))
  const view = render(<NotesCenter />)
  await act(async () => {})
  await act(async () => { fireEvent.click(screen.getByText('Test note')) })
  typeContent('kept draft')
  await advance()
  view.unmount()
  mocks.initialMarkdown = ''
  await act(async () => { render(<NotesCenter />) })
  expect(screen.getByText('annotateSaving')).toBeTruthy()
  expect(mocks.initialMarkdown).toBe('kept draft')
  await act(async () => { finish({ ...mocks.note, content: 'kept draft' }) })
  expect(mocks.update).toHaveBeenCalledWith('n1', 'Test note', 'kept draft', '')
})

it('restores the draft and error when a save fails while the page is gone, then retries it', async () => {
  let rejectUpdate!: (error: Error) => void
  mocks.update.mockImplementationOnce(() => new Promise((_resolve, reject) => { rejectUpdate = reject }))
  const view = render(<NotesCenter />)
  await act(async () => {})
  await act(async () => { fireEvent.click(screen.getByText('Test note')) })
  typeContent('kept draft')
  await advance()
  view.unmount()
  await act(async () => { rejectUpdate(new Error('disk full')) })
  mocks.initialMarkdown = ''
  await act(async () => { render(<NotesCenter />) })
  expect(screen.getByText('disk full')).toBeTruthy()
  expect(mocks.initialMarkdown).toBe('kept draft')
  await back()
  expect(mocks.persisted?.content).toBe('kept draft')
  expect(screen.queryByText('disk full')).toBeNull()
})

it('serializes an edit made during the flight and keeps it after the page is gone', async () => {
  let finish!: (note: Note) => void
  mocks.update.mockImplementationOnce(() => new Promise(resolve => { finish = resolve }))
  const view = render(<NotesCenter />)
  await act(async () => {})
  await act(async () => { fireEvent.click(screen.getByText('Test note')) })
  typeContent('draft A')
  await advance()
  typeContent('draft AB')
  view.unmount()
  await act(async () => { finish({ ...mocks.note, content: 'draft A' }) })
  expect(mocks.persisted?.content).toBe('draft AB')
  mocks.initialMarkdown = ''
  await act(async () => { render(<NotesCenter />) })
  expect(mocks.initialMarkdown).toBe('draft AB')
})

it('does not apply a late save onto a different note', async () => {
  let finish!: (note: Note) => void
  mocks.update.mockImplementationOnce(() => new Promise(resolve => { finish = resolve }))
  await openEditor()
  typeContent('note one draft')
  await advance()
  await act(async () => {
    showNote({ ...mocks.note, id: 'n2', title: 'Other', content: 'other body' })
  })
  await act(async () => { finish({ ...mocks.note, id: 'n1', title: 'Server title', content: 'note one draft' }) })
  expect(screen.getByRole('textbox')).toHaveProperty('value', 'Other')
  expect(mocks.initialMarkdown).toBe('other body')
  expect(mocks.update).toHaveBeenCalledTimes(1)
  expect(screen.queryByDisplayValue('Server title')).toBeNull()
})

it('does not resurrect a draft after it is deleted, even if a save completes later', async () => {
  let finish!: (note: Note) => void
  mocks.update.mockImplementationOnce(() => new Promise(resolve => { finish = resolve }))
  const view = render(<NotesCenter />)
  await act(async () => {})
  await act(async () => { fireEvent.click(screen.getByText('Test note')) })
  typeContent('doomed')
  await advance()
  window.confirm = () => true
  fireEvent.click(screen.getByRole('button', { name: 'chatDelete' }))
  await act(async () => { finish({ ...mocks.note, content: 'doomed' }) })
  expect(screen.queryByRole('button', { name: 'chatNotesBack' })).toBeNull()
  expect(mocks.remove).toHaveBeenCalledWith('n1')
  view.unmount()
  mocks.initialMarkdown = ''
  await act(async () => { render(<NotesCenter />) })
  expect(screen.queryByRole('button', { name: 'chatNotesBack' })).toBeNull()
  expect(mocks.initialMarkdown).toBe('')
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

it.each(['back', 'unmount'])('preserves unpublished editor text before %s', async (navigation) => {
  const view = render(<NotesCenter />)
  await act(async () => {})
  await act(async () => { fireEvent.click(screen.getByText('Test note')) })
  // Milkdown has changed its document, but its 200ms listener has not run.
  mocks.editor!.markdown = 'last keystrokes'
  if (navigation === 'back') await back()
  else await act(async () => { view.unmount() })
  expect(mocks.persisted?.content).toBe('last keystrokes')
  if (navigation === 'back') {
    await act(async () => { fireEvent.click(screen.getByText('Test note')) })
  } else {
    await act(async () => { render(<NotesCenter />) })
  }
  expect(mocks.initialMarkdown).toBe('last keystrokes')
})
