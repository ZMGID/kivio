import { useEffect, useId, useRef, useSyncExternalStore } from 'react'
import { createPortal } from 'react-dom'
import { AlertCircle, Check, Info, X } from 'lucide-react'
import { Button, IconButton } from './Button'
import type { Lang } from './i18n'
import { dismissNotice, getNotices, publishNotice, subscribeNotices } from './appNoticeStore'

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
  return createPortal(
    <aside className="kv-notices custom-scrollbar" aria-label={lang === 'zh' ? '操作提示' : 'Notifications'} data-tauri-drag-region="false">
      {notices.map(notice => {
        const compact = notice.message.replace(/\s+/g, ' ').trim()
        const chars = Array.from(compact)
        const summary = chars.length > 100 ? `${chars.slice(0, 100).join('')}…` : compact
        const Icon = notice.tone === 'success' ? Check : notice.tone === 'info' ? Info : AlertCircle
        return <div key={notice.id} className={`kv-notice ${notice.tone}`}>
          <Icon className="kv-notice-icon" size={16} aria-hidden="true" />
          <div className="kv-notice-copy">
            <p role={notice.tone === 'error' || notice.tone === 'warning' ? 'alert' : 'status'} className="kv-notice-summary" title={notice.message}>{summary}</p>
            {chars.length > 60 && <details>
              <summary>{lang === 'zh' ? '查看详情' : 'Details'}</summary>
              <p className="kv-notice-detail custom-scrollbar">{notice.message}</p>
            </details>}
            {notice.actionLabel && <Button size="sm" variant="ghost" onClick={notice.onAction}>{notice.actionLabel}</Button>}
          </div>
          <IconButton variant="ghost" size="xs" label={lang === 'zh' ? '关闭提示' : 'Dismiss notification'} onClick={() => dismissNotice(notice.id)}><X size={14} /></IconButton>
        </div>
      })}
    </aside>, document.body,
  )
}
