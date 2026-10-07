import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { LangContext } from '../../components/i18n'
import type { Conversation } from '../types'
import { StudyWorkspace, type StudyReadingSurface } from './StudyWorkspace'
import { createEmptyStudyPage, type StudyDocument } from './studyStorage'
const mocks = vi.hoisted(() => ({ open: vi.fn(), edit: vi.fn(), renderChat: vi.fn(), imageReady: true }))
vi.mock('./studyStorage', async original => ({ ...await original<typeof import('./studyStorage')>(), readStudyDocumentBlob: async () => new Blob(['image']) }))
vi.mock('./StudyReader', async () => {
  const { useEffect } = await import('react')
  return { StudyReader: ({ onContextChange, onAsk, page, region }: { onContextChange: (value: unknown) => void; onAsk: () => void; page: number; region: unknown }) => {
    useEffect(() => { onContextChange({ status: 'ready', page, pageCount: 2, region, imageDataUrl: mocks.imageReady ? 'data:image/png;base64,AA==' : undefined }) }, [onContextChange, page, region])
    return <><p>Original page fixture</p><button onClick={onAsk}>Ask about this page</button></>
  } }
})
vi.mock('./studyWorkspaceStore', async original => ({ ...await original<typeof import('./studyWorkspaceStore')>(), initializeStudy: vi.fn(), editStudyPage: mocks.edit }))
import { studyWorkspace } from './studyWorkspaceStore'
const makeDoc = (): StudyDocument => ({ id: 'a'.repeat(64), revision: 0, name: 'Paper.pdf', kind: 'pdf', pageCount: 2, createdAt: 1, updatedAt: 1, size: 10, lastPage: 1, pages: { 1: { ...createEmptyStudyPage(), question: 'Persistent composer draft' } } })
function setup(document = makeDoc(), conversation?: Conversation | null) {
  studyWorkspace.setState({ documents: [document], selectedDocumentId: document.id, loaded: true, importing: false, error: '', notice: '', dirtyIds: [], saveError: '' })
  const target = conversation === undefined ? { id: `conv_study_${document.id}_1`, revision: 0, title: 'Page 1', provider_id: 'p', model: 'vision', messages: [], created_at: 1, updated_at: 1, study_context: { materialId: document.id, page: 1 } } : conversation
  return render(<LangContext.Provider value="en"><StudyWorkspace conversation={target} busy={false} onOpenPage={mocks.open} renderChat={mocks.renderChat} /></LangContext.Provider>)
}
beforeEach(() => { vi.clearAllMocks(); mocks.open.mockResolvedValue(undefined); mocks.imageReady = true; mocks.renderChat.mockImplementation((surface: StudyReadingSurface) => <div aria-label="Shared Chat surface">{surface.controls}</div>); HTMLElement.prototype.scrollIntoView = vi.fn() })
function surface(): StudyReadingSurface { return mocks.renderChat.mock.calls.at(-1)![0] }

describe('Study material layout around the shared Chat surface', () => {
  it('opens the page through the supplied Chat owner and supplies original image/source context, not a second composer', async () => {
    const doc = makeDoc(); setup(doc)
    await screen.findByLabelText('Shared Chat surface')
    expect(mocks.open).toHaveBeenCalledWith(doc, 1)
    await waitFor(() => expect(surface().context?.imageDataUrl).toBe('data:image/png;base64,AA=='))
    expect(surface().source).toEqual({ page: 1, region: null, mode: 'read', attempt: '' })
    expect(screen.queryByLabelText('Question about this page')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Send' })).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Help mode' })).toHaveTextContent('Reading Q&A')
  })
  it('never renders another document’s conversation while loading the selected page', async () => {
    setup(makeDoc(), { id: 'other', revision: 0, title: 'other', provider_id: 'p', model: 'vision', messages: [], created_at: 1, updated_at: 1 })
    await screen.findByText('Original page fixture')
    expect(screen.getByText('Opening this page’s conversation…')).toBeInTheDocument()
    expect(mocks.renderChat).not.toHaveBeenCalled()
  })
  it('surfaces migration/open failures and retries without changing the material draft', async () => {
    mocks.open.mockRejectedValueOnce(new Error('Linked conversation unavailable'))
    setup()
    await screen.findByText('Error: Linked conversation unavailable')
    fireEvent.click(screen.getByRole('button', { name: 'Retry opening conversation' }))
    await screen.findByLabelText('Shared Chat surface')
    expect(mocks.open).toHaveBeenCalledTimes(2)
    expect(mocks.edit).not.toHaveBeenCalled()
  })
  it('passes Ask focus intent and restores source navigation without starting a reply', async () => {
    setup()
    await screen.findByLabelText('Shared Chat surface')
    fireEvent.click(screen.getByRole('button', { name: 'Ask about this page' }))
    expect(screen.getByRole('tab', { name: 'Help' })).toHaveAttribute('aria-selected', 'true')
    expect(surface().focusRequest).toBe(1)
    act(() => surface().showSource({ page: 1, region: null, mode: 'read', attempt: '' }))
    expect(screen.getByRole('tab', { name: 'Read' })).toHaveAttribute('aria-selected', 'true')
  })
  it('keeps source-specific context optional and opens it for checking with bounded shared inputs', async () => {
    setup()
    await screen.findByLabelText('Shared Chat surface')
    expect(screen.queryByRole('textbox', { name: 'Additional context' })).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Additional context' }))
    expect(screen.getByRole('textbox', { name: 'Additional context' })).toHaveStyle({ resize: 'none' })
    fireEvent.change(screen.getByRole('textbox', { name: 'Additional context' }), { target: { value: 'My reading goal' } })
    expect(mocks.edit).toHaveBeenCalledWith('a'.repeat(64), 1, { attempt: 'My reading goal' })
    fireEvent.click(screen.getByRole('button', { name: 'Help mode' }))
    fireEvent.click(screen.getByRole('option', { name: 'Check my attempt' }))
    expect(screen.getByRole('textbox', { name: 'My attempt' })).toHaveAttribute('aria-required', 'true')
    expect(surface().source.mode).toBe('check')
  })
  it('preserves legacy records in an explicit read-only archive, including withheld answers and manual text', async () => {
    const doc = makeDoc(); doc.pages['1'].correctedText = 'Old manual text'
    doc.pages['1'].history.push({ id: 'old', page: 1, mode: 'check', question: 'Old question', attempt: '', sourceText: 'Old extraction', answer: '{"withheldSolution":"OLD ANSWER"}', status: 'error', providerId: 'p', model: 'm', createdAt: 1 })
    setup(doc)
    await screen.findByLabelText('Shared Chat surface')
    const summary = screen.getByText('Legacy backup (read-only, may include full answers)')
    expect(summary.closest('details')).not.toHaveAttribute('open')
    fireEvent.click(summary)
    expect(screen.getByText(/OLD ANSWER/)).toBeVisible()
    expect(screen.getByText(/Old manual text/)).toBeVisible()
    expect(surface().source).not.toHaveProperty('sourceText')
  })
  it('restores collapsed materials and preserves visible save errors', async () => {
    const view = setup()
    await screen.findByLabelText('Shared Chat surface')
    expect(view.container.querySelector('.kv-study-layout')).toHaveClass('is-library-closed')
    fireEvent.click(screen.getByRole('button', { name: 'Expand library' }))
    expect(view.container.querySelector('.kv-study-layout')).not.toHaveClass('is-library-closed')
    act(() => studyWorkspace.setState(state => ({ ...state, saveError: 'Quota full', dirtyIds: ['a'.repeat(64)] })))
    expect(screen.getByRole('button', { name: 'Retry save' })).toBeInTheDocument()
    expect(screen.getByRole('alert')).toHaveTextContent('Quota full')
  })
})
