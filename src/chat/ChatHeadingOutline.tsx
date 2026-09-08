import { memo, useEffect, useMemo, useRef, useState, type FocusEvent } from 'react'
import { ChevronDown, ChevronUp } from 'lucide-react'
import { IconButton } from '../components/Button'
import { useT } from '../settings/i18n'
import { primaryHeadingDepth, type MarkdownHeadingOutlineItem } from './markdownHeadingOutline'

interface ChatHeadingOutlineProps {
  ownerMessageId: string
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

function ChatHeadingOutlineBase({
  ownerMessageId,
  items,
  activeAnchorId,
  onNavigate,
}: ChatHeadingOutlineProps) {
  const t = useT()
  const shellRef = useRef<HTMLElement>(null)
  const panelRef = useRef<HTMLDivElement>(null)
  const closeTimerRef = useRef<number | null>(null)
  const [open, setOpen] = useState(false)
  const [showChildren, setShowChildren] = useState(false)
  const primaryDepth = primaryHeadingDepth(items)
  const hasChildren = primaryDepth != null && items.some((item) => item.depth > primaryDepth)
  const visibleItems = useMemo(
    () => showChildren || primaryDepth == null
      ? items
      : items.filter((item) => item.depth === primaryDepth),
    [items, primaryDepth, showChildren],
  )

  useEffect(() => {
    setShowChildren(false)
  }, [ownerMessageId])

  useEffect(() => () => {
    if (closeTimerRef.current != null) window.clearTimeout(closeTimerRef.current)
  }, [])

  useEffect(() => {
    const panel = panelRef.current
    if (!panel) return
    const onWheel = (event: WheelEvent) => {
      event.preventDefault()
      event.stopPropagation()
      let delta = event.deltaY
      if (event.deltaMode === 1) delta *= 13
      else if (event.deltaMode === 2) delta *= panel.clientHeight
      panel.scrollTop += delta
    }
    panel.addEventListener('wheel', onWheel, { passive: false })
    return () => panel.removeEventListener('wheel', onWheel)
  }, [])

  const cancelClose = () => {
    if (closeTimerRef.current == null) return
    window.clearTimeout(closeTimerRef.current)
    closeTimerRef.current = null
  }

  const openRail = () => {
    cancelClose()
    setOpen(true)
  }

  const closeRailLater = () => {
    cancelClose()
    closeTimerRef.current = window.setTimeout(() => {
      closeTimerRef.current = null
      setOpen(false)
    }, 200)
  }

  const handleBlurCapture = (event: FocusEvent<HTMLElement>) => {
    const next = event.relatedTarget
    if (next instanceof Node && shellRef.current?.contains(next)) return
    closeRailLater()
  }

  if (items.length < 2 || primaryDepth == null) return null

  return (
    <aside
      ref={shellRef}
      className={`chat-heading-navigator${open ? ' is-expanded' : ''}`}
      aria-label={t.chatHeadingNavigator}
      onPointerEnter={openRail}
      onPointerLeave={closeRailLater}
      onFocusCapture={openRail}
      onBlurCapture={handleBlurCapture}
    >
      <div className="chat-heading-navigator-rail">
        {items.map((item) => {
          const active = item.anchorId === activeAnchorId
          return (
            <button
              key={item.anchorId}
              type="button"
              className={`chat-heading-navigator-tick ${active ? 'is-active' : ''}`}
              aria-current={active ? 'location' : undefined}
              aria-label={t.chatHeadingLabel.replace('{title}', item.title)}
              onClick={() => onNavigate(item)}
            >
              <span style={{ ['--heading-depth' as string]: String(item.depth - primaryDepth) }} />
            </button>
          )
        })}
      </div>

      <div
        ref={panelRef}
        className="chat-heading-navigator-panel custom-scrollbar"
        aria-hidden={!open}
      >
        {hasChildren && (
          <IconButton
            className="chat-heading-navigator-level-toggle"
            size="xs"
            variant="ghost"
            label={showChildren ? t.chatHeadingCollapseLevels : t.chatHeadingExpandLevels}
            aria-pressed={showChildren}
            tabIndex={open ? 0 : -1}
            onClick={() => setShowChildren((value) => !value)}
          >
            {showChildren ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
          </IconButton>
        )}
        {visibleItems.map((item, index) => {
          const active = item.anchorId === activeAnchorId
          return (
            <button
              key={item.anchorId}
              type="button"
              className={`chat-heading-navigator-item ${active ? 'is-active' : ''} ${index === 0 && hasChildren ? 'has-level-toggle' : ''}`}
              style={{ ['--heading-indent' as string]: `${(item.depth - primaryDepth) * 14}px` }}
              title={item.title}
              aria-current={active ? 'location' : undefined}
              tabIndex={open ? 0 : -1}
              onClick={() => onNavigate(item)}
            >
              <span>{item.title}</span>
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
    previous.ownerMessageId === next.ownerMessageId
    && previous.activeAnchorId === next.activeAnchorId
    && previous.onNavigate === next.onNavigate
    && sameItems(previous.items, next.items)
  ),
)
