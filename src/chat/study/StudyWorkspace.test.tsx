import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { LangContext } from '../../components/i18n'
import { StudyWorkspace } from './StudyWorkspace'
import { createEmptyStudyPage, type StudyDocument } from './studyStorage'

const mocks = vi.hoisted(() => ({ send: vi.fn(), edit: vi.fn(), open: vi.fn(), providers: true }))
vi.mock('../../api/settingsCache', () => ({
  subscribeSettings: () => () => {},
  getSettingsCached: async () => ({ providers: mocks.providers ? [{ id: 'test', name: 'Test provider', enabled: true, enabledModels: ['text', 'vision'], modelOverrides: { text: { capabilities: { vision: false } }, vision: { capabilities: { vision: true } } } }] : [], defaultModels: { chat: { providerId: 'test', model: 'vision' } } }),
}))
vi.mock('./studyStorage', async (importOriginal) => ({ ...await importOriginal<typeof import('./studyStorage')>(), readStudyDocumentBlob: async () => new Blob(['fake image']) }))
vi.mock('../ChatMarkdown', () => ({ ChatMarkdown: ({ content }: { content: string }) => <p>{content}</p> }))
vi.mock('./StudyReader', async () => {
  const { useEffect } = await import('react')
  return { StudyReader: ({ onContextChange, page, region }: { onContextChange: (value: unknown) => void; page: number; region: unknown }) => {
    useEffect(() => { onContextChange({ status: 'ready', page, pageCount: 1, region, text: 'x + 2 = 5', imageDataUrl: 'data:image/png;base64,AA==' }) }, [onContextChange, page, region])
    return <p>Readable page fixture</p>
  } }
})
vi.mock('./studyWorkspaceStore', async (importOriginal) => {
  const original = await importOriginal<typeof import('./studyWorkspaceStore')>()
  return { ...original, initializeStudy: vi.fn(), sendStudyHelp: mocks.send, editStudyPage: mocks.edit, openStudyPage: mocks.open }
})
import { studyWorkspace } from './studyWorkspaceStore'

const makeDoc = (): StudyDocument => ({ id: 'a'.repeat(64), name: 'Worksheet.pdf', kind: 'pdf', pageCount: 1, createdAt: 1, updatedAt: 1, size: 10, lastPage: 1, pages: { 1: { ...createEmptyStudyPage(), question: 'Why subtract two?' } } })
function setup(doc = makeDoc()) {
  studyWorkspace.setState({ documents: [doc], selectedDocumentId: doc.id, selectedTurnId: null, loaded: true, importing: false, error: '', notice: '', dirtyIds: [], saveError: '', activeRequest: null })
  return render(<LangContext.Provider value="en"><StudyWorkspace onOpenSettings={vi.fn()} /></LangContext.Provider>)
}

beforeEach(() => { vi.clearAllMocks(); mocks.providers = true; HTMLElement.prototype.scrollIntoView = vi.fn() })

describe('Study workspace interaction', () => {
  it('defaults to one hint, requires an attempt for checking, and uses explicit solution action', async () => {
    setup()
    expect(screen.getByRole('radio', { name: 'One hint' })).toHaveAttribute('aria-checked', 'true')
    await waitFor(() => expect(screen.getByRole('button', { name: 'Send' })).toBeEnabled())
    fireEvent.click(screen.getByRole('radio', { name: 'Check my attempt' }))
    expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled()
    fireEvent.click(screen.getByRole('radio', { name: 'Full solution' }))
    fireEvent.click(screen.getByRole('button', { name: 'Get full solution' }))
    await waitFor(() => expect(mocks.send).toHaveBeenCalledWith(expect.objectContaining({ mode: 'solution', documentId: 'a'.repeat(64), page: 1 })))
  })
  it('sends immutable image disclosure for retry even when current checkbox was unchecked', async () => {
    const doc = makeDoc()
    doc.pages['1'].history.push({ id: 'failed', page: 1, mode: 'hint', question: 'Original question', attempt: 'My attempt', sourceText: 'Original text', sourceImageUsed: true, answer: '', status: 'error', error: 'Provider failure', providerId: 'test', model: 'vision', createdAt: 1 })
    setup(doc)
    const imageCheckbox = await screen.findByLabelText('Include page / region image')
    fireEvent.click(imageCheckbox)
    expect(imageCheckbox).not.toBeChecked()
    fireEvent.click(screen.getByRole('button', { name: 'Retry this question' }))
    expect(imageCheckbox).toBeChecked()
    expect(imageCheckbox).toBeDisabled()
    expect(screen.getByText(/Sending shares.*page text and image.*Test provider/)).toBeInTheDocument()
  })
  it('preserves page-bound edits and keyboard pane navigation', async () => {
    setup()
    fireEvent.change(screen.getByLabelText('Question about this page'), { target: { value: 'New draft' } })
    expect(mocks.edit).toHaveBeenCalledWith('a'.repeat(64), 1, { question: 'New draft' })
    fireEvent.keyDown(screen.getByRole('tab', { name: 'Read' }), { key: 'ArrowRight' })
    expect(screen.getByRole('tab', { name: 'Help' })).toHaveAttribute('aria-selected', 'true')
  })
  it('does not invent a model when none is configured and keeps reading available', async () => {
    mocks.providers = false
    setup()
    await screen.findByText('Readable page fixture')
    expect(screen.getByText(/Configure a model to ask for help/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled()
    expect(mocks.send).not.toHaveBeenCalled()
  })
  it('shows save failures with a retry rather than a saved claim', async () => {
    setup()
    studyWorkspace.setState((state) => ({ ...state, saveError: 'Quota full', dirtyIds: ['a'.repeat(64)] }))
    await screen.findByRole('button', { name: 'Retry save' })
    expect(screen.queryByText('Saved on this device')).not.toBeInTheDocument()
  })
})
