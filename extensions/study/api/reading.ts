import { invoke } from '@tauri-apps/api/core'

export type ReadingOpenTarget = {
  view?: { kind: 'source' | 'mono' | 'dual'; runId?: string }
  noteId?: string
  page?: number
}
export type ReadingRect = { x: number; y: number; w: number; h: number }
export type ReadingAnchor = {
  page: number
  text: string
  rects?: ReadingRect[]
  mark?: 'highlight' | 'underline' | 'strike' | 'area'
  color?: string
  rect?: ReadingRect | null
  view?: { kind: 'mono' | 'dual'; runId: string } | null
}
export type ReadingNote = {
  id: string
  content: string
  anchor?: ReadingAnchor | null
  kind: 'highlight' | 'note' | 'answer'
}
export type Bibliography = {
  authors: string[]
  year: string
  publication: string
  doi: string
  url: string
  abstractText: string
  volume?: string
  issue?: string
  pages?: string
  publisher?: string
  isbn?: string
  itemType: string
}
export const emptyBibliography: Bibliography = {
  authors: [],
  year: '',
  publication: '',
  doi: '',
  url: '',
  abstractText: '',
  itemType: 'article',
}
export type ReadingCollection = {
  id: string
  name: string
  parentId: string | null
  deleted?: boolean
}
export type BibliographyLookup = {
  name: string
  bibliography: Bibliography
  tags: string[]
}
export type ReadingDocument = {
  id: string
  name: string
  conversationIds: string[]
  activeConversationId: string | null
  page: number
  offset: number
  notes: ReadingNote[]
  bibliography?: Bibliography
  collectionIds?: string[]
  tags?: string[]
  status?: 'unread' | 'reading' | 'read'
  favorite?: boolean
  trashedAt?: string | null
  addedAt?: string
  lastOpenedAt?: string
  updatedAt?: string
  hasPdf?: boolean
  fingerprint?: string
  noteDraft: string
}
export const readingApi = {
  missingConversations: (ids: string[]) =>
    invoke<string[]>('reading_missing_conversations', { ids }),
  prepareChat: (id: string, conversationId: string, pages: string[] | null, view: { kind: 'source' | 'mono' | 'dual'; runId?: string }) =>
    invoke<{ needsText: boolean; context: { name: string; content: string } | null }>('reading_prepare_chat', { id, conversationId, pages, ...view }),
  text: (id: string, view: { kind: 'source' | 'mono' | 'dual'; runId?: string }, pages?: string[]) =>
    invoke<string[] | null>('reading_text', { id, ...view, pages }),
  collections: () => invoke<ReadingCollection[]>('reading_collections'),
  saveCollections: (items: ReadingCollection[]) =>
    invoke<void>('reading_save_collections', { items }),
  create: (name: string, bibliography: Bibliography) =>
    invoke<ReadingDocument>('reading_create', { name, bibliography }),
  attach: (id: string, path: string) =>
    invoke<ReadingDocument>('reading_attach', { id, path }),
  lookup: (doi: string) =>
    invoke<BibliographyLookup>('reading_lookup', { doi }),
  importRis: (path: string) =>
    invoke<ReadingDocument[]>('reading_import_ris', { path }),
  exportRis: (ids: string[], path: string) =>
    invoke<void>('reading_export_ris', { ids, path }),
  list: () => invoke<{ documents: ReadingDocument[]; issues: string[] }>('reading_list'),
  importMany: (paths: string[]) => invoke<{ documents: ReadingDocument[]; failures: string[] }>('reading_import_many', { paths }),
  import: (path: string) => invoke<ReadingDocument>('reading_import', { path }),
  save: (document: ReadingDocument) =>
    invoke<void>('reading_save', { document }),
  pdf: async (id: string, view?: { kind: 'source' | 'mono' | 'dual'; runId?: string }) =>
    new Uint8Array(await invoke<ArrayBuffer>('reading_pdf', { id, ...view })),
}
