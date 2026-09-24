import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { api, type ArtifactLibraryItem, type ArtifactLibraryPage } from '../api/tauri'
import { open } from '@tauri-apps/plugin-dialog'
import { ArtifactsCenter, clearArtifactsPageCache } from './ArtifactsCenter'

vi.mock('../api/tauri', () => ({ api: { chatArtifactsList: vi.fn(), chatArtifactAction: vi.fn() } }))
vi.mock('../components/i18n', () => ({ useLang: () => 'zh' }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: vi.fn().mockResolvedValue(null), open: vi.fn().mockResolvedValue(null) }))

function work(id: string, overrides: Partial<ArtifactLibraryItem> = {}): ArtifactLibraryItem {
  return { id, workId: id, parentId: null, conversationId: 'conv_source', messageId: 'msg_source', title: '头像设计', createdAt: 2, sourceTool: 'mixer_generate_image', delivered: true, artifact: { id, name: `${id}.png`, mime_type: 'image/png' }, available: true, sourceAvailable: true, ...overrides }
}
function page(...items: ArtifactLibraryItem[]): ArtifactLibraryPage { return { items, warnings: 0 } }

beforeEach(() => {
  vi.clearAllMocks()
  clearArtifactsPageCache()
  vi.mocked(api.chatArtifactsList).mockResolvedValue(page())
  vi.mocked(api.chatArtifactAction).mockResolvedValue(null)
  HTMLDialogElement.prototype.showModal = function () { this.setAttribute('open', '') }
})
afterEach(() => { cleanup(); vi.unstubAllGlobals() })

describe('Works library', () => {
  it('shows indexed works before historical import finishes and keeps them visible on return', async () => {
    let finishImport!: (value: ArtifactLibraryPage) => void
    const pendingImport = new Promise<ArtifactLibraryPage>((resolve) => { finishImport = resolve })
    vi.mocked(api.chatArtifactsList).mockImplementation((importHistory) => importHistory
      ? pendingImport
      : Promise.resolve(page(work('indexed'))))
    const first = render(<ArtifactsCenter onOpenConversation={vi.fn()} />)
    expect(await screen.findByRole('button', { name: 'indexed.png' })).toBeTruthy()
    expect(api.chatArtifactsList).toHaveBeenCalledWith(false)
    expect(api.chatArtifactsList).toHaveBeenCalledWith(true)
    first.unmount()
    render(<ArtifactsCenter onOpenConversation={vi.fn()} />)
    expect(screen.getByRole('button', { name: 'indexed.png' })).toBeTruthy()
    finishImport(page(work('indexed'), work('legacy')))
    expect(await screen.findByRole('button', { name: 'legacy.png' })).toBeTruthy()
  })

  it('opens the original in the default application without reading an in-app preview', async () => {
    vi.mocked(api.chatArtifactsList).mockResolvedValue(page(work('latest', { workId: 'older', parentId: 'older' }), work('older', { createdAt: 1 })))
    render(<ArtifactsCenter onOpenConversation={vi.fn()} />)
    fireEvent.click(await screen.findByRole('button', { name: 'latest.png' }))
    await waitFor(() => expect(api.chatArtifactAction).toHaveBeenCalledWith('latest', 'open'))
    expect(api.chatArtifactAction).not.toHaveBeenCalledWith(expect.anything(), 'preview')
    expect(screen.queryByRole('dialog')).toBeNull()
    fireEvent.click(screen.getAllByRole('button', { name: '更多操作' })[0])
    fireEvent.click(screen.getByRole('menuitem', { name: '版本 1' }))
    await waitFor(() => expect(api.chatArtifactAction).toHaveBeenCalledWith('older', 'open'))
  })

  it('does not read originals merely to paint file cards', async () => {
    vi.mocked(api.chatArtifactsList).mockResolvedValue(page(work('html', { artifact: { name: 'index.html', mime_type: 'text/html', size_bytes: 1024 } })))
    render(<ArtifactsCenter onOpenConversation={vi.fn()} />)
    await screen.findByRole('button', { name: 'index.html' })
    expect(api.chatArtifactAction).not.toHaveBeenCalled()
  })

  it('reports a missing original without trying to open it', async () => {
    vi.mocked(api.chatArtifactsList).mockResolvedValue(page(work('missing', { available: false })))
    render(<ArtifactsCenter onOpenConversation={vi.fn()} />)
    fireEvent.click(await screen.findByRole('button', { name: 'missing.png' }))
    expect(screen.getByRole('alert').textContent).toContain('文件缺失')
    expect(api.chatArtifactAction).not.toHaveBeenCalled()
  })

  it('keeps populated filters and search in list view', async () => {
    vi.mocked(api.chatArtifactsList).mockResolvedValue(page(
      work('doc', { artifact: { name: '项目计划.docx', mime_type: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document' } }),
      work('sheet', { artifact: { name: '预算.xlsx', mime_type: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet' } }),
      work('slides', { artifact: { name: '项目汇报.pptx', mime_type: 'application/vnd.openxmlformats-officedocument.presentationml.presentation' } }),
    ))
    render(<ArtifactsCenter onOpenConversation={vi.fn()} />)
    await screen.findByRole('button', { name: '项目计划.docx' })
    fireEvent.click(screen.getByRole('button', { name: /^表格/ }))
    expect(screen.getByRole('button', { name: '预算.xlsx' })).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: /^全部/ }))
    fireEvent.click(screen.getByRole('button', { name: '列表视图' }))
    fireEvent.change(screen.getByRole('textbox', { name: '搜索作品' }), { target: { value: '项目' } })
    expect(screen.queryByRole('button', { name: '预算.xlsx' })).toBeNull()
    expect(screen.getByRole('button', { name: '项目计划.docx' })).toBeTruthy()
  })

  it('renames and deletes from the card menu', async () => {
    vi.spyOn(window, 'confirm').mockReturnValue(true)
    vi.mocked(api.chatArtifactsList).mockResolvedValue(page(work('one'), work('two')))
    render(<ArtifactsCenter onOpenConversation={vi.fn()} />)
    await screen.findByRole('button', { name: 'one.png' })
    fireEvent.click(screen.getAllByRole('button', { name: '更多操作' })[0])
    fireEvent.click(screen.getByRole('menuitem', { name: '重命名' }))
    fireEvent.change(screen.getByRole('textbox', { name: '作品名称' }), { target: { value: '封面.png' } })
    fireEvent.click(screen.getByRole('button', { name: '保存' }))
    await waitFor(() => expect(api.chatArtifactAction).toHaveBeenCalledWith('one', 'rename', undefined, '封面.png'))
    fireEvent.click(screen.getByRole('button', { name: '选择' }))
    fireEvent.click(screen.getByRole('button', { name: 'two.png' }))
    fireEvent.click(screen.getByRole('button', { name: '删除' }))
    await waitFor(() => expect(api.chatArtifactAction).toHaveBeenCalledWith('two', 'delete'))
  })

  it('saves same-named works without overwriting earlier exports', async () => {
    vi.mocked(open).mockResolvedValue('C:\\exports')
    vi.mocked(api.chatArtifactsList).mockResolvedValue(page(
      work('one', { artifact: { name: 'report.html', mime_type: 'text/html' } }),
      work('two', { artifact: { name: 'report.html', mime_type: 'text/html' } }),
    ))
    render(<ArtifactsCenter onOpenConversation={vi.fn()} />)
    await screen.findAllByRole('button', { name: 'report.html' })
    fireEvent.click(screen.getByRole('button', { name: '选择' }))
    fireEvent.click(screen.getByRole('button', { name: '全选' }))
    fireEvent.click(screen.getByRole('button', { name: '另存为' }))
    await waitFor(() => expect(api.chatArtifactAction).toHaveBeenCalledWith('one', 'export_unique', 'C:\\exports'))
    await waitFor(() => expect(api.chatArtifactAction).toHaveBeenCalledWith('two', 'export_unique', 'C:\\exports'))
  })

  it('continues a batch delete after one failure and refreshes the remaining works', async () => {
    vi.spyOn(window, 'confirm').mockReturnValue(true)
    let remaining = page(work('one'), work('two'))
    vi.mocked(api.chatArtifactsList).mockImplementation(async () => remaining)
    vi.mocked(api.chatArtifactAction).mockImplementation(async (id, action) => {
      if (action === 'delete' && id === 'one') throw new Error('locked')
      if (action === 'delete' && id === 'two') remaining = page(work('one'))
      return null
    })
    render(<ArtifactsCenter onOpenConversation={vi.fn()} />)
    await screen.findByRole('button', { name: 'one.png' })
    fireEvent.click(screen.getByRole('button', { name: '选择' }))
    fireEvent.click(screen.getByRole('button', { name: '全选' }))
    fireEvent.click(screen.getByRole('button', { name: '删除' }))
    await waitFor(() => expect(api.chatArtifactAction).toHaveBeenCalledWith('two', 'delete'))
    await waitFor(() => expect(screen.queryByRole('button', { name: 'two.png' })).toBeNull())
    expect(screen.getByRole('alert').textContent).toContain('1 个版本删除失败')
  })
})
