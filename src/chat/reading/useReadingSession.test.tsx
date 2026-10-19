import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { useReadingSession } from './useReadingSession'
import type { ReadingSession } from '../../api/readingOperations'
const api = vi.hoisted(() => ({ status: vi.fn(), translate: vi.fn(), selectView: vi.fn(), cancel: vi.fn() }))
vi.mock('../../api/readingOperations', () => ({ readingOperationsApi: api }))
const idle: ReadingSession = { translation: null, view: { kind: 'source', output: null, latest: null } }
const running: ReadingSession = { ...idle, translation: { runId: 'run', phase: 'running', percent: 24, stage: '', model: 'model', targetLang: 'zh', error: null } }
function Reader() {
  const reading = useReadingSession('paper')
  return <><span data-testid="phase">{reading.session?.translation?.phase || 'idle'}</span><span data-testid="version">{reading.session?.view.kind}</span>
    <button onClick={() => void reading.translate('provider', 'model', 'en', 'zh', null)}>翻译</button>
    <button onClick={() => void reading.selectView('source')}>原文</button>
    <button onClick={() => void reading.cancel()}>停止</button></>
}
afterEach(() => { cleanup(); vi.clearAllMocks() })
it('isolates old polls from a newer task and restores the running owner after reopening', async () => {
  let old!: (value: ReadingSession) => void
  api.status.mockImplementationOnce(() => new Promise<ReadingSession>(resolve => { old = resolve })).mockResolvedValue(running)
  api.translate.mockResolvedValue(running)
  const view = render(<Reader />)
  fireEvent.click(screen.getByText('翻译'))
  await waitFor(() => expect(screen.getByTestId('phase')).toHaveTextContent('running'))
  await act(async () => old(idle))
  expect(screen.getByTestId('phase')).toHaveTextContent('running')
  view.unmount()
  render(<Reader />)
  await waitFor(() => expect(screen.getByTestId('phase')).toHaveTextContent('running'))
  expect(api.translate).toHaveBeenCalledTimes(1)
  api.cancel.mockImplementation(async () => api.status.mockResolvedValue({ ...running, translation: { ...running.translation!, phase: 'cancelled' } }))
  fireEvent.click(screen.getByText('停止'))
  await waitFor(() => expect(screen.getByTestId('phase')).toHaveTextContent('cancelled'), { timeout: 2000 })
})
it('does not let a delayed status response undo a newer explicit version selection', async () => {
  let old!: (value: ReadingSession) => void
  api.status.mockImplementationOnce(() => new Promise<ReadingSession>(resolve => { old = resolve }))
  api.selectView.mockResolvedValue(idle)
  render(<Reader />)
  fireEvent.click(screen.getByText('原文'))
  await waitFor(() => expect(screen.getByTestId('version')).toHaveTextContent('source'))
  const output = { runId: 'run', targetLang: 'zh', page: null }
  await act(async () => old({ ...idle, view: { kind: 'mono', output, latest: output } }))
  expect(screen.getByTestId('version')).toHaveTextContent('source')
})
