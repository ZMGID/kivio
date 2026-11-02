import { fireEvent, render, screen } from '@testing-library/react'
import { convertFileSrc } from '@tauri-apps/api/core'
import type * as TauriCore from '@tauri-apps/api/core'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { api } from '../api/tauri'
import { MessageBubble } from './MessageBubble'
import type { ChatMessage } from './types'

vi.mock('@tauri-apps/api/core', async (importOriginal) => ({
  ...await importOriginal<typeof TauriCore>(),
  convertFileSrc: vi.fn((path: string, protocol?: string) => `${protocol}://localhost/${encodeURIComponent(path)}`),
}))

const attachmentId = 'htmlrender-12345678-1234-1234-1234-123456789abc'
const message: ChatMessage = {
  id: 'html-delivery', role: 'assistant', timestamp: 1, content: 'Final answer', stream_outcome: 'completed',
  tool_calls: [
    { id: 'read', name: 'read_file', source: 'native', status: 'completed' },
    { id: 'render', name: 'mcp__kivio__html_render', source: 'external_cli', status: 'completed',
      structured_content: { type: 'html_render', attachmentId, title: 'Interactive chart', height: 480 } },
    { id: 'preview', name: 'mcp__kivio__html_preview', source: 'external_cli', status: 'completed' },
  ],
  segments: [
    { id: 'read-segment', kind: 'tool', phase: 'tool_loop', order: 0, tool_call_id: 'read' },
    { id: 'render-segment', kind: 'tool', phase: 'tool_loop', order: 1, tool_call_id: 'render' },
    { id: 'preview-segment', kind: 'tool', phase: 'tool_loop', order: 2, tool_call_id: 'preview' },
    { id: 'answer', kind: 'text', phase: 'plain', order: 3, text: 'Final answer' },
  ],
}

afterEach(() => vi.restoreAllMocks())

describe('HTML render timeline', () => {
  it('shows completed external CLI pages outside Work at their timeline position', () => {
    const open = vi.spyOn(api, 'openHtmlRender').mockResolvedValue()
    const { container } = render(<MessageBubble message={message} conversationId="conv_1" />)
    const frame = screen.getByTitle('Interactive chart') as HTMLIFrameElement
    const work = screen.getByRole('button', { name: /^Worked/ })
    const answer = screen.getByText('Final answer')
    expect(frame.getAttribute('src')).toContain(`kivio-preview://localhost/${encodeURIComponent(`render/conv_1/${attachmentId}`)}#kivio-theme=`)
    expect(convertFileSrc).toHaveBeenCalledWith(`render/conv_1/${attachmentId}`, 'kivio-preview')
    expect(frame.style.height).toBe('480px')
    expect(frame.closest('[aria-label="过程分组"]')).toBeNull()
    expect(work.compareDocumentPosition(frame) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    expect(frame.compareDocumentPosition(answer) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    expect(work).toHaveAttribute('aria-expanded', 'false')
    fireEvent.click(work)
    const clusters = container.querySelectorAll<HTMLButtonElement>('[data-tool-call-cluster] > button')
    fireEvent.click(clusters[clusters.length - 1])
    expect(screen.getByText('Preview HTML').closest('[aria-label="过程分组"]')).not.toBeNull()
    expect(frame).toBeVisible()
    fireEvent.click(screen.getByRole('button', { name: '在浏览器打开' }))
    expect(open).toHaveBeenCalledWith('conv_1', attachmentId, expect.stringContaining('--chart-1:'))
  })

  it.each(['../other', 'htmlrender-../../private', 'htmlrender-12345678-1234-1234-1234-123456789abz'])(
    'does not build an unsafe URL or iframe for attachment ID %s', (invalid) => {
      vi.mocked(convertFileSrc).mockClear()
      const invalidMessage: ChatMessage = { ...message, tool_calls: message.tool_calls!.map(tool => tool.id === 'render'
        ? { ...tool, structured_content: { type: 'html_render', attachmentId: invalid, title: 'Unsafe', height: 480 } }
        : tool) }
      const { container } = render(<MessageBubble message={invalidMessage} conversationId="conv_1" />)
      expect(container.querySelector('iframe')).toBeNull()
      expect(convertFileSrc).not.toHaveBeenCalled()
      expect(api.htmlRenderUrl('conv_1', invalid)).toBeNull()
    },
  )

  it('rejects invalid conversation IDs before building a URL', () => {
    vi.mocked(convertFileSrc).mockClear()
    expect(api.htmlRenderUrl('conv_../other', attachmentId)).toBeNull()
    expect(convertFileSrc).not.toHaveBeenCalled()
  })
})
