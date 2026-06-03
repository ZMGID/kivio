import { PanelLeftClose, PanelLeftOpen, SquarePen } from 'lucide-react'
import {
  chatTitlebarPillButtonClass,
  chatTitlebarPillIconClass,
} from './platform'

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
  const ToggleIcon = sidebarExpanded ? PanelLeftClose : PanelLeftOpen
  const toggleLabel = sidebarExpanded ? '收起侧栏' : '展开侧栏'

  return (
    <div className={chatTitlebarPillButtonClass} data-tauri-drag-region="false">
      <button
        type="button"
        onClick={onToggleSidebar}
        className={chatTitlebarPillIconClass}
        title={toggleLabel}
        aria-label={toggleLabel}
      >
        <ToggleIcon size={15} strokeWidth={1.75} />
      </button>
      <span
        aria-hidden
        className="h-4 w-px shrink-0 bg-neutral-200/90 dark:bg-neutral-700"
      />
      <button
        type="button"
        onClick={onNewConversation}
        className={chatTitlebarPillIconClass}
        title="新建聊天"
        aria-label="新建聊天"
      >
        <SquarePen size={15} strokeWidth={1.75} />
      </button>
    </div>
  )
}
