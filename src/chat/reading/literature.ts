import type {
  Bibliography,
  ReadingCollection,
  ReadingDocument,
} from '../../api/reading'
export type LibraryFilter =
  | 'all'
  | 'unfiled'
  | 'unread'
  | 'reading'
  | 'read'
  | 'favorites'
  | 'trash'
  | 'duplicates'
  | `collection:${string}`
export const readingStatuses = [
  { value: 'unread', label: '未读' },
  { value: 'reading', label: '在读' },
  { value: 'read', label: '已读' },
]
export const itemTypes = [
  { value: 'article', label: '期刊文章' },
  { value: 'preprint', label: '预印本' },
  { value: 'conference', label: '会议论文' },
  { value: 'book', label: '图书' },
  { value: 'thesis', label: '学位论文' },
  { value: 'webpage', label: '网页' },
  { value: 'document', label: '其他文献' },
]
export function duplicateIds(documents: ReadingDocument[]) {
  const groups = new Map<string, string[]>()
  for (const doc of documents.filter((d) => !d.trashedAt)) {
    const b = doc.bibliography
    const keys = [
      b?.doi &&
        `doi:${b.doi
          .toLowerCase()
          .replace(/^https?:\/\/doi.org\//, '')
          .trim()}`,
      doc.fingerprint && `file:${doc.fingerprint}`,
      `title:${doc.name.toLowerCase().replace(/[\s\p{P}]/gu, '')}:${b?.year ?? ''}`,
    ].filter(Boolean) as string[]
    for (const key of keys)
      groups.set(key, [...(groups.get(key) ?? []), doc.id])
  }
  return new Set([...groups.values()].filter((ids) => ids.length > 1).flat())
}
export function filterLiterature(
  documents: ReadingDocument[],
  collections: ReadingCollection[],
  filter: LibraryFilter,
  query: string,
  tags: string[],
) {
  const active = new Set(collections.filter((c) => !c.deleted).map((c) => c.id))
  const duplicates =
    filter === 'duplicates' ? duplicateIds(documents) : new Set<string>()
  const terms = query.toLowerCase().trim().split(/\s+/).filter(Boolean)
  return documents.filter((doc) => {
    if (filter === 'trash' ? !doc.trashedAt : !!doc.trashedAt) return false
    if (
      filter.startsWith('collection:') &&
      !doc.collectionIds?.includes(filter.slice(11))
    )
      return false
    if (filter === 'unfiled' && doc.collectionIds?.some((id) => active.has(id)))
      return false
    if (
      ['unread', 'reading', 'read'].includes(filter) &&
      (doc.status ?? 'unread') !== filter
    )
      return false
    if (filter === 'favorites' && !doc.favorite) return false
    if (filter === 'duplicates' && !duplicates.has(doc.id)) return false
    if (!tags.every((tag) => doc.tags?.includes(tag))) return false
    const b = doc.bibliography
    const text = [
      doc.name,
      ...(b?.authors ?? []),
      b?.year,
      b?.publication,
      b?.doi,
      b?.abstractText,
      ...(doc.tags ?? []),
      ...doc.notes.map((n) => n.content),
    ]
      .join(' ')
      .toLowerCase()
    return terms.every((term) => text.includes(term))
  })
}
export function collectionDescendants(
  collections: ReadingCollection[],
  id: string,
): Set<string> {
  const result = new Set([id])
  let previous = 0
  while (result.size !== previous) {
    previous = result.size
    for (const c of collections)
      if (c.parentId && result.has(c.parentId)) result.add(c.id)
  }
  return result
}

export function mergeBibliography(
  current: Bibliography,
  incoming: Bibliography,
): Bibliography {
  const next = { ...current }
  for (const key of [
    'year',
    'publication',
    'doi',
    'url',
    'abstractText',
    'itemType',
  ] as const) {
    if (incoming[key].trim()) next[key] = incoming[key]
  }
  const authors = incoming.authors.filter((author) => author.trim())
  if (authors.length) next.authors = authors
  return next
}
