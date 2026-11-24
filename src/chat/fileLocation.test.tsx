import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { fileLocationAction } from './fileLocation'
import { ArtifactFileChip } from './GeneratedFileArtifacts'
import { ChatMarkdown } from './ChatMarkdown'
import { ChatAttachments } from './ChatAttachments'
import { AppNoticeHost } from '../components/AppNotice'

const invoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }))
vi.mock('./attachmentPreview', async (original) => ({
  ...await original<typeof import('./attachmentPreview')>(),
  loadAttachmentDataUrl: () => Promise.resolve('data:image/png;base64,AAAA'),
}))
beforeEach(() => invoke.mockReset().mockResolvedValue(undefined))

describe('file location actions', () => {
  it.each(['C:\\测试文件\\报告.md', '/tmp/report.md', '\\\\server\\share\\report.md'])('reveals absolute file %s without a conversation ID', async path => {
    await fileLocationAction(path, 'conv_test')!()
    expect(invoke).toHaveBeenCalledWith('chat_reveal_generated_artifact', { path })
  })
  it('resolves saved attachment filenames inside the conversation', async () => {
    await fileLocationAction('saved.png', 'conv_test')!()
    expect(invoke).toHaveBeenCalledWith('chat_reveal_attachment', { path: 'saved.png', conversationId: 'conv_test' })
  })
  it.each(['', 'memory://note.txt', 'data:image/png;base64,AAAA', 'https://example.com/a.png', '../file', '..', 'file.png'])('does not reveal non-local or unresolved path %s', path => {
    expect(fileLocationAction(path)).toBeUndefined()
  })
  it('opens the file menu without triggering the message menu or default file open', () => {
    const parent = vi.fn()
    render(<div onContextMenu={parent}><ArtifactFileChip artifact={{ name: '报告.md', path: '/tmp/报告.md' }} /></div>)
    fireEvent.contextMenu(screen.getByRole('button', { name: '打开文件 报告.md' }))
    expect(parent).not.toHaveBeenCalled()
    expect(invoke).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('menuitem', { name: '打开所在位置' }))
    expect(invoke).toHaveBeenCalledWith('chat_reveal_generated_artifact', { path: '/tmp/报告.md' })
  })
  it.each([
    ['/C:/Work/Kivio/output/tool-call-screenshots/expanded.png', 'C:/Work/Kivio/output/tool-call-screenshots/expanded.png'],
    ['file:///tmp/test%20report.pdf', '/tmp/test report.pdf'],
    ['src/chat/ChatMarkdown.tsx:1004', 'src/chat/ChatMarkdown.tsx'],
  ])('locates the file behind a Markdown link (%s)', (href, path) => {
    const parent = vi.fn()
    render(<div onContextMenu={parent}>
      <ChatMarkdown content={`[展开命令详情](${href})`} conversationId="conv_test" />
    </div>)
    fireEvent.contextMenu(screen.getByRole('link', { name: /展开命令详情/ }))
    fireEvent.click(screen.getByRole('menuitem', { name: '打开所在位置' }))
    expect(parent).not.toHaveBeenCalled()
    expect(invoke.mock.calls).toEqual([['reveal_local_file', { path, conversationId: 'conv_test' }]])
  })
  it('keeps the message menu available when right-clicking a web link', () => {
    const parent = vi.fn()
    render(<div onContextMenu={parent}>
      <ChatMarkdown content="[官网](https://example.com/a.png)" conversationId="conv_test" />
    </div>)
    fireEvent.contextMenu(screen.getByRole('link', { name: /官网/ }))
    expect(parent).toHaveBeenCalledOnce()
    expect(screen.queryByRole('menuitem', { name: '打开所在位置' })).not.toBeInTheDocument()
  })
  it('keeps the actual file path on a Markdown image whose display source is a data URL', () => {
    render(<ChatMarkdown content="![图表](artifact:art_image)" conversationId="conv_test" artifacts={[
      { id: 'art_image', name: '图表.png', path: 'saved.png', data_url: 'data:image/png;base64,AAAA', mime_type: 'image/png' },
    ]} />)
    fireEvent.contextMenu(screen.getByRole('button', { name: '预览图片' }))
    fireEvent.click(screen.getByRole('menuitem', { name: '打开所在位置' }))
    expect(invoke).toHaveBeenCalledWith('chat_reveal_attachment', { path: 'saved.png', conversationId: 'conv_test' })
  })
  it.each(['image', 'file'] as const)('reveals uploaded %s attachments', async type => {
    render(<ChatAttachments attachments={[{ id: 'uploaded', name: 'upload.png', path: '/tmp/upload.png', type }]} variant="user" />)
    const button = await screen.findByRole('button', { name: type === 'image' ? '预览图片' : 'upload.png' })
    fireEvent.contextMenu(button)
    fireEvent.click(screen.getByRole('menuitem', { name: '打开所在位置' }))
    expect(invoke).toHaveBeenCalledWith('chat_reveal_generated_artifact', { path: '/tmp/upload.png' })
  })
  it('reports a file that has been removed instead of silently failing', async () => {
    invoke.mockImplementation(command => command === 'chat_reveal_generated_artifact'
      ? Promise.reject(new Error('missing file')) : Promise.resolve(undefined))
    render(<><ArtifactFileChip artifact={{ name: '报告.md', path: '/tmp/报告.md' }} /><AppNoticeHost /></>)
    fireEvent.contextMenu(screen.getByRole('button', { name: '打开文件 报告.md' }))
    fireEvent.click(screen.getByRole('menuitem', { name: '打开所在位置' }))
    await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent('无法打开所在位置'))
    fireEvent.click(screen.getByRole('button', { name: '关闭提示' }))
  })
})
