import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ChatMarkdown } from './ChatMarkdown'
import { onChatImageViewerOpen } from './imageViewer'

const loadArtifactDataUrl = vi.hoisted(() => vi.fn())
const loadArtifactOriginalDataUrl = vi.hoisted(() => vi.fn())
const imageActions = vi.hoisted(() => ({ copy: vi.fn(), save: vi.fn(), dialog: vi.fn() }))
vi.mock('./attachmentPreview', () => ({ loadArtifactDataUrl, loadArtifactOriginalDataUrl }))
vi.mock('../api/tauri', () => ({ isTauriRuntime: () => true, api: {
  lensCopyImageToClipboard: imageActions.copy, lensSaveAnnotatedPng: imageActions.save,
} }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: imageActions.dialog }))

beforeEach(() => {
  loadArtifactDataUrl.mockReset()
  loadArtifactOriginalDataUrl.mockReset().mockResolvedValue(null)
  imageActions.copy.mockReset().mockResolvedValue({ success: true })
  imageActions.save.mockReset().mockResolvedValue({ success: true })
  imageActions.dialog.mockReset().mockResolvedValue('/tmp/saved.png')
})

describe('Markdown artifact thumbnail', () => {
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
