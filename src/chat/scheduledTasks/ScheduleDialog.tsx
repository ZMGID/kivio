import { useEffect, useId, useRef, type ReactNode } from 'react'
import { IconButton } from '../../components/Button'
import { useT } from '../../components/i18n'
import { X } from 'lucide-react'

export function ScheduleDialog({ title, onClose, children, width = 560 }: {
  title: string
  onClose: () => void
  children: ReactNode
  width?: number
}) {
  const ref = useRef<HTMLDialogElement>(null)
  // A drag that starts inside (e.g. selecting text) and ends on the backdrop
  // still fires `click` on the dialog; only a press that began on the backdrop closes.
  const pressedBackdrop = useRef(false)
  const titleId = useId()
  const t = useT()
  useEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null
    ref.current?.showModal()
    return () => { if (previous?.isConnected) previous.focus() }
  }, [])
  return <dialog ref={ref} className="kv kv-modal overflow-hidden" aria-labelledby={titleId}
    style={{ width: `min(${width}px, calc(100vw - 32px))`, maxWidth: width, maxHeight: 'calc(100dvh - 48px)', padding: 0 }}
    onCancel={event => { event.preventDefault(); onClose() }}
    onMouseDown={event => { pressedBackdrop.current = event.target === event.currentTarget }}
    onClick={event => {
      const fromBackdrop = pressedBackdrop.current && event.target === event.currentTarget
      pressedBackdrop.current = false
      if (fromBackdrop) onClose()
    }}>
    <div className="flex max-h-[calc(100dvh-48px)] min-h-0 flex-col">
      <header className="flex shrink-0 items-center justify-between gap-4 border-b border-[var(--color-border)] px-5 py-4">
        <h2 id={titleId} className="min-w-0 break-words text-[15px] font-semibold">{title}</h2>
        <IconButton label={t.chatSchedulesClose} variant="ghost" onClick={onClose}><X size={18} strokeWidth={1.75} /></IconButton>
      </header>
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden">{children}</div>
    </div>
  </dialog>
}
