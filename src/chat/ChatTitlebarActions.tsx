import { Plus } from 'lucide-react'
import { IconButton } from '../components/Button'
import { useT } from '../components/i18n'
import { SidebarToggleIcon } from './SidebarToggleIcon'

type ChatTitlebarActionsProps = {
  sidebarExpanded: boolean
  onToggleSidebar: () => void
  onNewConversation: () => void
}

export function ChatTitlebarActions({
  sidebarExpanded,
  onToggleSidebar,
  onNewConversation,
}: ChatTitlebarActionsProps) {
  const t = useT()
  const toggleLabel = sidebarExpanded ? t.chatCollapseSidebar : t.chatExpandSidebar

  return (
    <div
      className="inline-flex h-8 shrink-0 items-center gap-0.5"
      data-tauri-drag-region="false"
    >
      <IconButton
        label={toggleLabel}
        variant="ghost"
        size="sm"
        onClick={onToggleSidebar}
        className="group"
        aria-expanded={sidebarExpanded}
      >
        <SidebarToggleIcon side="left" open={sidebarExpanded} />
      </IconButton>
      <IconButton
        label={t.chatNewChat}
        variant="ghost"
        size="sm"
        onClick={onNewConversation}
      >
        <Plus size={15} strokeWidth={2} />
      </IconButton>
    </div>
  )
}
