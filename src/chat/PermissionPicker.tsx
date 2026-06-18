import { useEffect, useMemo, useState } from 'react'
import { ChevronDown, Check, ShieldAlert } from 'lucide-react'
import { chatTitlebarPillButtonClass } from './platform'
import { APPROVAL_POLICY_OPTIONS } from './approvalPolicies'
import { chatApi, type DetectedExternalAgent } from './api'
import type { AgentRuntimeConfig } from './types'

interface Option {
  value: string
  label: string
}

interface PermissionPickerProps {
  agentRuntime: AgentRuntimeConfig
  /** Set the external-CLI sandbox level (claude --permission-mode / codex --sandbox). */
  onSandboxChange: (level: string) => void
  /** Built-in agent tool-approval policy. */
  approvalPolicy?: string
  onApprovalPolicyChange?: (policy: string) => void
}

/**
 * Unified permission capsule shown next to the model pill. Adapts to the active runtime:
 * external CLI → sandbox level (per-agent); built-in chat → tool-approval policy. Hidden when
 * there is nothing to control (e.g. an external CLI with no sandbox flag).
 */
export function PermissionPicker({
  agentRuntime,
  onSandboxChange,
  approvalPolicy,
  onApprovalPolicyChange,
}: PermissionPickerProps) {
  const [open, setOpen] = useState(false)
  const [agents, setAgents] = useState<DetectedExternalAgent[]>([])

  useEffect(() => {
    void chatApi.detectExternalAgents().then(setAgents).catch(() => setAgents([]))
  }, [])

  const usesExternal = agentRuntime.kind === 'external' && !!agentRuntime.externalAgentId
  const agent = agents.find((item) => item.id === agentRuntime.externalAgentId)
  const sandboxOptions = useMemo(
    () => agent?.sandboxOptions ?? agent?.sandbox_options ?? [],
    [agent],
  )

  const options: Option[] = useMemo(
    () =>
      usesExternal
        ? sandboxOptions.map((o) => ({ value: o.id, label: o.label }))
        : APPROVAL_POLICY_OPTIONS.map((o) => ({ value: o.value, label: o.label })),
    [usesExternal, sandboxOptions],
  )

  const current = useMemo(() => {
    if (usesExternal) {
      if (agentRuntime.externalSandbox) return agentRuntime.externalSandbox
      const def = sandboxOptions.find((o) => o.label.includes('默认')) ?? sandboxOptions[0]
      return def?.id ?? ''
    }
    return approvalPolicy ?? APPROVAL_POLICY_OPTIONS[1]?.value ?? ''
  }, [usesExternal, agentRuntime.externalSandbox, sandboxOptions, approvalPolicy])

  if (usesExternal && options.length === 0) return null
  if (!usesExternal && !onApprovalPolicyChange) return null

  const currentLabel = options.find((o) => o.value === current)?.label ?? '权限'

  const pick = (value: string) => {
    if (usesExternal) {
      onSandboxChange(value)
    } else {
      onApprovalPolicyChange?.(value)
    }
    setOpen(false)
  }

  return (
    <div className="relative max-w-full min-w-0" data-tauri-drag-region="false">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className={`${chatTitlebarPillButtonClass} max-w-full min-w-0`}
        title={usesExternal ? '沙盒 / 权限等级' : '工具审批策略'}
      >
        <ShieldAlert size={14} className="shrink-0 text-neutral-500" strokeWidth={1.8} />
        <span className="max-w-[120px] truncate font-medium text-neutral-800 dark:text-neutral-200">
          {currentLabel}
        </span>
        <ChevronDown
          size={15}
          className={`shrink-0 text-neutral-400 transition-transform ${open ? 'rotate-180' : ''}`}
        />
      </button>
      {open && (
        <>
          <div className="fixed inset-0 z-10" onClick={() => setOpen(false)} aria-hidden />
          <div className="chat-model-selector-menu chat-motion-popover absolute left-0 top-full z-20 mt-2 max-h-[min(320px,50vh)] min-w-[180px] overflow-y-auto rounded-2xl border border-neutral-200/90 bg-white py-1 shadow-lg dark:border-neutral-700 dark:bg-neutral-900">
            {options.map((option) => (
              <button
                key={option.value}
                type="button"
                onClick={() => pick(option.value)}
                className="flex w-full items-center justify-between px-3 py-2 text-left text-sm hover:bg-neutral-100 dark:hover:bg-neutral-800"
              >
                <span>{option.label}</span>
                {option.value === current && (
                  <Check size={15} className="shrink-0 text-neutral-500" />
                )}
              </button>
            ))}
          </div>
        </>
      )}
    </div>
  )
}
