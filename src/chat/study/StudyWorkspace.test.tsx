import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
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
  return { StudyReader: ({ onContextChange, onAsk, page, region }: { onContextChange: (value: unknown) => void; onAsk?: () => void; page: number; region: unknown }) => {
    useEffect(() => { onContextChange({ status: 'ready', page, pageCount: 1, region, imageDataUrl: mocks.imageReady ? 'data:image/png;base64,AA==' : undefined }) }, [onContextChange, page, region])
    return <><p>Readable page fixture</p><button onClick={onAsk}>Ask about this page</button></>
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

function chooseMode(name: string) { fireEvent.click(screen.getByRole('button', { name: 'Help mode' })); fireEvent.click(screen.getByRole('option', { name })) }

describe('Study workspace image-only interaction', () => {
  it('defaults to reading without an attempt, selects vision, and keeps math choices available', async () => {
    setup()
    expect(screen.getByRole('button', { name: 'Help mode' })).toHaveTextContent('Reading Q&A')
    expect(screen.queryByLabelText('My attempt')).not.toBeInTheDocument()
    expect(screen.queryByRole('option')).not.toBeInTheDocument()
    await waitFor(() => expect(screen.getByRole('button', { name: 'Send' })).toBeEnabled())
    expect(screen.queryByLabelText('Include page / region image')).not.toBeInTheDocument()
    expect(screen.queryByLabelText('Corrected problem text')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Send' }))
    await waitFor(() => expect(mocks.send).toHaveBeenCalledWith(expect.objectContaining({ mode: 'read', model: 'vision', visionCapable: true, context: expect.objectContaining({ imageDataUrl: 'data:image/png;base64,AA==' }) })))
    chooseMode('Check my attempt')
    expect(screen.getByLabelText('My attempt')).toHaveAttribute('aria-required', 'true')
    expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled()
    chooseMode('Full solution')
    fireEvent.click(screen.getByRole('button', { name: 'Get full solution' }))
    await waitFor(() => expect(mocks.send).toHaveBeenCalledWith(expect.objectContaining({ mode: 'solution', documentId: 'a'.repeat(64), page: 1 })))
  })
  it('keeps reading context optional and preserves it when switching to checking work', async () => {
    setup()
    await waitFor(() => expect(screen.getByRole('button', { name: 'Send' })).toBeEnabled())
    expect(screen.getByLabelText('Question about this page')).toHaveAttribute('placeholder', 'What would you like to understand, explain or translate on this page?')
    fireEvent.click(screen.getByRole('button', { name: 'Add context' }))
    expect(screen.getByLabelText('Additional context')).toHaveAttribute('aria-required', 'false')
    fireEvent.change(screen.getByLabelText('Additional context'), { target: { value: 'Explain this figure for a new reader' } })
    expect(mocks.edit).toHaveBeenCalledWith('a'.repeat(64), 1, { attempt: 'Explain this figure for a new reader' })
    chooseMode('Check my attempt')
    expect(screen.getByLabelText('My attempt')).toHaveAttribute('aria-required', 'true')
    chooseMode('Reading Q&A')
    expect(screen.getByLabelText('Additional context')).toHaveAttribute('aria-required', 'false')
    expect(screen.getByRole('button', { name: 'Send' })).toBeEnabled()
  })
  it('labels saved reading context using that turn even when the current composer is a math mode', async () => {
    const doc = makeDoc()
    doc.pages['1'].history.push({ id: 'reading', page: 1, mode: 'read', question: 'Explain the legend', attempt: 'I am new to this topic', sourceText: '', sourceImageUsed: true, answer: 'Start with the axes.', status: 'complete', providerId: 'test', model: 'vision', createdAt: 1 })
    const view = setup(doc)
    chooseMode('One hint')
    expect(view.container.querySelector('.kv-study-question summary')).toHaveTextContent('Additional context')
    expect(screen.getByRole('article', { name: 'Reading Q&A: Explain the legend' })).toBeInTheDocument()
  })
  it('puts model identity first in the compact model menu while retaining the service name', async () => {
    setup()
    const trigger = await screen.findByRole('button', { name: 'Study model' })
    await waitFor(() => expect(trigger).toBeEnabled())
    fireEvent.click(trigger)
    const option = screen.getByRole('option', { name: 'vision · Test provider' })
    expect(option).toHaveAttribute('title', 'Test provider · vision')
    expect(screen.queryByRole('button', { name: 'Model provider' })).not.toBeInTheDocument()
    fireEvent.click(option)
    expect(trigger).toHaveFocus()
  })
  it('uses an image for legacy text-only retries and discloses it without an opt-out fallback', async () => {
    const doc = makeDoc()
    doc.pages['1'].history.push({ id: 'failed', page: 1, mode: 'hint', question: 'Original question', attempt: 'My attempt', sourceText: 'Old extracted text', sourceImageUsed: false, answer: '', status: 'error', error: 'Provider failure', providerId: 'test', model: 'text', createdAt: 1 })
    setup(doc)
    fireEvent.click(await screen.findByRole('button', { name: 'Retry this question' }))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Send retry' })).toBeEnabled())
    expect(screen.getByText(/discussion and image go to Test provider/)).toBeInTheDocument()
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
  it('keeps user question, optional context and notes inputs bounded with no native resize', async () => {
    setup()
    await screen.findByText('Readable page fixture')
    fireEvent.click(screen.getByRole('button', { name: 'Add context' }))
    for (const label of ['Question about this page', 'Additional context', 'Page notes']) expect(screen.getByLabelText(label)).toHaveStyle({ resize: 'none' })
  })
  it('keeps attempts saved while collapsed and opens/focuses them when checking', async () => {
    const doc = makeDoc(); doc.pages['1'].attempt = 'Keep this draft'
    setup(doc)
    expect(screen.queryByLabelText('My attempt')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'View context' }))
    expect(screen.getByLabelText('Additional context')).toHaveValue('Keep this draft')
    fireEvent.click(screen.getByRole('button', { name: 'Hide context' }))
    expect(screen.queryByLabelText('Additional context')).not.toBeInTheDocument()
    expect(mocks.edit).not.toHaveBeenCalled()
    chooseMode('Check my attempt')
    await waitFor(() => expect(screen.getByLabelText('My attempt')).toHaveFocus())
    expect(screen.getByLabelText('My attempt')).toHaveValue('Keep this draft')
  })
  it('starts a single material with its library collapsed and restores it without navigating', async () => {
    const view = setup()
    expect(view.container.querySelector('.kv-study-layout')).toHaveClass('is-library-closed')
    fireEvent.click(screen.getByRole('button', { name: 'Expand library' }))
    expect(view.container.querySelector('.kv-study-layout')).not.toHaveClass('is-library-closed')
    fireEvent.click(screen.getByRole('button', { name: 'Collapse library' }))
    expect(view.container.querySelector('.kv-study-layout')).toHaveClass('is-library-closed')
    expect(mocks.open).not.toHaveBeenCalled()
    expect(screen.getByLabelText('Question about this page')).toHaveValue('Why subtract two?')
  })
  it('moves from reading to the focused question without sending and keeps a visible source link', async () => {
    setup()
    fireEvent.click(await screen.findByRole('button', { name: 'Ask about this page' }))
    expect(screen.getByRole('tab', { name: 'Help' })).toHaveAttribute('aria-selected', 'true')
    await waitFor(() => expect(screen.getByLabelText('Question about this page')).toHaveFocus())
    expect(mocks.send).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: 'View page 1' }))
    expect(screen.getByRole('tab', { name: 'Read' })).toHaveAttribute('aria-selected', 'true')
  })
  it('sends via Ctrl or Meta Enter without hijacking composition, repeats, or invalid checks', async () => {
    setup()
    await waitFor(() => expect(screen.getByRole('button', { name: 'Send' })).toBeEnabled())
    const question = screen.getByLabelText('Question about this page')
    fireEvent.keyDown(question, { key: 'Enter', ctrlKey: true, isComposing: true })
    fireEvent.keyDown(question, { key: 'Enter', ctrlKey: true, repeat: true })
    expect(mocks.send).not.toHaveBeenCalled()
    fireEvent.keyDown(question, { key: 'Enter', ctrlKey: true })
    expect(mocks.send).toHaveBeenCalledTimes(1)
    chooseMode('Check my attempt')
    fireEvent.keyDown(question, { key: 'Enter', metaKey: true })
    expect(mocks.send).toHaveBeenCalledTimes(1)
    chooseMode('One hint')
    act(() => studyWorkspace.setState(state => ({ ...state, activeRequest: { documentId: 'a'.repeat(64), page: 1, turnId: 'active' } })))
    fireEvent.keyDown(question, { key: 'Enter', metaKey: true })
    expect(mocks.send).toHaveBeenCalledTimes(1)
  })
  it('shows save failures with a retry rather than a saved claim', async () => {
    setup()
    studyWorkspace.setState((state) => ({ ...state, saveError: 'Quota full', dirtyIds: ['a'.repeat(64)] }))
    await screen.findByRole('button', { name: 'Retry save' })
    expect(screen.queryByText('Saved on this device')).not.toBeInTheDocument()
  })
})
