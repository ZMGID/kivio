import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { MessageBubble } from './MessageBubble'
import type { ChatMessage } from './types'
import { openChatImageViewer } from './imageViewer'
import * as attachmentPreview from './attachmentPreview'

vi.mock('./imageViewer', () => ({ openChatImageViewer: vi.fn() }))

const prepared: ChatMessage = {
  id: 'selected-deliverables', role: 'assistant', timestamp: 1, content: '',
  artifacts: [
    { id: 'art_draft', name: 'qa-frame.png', mime_type: 'image/png', data_url: 'data:image/png;base64,AAAA' },
    { id: 'art_final', name: 'final.png', mime_type: 'image/png', data_url: 'data:image/png;base64,BBBB' },
    { id: 'art_video', name: 'demo.mp4', mime_type: 'video/mp4', path: '/work/demo.mp4' },
  ],
  tool_calls: [{ id: 'prepare', name: 'present_artifacts', source: 'native', status: 'completed',
    structured_content: { type: 'artifact_presentation', artifactIds: ['art_final', 'art_video'], mode: 'prepare' } }],
  segments: [{ id: 'prepare', kind: 'tool', phase: 'tool_loop', order: 0, tool_call_id: 'prepare' }],
}

function withAnswer(content: string, outcome = 'completed'): ChatMessage {
  return { ...prepared, content, stream_outcome: outcome, segments: [...prepared.segments!,
    { id: 'answer', kind: 'text', phase: 'plain', order: 1, text: content },
  ] }
}

describe('selected final deliverables', () => {
  it('loads a stored image read and opens its original in the owning conversation', async () => {
    const load = vi.spyOn(attachmentPreview, 'loadAttachmentDataUrl').mockResolvedValue('data:image/png;base64,AAAA')
    const message: ChatMessage = { id: 'stored-read', role: 'assistant', timestamp: 1, content: '',
      tool_calls: [{ id: 'read', name: 'read', status: 'completed',
        artifacts: [{ id: 'art_stored', name: 'stored.png', path: 'stored.png', mime_type: 'image/png' }],
      }],
      segments: [{ id: 'read', kind: 'tool', phase: 'tool_loop', order: 0, tool_call_id: 'read' }],
    }
    try {
      render(<MessageBubble message={message} conversationId="image-conversation" messageStreaming />)
      fireEvent.click(screen.getByRole('button', { name: /已查看 1 张图像/ }))
      fireEvent.click(await screen.findByRole('button', { name: '预览图片' }))
      expect(load).toHaveBeenCalledWith(expect.objectContaining({ path: 'stored.png' }), 'image-conversation')
      expect(openChatImageViewer).toHaveBeenCalledWith(expect.objectContaining({ path: 'stored.png', conversationId: 'image-conversation' }))
    } finally {
      load.mockRestore()
    }
  })

  it.each(['prepare', 'preview'])('resolves an earlier image selected by %s without a markdown reference', mode => {
    const previousImage = prepared.artifacts![1]
    const message: ChatMessage = { ...prepared, artifacts: [],
      tool_calls: [{ ...prepared.tool_calls![0], structured_content: {
        type: 'artifact_presentation', artifactIds: ['art_final'], mode,
      } }],
    }
    const { container, unmount } = render(<MessageBubble message={message}
      conversationArtifactsById={new Map([['art_final', previousImage]])} />)
    expect(screen.queryByText(/文件不可用/)).not.toBeInTheDocument()
    if (mode === 'preview') expect(screen.getByRole('img')).toHaveAttribute('src', previousImage.data_url)
    else expect(container.querySelector('[aria-label="交付文件"]')).toHaveTextContent('final.png')
    unmount()
    render(<MessageBubble message={message} conversationArtifactsById={new Map([['art_final', previousImage]])} />)
    expect(screen.queryByText(/文件不可用/)).not.toBeInTheDocument()
  })

  it('keeps intermediate files out of the answer and renders chosen files in place through completion and reload', () => {
    const { container, rerender, unmount } = render(<MessageBubble message={prepared} messageStreaming />)
    expect(screen.getByText('已准备 2 个文件')).toBeVisible()
    expect(container.querySelector('img')).toBeNull()
    expect(screen.queryByLabelText('展示文件')).not.toBeInTheDocument()
    const final = withAnswer('Result\n\n![Final preview](artifact:art_final)\n\nWatch the recording:\n\n[Recording](artifact:art_video)')
    const assertFinal = () => {
      expect(screen.getByRole('img')).toHaveAttribute('src', 'data:image/png;base64,BBBB')
      expect(screen.getAllByRole('button', { name: '打开文件 demo.mp4' })).toHaveLength(1)
      expect(screen.queryByText('qa-frame.png')).not.toBeInTheDocument()
      expect(screen.queryByLabelText('交付文件')).not.toBeInTheDocument()
      expect(screen.getByRole('img').closest('[aria-label="过程分组"]')).toBeNull()
    }
    // Explicitly close the process while the answer is still streaming.
    fireEvent.click(screen.getByRole('button', { name: 'Working' }))
    rerender(<MessageBubble message={final} messageStreaming />)
    assertFinal()
    rerender(<MessageBubble message={final} />)
    assertFinal()
    unmount()
    render(<MessageBubble message={final} />)
    assertFinal()
  })

  it('keeps a compact fallback only for prepared files omitted from the final answer', () => {
    render(<MessageBubble message={withAnswer('![Preview](artifact:art_final)')} />)
    const fallback = screen.getByLabelText('交付文件')
    expect(fallback).toHaveTextContent('demo.mp4')
    expect(fallback).not.toHaveTextContent('qa-frame.png')
    expect(fallback.querySelector('img')).toBeNull()
    expect(screen.getAllByRole('img')).toHaveLength(1)
  })

  it.each(['cancelled', 'error'])('keeps prepared files accessible inside Work after %s without claiming final delivery', outcome => {
    render(<MessageBubble message={{ ...prepared, stream_outcome: outcome }} />)
    expect(screen.queryByLabelText('交付文件')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: /^Worked/ }))
    const summary = screen.getByText('已准备 2 个文件')
    expect(summary.closest('[aria-label="过程分组"]')).not.toBeNull()
    expect(summary.closest('details')?.querySelectorAll('button')).toHaveLength(2)
  })

  it('shows an explicitly requested preview immediately and avoids a second gallery when the answer references it', () => {
    const preview: ChatMessage = { ...prepared, tool_calls: [{ ...prepared.tool_calls![0],
      structured_content: { type: 'artifact_presentation', artifactIds: ['art_final'], mode: 'preview' },
    }] }
    const { rerender } = render(<MessageBubble message={preview} messageStreaming />)
    expect(screen.getByLabelText('展示文件')).toBeVisible()
    expect(screen.getAllByRole('img')).toHaveLength(1)
    rerender(<MessageBubble message={{ ...withAnswer('![Final](artifact:art_final)'), tool_calls: preview.tool_calls }} />)
    expect(screen.getAllByRole('img')).toHaveLength(1)
    expect(screen.queryByLabelText('展示文件')).not.toBeInTheDocument()
  })

  it('does not treat an artifact link inside a code example as a delivered file', () => {
    render(<MessageBubble message={withAnswer('Example: `[Recording](artifact:art_video)`')} />)
    expect(screen.getByLabelText('交付文件')).toHaveTextContent('demo.mp4')
  })

  it('does not duplicate the same file across explicit preview calls', () => {
    const message: ChatMessage = { ...prepared,
      tool_calls: ['one', 'two'].map(id => ({ id, name: 'present_artifacts', source: 'native', status: 'completed',
        structured_content: { type: 'artifact_presentation', artifactIds: ['art_video'], mode: 'preview' } })),
      segments: ['one', 'two'].map((id, order) => ({ id, kind: 'tool', phase: 'tool_loop', order, tool_call_id: id })),
    }
    render(<MessageBubble message={message} messageStreaming />)
    expect(screen.getAllByRole('button', { name: '打开文件 demo.mp4' })).toHaveLength(1)
  })
})
