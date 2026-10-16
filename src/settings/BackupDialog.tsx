import { useEffect, useId, useRef, type ReactNode } from 'react'
import { X } from 'lucide-react'
import { IconButton } from '../components/Button'

/** Native modal keeps credential editing and restore selection in a focused, keyboard-safe surface. */
export function BackupDialog({ title, description, closeLabel, locked, onClose, children }: {
  title: string; description: string; closeLabel: string; locked: boolean; onClose: () => void; children: ReactNode
}) {
  const ref = useRef<HTMLDialogElement>(null)
  const titleId = useId()
  const descriptionId = useId()
  useEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null
    const dialog = ref.current
    dialog?.showModal()
    return () => { dialog?.close(); if (previous?.isConnected) previous.focus() }
  }, [])
  return <dialog ref={ref} className="kv kv-modal backup-dialog" aria-labelledby={titleId} aria-describedby={descriptionId}
    onKeyDown={(event) => event.stopPropagation()} onCancel={(event) => { event.preventDefault(); if (!locked) onClose() }}>
    <header className="backup-dialog-heading">
      <div><h2 id={titleId}>{title}</h2><p id={descriptionId} className="kv-row-desc">{description}</p></div>
      <IconButton label={closeLabel} variant="ghost" disabled={locked} onClick={onClose}><X size={16} /></IconButton>
    </header>
    <div className="backup-dialog-body custom-scrollbar">{children}</div>
  </dialog>
}
