import { memo, useEffect, useRef, useState, type CSSProperties, type FocusEvent } from 'react'
import { useT } from '../components/i18n'
import { primaryHeadingDepth, type MarkdownHeadingOutlineItem } from './markdownHeadingOutline'

interface ChatHeadingOutlineProps {
  items: MarkdownHeadingOutlineItem[]
  activeAnchorId: string | null
  onNavigate: (item: MarkdownHeadingOutlineItem) => void
}

function sameItems(a: MarkdownHeadingOutlineItem[], b: MarkdownHeadingOutlineItem[]): boolean {
  return a.length === b.length && a.every((item, index) => {
    const other = b[index]
    return item.anchorId === other.anchorId && item.title === other.title && item.depth === other.depth
  })
}

/**
 * 刻度与标题是同一行：收起时只露出刻度，悬停或聚焦时原地展开标题，
 * 每行位置不变，指针下的刻度就是对应标题，直接点击即可跳转。
 */
function ChatHeadingOutlineBase({
  items,
  activeAnchorId,
  onNavigate,
}: ChatHeadingOutlineProps) {
  const t = useT()
  const listRef = useRef<HTMLDivElement>(null)
  const [open, setOpen] = useState(false)
  const primaryDepth = primaryHeadingDepth(items)

  useEffect(() => {
    const list = listRef.current
    if (!list) return
    const onWheel = (event: WheelEvent) => {
      event.preventDefault()
      event.stopPropagation()
      let delta = event.deltaY
      if (event.deltaMode === 1) delta *= 13
      else if (event.deltaMode === 2) delta *= list.clientHeight
      list.scrollTop += delta
    }
    list.addEventListener('wheel', onWheel, { passive: false })
    return () => list.removeEventListener('wheel', onWheel)
  }, [items.length, primaryDepth])

  const handleBlurCapture = (event: FocusEvent<HTMLElement>) => {
    const next = event.relatedTarget
    if (next instanceof Node && listRef.current?.contains(next)) return
    setOpen(false)
  }

  if (items.length < 2 || primaryDepth == null) return null

  return (
    <aside
      className={`chat-heading-navigator${open ? ' is-expanded' : ''}`}
      aria-label={t.chatHeadingNavigator}
      style={{ ['--heading-count' as string]: String(items.length) } as CSSProperties}
    >
      <div
        ref={listRef}
        className="chat-heading-navigator-list custom-scrollbar"
        onPointerEnter={() => setOpen(true)}
        onPointerLeave={() => setOpen(false)}
        onFocusCapture={() => setOpen(true)}
        onBlurCapture={handleBlurCapture}
      >
        {items.map((item) => {
          const active = item.anchorId === activeAnchorId
          const level = item.depth - primaryDepth
          return (
            <button
              key={item.anchorId}
              type="button"
              className={`chat-heading-navigator-item ${active ? 'is-active' : ''}`}
              style={{ ['--heading-depth' as string]: String(level) } as CSSProperties}
              title={item.title}
              aria-label={t.chatHeadingLabel.replace('{title}', item.title)}
              aria-current={active ? 'location' : undefined}
              onClick={() => onNavigate(item)}
            >
              <span className="chat-heading-navigator-tick" aria-hidden="true" />
              <span className="chat-heading-navigator-title">{item.title}</span>
            </button>
          )
        })}
      </div>
    </aside>
  )
}

export const ChatHeadingOutline = memo(
  ChatHeadingOutlineBase,
  (previous, next) => (
    previous.activeAnchorId === next.activeAnchorId
    && previous.onNavigate === next.onNavigate
    && sameItems(previous.items, next.items)
  ),
)
