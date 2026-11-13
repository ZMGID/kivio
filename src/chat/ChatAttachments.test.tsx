import { describe, expect, it, vi } from 'vitest'
import { fireEvent, render, waitFor } from '@testing-library/react'

const { loadImage, openViewer } = vi.hoisted(() => ({ loadImage: vi.fn(), openViewer: vi.fn() }))
vi.mock('./imageViewer', () => ({ openChatImageViewer: openViewer }))

const PNG = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUg'

vi.mock('./attachmentPreview', () => ({
  loadAttachmentDataUrl: (...args: unknown[]) => loadImage(...args),
  openAttachment: () => Promise.resolve(),
}))

import { ChatAttachments } from './ChatAttachments'
loadImage.mockResolvedValue(PNG)

const imageAttachment = {
  id: 'a1',
  name: 'shot.png',
  type: 'image' as const,
  path: '/tmp/shot.png',
}

/**
 * 用户自己发的图（走 ChatAttachments）此前没有右键菜单，只有模型出图（ChatInlineImage）有。
 * 两条路现在共用 ChatImageContextMenu，这条钉住用户侧那条也接上了、且不冒泡给消息级菜单。
 */
describe('ChatAttachments 图片右键', () => {
  it('opens the image menu and does not bubble to the message menu', async () => {
    const onParentContextMenu = vi.fn()
    const { container } = render(
      <div onContextMenu={onParentContextMenu}>
        <ChatAttachments
          attachments={[imageAttachment]}
          variant="user"
        />
      </div>,
    )

    const button = await waitFor(() => {
      const el = container.querySelector('button[aria-label="预览图片"]')
      if (!el) throw new Error('图片还没加载出来')
      return el
    })
    button.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true }))

    await waitFor(() => expect(document.body.textContent).toContain('复制图片'))
    expect(onParentContextMenu).not.toHaveBeenCalled()
  })
})

describe('ChatAttachments 卡片形态', () => {
  it('renders file cards with a type label and filename', () => {
    const { container } = render(
      <ChatAttachments
        attachments={[
          { id: 'f1', name: 'notes.pdf', type: 'file', path: '/tmp/notes.pdf' },
        ]}
        variant="user"
      />,
    )

    expect(container.textContent).toContain('notes.pdf')
    expect(container.textContent).toContain('PDF')
  })

  it('renders dropped folders as FOLDER cards', () => {
    const { container } = render(
      <ChatAttachments
        attachments={[
          { id: 'd1', name: 'testend', type: 'folder', path: 'E:\\ZM database\\testend' },
        ]}
        variant="composer"
      />,
    )

    expect(container.textContent).toContain('testend')
    expect(container.textContent).toContain('FOLDER')
  })
})

it('loads only a thumbnail until the viewer is requested', async () => {
  loadImage.mockReset().mockResolvedValueOnce(PNG).mockResolvedValueOnce('data:image/png;base64,ORIGINAL')
  openViewer.mockClear()
  const { container } = render(<ChatAttachments attachments={[imageAttachment]} variant="user" />)
  await waitFor(() => expect(container.querySelector('img')).toHaveAttribute('src', PNG))
  expect(loadImage).toHaveBeenCalledTimes(1)
  expect(loadImage).toHaveBeenCalledWith(imageAttachment, undefined, true)
  fireEvent.click(container.querySelector('button[aria-label="预览图片"]')!)
  await waitFor(() => expect(openViewer).toHaveBeenCalledWith(expect.objectContaining({ src: 'data:image/png;base64,ORIGINAL' })))
  expect(loadImage).toHaveBeenLastCalledWith(imageAttachment, undefined)
})
