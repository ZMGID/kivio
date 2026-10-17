export type AppNotice = { id: string; message: string; tone: 'error' | 'warning' | 'success' | 'info'; actionLabel?: string; onAction?: () => void }
let notices: AppNotice[] = []
let sequence = 0
const listeners = new Set<() => void>()

export function subscribeNotices(listener: () => void) {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}
export function getNotices() { return notices }

export function dismissNotice(id: string) {
  const next = notices.filter(notice => notice.id !== id)
  if (next.length === notices.length) return
  notices = next
  listeners.forEach(listener => listener())
}

export function publishNotice(notice: AppNotice) {
  if (!notice.message.trim()) return
  notices = [...notices.filter(item => item.id !== notice.id), notice].slice(-3)
  listeners.forEach(listener => listener())
}

/** User-initiated operation failures; background diagnostics stay with their owner. */
export function notifyError(message: string) {
  if (notices.some(notice => notice.message === message && notice.tone === 'error')) return
  publishNotice({ id: `operation-${++sequence}`, message, tone: 'error' })
}
