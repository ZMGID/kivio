import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ChatImageViewer } from './ChatImageViewer'

const readOriginal = vi.hoisted(() => vi.fn())
const copy = vi.hoisted(() => vi.fn())
vi.mock('./attachmentPreview', () => ({ loadArtifactDataUrl: readOriginal, loadArtifactOriginalDataUrl: readOriginal }))
vi.mock('../api/tauri', () => ({ api: { lensCopyImageToClipboard: copy } }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: vi.fn() }))

beforeEach(() => { readOriginal.mockReset(); copy.mockReset().mockResolvedValue({ success: true }) })

describe('ChatImageViewer original export', () => {
  const item = { src: 'data:image/png;base64,VEhVTUI=', path: 'original.png', conversationId: 'image-conversation', name: 'original.png' }

  it('waits for the original before copying', async () => {
    let resolve!: (src: string) => void
    readOriginal.mockReturnValue(new Promise(r => { resolve = r }))
    render(<ChatImageViewer item={item} onClose={() => {}} />)
    const button = screen.getByRole('button', { name: '复制图片' })
    expect(button).toBeDisabled()
    resolve('data:image/png;base64,T1JJR0lOQUw=')
    await waitFor(() => expect(button).toBeEnabled())
    fireEvent.click(button)
    await waitFor(() => expect(copy).toHaveBeenCalledWith('T1JJR0lOQUw='))
  })

  it('shows original read failure and keeps export disabled', async () => {
    readOriginal.mockResolvedValue(null)
    render(<ChatImageViewer item={item} onClose={() => {}} />)
    expect(await screen.findByText('无法读取原图')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '复制图片' })).toBeDisabled()
    expect(screen.getByRole('button', { name: '图片另存为' })).toBeDisabled()
  })
})
