import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ChatMarkdown } from './ChatMarkdown'
import { clearSettledMarkdownCache, settledMarkdownCacheSize } from './settledMarkdownCache'

describe('ChatMarkdown settled cache', () => {
  it('reuses a bounded settled normalization entry across remounts', () => {
    clearSettledMarkdownCache()
    const content = 'A settled answer with **markdown**.'
    const first = render(<ChatMarkdown content={content} />)
    first.unmount()
    render(<ChatMarkdown content={content} />)
    expect(settledMarkdownCacheSize()).toBe(1)
  })
})

// 回归测试：ChatMarkdown 因 props 变化（如 artifacts/citations 换引用）重渲时，公式节点
// 仍应保留在文档中，避免切换对话时出现「原始 LaTeX → 公式」的闪烁。
describe('ChatMarkdown 公式稳定性', () => {
  it('artifacts 换引用重渲时，公式节点不被 remount', () => {
    const { container, rerender } = render(
      <ChatMarkdown content={'目标函数 $Z_1$ 最小化'} artifacts={[]} />,
    )
    const before = container.querySelector('.katex')
    expect(before).not.toBeNull()

    // 模拟切模型/思考等级时上层重渲传入的新 artifacts 引用（内容不变）。
    rerender(<ChatMarkdown content={'目标函数 $Z_1$ 最小化'} artifacts={[]} />)
    const after = container.querySelector('.katex')

    expect(after).toBe(before) // 同一个 DOM 节点 = 未 remount
  })
})

describe('ChatMarkdown artifact 图片', () => {
  it('places files and images by exact ID and resolves artifacts arriving after the text', () => {
    const content = 'Before\n\n![Selected](artifact:art_second)\n\nBetween\n\n[Video](artifact:art_video)\n\nAfter'
    const { container, rerender } = render(<ChatMarkdown content={content} artifacts={[]} />)
    expect(screen.getAllByRole('status')).toHaveLength(2)
    rerender(<ChatMarkdown content={content} artifacts={[
      { id: 'art_first', name: 'same.png', mime_type: 'image/png', data_url: 'data:image/png;base64,AAAA' },
      { id: 'art_second', name: 'same.png', mime_type: 'image/png', data_url: 'data:image/png;base64,BBBB' },
      { id: 'art_video', name: 'demo.mp4', mime_type: 'video/mp4', path: '/work/demo.mp4' },
    ]} />)
    const image = container.querySelector('img')!
    const video = screen.getByRole('button', { name: '打开文件 demo.mp4' })
    expect(image).toHaveAttribute('src', 'data:image/png;base64,BBBB')
    expect(image.compareDocumentPosition(screen.getByText('Between')) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    expect(screen.getByText('Between').compareDocumentPosition(video) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    expect(video.compareDocumentPosition(screen.getByText('After')) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    expect(screen.queryByRole('status')).not.toBeInTheDocument()
    expect(container.querySelector('p div')).toBeNull()
    fireEvent.click(video)
  })

  it('does not send malformed artifact references to local file handling', () => {
    render(<ChatMarkdown content={'[Bad file](artifact:../../secrets)'} />)
    expect(screen.getByRole('status')).toHaveTextContent('文件不可用')
    expect(screen.queryByRole('link')).not.toBeInTheDocument()
  })

  it('renders reference-style artifact links as file cards', () => {
    render(<ChatMarkdown content={'[Report][result]\n\n[result]: artifact:art_report'} artifacts={[
      { id: 'art_report', name: 'report.pdf', mime_type: 'application/pdf', path: '/work/report.pdf' },
    ]} />)
    expect(screen.getByRole('button', { name: '打开文件 report.pdf' })).toBeVisible()
  })

  it('Streamdown 清洗前保留相对图片路径，并映射到 artifact data URL', () => {
    const { container } = render(
      <ChatMarkdown
        content={'![结果图](assets/chart.png)'}
        artifacts={[{
          name: 'assets/chart.png',
          mimeType: 'image/png',
          dataUrl: 'data:image/png;base64,AAAA',
        }]}
      />,
    )

    expect(container.querySelector('img')).toHaveAttribute('src', 'data:image/png;base64,AAAA')
  })

  it('renders consecutive markdown images with their corresponding artifacts', () => {
    const { container } = render(
      <ChatMarkdown
        content={'![a](one.png)\n\n![b](two.png)'}
        artifacts={[
          { name: 'one.png', mimeType: 'image/png', dataUrl: 'data:image/png;base64,AAAA' },
          { name: 'two.png', mimeType: 'image/png', dataUrl: 'data:image/png;base64,BBBB' },
        ]}
      />,
    )
    const images = container.querySelectorAll('img')
    expect(images).toHaveLength(2)
    expect(images[0]).toHaveAttribute('src', 'data:image/png;base64,AAAA')
    expect(images[1]).toHaveAttribute('src', 'data:image/png;base64,BBBB')
  })
})

describe('ChatMarkdown 标题目录锚点', () => {
  it('registers a settled answer outline and renders stable heading ids', async () => {
    const onChange = vi.fn()
    const { container, unmount } = render(
      <ChatMarkdown
        content={'# Start\n\n## Next'}
        outlineSource={{ ownerMessageId: 'answer-1', sourceId: 'answer-1', onChange }}
      />,
    )
    await waitFor(() => {
      expect(container.querySelector('h1')).toHaveAttribute('id', 'user-content-chat-heading-answer-1-0')
      expect(container.querySelector('h2')).toHaveAttribute('id', 'user-content-chat-heading-answer-1-1')
      expect(onChange).toHaveBeenCalledWith(expect.objectContaining({
        ownerMessageId: 'answer-1',
        sourceId: 'answer-1',
        items: expect.arrayContaining([
          expect.objectContaining({ title: 'Start', depth: 1 }),
          expect.objectContaining({ title: 'Next', depth: 2 }),
        ]),
      }))
    })
    unmount()
    expect(onChange).toHaveBeenLastCalledWith({ ownerMessageId: 'answer-1', sourceId: 'answer-1', items: null })
  })
})
