import { createContext } from 'react'
import type { PendingAttachment } from './types'

/** File attachments use a compact row in embedded reading workspaces. */
export const AttachmentPresentationContext = createContext<'card' | 'compact'>('card')

export function imageAttachmentNumbers(attachments: readonly Pick<PendingAttachment, 'id' | 'type'>[]) {
  const numbers = new Map<string, number>()
  for (const attachment of attachments) {
    if (attachment.type === 'image') numbers.set(attachment.id, numbers.size + 1)
  }
  return numbers
}
