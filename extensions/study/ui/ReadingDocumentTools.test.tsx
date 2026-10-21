import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { render } from '../../../src/test/renderWithNotices'
import ReadingDocumentTools from './ReadingDocumentTools'
import type { ReadingDocument } from '../api/reading'
import type { ReadingSession } from '../api/readingOperations'

vi.mock('../api/readingSetup', () => ({ readingSetupApi: { status: async () => ({ installed: true, pandocInstalled: true }) } }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: vi.fn() }))
const empty: ReadingSession = { translation: null, view: { kind: 'source', output: null, latest: null } }
const props = {
  doc: { id: 'paper', name: 'Paper', notes: [] } as unknown as ReadingDocument,
  page: 2, ready: true, noteDraft: '', model: { providerId: 'provider', model: 'test-model' }, modelControls: <span>test-model</span>,
  onConfigure: vi.fn(), onNotes: vi.fn(), onHistory: vi.fn(), onPageImage: vi.fn(), capturing: false, side: null,
  session: empty, onTranslate: vi.fn().mockResolvedValue(undefined), onCancel: vi.fn().mockResolvedValue(undefined), onRead: vi.fn(),
}
const running: ReadingSession = { ...empty, translation: { runId: 'run', phase: 'running', percent: 24, stage: 'Translate Paragraphs', model: 'test-model', targetLang: 'zh', error: null } }
const output = { runId: 'run', targetLang: 'zh', page: null }
const completed: ReadingSession = { translation: { ...running.translation!, phase: 'ready', percent: 100 }, view: { kind: 'mono', output, latest: output } }
afterEach(() => { cleanup(); vi.clearAllMocks() })

it('defaults to translating the whole document, without requiring an export to read it', async () => {
  render(<ReadingDocumentTools {...props} />)
  fireEvent.click(screen.getByRole('button', { name: '论文翻译' }))
  fireEvent.click(await screen.findByRole('button', { name: '开始翻译' }))
  await waitFor(() => expect(props.onTranslate).toHaveBeenCalledWith('provider', 'test-model', 'en', 'zh', null))
})

it('dismisses settings on completion and provides in-reader actions for an existing result', async () => {
  const view = render(<ReadingDocumentTools {...props} session={running} />)
  fireEvent.click(screen.getByRole('button', { name: '论文翻译' }))
  expect(screen.getByRole('progressbar', { name: '论文翻译进度' })).toHaveAttribute('value', '24')
  view.rerender(<ReadingDocumentTools {...props} session={completed} />)
  expect(screen.queryByLabelText('论文翻译设置')).not.toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: '论文翻译' }))
  fireEvent.click(screen.getByRole('button', { name: '双语对照' }))
  expect(props.onRead).toHaveBeenCalledWith('dual')
  expect(screen.queryByRole('button', { name: '保存双语 PDF' })).not.toBeInTheDocument()
})


it('offers the original alongside translated versions without opening translation settings', () => {
  render(<ReadingDocumentTools {...props} session={completed} />)
  fireEvent.click(screen.getByRole('button', { name: '论文翻译' }))
  expect(screen.queryByLabelText('原文语言')).not.toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: '阅读原文' }))
  expect(props.onRead).toHaveBeenCalledWith('source')
})
