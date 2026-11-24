import { useState, type MouseEvent } from 'react'
import { FolderIcon } from '../components/FileIcon'
import { notifyError } from '../components/appNoticeStore'
import { useT } from '../components/i18n'
import { DockContextMenu, type DockMenuAnchor } from './dock/DockContextMenu'

export function useFileLocationMenu(onRevealLocation?: () => Promise<void>) {
  const t = useT()
  const [anchor, setAnchor] = useState<DockMenuAnchor | null>(null)
  return {
    onContextMenu: onRevealLocation ? (event: MouseEvent) => {
      event.preventDefault()
      event.stopPropagation()
      setAnchor({ left: event.clientX, top: event.clientY })
    } : undefined,
    menu: anchor && onRevealLocation && <DockContextMenu anchor={anchor} onClose={() => setAnchor(null)} items={[{
      key: 'reveal', label: t.chatMsgRevealLocation, icon: <FolderIcon open size={16} />,
      onSelect: () => { void onRevealLocation().catch(() => notifyError(t.chatMsgRevealLocationFailed)) },
    }]} />,
  }
}
