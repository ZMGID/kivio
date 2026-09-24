import { beforeEach, describe, expect, it, vi } from 'vitest'
import { loadAttachmentDataUrl, loadArtifactDataUrl, loadArtifactOriginalDataUrl } from './attachmentPreview'

const invoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke }))
vi.mock('./utils', () => ({ isTauriRuntime: () => true }))

beforeEach(() => { invoke.mockReset() })

describe('attachment preview reads', () => {
  it('shares an in-flight read and reuses a stable sent attachment', async () => {
    let resolve!: (result: { success: boolean; data: string }) => void
    invoke.mockReturnValue(new Promise(r => { resolve = r }))
    const attachment = { path: 'stable-image-a.png', name: 'image', type: 'image' as const }
    const first = loadAttachmentDataUrl(attachment, 'conversation-a')
    const second = loadArtifactDataUrl({ path: attachment.path }, 'conversation-a')
    expect(invoke).toHaveBeenCalledTimes(1)
    resolve({ success: true, data: 'data:image/png;base64,AAAA' })
    expect(await first).toBe('data:image/png;base64,AAAA')
    expect(await second).toBe('data:image/png;base64,AAAA')
    expect(await loadAttachmentDataUrl(attachment, 'conversation-a')).toBe('data:image/png;base64,AAAA')
    expect(invoke).toHaveBeenCalledTimes(1)
    await loadAttachmentDataUrl(attachment, 'conversation-b')
    expect(invoke).toHaveBeenCalledTimes(2)
  })

  it('does not retain mutable absolute paths and retries failures', async () => {
    invoke.mockResolvedValueOnce({ success: false }).mockResolvedValueOnce({ success: true, data: 'first' })
      .mockResolvedValueOnce({ success: true, data: 'second' })
    const attachment = { path: '/tmp/mutable-image-a.png', name: 'image', type: 'image' as const }
    expect(await loadAttachmentDataUrl(attachment)).toBeNull()
    expect(await loadAttachmentDataUrl(attachment)).toBe('first')
    expect(await loadAttachmentDataUrl(attachment)).toBe('second')
    expect(invoke).toHaveBeenCalledTimes(3)
  })

  it('reports original read failure separately from the display thumbnail fallback', async () => {
    invoke.mockResolvedValue({ success: false })
    const artifact = { path: 'unreadable-original.png', dataUrl: 'data:image/png;base64,VEhVTUI=' }
    expect(await loadArtifactOriginalDataUrl(artifact, 'conversation-a')).toBeNull()
    expect(await loadArtifactDataUrl(artifact, 'conversation-a')).toBe(artifact.dataUrl)
  })

  it('does not cache a payload larger than the byte budget', async () => {
    const oversized = `data:image/png;base64,${'A'.repeat(13 * 1024 * 1024)}`
    invoke.mockResolvedValue({ success: true, data: oversized })
    const attachment = { path: 'oversized-image-a.png', name: 'image', type: 'image' as const }
    await loadAttachmentDataUrl(attachment, 'conversation-a')
    await loadAttachmentDataUrl(attachment, 'conversation-a')
    expect(invoke).toHaveBeenCalledTimes(2)
  })

  it('evicts old previews when their combined bytes exceed the budget', async () => {
    const payload = `data:image/png;base64,${'B'.repeat(7 * 1024 * 1024)}`
    invoke.mockResolvedValue({ success: true, data: payload })
    const first = { path: 'cache-first.png', name: 'first', type: 'image' as const }
    const second = { path: 'cache-second.png', name: 'second', type: 'image' as const }
    await loadAttachmentDataUrl(first, 'cache-conversation')
    await loadAttachmentDataUrl(second, 'cache-conversation')
    await loadAttachmentDataUrl(first, 'cache-conversation')
    expect(invoke).toHaveBeenCalledTimes(3)
  })
})
