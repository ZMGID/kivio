import { createContext } from 'react'
import { i18n } from '../components/i18n'
import type { ComposerAttachment } from './composerDraft'
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

function imageReference(label: string, number: number) {
  return `[${label.replace('{n}', String(number))}]`
}

export function textWithImageReferences(text: string, attachments: readonly ComposerAttachment[], label: string) {
  const numbers = imageAttachmentNumbers(attachments)
  const references = attachments.flatMap(attachment => {
    const number = numbers.get(attachment.id)
    return number === undefined ? [] : [{ number, offset: Math.min(attachment.offset ?? text.length, text.length) }]
  }).sort((a, b) => a.offset - b.offset)

  let body = ''
  let from = 0
  for (const { number, offset } of references) {
    body += text.slice(from, offset) + imageReference(label, number)
    from = offset
  }
  return body + text.slice(from)
}

export function restoreImageReferences(
  text: string, attachments: readonly PendingAttachment[],
): { text: string; attachments: readonly ComposerAttachment[] } {
  const numbers = imageAttachmentNumbers(attachments)
  const restored: ComposerAttachment[] = []
  let body = ''
  let from = 0
  for (const attachment of attachments) {
    const number = numbers.get(attachment.id)
    if (number === undefined) {
      restored.push(attachment)
      continue
    }
    const references = Object.values(i18n).map(language => imageReference(language.chatComposerImageReference, number))
    const match = references.map(reference => ({ reference, index: text.indexOf(reference, from) }))
      .filter(({ index }) => index !== -1)
      .sort((a, b) => a.index - b.index)[0]
    if (!match) return { text, attachments }
    body += text.slice(from, match.index)
    restored.push({ ...attachment, offset: body.length })
    from = match.index + match.reference.length
  }
  return { text: body + text.slice(from), attachments: restored }
}
