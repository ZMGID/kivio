import { useEffect, useRef } from 'react'
import { Minimize2 } from 'lucide-react'
import { TimelineDivider } from './TimelineDivider'
import { i18n, type Lang } from '../components/i18n'

export function CompactionInProgress({ lang = 'zh' }: { lang?: Lang }) {
  const overlay = useRef<HTMLSpanElement>(null)
  useEffect(() => {
    const node = overlay.current
    if (!node) return
    let visible = true
    const update = () => {
      node.style.animationPlayState = visible && !document.hidden ? 'running' : 'paused'
    }
    const observer = typeof IntersectionObserver === 'undefined' ? null : new IntersectionObserver(([entry]) => {
      visible = entry.isIntersecting
      update()
    })
    observer?.observe(node)
    document.addEventListener('visibilitychange', update)
    update()
    return () => {
      observer?.disconnect()
      document.removeEventListener('visibilitychange', update)
    }
  }, [])

  const label = i18n[lang].contextCompressing
  const content = <><Minimize2 size={14} aria-hidden="true" />{label}</>
  return (
    <TimelineDivider pending label={label}>
      <span className="chat-compaction-progress-label">
        {content}
        <span ref={overlay} className="chat-compaction-highlight" aria-hidden="true">{content}</span>
      </span>
    </TimelineDivider>
  )
}
