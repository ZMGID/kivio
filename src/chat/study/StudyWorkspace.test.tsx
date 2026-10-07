import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { LangContext } from '../../components/i18n'
import { StudyWorkspace } from './StudyWorkspace'
import { createEmptyStudyPage, type StudyDocument } from './studyStorage'

const mocks = vi.hoisted(() => ({ send: vi.fn(), edit: vi.fn(), open: vi.fn(), providers: true, vision: true, imageReady: true }))
vi.mock('../../api/settingsCache', () => ({
  subscribeSettings: () => () => {},
  getSettingsCached: async () => ({ providers: mocks.providers ? [{ id: 'test', name: 'Test provider', enabled: true, enabledModels: mocks.vision ? ['text', 'vision'] : ['text'], modelOverrides: { text: { capabilities: { vision: false } }, vision: { capabilities: { vision: true } } } }] : [], defaultModels: { chat: { providerId: 'test', model: 'text' } } }),
}))
vi.mock('./studyStorage', async (importOriginal) => ({ ...await importOriginal<typeof import('./studyStorage')>(), readStudyDocumentBlob: async () => new Blob(['fake image']) }))
vi.mock('../ChatMarkdown', () => ({ ChatMarkdown: ({ content }: { content: string }) => <p>{content}</p> }))
vi.mock('./StudyReader', async () => {
  const { useEffect } = await import('react')
  return { StudyReader: ({ onContextChange, page, region }: { onContextChange: (value: unknown) => void; page: number; region: unknown }) => {
    useEffect(() => { onContextChange({ status: 'ready', page, pageCount: 1, region, imageDataUrl: mocks.imageReady ? 'data:image/png;base64,AA==' : undefined }) }, [onContextChange, page, region])
    return <p>Readable page fixture</p>
  } }
})
vi.mock('./studyWorkspaceStore', async (importOriginal) => {
  const original = await importOriginal<typeof import('./studyWorkspaceStore')>()
  return { ...original, initializeStudy: vi.fn(), sendStudyHelp: mocks.send, editStudyPage: mocks.edit, openStudyPage: mocks.open }
})
import { studyWorkspace } from './studyWorkspaceStore'
const makeDoc = (): StudyDocument => ({ id: 'a'.repeat(64), revision: 0, name: 'Worksheet.pdf', kind: 'pdf', pageCount: 1, createdAt: 1, updatedAt: 1, size: 10, lastPage: 1, pages: { 1: { ...createEmptyStudyPage(), question: 'Why subtract two?' } } })
function setup(doc = makeDoc()) {
  studyWorkspace.setState({ documents: [doc], selectedDocumentId: doc.id, selectedTurnId: null, loaded: true, importing: false, error: '', notice: '', dirtyIds: [], saveError: '', activeRequest: null })
  return render(<LangContext.Provider value="en"><StudyWorkspace onOpenSettings={vi.fn()} /></LangContext.Provider>)
}
beforeEach(() => { vi.clearAllMocks(); mocks.providers = true; mocks.vision = true; mocks.imageReady = true; HTMLElement.prototype.scrollIntoView = vi.fn() })

describe('Study workspace image-only interaction', () => {
  it('defaults to one hint, selects an eligible vision model, requires an attempt for checking, and offers explicit solution', async () => {
    setup()
    expect(screen.getByRole('radio', { name: 'One hint' })).toHaveAttribute('aria-checked', 'true')
    await waitFor(() => expect(screen.getByRole('button', { name: 'Send' })).toBeEnabled())
    expect(screen.queryByLabelText('Include page / region image')).not.toBeInTheDocument()
    expect(screen.queryByLabelText('Corrected problem text')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Send' }))
    await waitFor(() => expect(mocks.send).toHaveBeenCalledWith(expect.objectContaining({ mode: 'hint', model: 'vision', visionCapable: true, context: expect.objectContaining({ imageDataUrl: 'data:image/png;base64,AA==' }) })))
    fireEvent.click(screen.getByRole('radio', { name: 'Check my attempt' }))
    expect(screen.getByLabelText('My attempt')).toHaveAttribute('aria-required', 'true')
    expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled()
    fireEvent.click(screen.getByRole('radio', { name: 'Full solution' }))
    fireEvent.click(screen.getByRole('button', { name: 'Get full solution' }))
    await waitFor(() => expect(mocks.send).toHaveBeenCalledWith(expect.objectContaining({ mode: 'solution', documentId: 'a'.repeat(64), page: 1 })))
  })
  it('uses an image for legacy text-only retries and discloses it without an opt-out fallback', async () => {
    const doc = makeDoc()
    doc.pages['1'].history.push({ id: 'failed', page: 1, mode: 'hint', question: 'Original question', attempt: 'My attempt', sourceText: 'Old extracted text', sourceImageUsed: false, answer: '', status: 'error', error: 'Provider failure', providerId: 'test', model: 'text', createdAt: 1 })
    setup(doc)
    fireEvent.click(await screen.findByRole('button', { name: 'Retry this question' }))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Send retry' })).toBeEnabled())
    expect(screen.getByText(/Sending shares.*page \/ region image.*Test provider/)).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Send retry' }))
    await waitFor(() => expect(mocks.send).toHaveBeenCalledWith(expect.objectContaining({ visionCapable: true, model: 'vision', retry: expect.objectContaining({ id: 'failed' }) })))
  })
  it('preserves page-bound edits and keyboard pane navigation', async () => {
    setup()
    fireEvent.change(screen.getByLabelText('Question about this page'), { target: { value: 'New draft' } })
    expect(mocks.edit).toHaveBeenCalledWith('a'.repeat(64), 1, { question: 'New draft' })
    fireEvent.keyDown(screen.getByRole('tab', { name: 'Read' }), { key: 'ArrowRight' })
    expect(screen.getByRole('tab', { name: 'Help' })).toHaveAttribute('aria-selected', 'true')
  })
  it.each([false, true])('does not silently use a text-only or absent provider (providers=%s)', providers => {
    mocks.providers = providers; mocks.vision = false
    setup()
    return waitFor(() => {
      expect(screen.getByText(/Choose an image-capable model in Settings/)).toBeInTheDocument()
      expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled()
      expect(mocks.send).not.toHaveBeenCalled()
    })
  })
  it('requires a rendered image even when a legacy manual source draft exists', async () => {
    mocks.imageReady = false
    const doc = makeDoc(); doc.pages['1'].correctedText = 'My old manually added formula'
    setup(doc)
    await screen.findByText('The page image is not ready. Wait for rendering or select a smaller area.')
    expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled()
    expect(screen.getByText('Earlier manually added text (not sent)')).toBeInTheDocument()
    expect(screen.getByText('My old manually added formula')).toBeInTheDocument()
    expect(screen.queryByLabelText('Corrected problem text')).not.toBeInTheDocument()
  })
  it('keeps user question, attempt and notes inputs bounded with no native resize', async () => {
    setup()
    await screen.findByText('Readable page fixture')
    for (const label of ['Question about this page', 'My attempt', 'Page notes']) expect(screen.getByLabelText(label)).toHaveStyle({ resize: 'none' })
  })
  it('shows save failures with a retry rather than a saved claim', async () => {
    setup()
    studyWorkspace.setState((state) => ({ ...state, saveError: 'Quota full', dirtyIds: ['a'.repeat(64)] }))
    await screen.findByRole('button', { name: 'Retry save' })
    expect(screen.queryByText('Saved on this device')).not.toBeInTheDocument()
  })
})
