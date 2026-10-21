import type {
  Bibliography,
  ReadingCollection,
  ReadingDocument,
} from '../api/reading'
export type LibraryFilter =
  | 'all'
  | 'favorites'
  | 'trash'
  | 'duplicates'
  | `collection:${string}`
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
const searchText = new WeakMap<ReadingDocument, string>()
export function filterLiterature(
  documents: ReadingDocument[],
  collections: ReadingCollection[],
  filter: LibraryFilter,
  query: string,
  tags: string[],
  unfiledOnly = false,
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
    if (unfiledOnly && doc.collectionIds?.some((id) => active.has(id)))
      return false
    if (filter === 'favorites' && !doc.favorite) return false
    if (filter === 'duplicates' && !duplicates.has(doc.id)) return false
    if (!tags.every((tag) => doc.tags?.includes(tag))) return false
    if (!terms.length) return true
    const b = doc.bibliography
    let text = searchText.get(doc)
    if (text === undefined) {
      text = [
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
      searchText.set(doc, text)
    }
    return terms.every((term) => text!.includes(term))
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
    'volume', 'issue', 'pages', 'publisher', 'isbn',
  ] as const) {
    if (incoming[key]?.trim()) next[key] = incoming[key]
  }
  const authors = incoming.authors.filter((author) => author.trim())
  if (authors.length) next.authors = authors
  return next
}

// Keep the list readable before allocating optional side panels.
export function literatureLayout(width: number, navigation = 240, inspector = 320) {
  const showNavigation = width >= 600
  const showInspector = width >= 900
  const navigationMax = Math.max(200, Math.min(320, width - 360 - (showInspector ? 292 : 6)))
  const navigationWidth = Math.max(200, Math.min(navigationMax, navigation))
  const inspectorMax = Math.max(280, Math.min(420, width - navigationWidth - 372))
  const inspectorWidth = Math.max(280, Math.min(inspectorMax, inspector))
  return { showNavigation, showInspector, navigationWidth, inspectorWidth, navigationMax: showInspector ? Math.min(navigationMax, width - inspectorWidth - 372) : navigationMax, inspectorMax }
}
