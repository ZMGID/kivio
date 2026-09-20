import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { api, type ArtifactLibraryItem } from '../api/tauri'
import { ArtifactsCenter } from './ArtifactsCenter'

vi.mock('../api/tauri', () => ({ api: { chatArtifactsList: vi.fn(), chatArtifactAction: vi.fn() } }))
vi.mock('../components/i18n', () => ({ useLang: () => 'zh' }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: vi.fn().mockResolvedValue(null), open: vi.fn().mockResolvedValue(null) }))

function work(id: string, overrides: Partial<ArtifactLibraryItem> = {}): ArtifactLibraryItem {
  return { id, workId: id, parentId: null, conversationId: 'conv_source', messageId: 'msg_source', title: '头像设计', createdAt: 2, sourceTool: 'mixer_generate_image', delivered: true, artifact: { id, name: `${id}.png`, mime_type: 'image/png' }, available: true, sourceAvailable: true, ...overrides }
}
beforeEach(() => {
  vi.clearAllMocks()
  HTMLDialogElement.prototype.showModal = function () { this.setAttribute('open', '') }
  vi.mocked(api.chatArtifactAction).mockResolvedValue('data:image/png;base64,aGVsbG8=')
})
afterEach(() => { cleanup(); vi.unstubAllGlobals() })

describe('Works library', () => {
  it('groups edits, opens an immutable earlier version, and returns to the source conversation', async () => {
    vi.mocked(api.chatArtifactsList).mockResolvedValue({ items: [work('art_new', { workId: 'art_old', parentId: 'art_old' }), work('art_old', { createdAt: 1 })], warnings: 0 })
    const onSource = vi.fn()
    render(<ArtifactsCenter onOpenConversation={onSource} />)
    fireEvent.click(await screen.findByRole('button', { name: /art_new.png/ }))
    await waitFor(() => expect(api.chatArtifactAction).toHaveBeenCalledWith('art_new', 'preview'))
    fireEvent.click(screen.getByRole('button', { name: '历史版本' }))
    expect(screen.getByRole('dialog').contains(screen.getByRole('listbox'))).toBe(true)
    fireEvent.click(screen.getByRole('option', { name: '版本 1' }))
    await waitFor(() => expect(api.chatArtifactAction).toHaveBeenCalledWith('art_old', 'preview'))
    fireEvent.click(screen.getByRole('button', { name: '回到聊天修改' }))
    expect(onSource).toHaveBeenCalledWith('conv_source')
  })
  it('filters files, reports missing originals, and disables unavailable actions', async () => {
    vi.mocked(api.chatArtifactsList).mockResolvedValue({ items: [work('art_image'), work('art_doc', { artifact: { name: 'report.pdf', mime_type: 'application/pdf' }, available: false, sourceAvailable: false })], warnings: 0 })
    render(<ArtifactsCenter onOpenConversation={vi.fn()} />)
    await screen.findByRole('button', { name: /report.pdf/ })
    fireEvent.click(screen.getByRole('button', { name: /^文档/ }))
    expect(screen.queryByRole('button', { name: /art_image.png/ })).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: /report.pdf/ }))
    expect(screen.getByRole('button', { name: '另存为' }).hasAttribute('disabled')).toBe(true)
    expect(screen.getByRole('button', { name: '回到聊天修改' }).hasAttribute('disabled')).toBe(true)
    expect(api.chatArtifactAction).not.toHaveBeenCalled()
  })
  it('ignores a late preview when a different work is opened', async () => {
    let completeFirst!: (v: string) => void
    vi.mocked(api.chatArtifactsList).mockResolvedValue({ items: [work('art_first'), work('art_second')], warnings: 0 })
    vi.mocked(api.chatArtifactAction).mockImplementation((id) => id === 'art_first' ? new Promise((resolve) => { completeFirst = resolve }) : Promise.resolve('data:image/png;base64,c2Vjb25k'))
    render(<ArtifactsCenter onOpenConversation={vi.fn()} />)
    fireEvent.click(await screen.findByRole('button', { name: /art_first.png/ }))
    fireEvent.click(screen.getByRole('button', { name: '关闭预览' }))
    fireEvent.click(screen.getByRole('button', { name: /art_second.png/ }))
    await screen.findByRole('img', { name: 'art_second.png' })
    await act(async () => completeFirst('data:image/png;base64,Zmlyc3Q='))
    expect(screen.getByRole('img', { name: 'art_second.png' }).getAttribute('src')).toBe('data:image/png;base64,c2Vjb25k')
  })
  it('retries failed loads without losing navigation', async () => {
    vi.mocked(api.chatArtifactsList).mockRejectedValueOnce(new Error('disk unavailable')).mockResolvedValueOnce({ items: [work('art_ready')], warnings: 0 })
    render(<ArtifactsCenter onOpenConversation={vi.fn()} />)
    await screen.findByRole('alert')
    fireEvent.click(screen.getByRole('button', { name: '重试' }))
    expect(await screen.findByRole('button', { name: /art_ready.png/ })).toBeTruthy()
  })
  it('separates documents, spreadsheets and presentations, retaining search in list view', async () => {
    vi.mocked(api.chatArtifactsList).mockResolvedValue({ items: [
      work('doc', { artifact: { name: '项目计划.docx', mime_type: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document' } }),
      work('sheet', { artifact: { name: '预算.xlsx', mime_type: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet' } }),
      work('slides', { artifact: { name: '项目汇报.pptx', mime_type: 'application/vnd.openxmlformats-officedocument.presentationml.presentation' } }),
    ], warnings: 0 })
    render(<ArtifactsCenter onOpenConversation={vi.fn()} />)
    await screen.findByRole('button', { name: /项目计划.docx/ })
    fireEvent.click(screen.getByRole('button', { name: /^表格/ }))
    expect(screen.getByRole('button', { name: /预算.xlsx/ })).toBeTruthy()
    expect(screen.queryByRole('button', { name: /项目计划.docx/ })).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: /^演示稿/ }))
    expect(screen.getByRole('button', { name: /项目汇报.pptx/ })).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: /^全部/ }))
    fireEvent.click(screen.getByRole('button', { name: '列表视图' }))
    fireEvent.change(screen.getByRole('textbox', { name: '搜索作品' }), { target: { value: '项目' } })
    expect(screen.queryByRole('button', { name: /预算.xlsx/ })).toBeNull()
    expect(screen.getByRole('button', { name: /项目计划.docx/ })).toBeTruthy()
  })
  it('reads real UTF-8 text for a cover and reuses that original in the reader', async () => {
    let intersect!: (entries: { isIntersecting: boolean }[]) => void
    vi.stubGlobal('IntersectionObserver', class {
      constructor(callback: typeof intersect) { intersect = callback }
      observe() {}
      disconnect() {}
    })
    const content = '项目计划\n下周交付第一版文档。'
    const data = `data:text/plain;base64,${btoa(String.fromCharCode(...new TextEncoder().encode(content)))}`
    vi.mocked(api.chatArtifactsList).mockResolvedValue({ items: [work('text', { artifact: { name: '项目计划.txt', mime_type: 'text/plain', size_bytes: 120 } })], warnings: 0 })
    vi.mocked(api.chatArtifactAction).mockResolvedValue(data)
    render(<ArtifactsCenter onOpenConversation={vi.fn()} />)
    await screen.findByRole('button', { name: '项目计划.txt' })
    expect(api.chatArtifactAction).not.toHaveBeenCalled()
    act(() => intersect([{ isIntersecting: true }]))
    expect(await screen.findByText(/下周交付第一版文档/)).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: '项目计划.txt' }))
    await waitFor(() => expect(screen.getByRole('dialog').textContent).toContain(content))
    expect(api.chatArtifactAction).toHaveBeenCalledTimes(1)
  })
  it('hides empty type filters and keeps populated ones', async () => {
    vi.mocked(api.chatArtifactsList).mockResolvedValue({ items: [work('art_image')], warnings: 0 })
    render(<ArtifactsCenter onOpenConversation={vi.fn()} />)
    await screen.findByRole('button', { name: 'art_image.png' })
    expect(screen.getByRole('button', { name: '图片' })).toBeTruthy()
    expect(screen.queryByRole('button', { name: '文档' })).toBeNull()
    expect(screen.queryByRole('button', { name: '表格' })).toBeNull()
  })
  it('renames from the reader and deletes a selected work from the library', async () => {
    vi.spyOn(window, 'confirm').mockReturnValue(true)
    vi.mocked(api.chatArtifactsList).mockResolvedValue({ items: [work('art_image'), work('art_other')], warnings: 0 })
    render(<ArtifactsCenter onOpenConversation={vi.fn()} />)
    fireEvent.click(await screen.findByRole('button', { name: 'art_image.png' }))
    fireEvent.click(screen.getByRole('button', { name: '重命名' }))
    fireEvent.change(screen.getByRole('textbox', { name: '作品名称' }), { target: { value: '封面.png' } })
    fireEvent.click(screen.getByRole('button', { name: '保存' }))
    await waitFor(() => expect(api.chatArtifactAction).toHaveBeenCalledWith('art_image', 'rename', undefined, '封面.png'))
    fireEvent.click(screen.getByRole('button', { name: '关闭预览' }))
    fireEvent.click(screen.getByRole('button', { name: '选择' }))
    fireEvent.click(screen.getByRole('button', { name: 'art_other.png' }))
    fireEvent.click(screen.getByRole('button', { name: '删除' }))
    await waitFor(() => expect(api.chatArtifactAction).toHaveBeenCalledWith('art_other', 'delete'))
    expect(window.confirm).toHaveBeenCalled()
  })
  it('keeps import warnings out of the empty canvas', async () => {
    vi.mocked(api.chatArtifactsList).mockResolvedValue({ items: [], warnings: 3 })
    render(<ArtifactsCenter onOpenConversation={vi.fn()} />)
    const heading = await screen.findByRole('heading', { name: '你的创作，从这里开始' })
    expect(heading.closest('.kv-works-empty')?.textContent).not.toMatch(/历史内容未导入/)
    expect(screen.getByText('3 项历史内容未导入').closest('.kv-works-empty')).toBeNull()
  })
})
