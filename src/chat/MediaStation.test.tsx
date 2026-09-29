import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { MediaStation } from './MediaStation'
import { mediaStationApi, type MediaJob } from '../api/mediaStation'
import { getSettingsCached } from '../api/settingsCache'
import type { Settings } from '../api/tauri'

vi.mock('../api/mediaStation', () => ({ mediaStationApi: { list: vi.fn(), start: vi.fn(), cancel: vi.fn(), read: vi.fn(), export: vi.fn(), reference: vi.fn() } }))
vi.mock('../api/settingsCache', () => ({ getSettingsCached: vi.fn() }))
vi.mock('../api/tauri', () => ({ isTauriRuntime: () => true }))
vi.mock('../components/i18n', () => ({ useLang: () => 'zh' }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }))

const job: MediaJob = { id: 'job', createdAt: 1, status: 'running', error: null, outputs: [], request: { kind: 'image', providerId: 'p', model: 'gpt-image-1', prompt: '晨光花瓶', aspectRatio: '1:1', duration: 5, referencePaths: [] } }
beforeEach(() => {
  vi.clearAllMocks()
  vi.mocked(mediaStationApi.list).mockResolvedValue([])
  vi.mocked(getSettingsCached).mockResolvedValue({ providers: [{ id: 'p', name: 'Provider', enabled: true, enabledModels: ['gpt-image-1'] }], defaultModels: { imageGeneration: { providerId: 'p', model: 'gpt-image-1' } } } as Settings)
})

it('submits once, observes the real task result, and keeps drafts across navigation', async () => {
  let resolve!: (job: MediaJob) => void
  vi.mocked(mediaStationApi.start).mockImplementation(() => new Promise((r) => { resolve = r }))
  const view = render(<MediaStation onOpenSettings={vi.fn()} />)
  await waitFor(() => expect(screen.getByLabelText('模型')).toHaveValue('gpt-image-1'))
  fireEvent.change(screen.getByLabelText('创作描述'), { target: { value: '晨光花瓶' } })
  const generate = screen.getByRole('button', { name: '开始生成' })
  fireEvent.click(generate); fireEvent.click(generate)
  expect(mediaStationApi.start).toHaveBeenCalledTimes(1)
  vi.mocked(mediaStationApi.list).mockResolvedValue([job])
  await act(async () => resolve(job))
  expect(await screen.findByText('正在生成，稍后回来也可以。')).toBeTruthy()
  view.unmount()
  render(<MediaStation onOpenSettings={vi.fn()} />)
  expect(screen.getByLabelText('创作描述')).toHaveValue('晨光花瓶')
  expect(await screen.findByRole('button', { name: /生成中 晨光花瓶/ })).toBeTruthy()
  expect(mediaStationApi.cancel).not.toHaveBeenCalled()
})

it('shows a failed submission and allows an explicit retry without losing the prompt', async () => {
  vi.mocked(mediaStationApi.start).mockRejectedValue(new Error('HTTP 429'))
  render(<MediaStation onOpenSettings={vi.fn()} />)
  await waitFor(() => expect(screen.getByLabelText('模型')).toHaveValue('gpt-image-1'))
  fireEvent.change(screen.getByLabelText('创作描述'), { target: { value: 'retry prompt' } })
  fireEvent.click(screen.getByRole('button', { name: '开始生成' }))
  expect(await screen.findByRole('alert')).toHaveTextContent('HTTP 429')
  expect(screen.getByLabelText('创作描述')).toHaveValue('retry prompt')
  expect(screen.getByRole('button', { name: '开始生成' })).toBeEnabled()
  expect(mediaStationApi.start).toHaveBeenCalledTimes(1)
})

it('stops waiting and presents the persisted terminal state', async () => {
  vi.mocked(mediaStationApi.list).mockResolvedValue([job])
  vi.mocked(mediaStationApi.cancel).mockImplementation(async () => {
    vi.mocked(mediaStationApi.list).mockResolvedValue([{ ...job, status: 'cancelled', error: '供应商可能继续计费' }])
  })
  render(<MediaStation onOpenSettings={vi.fn()} />)
  fireEvent.click(await screen.findByRole('button', { name: /生成中 晨光花瓶/ }))
  fireEvent.click(screen.getByRole('button', { name: '停止等待' }))
  expect(await screen.findByRole('status')).toHaveTextContent('供应商可能继续计费')
  expect(screen.queryByText('正在生成，稍后回来也可以。')).toBeNull()
})

it('reports history failures instead of silently presenting empty history', async () => {
  vi.mocked(mediaStationApi.list).mockRejectedValue(new Error('Cannot read media history'))
  render(<MediaStation onOpenSettings={vi.fn()} />)
  expect(await screen.findByRole('alert')).toHaveTextContent('Cannot read media history')
})


it('uses a saved image as the video first frame without submitting a paid task', async () => {
  vi.stubGlobal('URL', { createObjectURL: vi.fn(() => 'blob:preview'), revokeObjectURL: vi.fn() })
  vi.mocked(mediaStationApi.read).mockResolvedValue([1, 2, 3])
  vi.mocked(mediaStationApi.reference).mockResolvedValue('C:/media/image.png')
  vi.mocked(mediaStationApi.list).mockResolvedValue([{ ...job, status: 'completed', outputs: [{ name: 'image.png', mimeType: 'image/png', preview: '' }] }])
  const view = render(<MediaStation onOpenSettings={vi.fn()} />)
  fireEvent.click(await screen.findByRole('button', { name: /已完成 晨光花瓶/ }))
  fireEvent.click(await screen.findByRole('button', { name: '用作视频首帧' }))
  expect(await screen.findByText('image.png')).toBeTruthy()
  expect(screen.getByRole('button', { name: '生视频' })).toHaveAttribute('aria-pressed', 'true')
  expect(screen.getByLabelText('模型')).toHaveValue('')
  expect(mediaStationApi.start).not.toHaveBeenCalled()
  view.unmount()
  expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:preview')
  vi.unstubAllGlobals()
})
