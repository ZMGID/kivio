import { memo, useMemo, useRef, useState } from 'react'
import { Check } from 'lucide-react'
import { useT } from '../components/i18n'
import { derivePermissionModes, useDetectedExternalAgents } from './permissionModes'
import { chatTitlebarIconButtonClass } from './platform'
import { usePopoverMaxHeight } from './usePopoverMaxHeight'
import type { AgentRuntimeConfig } from './types'
import { usePopoverMenu } from './usePopoverMenu'

interface PermissionPickerProps {
  agentRuntime: AgentRuntimeConfig
  conversationId?: string | null
  /** Built-in agent tool-approval policy. */
  approvalPolicy?: string
  onApprovalPolicyChange?: (policy: string) => void
  /** Local-CLI sandbox level. */
  onSandboxChange?: (value: string) => void | Promise<void>
  /** A running CLI turn keeps the sandbox it started with, so changing it mid-run would only misreport. */
  sandboxLocked?: boolean
}

/**
 * Permission capsule next to the model pill. Built-in chat: tool-approval policy. Local-CLI
 * conversation: that CLI's own sandbox/permission levels (the composer no longer has a mode pill).
 */
function PermissionPickerBase({
  agentRuntime,
  conversationId,
  approvalPolicy,
  onApprovalPolicyChange,
  onSandboxChange,
  sandboxLocked = false,
}: PermissionPickerProps) {
  const usesExternal = agentRuntime.kind === 'external' && !!agentRuntime.externalAgentId
  const t = useT()
  const [open, setOpen] = useState(false)
  const menuRef = useRef<HTMLDivElement>(null)
  usePopoverMenu(open, () => setOpen(false), menuRef)
  const maxH = usePopoverMaxHeight(open, menuRef, 'down', 320)

  const agents = useDetectedExternalAgents(usesExternal ? conversationId : null)
  const { options, current } = useMemo(
    () => derivePermissionModes({ target: 'titlebar', agentRuntime, agents, approvalPolicy }),
    [agentRuntime, agents, approvalPolicy],
  )

  if (options.length === 0) return null
  if (!(usesExternal ? onSandboxChange : onApprovalPolicyChange)) return null

  const activeOption = options.find((option) => option.value === current)
  const currentLabel = activeOption?.label ?? t.chatPermission
  const CurrentIcon = activeOption?.icon ?? options[0].icon

  const pick = (value: string) => {
    if (usesExternal) void onSandboxChange?.(value)
    else onApprovalPolicyChange?.(value)
    setOpen(false)
  }

  return (
    <div className="relative" data-tauri-drag-region="false">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        aria-haspopup="menu"
        aria-expanded={open}
        className={`${chatTitlebarIconButtonClass} ${
          open
            ? 'bg-neutral-900/[0.06] text-neutral-800'
            : 'hover:text-neutral-800'
        }`}
        title={t.chatApprovalPolicy.replace('{name}', currentLabel)}
        aria-label={t.chatApprovalPolicy.replace('{name}', currentLabel)}
      >
        <CurrentIcon size={16} strokeWidth={1.8} />
      </button>
      {open && (
        <>
          <div className="fixed inset-0 z-10" onClick={() => setOpen(false)} aria-hidden />
          <div role="menu" ref={menuRef} style={{ maxHeight: maxH }} className="chat-model-selector-menu chat-motion-popover absolute left-0 top-full z-20 mt-2 min-w-[180px] overflow-y-auto kv-menu">
            {options.map((option) => {
              const active = option.value === current
              return (
                <button
                  key={option.value}
                  role="menuitemradio"
                  aria-checked={active}
                  type="button"
                  disabled={usesExternal && sandboxLocked}
                  onClick={() => pick(option.value)}
                  className={`kv-menu-row justify-between transition-colors ${
                    active
                      ? 'bg-neutral-100 font-medium text-neutral-900'
                      : 'text-neutral-700 hover:bg-neutral-50'
                  }`}
                >
                  <span>{option.label}</span>
                  {active && (
                    <Check size={15} className="shrink-0 text-neutral-500 dark:text-neutral-400" />
                  )}
                </button>
              )
            })}
          </div>
        </>
      )}
    </div>
  )
}

// memo：顶栏选择器，仅在 props 变化时重渲。
export const PermissionPicker = memo(PermissionPickerBase)
