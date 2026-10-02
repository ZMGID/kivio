import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { MediaStation } from './MediaStation'
import { mediaStationApi, type MediaJob } from '../api/mediaStation'
import { getSettingsCached } from '../api/settingsCache'
import type { Settings } from '../api/tauri'

vi.mock('../api/mediaStation', () => ({ mediaStationApi: { list: vi.fn(), start: vi.fn(), cancel: vi.fn(), resume: vi.fn(), read: vi.fn(), export: vi.fn(), reference: vi.fn() } }))
vi.mock('../api/settingsCache', () => ({ getSettingsCached: vi.fn() }))
vi.mock('../api/tauri', () => ({ isTauriRuntime: () => true }))
vi.mock('../components/i18n', () => ({ useLang: () => 'zh' }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }))

const openHistory = async () => fireEvent.click(await screen.findByRole('button', { name: /创作记录/ }))

const job: MediaJob = { id: 'job', createdAt: 1, status: 'running', error: null, outputs: [], request: { kind: 'image', providerId: 'p', model: 'gpt-image-1', prompt: '晨光花瓶', aspectRatio: '1:1', duration: 5, referencePaths: [] } }
beforeEach(() => {
  vi.clearAllMocks()
  HTMLDialogElement.prototype.showModal = function () { this.setAttribute('open', '') }
  HTMLDialogElement.prototype.close = function () { this.removeAttribute('open') }
  vi.mocked(mediaStationApi.list).mockResolvedValue([])
  vi.mocked(getSettingsCached).mockResolvedValue({ providers: [{ id: 'p', name: 'Provider', enabled: true, enabledModels: ['gpt-image-1'] }], defaultModels: { imageGeneration: { providerId: 'p', model: 'gpt-image-1' } } } as Settings)
})

it('submits once, observes the real task result, and keeps drafts across navigation', async () => {
  let resolve!: (job: MediaJob) => void
  vi.mocked(mediaStationApi.start).mockImplementation(() => new Promise((r) => { resolve = r }))
  const view = render(<MediaStation onOpenSettings={vi.fn()} />)
  await waitFor(() => expect(screen.getByLabelText('模型')).toHaveTextContent('gpt-image-1'))
  fireEvent.change(screen.getByLabelText('创作描述'), { target: { value: '晨光花瓶' } })
  const generate = screen.getByRole('button', { name: '开始生成' })
  fireEvent.click(generate); fireEvent.click(generate)
  expect(mediaStationApi.start).toHaveBeenCalledTimes(1)
  vi.mocked(mediaStationApi.list).mockResolvedValue([job])
  await act(async () => resolve(job))
  expect(await screen.findByText('正在生成，稍后回来也可以。')).toBeTruthy()
  expect(screen.queryByRole('dialog')).toBeNull()
  view.unmount()
  render(<MediaStation onOpenSettings={vi.fn()} />)
  expect(screen.getByLabelText('创作描述')).toHaveValue('晨光花瓶')
  expect(await screen.findByRole('article', { name: '本次生成' })).toBeTruthy()
  await openHistory()
  expect(await screen.findByRole('button', { name: /生成中 晨光花瓶/ })).toBeTruthy()
  expect(mediaStationApi.cancel).not.toHaveBeenCalled()
})

it('shows a failed submission and allows an explicit retry without losing the prompt', async () => {
  vi.mocked(mediaStationApi.start).mockRejectedValue(new Error('HTTP 429'))
  render(<MediaStation onOpenSettings={vi.fn()} />)
  await waitFor(() => expect(screen.getByLabelText('模型')).toHaveTextContent('gpt-image-1'))
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
  await openHistory()
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
  await openHistory()
  fireEvent.click(await screen.findByRole('button', { name: /晨光花瓶/ }))
  fireEvent.click(await screen.findByRole('button', { name: '用作视频首帧' }))
  expect(await screen.findByText('image.png')).toBeTruthy()
  expect(screen.getByRole('button', { name: '生视频' })).toHaveAttribute('aria-pressed', 'true')
  expect(screen.getByLabelText('模型')).toHaveTextContent('')
  expect(screen.getByLabelText('模型')).toBeDisabled()
  expect(mediaStationApi.start).not.toHaveBeenCalled()
  view.unmount()
  expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:preview')
  vi.unstubAllGlobals()
})

it('lists only providers and models that can generate the selected media kind', async () => {
  vi.mocked(getSettingsCached).mockResolvedValue({
    providers: [
      { id: 'chat', name: 'Chat only', enabled: true, enabledModels: ['gpt-4o'] },
      { id: 'p', name: 'Provider', enabled: true, enabledModels: ['gpt-4o', 'gpt-image-1'] },
      { id: 'x', name: 'xAI', enabled: true, enabledModels: ['grok-imagine-video'] },
    ],
    defaultModels: { imageGeneration: { providerId: 'chat', model: 'gpt-4o' } },
  } as Settings)
  render(<MediaStation onOpenSettings={vi.fn()} />)
  fireEvent.click(screen.getByRole('button', { name: '生图' }))
  await waitFor(() => expect(screen.getByLabelText('模型')).toHaveTextContent('gpt-image-1'))
  fireEvent.click(screen.getByLabelText('供应商'))
  expect(screen.queryByRole('option', { name: /Chat only/ })).toBeNull()
  expect(screen.queryByRole('option', { name: /xAI/ })).toBeNull()
  fireEvent.click(screen.getByLabelText('模型'))
  expect(screen.queryByRole('option', { name: /gpt-4o/ })).toBeNull()
  fireEvent.click(screen.getByRole('button', { name: '生视频' }))
  await waitFor(() => expect(screen.getByLabelText('供应商')).toHaveTextContent('xAI'))
  expect(screen.getByLabelText('模型')).toHaveTextContent('grok-imagine-video')
})

it('fetches an accepted video again instead of submitting a new paid task', async () => {
  const video = { ...job, status: 'failed' as const, error: 'HTTP 502 Bad Gateway', providerTaskId: 'remote-1', request: { ...job.request, kind: 'video' as const, model: 'grok-imagine-video' } }
  vi.mocked(mediaStationApi.list).mockResolvedValue([video])
  vi.mocked(mediaStationApi.resume).mockResolvedValue({ ...video, status: 'running', error: null })
  render(<MediaStation onOpenSettings={vi.fn()} />)
  await openHistory()
  fireEvent.click(await screen.findByRole('button', { name: /失败 晨光花瓶/ }))
  fireEvent.click(screen.getByRole('button', { name: '继续获取结果' }))
  await waitFor(() => expect(mediaStationApi.resume).toHaveBeenCalledWith(video.id))
  expect(mediaStationApi.start).not.toHaveBeenCalled()
})

it('hides providers that media generation cannot use', async () => {
  vi.mocked(getSettingsCached).mockResolvedValue({
    providers: [
      { id: 'oauth', name: 'OAuth', enabled: true, enabledModels: ['gpt-image-1'], apiFormat: 'openai_responses', request: { oauth: { provider: 'codex' } } },
      { id: 'claude', name: 'Claude', enabled: true, enabledModels: ['gpt-image-1'], apiFormat: 'anthropic_messages', request: {} },
      { id: 'p', name: 'Provider', enabled: true, enabledModels: ['gpt-image-1'], apiFormat: 'openai_chat', request: {} },
    ],
    defaultModels: { imageGeneration: { providerId: 'oauth', model: 'gpt-image-1' } },
  } as unknown as Settings)
  render(<MediaStation onOpenSettings={vi.fn()} />)
  fireEvent.click(screen.getByRole('button', { name: '生图' }))
  await waitFor(() => expect(screen.getByLabelText('供应商')).toHaveTextContent('Provider'))
  fireEvent.click(screen.getByLabelText('供应商'))
  expect(screen.queryByRole('option', { name: /OAuth/ })).toBeNull()
  expect(screen.queryByRole('option', { name: /Claude/ })).toBeNull()
})

it('offers video models from the model library and the per-model capability toggle', async () => {
  vi.mocked(getSettingsCached).mockResolvedValue({
    providers: [
      { id: 'ark', name: 'Doubao', enabled: true, enabledModels: ['doubao-seed-2.0-pro', 'doubao-seedance-2-5-260628'], apiFormat: 'openai_chat', request: {} },
      { id: 'relay', name: 'Relay', enabled: true, enabledModels: ['my-video'], apiFormat: 'openai_chat', request: {}, modelOverrides: { 'my-video': { capabilities: { videoGeneration: true } } } },
    ],
    defaultModels: { imageGeneration: { providerId: '', model: '' } },
  } as unknown as Settings)
  render(<MediaStation onOpenSettings={vi.fn()} />)
  fireEvent.click(screen.getByRole('button', { name: '生视频' }))
  await waitFor(() => expect(screen.getByLabelText('模型')).toHaveTextContent('doubao-seedance-2-5-260628'))
  fireEvent.click(screen.getByLabelText('模型'))
  expect(screen.queryByRole('option', { name: /doubao-seed-2\.0-pro/ })).toBeNull()
  fireEvent.click(screen.getByLabelText('模型'))
  fireEvent.click(screen.getByLabelText('供应商'))
  fireEvent.click(screen.getByRole('option', { name: /Relay/ }))
  await waitFor(() => expect(screen.getByLabelText('模型')).toHaveTextContent('my-video'))
})

it('opens a creation as a dialog over the history instead of inside it', async () => {
  vi.mocked(mediaStationApi.list).mockResolvedValue([job])
  render(<MediaStation onOpenSettings={vi.fn()} />)
  await openHistory()
  expect(screen.queryByRole('dialog')).toBeNull()
  expect(screen.queryByLabelText('创作描述')).toBeNull()
  fireEvent.click(await screen.findByRole('button', { name: /生成中 晨光花瓶/ }))
  const dialog = screen.getByRole('dialog', { name: '作品详情' })
  expect(dialog).toHaveAttribute('open')
  expect(screen.getByRole('button', { name: /生成中 晨光花瓶/ })).toBeTruthy()
  fireEvent.click(screen.getByRole('button', { name: '关闭' }))
  expect(screen.queryByRole('dialog')).toBeNull()
})

it('rotates starter ideas and fills in both the prompt and its aspect ratio', async () => {
  render(<MediaStation onOpenSettings={vi.fn()} />)
  fireEvent.click(screen.getByRole('button', { name: '生图' }))
  const titles = () => screen.getAllByRole('button').filter((b) => b.classList.contains('kv-media-idea')).map((b) => b.querySelector('strong')!.textContent)
  const first = titles()
  expect(first).toHaveLength(4)
  fireEvent.click(screen.getByRole('button', { name: '换一批' }))
  expect(titles()).not.toEqual(first)
  const idea = screen.getAllByRole('button').find((b) => b.querySelector('strong')?.textContent === '水墨山水')
    ?? (fireEvent.click(screen.getByRole('button', { name: '换一批' })), screen.getAllByRole('button').find((b) => b.querySelector('strong')?.textContent === '水墨山水'))
    ?? (fireEvent.click(screen.getByRole('button', { name: '换一批' })), screen.getAllByRole('button').find((b) => b.querySelector('strong')?.textContent === '水墨山水'))
  fireEvent.click(idea!)
  expect(screen.getByLabelText('创作描述')).toHaveValue('水墨山水，远山与云雾，一叶扁舟，大面积留白。')
  expect(screen.getByLabelText('画幅')).toHaveTextContent('16:9')
})
