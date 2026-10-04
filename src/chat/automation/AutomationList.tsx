import { Button, IconButton } from '../../components/Button'
import { Toggle } from '../../settings/public/controls'
import { MessageSquarePlus, Plus, Trash2 } from 'lucide-react'
import { useT } from '../../components/i18n'
import { catalogEntry } from './nodeCatalog'
import type { AutomationMeta } from '../../api/automationContracts'

function triggerLabel(meta: AutomationMeta, t: ReturnType<typeof useT>): string {
  if (!meta.triggerType) return t.chatAutomationNoTrigger
  return catalogEntry(meta.triggerType)?.label(t) ?? meta.triggerType
}

export function AutomationList({
  items,
  loading,
  error,
  onCreate,
  onCreateByChat,
  onOpen,
  onToggle,
  onDelete,
}: {
  items: AutomationMeta[]
  loading: boolean
  error: string
  onCreate: () => void
  onCreateByChat: () => void
  onOpen: (id: string) => void
  onToggle: (id: string, enabled: boolean) => void
  onDelete: (id: string) => void
}) {
  const t = useT()
  return (
    <div className="custom-scrollbar mx-auto flex min-h-0 w-full max-w-[880px] flex-1 flex-col overflow-y-auto px-6 pb-6">
        {error && <p role="alert" className="text-[13px] text-red-600 dark:text-red-400">{error}</p>}
        {loading && items.length === 0 ? (
          <p className="text-[13px] text-neutral-400">{t.chatLoading}</p>
        ) : items.length === 0 ? (
          <div className="flex flex-1 flex-col items-center justify-center rounded-xl border border-dashed border-[var(--theme-surface-border)] px-6 py-16 text-center dark:border-white/[0.1]">
            <p className="text-[15px] font-medium text-neutral-800">
              {t.chatAutomationEmpty}
            </p>
            <p className="mt-1 max-w-[28rem] text-[13px] text-neutral-500 dark:text-neutral-400">
              {t.chatAutomationEmptyHint}
            </p>
            <div className="mt-4 flex items-center gap-2">
              <Button size="sm" variant="primary" onClick={onCreateByChat}>
                <MessageSquarePlus size={14} strokeWidth={1.75} />
                {t.chatTasksCreateByChat}
              </Button>
              <Button size="sm" onClick={onCreate}>
                <Plus size={14} />
                {t.chatTasksCreateManually}
              </Button>
            </div>
          </div>
        ) : (
          <ul className="flex flex-col gap-2">
            {items.map((item) => {
              const Icon = catalogEntry(item.triggerType ?? '')?.icon
              return (
                <li key={item.id}>
                  <div className="flex items-center gap-3 rounded-xl border border-[var(--theme-surface-border)] bg-[var(--theme-surface)] px-4 py-3 dark:border-white/[0.08]">
                    <button
                      type="button"
                      className="flex min-w-0 flex-1 items-center gap-3 text-left"
                      onClick={() => onOpen(item.id)}
                    >
                      {Icon ? (
                        <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-[var(--theme-surface-muted)] text-neutral-600 dark:bg-white/[0.06]">
                          <Icon size={16} strokeWidth={1.75} />
                        </span>
                      ) : null}
                      <span className="min-w-0">
                        <span className="block truncate text-[14px] font-medium text-neutral-900">
                          {item.name.trim() || t.chatAutomationUntitled}
                        </span>
                        <span className="mt-0.5 block text-[12px] text-neutral-500 dark:text-neutral-400">
                          {triggerLabel(item, t)}
                          {' · '}
                          {item.enabled ? t.chatAutomationEnabled : t.chatAutomationDisabled}
                        </span>
                      </span>
                    </button>
                    <Toggle
                      checked={item.enabled}
                      onChange={(enabled) => onToggle(item.id, enabled)}
                      ariaLabel={t.chatAutomationEnabled}
                    />
                    <IconButton
                      size="sm"
                      label={t.chatAutomationDelete}
                      onClick={() => onDelete(item.id)}
                    >
                      <Trash2 size={14} />
                    </IconButton>
                  </div>
                </li>
              )
            })}
          </ul>
        )}
      </div>
  )
}
