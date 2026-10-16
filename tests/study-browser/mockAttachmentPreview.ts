// Only the native image-read boundary is replaced; ChatAttachments and the
// common message renderer remain the real production components.
export * from '../../src/chat/attachmentPreview'
import { readFixtureImage } from './mockTransport'
export async function loadAttachmentDataUrl(attachment: { path: string; type: string }) {
  return attachment.type === 'image' ? readFixtureImage(attachment.path) : null
}
