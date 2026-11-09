import { useEffect, useId, useRef, useState, useSyncExternalStore } from 'react'
import { createPortal } from 'react-dom'
import { AlertCircle, Check, Info, X } from 'lucide-react'
import { Button, IconButton } from './Button'
import type { Lang } from './i18n'
import { dismissNotice, getNotices, publishNotice, subscribeNotices, type AppNotice } from './appNoticeStore'

/** 退场（--kv-dur-normal）的兜底时长：animationend 未触发时仍会卸载。 */
const NOTICE_EXIT_MS = 260

type NoticeProps = {
  message: string | null | undefined
  tone?: 'error' | 'warning' | 'success' | 'info'
  duration?: number
  actionLabel?: string
  onAction?: () => void
}

/** State belongs to the caller; presentation and dismissal use the shared host. */
export function OperationNotice({ message, tone = 'info', duration = 0, actionLabel, onAction }: NoticeProps) {
  const id = useId()
  const actionRef = useRef(onAction)
  actionRef.current = onAction
  useEffect(() => {
    if (!message) return
    publishNotice({ id, message, tone, actionLabel, onAction: actionLabel ? () => actionRef.current?.() : undefined })
    const timer = duration > 0 ? setTimeout(() => dismissNotice(id), duration) : undefined
    return () => { clearTimeout(timer); dismissNotice(id) }
  }, [id, message, tone, duration, actionLabel])
  return null
}

export function ErrorNotice(props: Omit<NoticeProps, 'tone' | 'duration'> & { tone?: 'error' | 'warning' }) {
  return <OperationNotice {...props} tone={props.tone ?? 'error'} />
}

export function AppNoticeHost({ lang = 'zh' }: { lang?: Lang }) {
  const notices = useSyncExternalStore(subscribeNotices, getNotices, getNotices)
  const [rendered, setRendered] = useState<RenderedNotice[]>(() => notices.map(notice => ({ notice, leaving: false })))
  const [synced, setSynced] = useState(notices)
  if (synced !== notices) {
    setSynced(notices)
    setRendered(prev => mergeNotices(prev, notices))
  }
  const hasLeaving = rendered.some(item => item.leaving)
  const removeLeaving = (id: string) => setRendered(prev => prev.filter(item => !(item.leaving && item.notice.id === id)))
  useEffect(() => {
    if (!hasLeaving) return
    // animationend 未触发（后台窗口、测试环境）时的兜底。
    const timer = window.setTimeout(() => setRendered(prev => prev.filter(item => !item.leaving)), NOTICE_EXIT_MS)
    return () => window.clearTimeout(timer)
  }, [hasLeaving, rendered])

  return createPortal(
    <aside className="kv-notices custom-scrollbar" aria-label={lang === 'zh' ? '操作提示' : 'Notifications'} data-tauri-drag-region="false">
      {rendered.map(({ notice, leaving }) => {
        const compact = notice.message.replace(/\s+/g, ' ').trim()
        const chars = Array.from(compact)
        const summary = chars.length > 100 ? `${chars.slice(0, 100).join('')}…` : compact
        const Icon = notice.tone === 'success' ? Check : notice.tone === 'info' ? Info : AlertCircle
        return <div
          key={notice.id}
          className={`kv-notice-slot${leaving ? ' is-leaving' : ''}`}
          aria-hidden={leaving || undefined}
          onAnimationEnd={(event) => {
            if (leaving && event.target === event.currentTarget) removeLeaving(notice.id)
          }}
        ><div className={`kv-notice ${notice.tone}`}>
          <Icon className="kv-notice-icon" size={16} aria-hidden="true" />
          <div className="kv-notice-copy">
            <p role={leaving ? undefined : notice.tone === 'error' || notice.tone === 'warning' ? 'alert' : 'status'} className="kv-notice-summary" title={notice.message}>{summary}</p>
            {chars.length > 60 && <details>
              <summary>{lang === 'zh' ? '查看详情' : 'Details'}</summary>
              <p className="kv-notice-detail custom-scrollbar">{notice.message}</p>
            </details>}
            {notice.actionLabel && <Button size="sm" variant="ghost" tabIndex={leaving ? -1 : undefined} onClick={notice.onAction}>{notice.actionLabel}</Button>}
          </div>
          <IconButton variant="ghost" size="xs" tabIndex={leaving ? -1 : undefined} label={lang === 'zh' ? '关闭提示' : 'Dismiss notification'} onClick={() => dismissNotice(notice.id)}><X size={14} /></IconButton>
        </div></div>
      })}
    </aside>, document.body,
  )
}

type RenderedNotice = { notice: AppNotice; leaving: boolean }

/** 已移除的提示留在原位播退场；仍在的提示取最新内容，新提示追加在末尾。 */
function mergeNotices(prev: RenderedNotice[], current: AppNotice[]): RenderedNotice[] {
  const byId = new Map(current.map(notice => [notice.id, notice]))
  const merged = prev.map(item => {
    const next = byId.get(item.notice.id)
    return next ? { notice: next, leaving: false } : { notice: item.notice, leaving: true }
  })
  const known = new Set(prev.map(item => item.notice.id))
  for (const notice of current) if (!known.has(notice.id)) merged.push({ notice, leaving: false })
  return merged
}
