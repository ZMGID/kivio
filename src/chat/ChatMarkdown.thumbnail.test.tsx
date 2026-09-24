import { fireEvent, render } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ChatMarkdown } from './ChatMarkdown'
import { onChatImageViewerOpen } from './imageViewer'

const loadArtifactDataUrl = vi.hoisted(() => vi.fn())
vi.mock('./attachmentPreview', () => ({ loadArtifactDataUrl }))

describe('Markdown artifact thumbnail', () => {
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
