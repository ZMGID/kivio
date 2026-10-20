export function readingCitation(
  href: string,
  pageCount: number,
): number | null {
  const match = /(?:^|#)reading-page-(\d+)$/.exec(href)
  const page = match ? Number(match[1]) : 0
  return page >= 1 && page <= pageCount ? page : null
}

/** Explicit references keep a quoted region tied to its PDF edition and translation run. */
export function readingReference(anchor: import('../../api/reading').ReadingAnchor): string {
  const params = new URLSearchParams({ page: String(anchor.page), kind: anchor.view?.kind ?? 'source' })
  if (anchor.view) params.set('run', anchor.view.runId)
  if (anchor.rect) params.set('rect', [anchor.rect.x, anchor.rect.y, anchor.rect.w, anchor.rect.h].join(','))
  return `#reading-ref?${params}`
}

export function readingReferenceAnchor(href: string): import('../../api/reading').ReadingAnchor | null {
  if (!href.startsWith('#reading-ref?')) return null
  const params = new URLSearchParams(href.slice('#reading-ref?'.length))
  const page = Number(params.get('page')), kind = params.get('kind'), runId = params.get('run')
  if (!Number.isSafeInteger(page) || page < 1 || !['source', 'mono', 'dual'].includes(kind ?? '') || (kind !== 'source' && !runId)) return null
  const anchor: import('../../api/reading').ReadingAnchor = { page, text: '', view: kind === 'mono' || kind === 'dual' ? { kind, runId: runId! } : undefined }
  if (params.has('rect')) {
    const values = params.get('rect')!.split(',').map(Number)
    if (values.length !== 4 || !values.every(Number.isFinite)) return null
    const [x, y, w, h] = values
    if (x < 0 || y < 0 || w <= 0 || h <= 0 || x >= 1 || y >= 1 || x + w > 1.001 || y + h > 1.001) return null
    anchor.rect = { x, y, w, h }
  }
  return anchor
}
