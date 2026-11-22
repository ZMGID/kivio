import { useLayoutEffect, useRef, useState, type ReactElement } from 'react'

// Keep one continuous conversation scrollbar. Offscreen process rows reserve their
// measured height so scrolling back to a tool doesn't require opening a new page.
const rowsPerBlock = 16

function WorkBlock({ rows, estimatedHeight, liveTail }: {
  rows: ReactElement[]
  estimatedHeight: number
  liveTail: boolean
}) {
  const block = useRef<HTMLDivElement>(null)
  const measured = useRef<number | undefined>(undefined)
  const [visible, setVisible] = useState(liveTail || typeof IntersectionObserver === 'undefined')
  const [focused, setFocused] = useState(false)
  const anchor = useRef<{ viewport: HTMLElement; height: number } | null>(null)
  useLayoutEffect(() => {
    const element = block.current
    if (!element || liveTail || typeof IntersectionObserver === 'undefined') return
    const viewport = element.closest<HTMLElement>('[data-work-scroll], .chat-message-group-col-body, .chat-scroll-viewport')
    if (!viewport) {
      setVisible(true)
      return
    }
    const observer = new IntersectionObserver(([entry]) => {
      if (!entry.isIntersecting && element.firstElementChild) {
        measured.current = element.getBoundingClientRect().height
      }
      if (entry.isIntersecting && !element.firstElementChild
        && entry.boundingClientRect.bottom <= viewport.getBoundingClientRect().top) {
        anchor.current = { viewport, height: entry.boundingClientRect.height }
      }
      setVisible(entry.isIntersecting)
    }, { root: viewport, rootMargin: '100% 0px' })
    observer.observe(element)
    return () => observer.disconnect()
  }, [liveTail])
  useLayoutEffect(() => {
    const saved = anchor.current
    anchor.current = null
    if (saved && visible && block.current) {
      saved.viewport.scrollTop += block.current.getBoundingClientRect().height - saved.height
    }
  }, [visible])
  const mounted = liveTail || visible || focused
  return <div ref={block} className="space-y-1.5" data-chat-work-block={String(rows[0].key)}
    data-chat-work-sources={rows.flatMap(row => {
      const source = (row.props as { 'data-chat-work-source'?: string })['data-chat-work-source']
      return source ? [encodeURIComponent(source)] : []
    }).join(' ') || undefined}
    onFocusCapture={() => setFocused(true)}
    onBlurCapture={event => {
      if (event.relatedTarget instanceof Node && event.currentTarget.contains(event.relatedTarget)) return
      if (!visible) measured.current = event.currentTarget.getBoundingClientRect().height
      setFocused(false)
    }}
    style={mounted ? undefined : { height: measured.current ?? estimatedHeight }}>
    {mounted && rows}
  </div>
}

// Keep interactive deliveries and answers in the normal timeline. Only batch
// consecutive process rows, leaving the leading header and live tail mounted.
export function WorkTimelineWindow({ rows, canDefer, rowHeight }: {
  rows: ReactElement[]
  canDefer?: (row: ReactElement) => boolean
  rowHeight: number
}) {
  if (rows.length <= rowsPerBlock) return rows
  const result: ReactElement[] = []
  let start = 0
  while (start < rows.length) {
    if (canDefer && !canDefer(rows[start])) {
      result.push(rows[start++])
      continue
    }
    let end = start + 1
    while (end < rows.length && (!canDefer || canDefer(rows[end]))) end++
    const firstBlockEnd = Math.min(start + rowsPerBlock, end)
    result.push(...rows.slice(start, firstBlockEnd))
    for (let index = firstBlockEnd; index < end; index += rowsPerBlock) {
      const block = rows.slice(index, Math.min(index + rowsPerBlock, end))
      result.push(<WorkBlock key={'work-block-' + block[0].key} rows={block}
        estimatedHeight={rowHeight * block.length} liveTail={end - start <= rowsPerBlock * 3 || index + rowsPerBlock >= end} />)
    }
    start = end
  }
  return result
}
