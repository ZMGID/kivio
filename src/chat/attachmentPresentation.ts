import { createContext } from 'react'

/** File attachments use a compact row in embedded reading workspaces. */
export const AttachmentPresentationContext = createContext<'card' | 'compact'>('card')
