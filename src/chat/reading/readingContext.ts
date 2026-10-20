export function readingCitation(
  href: string,
  pageCount: number,
): number | null {
  const match = /(?:^|#)reading-page-(\d+)$/.exec(href)
  const page = match ? Number(match[1]) : 0
  return page >= 1 && page <= pageCount ? page : null
}
