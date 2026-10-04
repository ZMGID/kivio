import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { MediaStation, resetMediaWindowForTests } from './MediaStation'
import { mediaStationApi, type MediaJob } from '../api/mediaStation'
import { getSettingsCached } from '../api/settingsCache'
import type { Settings } from '../api/tauri'
import { confirmDialog } from '../components/dialogQueue'

vi.mock('../api/mediaStation', () => ({ mediaStationApi: { list: vi.fn(), start: vi.fn(), cancel: vi.fn(), resume: vi.fn(), read: vi.fn(), export: vi.fn(), reference: vi.fn(), delete: vi.fn() } }))
vi.mock('../api/settingsCache', () => ({ getSettingsCached: vi.fn() }))
vi.mock('../api/tauri', () => ({ isTauriRuntime: () => true }))
vi.mock('../components/i18n', () => ({ useLang: () => 'zh' }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }))
vi.mock('../components/dialogQueue', () => ({ confirmDialog: vi.fn() }))

const openHistory = async () => fireEvent.click(await screen.findByRole('button', { name: /创作记录/ }))

const job: MediaJob = { id: 'job', createdAt: 1, status: 'running', error: null, outputs: [], request: { kind: 'image', providerId: 'p', model: 'gpt-image-1', prompt: '晨光花瓶', aspectRatio: '1:1', duration: 5, referencePaths: [] } }

const openFlights: Array<{ reject: (error: Error) => void; done: Promise<void> }> = []
function park<T>() {
  const { promise, resolve, reject } = Promise.withResolvers<T>()
  openFlights.push({ reject, done: promise.then(() => undefined, () => undefined) })
  return { promise, resolve, reject }
}
afterEach(async () => {
  const flights = openFlights.splice(0)
  for (const flight of flights) flight.reject(new Error('isolated'))
  await Promise.all(flights.map((flight) => flight.done))
  await Promise.resolve()
  await Promise.resolve()
  await Promise.resolve()
  resetMediaWindowForTests()
})

beforeEach(() => {
  resetMediaWindowForTests()
  vi.clearAllMocks()
  HTMLDialogElement.prototype.showModal = function () { this.setAttribute('open', '') }
  HTMLDialogElement.prototype.close = function () { this.removeAttribute('open') }
  vi.mocked(mediaStationApi.list).mockResolvedValue([])
  vi.mocked(confirmDialog).mockResolvedValue(true)
  vi.mocked(mediaStationApi.delete).mockImplementation(async (ids) => ({ deletedIds: ids, failures: [] }))
  vi.mocked(getSettingsCached).mockResolvedValue({ providers: [{ id: 'p', name: 'Provider', enabled: true, enabledModels: ['gpt-image-1'] }], defaultModels: { imageGeneration: { providerId: 'p', model: 'gpt-image-1' } } } as Settings)
})

it('submits once, observes the real task result, and keeps drafts across navigation', async () => {
  const started = park<MediaJob>()
  vi.mocked(mediaStationApi.start).mockReturnValue(started.promise)
  const view = render(<MediaStation onOpenSettings={vi.fn()} />)
  await waitFor(() => expect(screen.getByLabelText('模型')).toHaveTextContent('gpt-image-1'))
  fireEvent.change(screen.getByLabelText('创作描述'), { target: { value: '晨光花瓶' } })
  const generate = screen.getByRole('button', { name: '开始生成' })
  fireEvent.click(generate); fireEvent.click(generate)
  await waitFor(() => expect(mediaStationApi.start).toHaveBeenCalledTimes(1))
  vi.mocked(mediaStationApi.list).mockResolvedValue([job])
  await act(async () => { started.resolve(job) })
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

it('keeps each media kind’s result across switches, late submissions, dismissal, and navigation', async () => {
  const image: MediaJob = { ...job, id: 'mode-image', status: 'failed', error: 'image result' }
  const video: MediaJob = { ...job, id: 'mode-video', status: 'failed', error: 'video result', request: { ...job.request, kind: 'video', model: 'grok-imagine-video' } }
  vi.mocked(getSettingsCached).mockResolvedValue({
    providers: [{ id: 'p', name: 'Provider', enabled: true, enabledModels: ['gpt-image-1', 'grok-imagine-video'] }],
    defaultModels: { imageGeneration: { providerId: 'p', model: 'gpt-image-1' } },
  } as Settings)
  vi.mocked(mediaStationApi.list).mockResolvedValue([image, video])
  vi.mocked(mediaStationApi.start).mockResolvedValueOnce(image)
  const videoStart = park<MediaJob>()
  const view = render(<MediaStation onOpenSettings={vi.fn()} />)
  fireEvent.click(screen.getByRole('button', { name: '生图' }))
  await waitFor(() => expect(screen.getByLabelText('模型')).toHaveTextContent('gpt-image-1'))
  fireEvent.change(screen.getByLabelText('创作描述'), { target: { value: 'mode isolation' } })
  fireEvent.click(screen.getByRole('button', { name: '开始生成' }))
  expect(await screen.findByText('image result')).toBeTruthy()

  fireEvent.click(screen.getByRole('button', { name: '生视频' }))
  expect(screen.queryByRole('article', { name: '本次生成' })).toBeNull()
  expect(screen.getByText('想让什么动起来')).toBeTruthy()
  await waitFor(() => expect(screen.getByLabelText('模型')).toHaveTextContent('grok-imagine-video'))
  vi.mocked(mediaStationApi.start).mockImplementationOnce(() => videoStart.promise)
  fireEvent.click(screen.getByRole('button', { name: '开始生成' }))
  fireEvent.click(screen.getByRole('button', { name: '生图' }))
  await act(async () => { videoStart.resolve(video) })
  expect(screen.getByText('image result')).toBeTruthy()
  expect(screen.queryByText('video result')).toBeNull()

  fireEvent.click(screen.getByRole('button', { name: '生视频' }))
  expect(screen.getByText('video result')).toBeTruthy()
  fireEvent.click(screen.getByRole('button', { name: '收起' }))
  expect(screen.getByText('想让什么动起来')).toBeTruthy()
  fireEvent.click(screen.getByRole('button', { name: '生图' }))
  expect(screen.getByText('image result')).toBeTruthy()
  view.unmount()
  render(<MediaStation onOpenSettings={vi.fn()} />)
  expect(await screen.findByText('image result')).toBeTruthy()
  fireEvent.click(screen.getByRole('button', { name: '生视频' }))
  expect(screen.queryByRole('article', { name: '本次生成' })).toBeNull()
})

it('cancels permanent deletion without removing a record or source files', async () => {
  vi.mocked(mediaStationApi.list).mockResolvedValue([{ ...job, status: 'completed' }])
  vi.mocked(confirmDialog).mockResolvedValue(false)
  render(<MediaStation onOpenSettings={vi.fn()} />)
  await openHistory()
  fireEvent.click(await screen.findByRole('button', { name: /晨光花瓶/ }))
  fireEvent.click(screen.getByRole('button', { name: '删除' }))
  await waitFor(() => expect(confirmDialog).toHaveBeenCalledWith(expect.objectContaining({ danger: true, message: expect.stringMatching(/源文件.*无法撤销.*已导出的副本不受影响/) })))
  expect(mediaStationApi.delete).not.toHaveBeenCalled()
  expect(screen.getByRole('dialog', { name: '作品详情' })).toBeTruthy()
})

it('selects only filtered deletable records without opening a detail dialog', async () => {
  const image = { ...job, id: 'select-image', status: 'completed' as const }
  const video = { ...job, id: 'select-video', status: 'completed' as const, request: { ...job.request, kind: 'video' as const, prompt: '花瓶视频' } }
  vi.mocked(mediaStationApi.list).mockResolvedValue([image, video, { ...job, id: 'select-running', request: { ...job.request, prompt: '花瓶生成中' } }, { ...image, id: 'other', request: { ...job.request, prompt: '其他图片' } }])
  render(<MediaStation onOpenSettings={vi.fn()} />)
  await openHistory()
  await screen.findByRole('button', { name: /其他图片/ })
  fireEvent.click(screen.getByRole('button', { name: '图片' }))
  fireEvent.change(screen.getByLabelText('搜索描述'), { target: { value: '花瓶' } })
  const card = screen.getByRole('checkbox', { name: '选择记录：晨光花瓶' })
  fireEvent.click(card)
  expect(card).toHaveAttribute('aria-checked', 'true')
  expect(screen.queryByRole('dialog')).toBeNull()
  fireEvent.click(screen.getByRole('button', { name: /晨光花瓶/ }))
  expect(screen.getByRole('dialog', { name: '作品详情' })).toBeTruthy()
  fireEvent.click(screen.getByRole('button', { name: '关闭' }))
  expect(card).toHaveAttribute('aria-checked', 'true')
  fireEvent.click(card)
  expect(screen.queryByRole('status')).toBeNull()
  fireEvent.click(screen.getByRole('button', { name: '全选当前结果' }))
  expect(screen.getByRole('checkbox', { name: '选择记录：花瓶生成中' })).toBeDisabled()
  expect(screen.getByText('已选择 1 条')).toBeTruthy()
  fireEvent.click(screen.getByRole('button', { name: '删除所选' }))
  await waitFor(() => expect(mediaStationApi.delete).toHaveBeenCalledWith(['select-image']))
  await waitFor(() => expect(screen.queryByRole('checkbox', { name: '选择记录：晨光花瓶' })).toBeNull())
  expect(screen.queryByRole('button', { name: '删除所选' })).toBeNull()
  expect(screen.getByRole('checkbox', { name: '选择记录：花瓶生成中' })).toBeDisabled()
})

it('keeps failed deletions selected and prevents stale refresh results restoring successes', async () => {
  const success = { ...job, id: 'delete-ok', status: 'failed' as const, request: { ...job.request, prompt: '成功删除' } }
  const failure = { ...success, id: 'delete-failed', request: { ...job.request, prompt: '保留失败' } }
  vi.mocked(mediaStationApi.list).mockResolvedValue([success, failure])
  vi.mocked(mediaStationApi.delete).mockResolvedValue({ deletedIds: [success.id], failures: [{ id: failure.id, error: '文件被占用，请关闭后重试' }] })
  render(<MediaStation onOpenSettings={vi.fn()} />)
  await openHistory()
  await screen.findByRole('button', { name: /成功删除/ })
  fireEvent.click(screen.getByRole('button', { name: '全选当前结果' }))
  fireEvent.click(screen.getByRole('button', { name: '删除所选' }))
  expect(await screen.findByRole('alert')).toHaveTextContent('delete-failed: 文件被占用，请关闭后重试')
  expect(screen.queryByRole('checkbox', { name: '选择记录：成功删除' })).toBeNull()
  expect(screen.getByRole('checkbox', { name: '选择记录：保留失败' })).toHaveAttribute('aria-checked', 'true')
  expect(screen.getByText('已选择 1 条')).toBeTruthy()
  fireEvent.click(screen.getByRole('button', { name: '刷新记录' }))
  await waitFor(() => expect(mediaStationApi.list).toHaveBeenCalledTimes(3))
  expect(screen.queryByRole('checkbox', { name: '选择记录：成功删除' })).toBeNull()
})

it('guards running detail deletion until waiting is stopped', async () => {
  vi.mocked(mediaStationApi.list).mockResolvedValue([job])
  render(<MediaStation onOpenSettings={vi.fn()} />)
  await openHistory()
  fireEvent.click(await screen.findByRole('button', { name: /生成中 晨光花瓶/ }))
  expect(screen.getByRole('button', { name: '删除' })).toBeDisabled()
  expect(screen.getByText('请先停止等待再删除')).toBeTruthy()
  expect(confirmDialog).not.toHaveBeenCalled()
  expect(mediaStationApi.delete).not.toHaveBeenCalled()
})

it('continues editing the chosen image output with original settings without changing the old job', async () => {
  vi.stubGlobal('URL', { createObjectURL: vi.fn(() => 'blob:edit'), revokeObjectURL: vi.fn() })
  vi.mocked(mediaStationApi.read).mockResolvedValue([1])
  vi.mocked(mediaStationApi.reference).mockResolvedValue('/media/second.png')
  const original: MediaJob = { ...job, id: 'edit-image', status: 'completed', request: { ...job.request, aspectRatio: '3:4', referencePaths: ['/old/reference.png'] }, outputs: [{ name: 'first.png', mimeType: 'image/png', preview: '' }, { name: 'second.png', mimeType: 'image/png', preview: '' }] }
  vi.mocked(mediaStationApi.list).mockResolvedValue([original])
  const view = render(<MediaStation onOpenSettings={vi.fn()} />)
  await openHistory()
  fireEvent.click(await screen.findByRole('button', { name: /晨光花瓶/ }))
  fireEvent.click(screen.getByRole('button', { name: '继续编辑 2' }))
  expect(await screen.findByText('second.png')).toBeTruthy()
  expect(mediaStationApi.reference).toHaveBeenCalledWith('edit-image', 1)
  expect(screen.getByLabelText('创作描述')).toHaveValue(original.request.prompt)
  expect(screen.getByLabelText('供应商')).toHaveTextContent('Provider')
  expect(screen.getByLabelText('模型')).toHaveTextContent('gpt-image-1')
  expect(screen.getByLabelText('画幅')).toHaveTextContent('3:4')
  expect(screen.getByText(/原作品不会被覆盖/)).toBeTruthy()
  expect(mediaStationApi.start).not.toHaveBeenCalled()
  expect(original.request.referencePaths).toEqual(['/old/reference.png'])
  await openHistory()
  expect(await screen.findByRole('button', { name: /晨光花瓶/ })).toBeTruthy()
  view.unmount()
  vi.unstubAllGlobals()
})

it('changes video parameters for a new generation without editing or charging for the original', async () => {
  const original: MediaJob = { ...job, id: 'edit-video', status: 'completed', request: { ...job.request, kind: 'video', model: 'grok-imagine-video', aspectRatio: '16:9', duration: 10 } }
  vi.mocked(getSettingsCached).mockResolvedValue({ providers: [{ id: 'p', name: 'Provider', enabled: true, enabledModels: ['gpt-image-1', 'grok-imagine-video'] }], defaultModels: { imageGeneration: { providerId: 'p', model: 'gpt-image-1' } } } as Settings)
  vi.mocked(mediaStationApi.list).mockResolvedValue([original])
  render(<MediaStation onOpenSettings={vi.fn()} />)
  await openHistory()
  fireEvent.click(await screen.findByRole('button', { name: /晨光花瓶/ }))
  expect(screen.queryByRole('button', { name: '继续编辑' })).toBeNull()
  fireEvent.click(screen.getByRole('button', { name: '修改参数 / 重新生成' }))
  expect(screen.getByLabelText('创作描述')).toHaveValue(original.request.prompt)
  await waitFor(() => expect(screen.getByLabelText('模型')).toHaveTextContent('grok-imagine-video'))
  expect(screen.getByLabelText('时长')).toHaveTextContent('10 秒')
  expect(screen.getByText(/不会编辑或覆盖原视频/)).toBeTruthy()
  expect(mediaStationApi.reference).not.toHaveBeenCalled()
  expect(mediaStationApi.start).not.toHaveBeenCalled()
})

it('deletes a latest result and clears its remembered id across navigation', async () => {
  const latest: MediaJob = { ...job, id: 'latest-delete', status: 'failed', error: 'latest deletion result' }
  vi.mocked(mediaStationApi.start).mockResolvedValue(latest)
  vi.mocked(mediaStationApi.list).mockResolvedValue([latest])
  const view = render(<MediaStation onOpenSettings={vi.fn()} />)
  fireEvent.click(screen.getByRole('button', { name: '生图' }))
  await waitFor(() => expect(screen.getByLabelText('模型')).toHaveTextContent('gpt-image-1'))
  fireEvent.change(screen.getByLabelText('创作描述'), { target: { value: 'latest delete' } })
  fireEvent.click(screen.getByRole('button', { name: '开始生成' }))
  expect(await screen.findByRole('article', { name: '本次生成' })).toHaveTextContent('latest deletion result')
  fireEvent.click(screen.getByRole('button', { name: '删除' }))
  await waitFor(() => expect(screen.queryByRole('article', { name: '本次生成' })).toBeNull())
  expect(mediaStationApi.delete).toHaveBeenCalledWith(['latest-delete'])
  view.unmount()
  render(<MediaStation onOpenSettings={vi.fn()} />)
  await waitFor(() => expect(screen.getByLabelText('模型')).toHaveTextContent('gpt-image-1'))
  expect(screen.queryByRole('article', { name: '本次生成' })).toBeNull()
})

it('preserves a detail on delete failure and closes it only after a successful retry', async () => {
  const record: MediaJob = { ...job, id: 'detail-delete', status: 'completed' }
  vi.mocked(mediaStationApi.list).mockResolvedValue([record])
  vi.mocked(mediaStationApi.delete).mockResolvedValueOnce({ deletedIds: [], failures: [{ id: record.id, error: 'permission denied' }] }).mockResolvedValueOnce({ deletedIds: [record.id], failures: [] })
  render(<MediaStation onOpenSettings={vi.fn()} />)
  await openHistory()
  fireEvent.click(await screen.findByRole('button', { name: /晨光花瓶/ }))
  fireEvent.click(screen.getByRole('button', { name: '删除' }))
  expect(await screen.findByRole('alert')).toHaveTextContent('permission denied')
  expect(screen.getByRole('dialog')).toContainElement(screen.getByRole('alert'))
  fireEvent.click(screen.getByRole('button', { name: '删除' }))
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
  expect(screen.queryByRole('button', { name: /晨光花瓶/ })).toBeNull()
})

it('ignores a history response started before successful deletion', async () => {
  const record: MediaJob = { ...job, id: 'late-delete', status: 'completed' }
  const staleList = park<MediaJob[]>()
  vi.mocked(mediaStationApi.list).mockResolvedValueOnce([record]).mockReturnValueOnce(staleList.promise).mockResolvedValue([record])
  render(<MediaStation onOpenSettings={vi.fn()} />)
  await openHistory()
  await screen.findByRole('button', { name: /晨光花瓶/ })
  fireEvent.click(screen.getByRole('button', { name: '刷新记录' }))
  await waitFor(() => expect(mediaStationApi.list).toHaveBeenCalledTimes(2))
  fireEvent.click(screen.getByRole('button', { name: /晨光花瓶/ }))
  fireEvent.click(screen.getByRole('button', { name: '删除' }))
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
  await act(async () => { staleList.resolve([record]) })
  expect(screen.queryByRole('button', { name: /晨光花瓶/ })).toBeNull()
})

it('keeps one in-flight generation disabled across navigation and binds that request', async () => {
  const started = park<MediaJob>()
  vi.mocked(mediaStationApi.start).mockReturnValue(started.promise)
  const view = render(<MediaStation onOpenSettings={vi.fn()} />)
  await waitFor(() => expect(screen.getByLabelText('模型')).toHaveTextContent('gpt-image-1'))
  fireEvent.change(screen.getByLabelText('创作描述'), { target: { value: '  原始描述  ' } })
  const generate = screen.getByRole('button', { name: '开始生成' })
  fireEvent.click(generate)
  fireEvent.click(generate)
  fireEvent.change(screen.getByLabelText('创作描述'), { target: { value: '后来改的' } })
  await waitFor(() => expect(mediaStationApi.start).toHaveBeenCalledTimes(1))
  expect(mediaStationApi.start).toHaveBeenCalledWith(expect.objectContaining({ prompt: '原始描述', kind: 'image' }))
  view.unmount()
  render(<MediaStation onOpenSettings={vi.fn()} />)
  await waitFor(() => expect(screen.getByLabelText('模型')).toHaveTextContent('gpt-image-1'))
  const returned = screen.getByRole('button', { name: '开始生成' })
  expect(returned).toBeDisabled()
  fireEvent.click(returned)
  expect(mediaStationApi.start).toHaveBeenCalledTimes(1)
  expect(screen.getByLabelText('创作描述')).toHaveValue('后来改的')
  const created = { ...job, id: 'created-1', request: { ...job.request, prompt: '原始描述' } }
  vi.mocked(mediaStationApi.list).mockResolvedValue([created])
  await act(async () => { started.resolve(created) })
  expect(await screen.findByRole('article', { name: '本次生成' })).toHaveTextContent('原始描述')
  expect(screen.getByLabelText('创作描述')).toHaveValue('后来改的')
  await waitFor(() => expect(screen.getByRole('button', { name: '开始生成' })).toBeEnabled())
})

it('shows a generation failure from while the page was gone and lets the same draft retry', async () => {
  const started = park<MediaJob>()
  vi.mocked(mediaStationApi.start).mockReturnValue(started.promise)
  const view = render(<MediaStation onOpenSettings={vi.fn()} />)
  await waitFor(() => expect(screen.getByLabelText('模型')).toHaveTextContent('gpt-image-1'))
  fireEvent.change(screen.getByLabelText('创作描述'), { target: { value: 'keep me' } })
  fireEvent.click(screen.getByRole('button', { name: '开始生成' }))
  await waitFor(() => expect(mediaStationApi.start).toHaveBeenCalledTimes(1))
  view.unmount()
  render(<MediaStation onOpenSettings={vi.fn()} />)
  await waitFor(() => expect(screen.getByLabelText('模型')).toHaveTextContent('gpt-image-1'))
  expect(screen.getByRole('button', { name: '开始生成' })).toBeDisabled()
  await act(async () => { started.reject(new Error('HTTP 429')) })
  expect(await screen.findByRole('alert')).toHaveTextContent('HTTP 429')
  expect(screen.getByLabelText('创作描述')).toHaveValue('keep me')
  expect(screen.getByRole('button', { name: '开始生成' })).toBeEnabled()
  const created = { ...job, id: 'retry-1', status: 'completed' as const, request: { ...job.request, prompt: 'keep me' } }
  vi.mocked(mediaStationApi.start).mockResolvedValue(created)
  vi.mocked(mediaStationApi.list).mockResolvedValue([created])
  fireEvent.click(screen.getByRole('button', { name: '开始生成' }))
  expect(await screen.findByRole('article', { name: '本次生成' })).toBeTruthy()
  expect(mediaStationApi.start).toHaveBeenCalledTimes(2)
  expect(screen.queryByRole('alert')).toBeNull()
})

it('keeps a registered running job in history when start resolves away', async () => {
  const started = park<MediaJob>()
  vi.mocked(mediaStationApi.start).mockReturnValue(started.promise)
  const view = render(<MediaStation onOpenSettings={vi.fn()} />)
  await waitFor(() => expect(screen.getByLabelText('模型')).toHaveTextContent('gpt-image-1'))
  fireEvent.change(screen.getByLabelText('创作描述'), { target: { value: '晨光花瓶' } })
  fireEvent.click(screen.getByRole('button', { name: '开始生成' }))
  await waitFor(() => expect(mediaStationApi.start).toHaveBeenCalledTimes(1))
  view.unmount()
  const created = { ...job, id: 'still-running', status: 'running' as const }
  vi.mocked(mediaStationApi.list).mockResolvedValue([created])
  await act(async () => { started.resolve(created) })
  render(<MediaStation onOpenSettings={vi.fn()} />)
  await openHistory()
  expect(await screen.findByRole('button', { name: /生成中 晨光花瓶/ })).toBeTruthy()
  expect(mediaStationApi.cancel).not.toHaveBeenCalled()
  expect(mediaStationApi.start).toHaveBeenCalledTimes(1)
})

it('applies a late resume only to its job and leaves the newer detail in place', async () => {
  const first = { ...job, id: 'resume-a', status: 'failed' as const, error: 'old', providerTaskId: 'remote-a', request: { ...job.request, prompt: '第一张' } }
  const second = { ...job, id: 'resume-b', status: 'completed' as const, request: { ...job.request, prompt: '第二张' } }
  const resumed = park<MediaJob>()
  vi.mocked(mediaStationApi.list).mockResolvedValue([first, second])
  vi.mocked(mediaStationApi.resume).mockReturnValue(resumed.promise)
  render(<MediaStation onOpenSettings={vi.fn()} />)
  await openHistory()
  fireEvent.click(await screen.findByRole('button', { name: /失败 第一张/ }))
  fireEvent.click(screen.getByRole('button', { name: '继续获取结果' }))
  await waitFor(() => expect(mediaStationApi.resume).toHaveBeenCalledWith('resume-a'))
  fireEvent.click(screen.getByRole('button', { name: '关闭' }))
  fireEvent.click(await screen.findByRole('button', { name: /第二张/ }))
  expect(screen.getByRole('dialog')).toHaveTextContent('第二张')
  vi.mocked(mediaStationApi.list).mockResolvedValue([{ ...first, status: 'running', error: null }, second])
  await act(async () => { resumed.resolve({ ...first, status: 'running', error: null }) })
  expect(screen.getByRole('dialog')).toHaveTextContent('第二张')
  expect(screen.queryByText('正在生成，稍后回来也可以。')).toBeNull()
  expect(mediaStationApi.start).not.toHaveBeenCalled()
})

it('does not let a late resume restore a creation deleted while it was in flight', async () => {
  const record = { ...job, id: 'resume-deleted', status: 'failed' as const, error: 'x', providerTaskId: 'remote' }
  const resumed = park<MediaJob>()
  vi.mocked(mediaStationApi.list).mockResolvedValue([record])
  vi.mocked(mediaStationApi.resume).mockReturnValue(resumed.promise)
  render(<MediaStation onOpenSettings={vi.fn()} />)
  await openHistory()
  fireEvent.click(await screen.findByRole('button', { name: /失败 晨光花瓶/ }))
  fireEvent.click(screen.getByRole('button', { name: '继续获取结果' }))
  await waitFor(() => expect(mediaStationApi.resume).toHaveBeenCalledOnce())
  fireEvent.click(screen.getByRole('button', { name: '删除' }))
  await waitFor(() => expect(mediaStationApi.delete).toHaveBeenCalledWith(['resume-deleted']))
  await act(async () => { resumed.resolve({ ...record, status: 'running', error: null }) })
  await waitFor(() => expect(screen.queryByRole('button', { name: /晨光花瓶/ })).toBeNull())
  expect(screen.queryByText('正在生成，稍后回来也可以。')).toBeNull()
})

it('keeps a deletion that finishes away from resurrecting on the next history read', async () => {
  const record = { ...job, id: 'away-delete', status: 'completed' as const }
  const pendingDelete = park<{ deletedIds: string[]; failures: [] }>()
  vi.mocked(mediaStationApi.list).mockResolvedValue([record])
  vi.mocked(mediaStationApi.delete).mockReturnValue(pendingDelete.promise)
  const view = render(<MediaStation onOpenSettings={vi.fn()} />)
  await openHistory()
  fireEvent.click(await screen.findByRole('button', { name: /晨光花瓶/ }))
  fireEvent.click(screen.getByRole('button', { name: '删除' }))
  await waitFor(() => expect(mediaStationApi.delete).toHaveBeenCalledWith(['away-delete']))
  view.unmount()
  await act(async () => { pendingDelete.resolve({ deletedIds: ['away-delete'], failures: [] }) })
  render(<MediaStation onOpenSettings={vi.fn()} />)
  expect(await screen.findByRole('heading', { name: '还没有作品' })).toBeInTheDocument()
  expect(screen.queryByRole('button', { name: /晨光花瓶/ })).toBeNull()
})

it('keeps resume busy and reports the failure after returning', async () => {
  const video = { ...job, id: 'resume-away', status: 'failed' as const, error: 'HTTP 502', providerTaskId: 'remote-1', request: { ...job.request, kind: 'video' as const } }
  const resumed = park<MediaJob>()
  vi.mocked(mediaStationApi.list).mockResolvedValue([video])
  vi.mocked(mediaStationApi.resume).mockReturnValue(resumed.promise)
  const view = render(<MediaStation onOpenSettings={vi.fn()} />)
  await openHistory()
  fireEvent.click(await screen.findByRole('button', { name: /失败 晨光花瓶/ }))
  fireEvent.click(screen.getByRole('button', { name: '继续获取结果' }))
  await waitFor(() => expect(mediaStationApi.resume).toHaveBeenCalledOnce())
  view.unmount()
  render(<MediaStation onOpenSettings={vi.fn()} />)
  expect(await screen.findByRole('button', { name: '继续获取结果' })).toBeDisabled()
  await act(async () => { resumed.reject(new Error('still failing')) })
  expect(await screen.findByRole('alert')).toHaveTextContent('still failing')
  expect(screen.getByRole('button', { name: '继续获取结果' })).toBeEnabled()
  expect(mediaStationApi.start).not.toHaveBeenCalled()
})

async function openImageForReference() {
  vi.stubGlobal('URL', { createObjectURL: vi.fn(() => 'blob:reference'), revokeObjectURL: vi.fn() })
  vi.mocked(mediaStationApi.read).mockResolvedValue([1])
  vi.mocked(mediaStationApi.list).mockResolvedValue([{
    ...job, status: 'completed', outputs: [{ name: 'source.png', mimeType: 'image/png', preview: '' }],
  }])
  const view = render(<MediaStation onOpenSettings={vi.fn()} />)
  await waitFor(() => expect(screen.getByLabelText('模型')).toHaveTextContent('gpt-image-1'))
  await openHistory()
  fireEvent.click(await screen.findByRole('button', { name: /晨光花瓶/ }))
  return view
}

it('applies the single image reference result after repeated edit clicks', async () => {
  const reference = park<string>()
  vi.mocked(mediaStationApi.reference).mockReturnValue(reference.promise)
  const view = await openImageForReference()
  const edit = screen.getByRole('button', { name: '继续编辑' })
  fireEvent.click(edit)
  fireEvent.click(edit)
  await waitFor(() => expect(mediaStationApi.reference).toHaveBeenCalledOnce())
  await act(async () => { reference.resolve('/media/copied.png') })
  expect(await screen.findByText('copied.png')).toBeInTheDocument()
  expect(screen.queryByRole('dialog')).toBeNull()
  expect(screen.getByLabelText('创作描述')).toHaveValue(job.request.prompt)
  view.unmount()
  vi.unstubAllGlobals()
})

it.each(['success', 'failure'])('ignores a first-frame %s after newer edits across navigation', async (outcome) => {
  const reference = park<string>()
  vi.mocked(mediaStationApi.reference).mockReturnValue(reference.promise)
  const view = await openImageForReference()
  fireEvent.click(screen.getByRole('button', { name: '用作视频首帧' }))
  fireEvent.change(screen.getByLabelText('创作描述'), { target: { value: 'new image prompt' } })
  fireEvent.click(screen.getByRole('button', { name: '生图' }))
  view.unmount()
  const returned = render(<MediaStation onOpenSettings={vi.fn()} />)
  await act(async () => {
    if (outcome === 'success') reference.resolve('/media/stale.png')
    else reference.reject(new Error('retired reference failed'))
  })
  expect(screen.getByLabelText('创作描述')).toHaveValue('new image prompt')
  expect(screen.getByRole('button', { name: '生图' })).toHaveAttribute('aria-pressed', 'true')
  expect(screen.queryByText('stale.png')).toBeNull()
  expect(screen.queryByRole('alert')).toBeNull()
  returned.unmount()
  vi.unstubAllGlobals()
})

it('does not let an old edit reference close a newer detail', async () => {
  const reference = park<string>()
  vi.mocked(mediaStationApi.reference).mockReturnValue(reference.promise)
  const view = await openImageForReference()
  fireEvent.click(screen.getByRole('button', { name: '继续编辑' }))
  fireEvent.click(screen.getByRole('button', { name: '关闭' }))
  vi.mocked(mediaStationApi.list).mockResolvedValue([{ ...job, id: 'new-detail', request: { ...job.request, prompt: 'new detail' } }])
  fireEvent.click(screen.getByRole('button', { name: '刷新记录' }))
  fireEvent.click(await screen.findByRole('button', { name: /new detail/ }))
  await act(async () => { reference.resolve('/media/stale.png') })
  expect(screen.getByRole('dialog')).toHaveTextContent('new detail')
  expect(screen.queryByText('stale.png')).toBeNull()
  view.unmount()
  vi.unstubAllGlobals()
})
