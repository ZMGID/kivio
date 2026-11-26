import { act, fireEvent, screen } from '@testing-library/react'
import { render } from '../test/renderWithNotices'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { NotesCenter } from './NotesCenter'
import type { Note, NoteEdit } from '../api/tauri'
import { NOTE_SAVE_DEBOUNCE_MS, resetNoteDraftStore, showNote } from './notesDraftStore'

const mocks = vi.hoisted(() => ({
  update: vi.fn(),
  remove: vi.fn(),
  search: vi.fn(),
  trashList: vi.fn(),
  restore: vi.fn(),
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
    notesSearch: mocks.search,
    notesFoldersList: async () => ['Work'],
    notesRead: async () => mocks.persisted,
    notesDirPath: async () => '',
    notesUpdate: mocks.update,
    notesDelete: mocks.remove,
    notesTrashList: mocks.trashList,
    notesRestore: mocks.restore,
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
  mocks.trashList.mockResolvedValue([])
  mocks.restore.mockResolvedValue(undefined)
  mocks.persisted = { ...mocks.note }
  mocks.search.mockImplementation(async (query: string) => {
    const note = mocks.persisted
    return note?.content.includes(query) ? [{ ...note, preview: note.content.slice(-90) }] : []
  })
  mocks.update.mockImplementation(async (id: string, edit: NoteEdit) => {
    const note = mocks.persisted
    if (!note) throw new Error('note missing')
    mocks.persisted = {
      ...note, id,
      title: edit.title?.after ?? note.title,
      content: edit.content?.after ?? note.content,
      folder: edit.folder?.after ?? note.folder,
    }
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

it('searches across folders without separate source tabs', async () => {
  const content = 'preface '.repeat(40) + 'needle past the preview'
  mocks.persisted = { ...mocks.note, content, folder: 'Work', origin: 'chat' }
  await act(async () => { render(<NotesCenter />) })
  expect(screen.queryByText('Test note')).toBeNull()
  expect(screen.queryByText('chatNotesTabChat')).toBeNull()
  expect(screen.queryByText('chatNotesTabLibrary')).toBeNull()
  await act(async () => { fireEvent.change(screen.getByPlaceholderText('chatNotesSearchPlaceholder'), { target: { value: 'needle past the preview' } }) })
  await advance(200)
  expect(screen.getByText(/needle past the preview/)).toBeInTheDocument()
  expect(screen.getByText('Work')).toBeInTheDocument()
  await act(async () => { fireEvent.click(screen.getByText('Test note')) })
  expect(mocks.initialMarkdown).toBe(content)
})

it.each(['user', 'chat'])('shows a moved %s note in its folder and after reopening', async (origin) => {
  mocks.persisted = { ...mocks.note, origin }
  const view = render(<NotesCenter />)
  await act(async () => {})
  await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'chatNotesMoveToFolder' })) })
  await act(async () => { fireEvent.click(screen.getByText('Work', { selector: 'span' })) })
  expect(mocks.persisted).toMatchObject({ folder: 'Work', origin, content: 'original' })
  await act(async () => { fireEvent.click(screen.getByText('Work')) })
  expect(screen.getByText('Test note')).toBeInTheDocument()
  view.unmount()
  await act(async () => { render(<NotesCenter />) })
  await act(async () => { fireEvent.click(screen.getByText('Work')) })
  expect(screen.getByText('Test note')).toBeInTheDocument()
  await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'chatNotesMoveToFolder' })) })
  await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'chatNotesLibraryRoot' })) })
  await act(async () => { fireEvent.click(screen.getByRole('button', { name: /chatNavNotes/ })) })
  expect(screen.getByText('Test note')).toBeInTheDocument()
  expect(mocks.persisted).toMatchObject({ folder: '', origin, content: 'original' })
})

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
  expect(screen.queryByRole('alert')).toBeNull()
  await act(async () => { fireEvent.click(screen.getByText('Test note')) })
  expect(mocks.initialMarkdown).toBe('repaired draft')
})

it('does not write an unchanged note', async () => {
  await openEditor()
  await back()
  expect(mocks.update).not.toHaveBeenCalled()
})

it('shows pending and failed saves accurately and retries from the editor', async () => {
  mocks.update.mockRejectedValueOnce(new Error('disk full'))
  await openEditor()
  typeContent('pending draft')
  expect(screen.getByText('chatNotesPending')).toBeInTheDocument()
  await advance()
  expect(screen.getByText('chatNotesSaveFailed')).toBeInTheDocument()
  expect(screen.queryByText('saved')).toBeNull()
  await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'chatNotesRetrySave' })) })
  expect(screen.getByText('saved')).toBeInTheDocument()
  expect(mocks.persisted?.content).toBe('pending draft')
})

it('lets the move button handle Enter without opening its note', async () => {
  await act(async () => { render(<NotesCenter />) })
  const move = screen.getByRole('button', { name: 'chatNotesMoveToFolder' })
  await act(async () => { expect(fireEvent.keyDown(move, { key: 'Enter' })).toBe(true) })
  expect(screen.queryByRole('button', { name: 'chatNotesBack' })).toBeNull()
})

it('keeps the visible baseline and draft when an outside edit conflicts', async () => {
  await openEditor()
  mocks.persisted = { ...mocks.note, content: 'outside edit' }
  fireEvent.change(screen.getByRole('textbox'), { target: { value: 'Renamed' } })
  await advance()
  expect(mocks.update).toHaveBeenLastCalledWith('n1', { title: { before: 'Test note', after: 'Renamed' } })
  expect(mocks.persisted.content).toBe('outside edit')
  expect(mocks.editor?.markdown).toBe('original')

  mocks.update.mockRejectedValueOnce({ kind: 'conflict', field: 'content' })
  typeContent('my edit')
  await advance()
  expect(mocks.update).toHaveBeenLastCalledWith('n1', { content: { before: 'original', after: 'my edit' } })
  expect(screen.getByText('chatNotesConflict')).toBeInTheDocument()
  expect(mocks.editor?.markdown).toBe('my edit')
  expect(mocks.persisted.content).toBe('outside edit')
  window.confirm = () => true
  await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'chatNotesReload' })) })
  expect(mocks.editor?.markdown).toBe('outside edit')
})

it('restores a previously deleted note from Trash', async () => {
  mocks.trashList.mockResolvedValue([{ ...mocks.note, id: 'trash-id', preview: 'original' }])
  await act(async () => { render(<NotesCenter />) })
  await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'chatNotesTrash' })) })
  await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'chatNotesRestore' })) })
  expect(mocks.restore).toHaveBeenCalledWith('trash-id')
  expect(screen.getByText('chatNotesTrashEmpty')).toBeInTheDocument()
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
  expect(mocks.update).toHaveBeenCalledWith('n1', { content: { before: 'original', after: 'kept draft' } })
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
  expect(screen.queryByRole('alert')).toBeNull()
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
  await act(async () => {
    mocks.persisted = { ...mocks.note, content: 'updated content' }
    finish(mocks.persisted)
  })
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
