import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ChatMarkdown } from './ChatMarkdown'
import { onChatImageViewerOpen } from './imageViewer'

const loadArtifactDataUrl = vi.hoisted(() => vi.fn())
const loadArtifactOriginalDataUrl = vi.hoisted(() => vi.fn())
const loadAttachmentDataUrl = vi.hoisted(() => vi.fn())
const loadWorkspaceImagePreview = vi.hoisted(() => vi.fn())
const imageActions = vi.hoisted(() => ({ copy: vi.fn(), save: vi.fn(), dialog: vi.fn() }))
vi.mock('./attachmentPreview', () => ({
  loadArtifactDataUrl, loadArtifactOriginalDataUrl, loadAttachmentDataUrl, loadWorkspaceImagePreview,
}))
vi.mock('../api/tauri', () => ({ isTauriRuntime: () => true, api: {
  lensCopyImageToClipboard: imageActions.copy, lensSaveAnnotatedPng: imageActions.save,
} }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: imageActions.dialog }))

beforeEach(() => {
  loadArtifactDataUrl.mockReset()
  loadArtifactOriginalDataUrl.mockReset().mockResolvedValue(null)
  loadAttachmentDataUrl.mockReset().mockResolvedValue(null)
  loadWorkspaceImagePreview.mockReset().mockResolvedValue(null)
  imageActions.copy.mockReset().mockResolvedValue({ success: true })
  imageActions.save.mockReset().mockResolvedValue({ success: true })
  imageActions.dialog.mockReset().mockResolvedValue('/tmp/saved.png')
})

describe('Markdown artifact thumbnail', () => {
  it.each([
    ['/C:/Work/Kivio/output/tool-call-screenshots/light.png', 'C:/Work/Kivio/output/tool-call-screenshots/light.png'],
    ['C:/Work/Kivio/output/screenshot.png', 'C:/Work/Kivio/output/screenshot.png'],
    ['file:///tmp/my%20image.png', '/tmp/my image.png'],
    ['/tmp/render.png', '/tmp/render.png'],
  ])('previews local Markdown image %s and opens its original', async (href, path) => {
    const preview = 'data:image/png;base64,AAAA'
    loadAttachmentDataUrl.mockResolvedValue(preview)
    const opened = vi.fn()
    const unsubscribe = onChatImageViewerOpen(opened)
    try {
      render(<ChatMarkdown content={`![截图](${href})`} conversationId="local" />)
      await waitFor(() => expect(screen.getByRole('img', { name: '截图' })).toHaveAttribute('src', preview))
      fireEvent.click(screen.getByRole('button', { name: '预览图片' }))
      expect(opened).toHaveBeenCalledWith(expect.objectContaining({ src: preview, path }))
    } finally {
      unsubscribe()
    }
  })

  it('retries the disk read after a local image is unavailable', async () => {
    loadAttachmentDataUrl.mockResolvedValueOnce(null).mockResolvedValueOnce('data:image/png;base64,AAAA')
    render(<ChatMarkdown content="![截图](/tmp/saved-later.png)" />)
    const retry = await screen.findByRole('button', { name: '重试加载原图' })
    fireEvent.click(retry)
    expect(await screen.findByRole('img', { name: '截图' })).toHaveAttribute('src', 'data:image/png;base64,AAAA')
    expect(loadAttachmentDataUrl).toHaveBeenCalledTimes(2)
  })

  it('shows a relative screenshot from the workspace and gives the viewer the resolved file', async () => {
    const path = 'C:/Work/Kivio/output/chart.png'
    const src = 'data:image/png;base64,AAAA'
    loadWorkspaceImagePreview.mockResolvedValue({ path, src })
    const opened = vi.fn()
    const unsubscribe = onChatImageViewerOpen(opened)
    try {
      render(<ChatMarkdown content="![图表](./output/chart.png)" conversationId="cli-conversation" />)
      expect(await screen.findByRole('img', { name: '图表' })).toHaveAttribute('src', src)
      fireEvent.click(screen.getByRole('button', { name: '预览图片' }))
      expect(opened).toHaveBeenCalledWith(expect.objectContaining({ path, src }))
      expect(loadWorkspaceImagePreview).toHaveBeenCalledWith('./output/chart.png', 'cli-conversation')
    } finally {
      unsubscribe()
    }
  })

  it('keeps web images as web URLs and skips native reads for non-image paths', () => {
    render(<ChatMarkdown content="![web](https://example.com/screenshot.png) ![text](/tmp/notes.txt)" />)
    expect(screen.getByRole('img', { name: 'web' })).toHaveAttribute('src', 'https://example.com/screenshot.png')
    expect(screen.queryByRole('img', { name: 'text' })).not.toBeInTheDocument()
    expect(loadAttachmentDataUrl).not.toHaveBeenCalled()
  })

  it.each(['复制图片', '图片另存为…'])('exports a complete inline-only image with %s', async action => {
    const view = render(<ChatMarkdown content="![chart](artifact:art_inline)" conversationId="inline"
      artifacts={[{ id: 'art_inline', name: 'chart.png', mime_type: 'image/png', data_url: 'data:image/png;base64,AAAA' }]} />)
    fireEvent.contextMenu(view.container.querySelector('[data-chat-inline-image]')!)
    fireEvent.click(screen.getByRole('menuitem', { name: action }))
    await waitFor(() => expect(action === '复制图片' ? imageActions.copy : imageActions.save)
      .toHaveBeenCalledWith(...(action === '复制图片' ? ['AAAA'] : ['AAAA', '/tmp/saved.png'])))
    expect(loadArtifactOriginalDataUrl).not.toHaveBeenCalled()
  })

  it.each(['复制图片', '图片另存为…'])('exports the original behind a thumbnail with %s', async action => {
    loadArtifactOriginalDataUrl.mockResolvedValue('data:image/png;base64,BBBB')
    const view = render(<ChatMarkdown content="![chart](artifact:art_original)" conversationId="original"
      artifacts={[{ id: 'art_original', name: 'chart.png', mime_type: 'image/png', data_url: 'data:image/png;base64,AAAA', path: 'original.png' }]} />)
    fireEvent.contextMenu(view.container.querySelector('[data-chat-inline-image]')!)
    fireEvent.click(screen.getByRole('menuitem', { name: action }))
    await waitFor(() => expect(action === '复制图片' ? imageActions.copy : imageActions.save)
      .toHaveBeenCalledWith(...(action === '复制图片' ? ['BBBB'] : ['BBBB', '/tmp/saved.png'])))
    expect(loadArtifactOriginalDataUrl).toHaveBeenCalledWith({ path: 'original.png' }, 'original')
  })

  it('does not export a thumbnail when its disk original is unavailable', async () => {
    const view = render(<ChatMarkdown content="![chart](artifact:art_missing)" conversationId="missing"
      artifacts={[{ id: 'art_missing', name: 'chart.png', data_url: 'data:image/png;base64,AAAA', path: 'missing.png' }]} />)
    fireEvent.contextMenu(view.container.querySelector('[data-chat-inline-image]')!)
    fireEvent.click(screen.getByRole('menuitem', { name: '复制图片' }))
    expect(await screen.findByText('无法读取原图，请重试。')).toBeInTheDocument()
    expect(imageActions.copy).not.toHaveBeenCalled()
  })

  it('uses the saved thumbnail in the list and passes the original path to the viewer', () => {
    const opened = vi.fn()
    const unsubscribe = onChatImageViewerOpen(opened)
    try {
      const thumbnail = 'data:image/png;base64,AAAA'
      const view = render(
        <ChatMarkdown
          content="![chart](artifact:art_chart)"
          conversationId="thumbnail-conversation"
          artifacts={[{ id: 'art_chart', name: 'chart.png', mime_type: 'image/png', data_url: thumbnail, path: 'chart-original.png' }]}
        />,
      )
      expect(view.container.querySelector('img')).toHaveAttribute('src', thumbnail)
      expect(loadArtifactDataUrl).not.toHaveBeenCalled()
      fireEvent.click(view.container.querySelector('[data-chat-inline-image]')!)
      expect(opened).toHaveBeenCalledWith(expect.objectContaining({
        src: thumbnail, path: 'chart-original.png', conversationId: 'thumbnail-conversation',
      }))
    } finally {
      unsubscribe()
    }
  })
})
