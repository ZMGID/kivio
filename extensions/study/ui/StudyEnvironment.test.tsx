import { act, cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { render } from '../../../src/test/renderWithNotices'
import StudyEnvironment from './StudyEnvironment'
import { readingSetupApi, type StudySetupStatus } from '../api/readingSetup'

vi.mock('../api/readingSetup', () => ({ readingSetupApi: { status: vi.fn(), save: vi.fn(), start: vi.fn() } }))
vi.mock('../../../src/chat/KivioBlob', () => ({ KivioBlob: () => <span>墨墨</span> }))
let disk: StudySetupStatus
beforeEach(() => {
  vi.clearAllMocks()
  disk = { acknowledged: false, pdf2zh: true, installed: false, version: '2.9.0', pandoc: false, pandocInstalled: false, pandocVersion: '3.12.1', pandocDownloadBytes: 42313957, progress: null }
  vi.mocked(readingSetupApi.status).mockImplementation(async () => structuredClone(disk))
  vi.mocked(readingSetupApi.save).mockImplementation(async (selection, acknowledge) => {
    disk = { ...disk, ...selection, acknowledged: acknowledge || disk.acknowledged, progress: null }
    return structuredClone(disk)
  })
  Object.defineProperty(HTMLDialogElement.prototype, 'showModal', { configurable: true, value(this: HTMLDialogElement) { this.open = true } })
})
afterEach(() => { cleanup(); vi.restoreAllMocks() })
function Surface() { return <StudyEnvironment>{open => <button onClick={open}>管理 Study 组件</button>}</StudyEnvironment> }

it('persists skipping PDF2zh without installing, restores the library and can reopen component management', async () => {
  const view = render(<Surface />)
  fireEvent.click(await screen.findByRole('button', { name: '配置选项' }))
  fireEvent.click(screen.getByRole('switch', { name: '安装 PDF2zh 论文翻译' }))
  fireEvent.click(screen.getByRole('button', { name: '确定' }))
  fireEvent.click(await screen.findByRole('button', { name: '进入 Kivio Study' }))
  await screen.findByRole('button', { name: '管理 Study 组件' })
  expect(readingSetupApi.start).not.toHaveBeenCalled()
  view.unmount()
  render(<Surface />)
  fireEvent.click(await screen.findByRole('button', { name: '管理 Study 组件' }))
  expect(screen.getByRole('switch', { name: '安装 PDF2zh 论文翻译' })).toHaveAttribute('aria-checked', 'false')
})

it('keeps an installation failure recoverable and only enters after verified readiness', async () => {
  disk.progress = { component: 'pdf2zh', steps: 5, phase: 'error', step: 3, downloaded: 0, total: null, error: '下载失败，请重试' }
  vi.mocked(readingSetupApi.start).mockImplementation(async () => {
    disk = { ...disk, installed: true, progress: { component: 'pdf2zh', steps: 5, phase: 'ready', step: 5, downloaded: 0, total: null, error: null } }
    return structuredClone(disk)
  })
  render(<Surface />)
  fireEvent.click(await screen.findByRole('button', { name: '重试配置' }))
  expect(screen.queryByRole('button', { name: '管理 Study 组件' })).not.toBeInTheDocument()
  fireEvent.click(await screen.findByRole('button', { name: '进入 Kivio Study' }))
  await screen.findByRole('button', { name: '管理 Study 组件' })
  expect(disk.acknowledged).toBe(true)
})

it('recovers a running background job after remount and blocks changing its dependencies', async () => {
  disk.progress = { component: 'pdf2zh', steps: 5, phase: 'assets', step: 4, downloaded: 0, total: null, error: null }
  const view = render(<Surface />)
  await waitFor(() => expect(screen.getByRole('button', { name: '正在配置…' })).toBeDisabled())
  expect(screen.getByRole('button', { name: '配置选项' })).toBeDisabled()
  view.unmount()
  render(<Surface />)
  await waitFor(() => expect(screen.getByRole('button', { name: '正在配置…' })).toBeDisabled())
  await act(async () => {
    disk = { ...disk, installed: true, progress: { component: 'pdf2zh', steps: 5, phase: 'ready', step: 5, downloaded: 0, total: null, error: null } }
  })
  await screen.findByRole('button', { name: '进入 Kivio Study' }, { timeout: 2500 })
  expect(readingSetupApi.start).not.toHaveBeenCalled()
})


it('does not let a late status response reopen onboarding after the user enters Study', async () => {
  const running = { component: 'pdf2zh' as const, steps: 5, phase: 'verify' as const, step: 5, downloaded: 0, total: null, error: null }
  const ready = { ...disk, installed: true, progress: null }
  let finish!: (status: StudySetupStatus) => void
  vi.mocked(readingSetupApi.status)
    .mockResolvedValueOnce({ ...disk, progress: running })
    .mockResolvedValueOnce(ready)
    .mockImplementationOnce(() => new Promise(resolve => { finish = resolve }))
  render(<Surface />)
  await waitFor(() => expect(finish).toBeTypeOf('function'))
  fireEvent.click(screen.getByRole('button', { name: '进入 Kivio Study' }))
  await screen.findByRole('button', { name: '管理 Study 组件' })
  await act(async () => { finish(ready) })
  expect(screen.getByRole('button', { name: '管理 Study 组件' })).toBeInTheDocument()
})


it('installs Pandoc independently and waits for all selected components before entering Study', async () => {
  disk = { ...disk, acknowledged: true, installed: true }
  vi.mocked(readingSetupApi.start).mockImplementation(async () => {
    disk = { ...disk, progress: { component: 'pandoc', steps: 3, phase: 'download', step: 1, downloaded: 1024, total: 42313957, error: null } }
    return structuredClone(disk)
  })
  render(<Surface />)
  fireEvent.click(await screen.findByRole('button', { name: '管理 Study 组件' }))
  expect(screen.getByRole('switch', { name: '安装 Pandoc 文档转换' })).toHaveAttribute('aria-checked', 'false')
  fireEvent.click(screen.getByRole('switch', { name: '安装 Pandoc 文档转换' }))
  fireEvent.click(screen.getByRole('button', { name: '确定' }))
  fireEvent.click(await screen.findByRole('button', { name: '开始配置' }))
  await screen.findByText('正在下载 Pandoc')
  expect(screen.queryByRole('button', { name: '进入 Kivio Study' })).not.toBeInTheDocument()
  expect(screen.getByRole('button', { name: '配置选项' })).toBeDisabled()
  await act(async () => {
    disk = { ...disk, pandocInstalled: true, progress: { component: 'pandoc', steps: 3, phase: 'ready', step: 3, downloaded: 0, total: null, error: null } }
  })
  fireEvent.click(await screen.findByRole('button', { name: '进入 Kivio Study' }, { timeout: 2500 }))
  fireEvent.click(await screen.findByRole('button', { name: '管理 Study 组件' }))
  expect(screen.getByRole('switch', { name: '安装 PDF2zh 论文翻译' })).toHaveAttribute('aria-checked', 'true')
  expect(screen.getByRole('switch', { name: '安装 Pandoc 文档转换' })).toHaveAttribute('aria-checked', 'true')
})
