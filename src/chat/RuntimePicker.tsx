import { memo, useCallback, useEffect, useMemo, useState } from 'react'
import { ChevronDown, Check } from 'lucide-react'
import { AgentIcon } from './AgentIcon'
import { chatApi, type DetectedExternalAgent } from './api'
import { chatTitlebarPillButtonClass } from './platform'
import type { AgentRuntimeConfig } from './types'
import './runtimePicker.css'

const KIVIO_LOGO_SRC = '/logo-mark.png'

interface RuntimePickerProps {
  agentRuntime: AgentRuntimeConfig
  onRuntimeChange: (runtime: AgentRuntimeConfig) => void
}

const BUILTIN: AgentRuntimeConfig = {
  kind: 'builtin',
  externalAgentId: null,
  externalModel: null,
  externalReasoning: null,
}

function externalRuntime(agentId: string, model?: string | null): AgentRuntimeConfig {
  return {
    kind: 'external',
    externalAgentId: agentId,
    externalModel: model ?? 'default',
    externalReasoning: null,
  }
}

function RuntimePickerBase({ agentRuntime, onRuntimeChange }: RuntimePickerProps) {
  const [open, setOpen] = useState(false)
  const [agents, setAgents] = useState<DetectedExternalAgent[]>([])

  const loadAgents = useCallback(async () => {
    try {
      const list = await chatApi.detectExternalAgents()
      setAgents(list)
    } catch (err) {
      console.error('Failed to detect external agents:', err)
      setAgents([])
    }
  }, [])

  useEffect(() => {
    void loadAgents()
  }, [loadAgents])

  const usesExternal = agentRuntime.kind === 'external' && !!agentRuntime.externalAgentId
  const availableAgents = useMemo(
    () => agents.filter((agent) => agent.available),
    [agents],
  )
  const currentAgent = agents.find((item) => item.id === agentRuntime.externalAgentId)

  const label = useMemo(() => {
    if (!usesExternal) return '內建 Agent'
    return currentAgent?.name ?? agentRuntime.externalAgentId ?? '本機 CLI'
  }, [agentRuntime.externalAgentId, currentAgent?.name, usesExternal])

  const selectBuiltin = () => {
    onRuntimeChange(BUILTIN)
    setOpen(false)
  }

  const selectExternal = (agent: DetectedExternalAgent) => {
    if (!agent.available) return
    const defaultModel = agent.models[0]?.id ?? 'default'
    onRuntimeChange(externalRuntime(agent.id, defaultModel))
    setOpen(false)
  }

  const selectLocalCliMode = () => {
    if (usesExternal && currentAgent?.available) return
    const firstAvailable = availableAgents[0]
    if (firstAvailable) {
      selectExternal(firstAvailable)
    }
  }

  return (
    <div className="kv-runtime-picker" data-tauri-drag-region="false">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className={`kv-runtime-picker__chip${open ? ' is-open' : ''}`}
        title={label}
        aria-label={label}
        aria-haspopup="menu"
        aria-expanded={open}
      >
        {usesExternal && currentAgent ? (
          <AgentIcon id={currentAgent.id} size={18} />
        ) : (
          <img
            src={KIVIO_LOGO_SRC}
            alt=""
            aria-hidden="true"
            className="kv-runtime-picker__builtin-logo"
            width={18}
            height={18}
            draggable={false}
          />
        )}
      </button>

      {open && (
        <>
          <div className="fixed inset-0 z-10" onClick={() => setOpen(false)} aria-hidden />
          <div
            className="kv-runtime-picker__popover chat-motion-popover"
            role="menu"
          >
            <div className="kv-runtime-picker__row">
              <span className="kv-runtime-picker__label">模式</span>
              <div className="kv-runtime-picker__seg" role="tablist">
                <button
                  type="button"
                  role="tab"
                  aria-selected={!usesExternal}
                  className={`kv-runtime-picker__seg-btn${!usesExternal ? ' is-active' : ''}`}
                  onClick={selectBuiltin}
                >
                  內建 Agent
                </button>
                <button
                  type="button"
                  role="tab"
                  aria-selected={usesExternal}
                  disabled={availableAgents.length === 0}
                  className={`kv-runtime-picker__seg-btn${usesExternal ? ' is-active' : ''}`}
                  onClick={selectLocalCliMode}
                >
                  本機 CLI
                </button>
              </div>
            </div>

            <div className="kv-runtime-picker__row">
              <span className="kv-runtime-picker__label">代理</span>
              {agents.length === 0 ? (
                <span className="kv-runtime-picker__hint">正在偵測本機 CLI…</span>
              ) : availableAgents.length === 0 ? (
                <span className="kv-runtime-picker__hint">PATH 中未發現可用 CLI</span>
              ) : (
                <div className="kv-runtime-picker__agent-grid" role="radiogroup">
                  {availableAgents.map((agent) => {
                    const active = usesExternal && agentRuntime.externalAgentId === agent.id
                    return (
                      <button
                        key={agent.id}
                        type="button"
                        role="radio"
                        aria-checked={active}
                        title={agent.version ?? undefined}
                        onClick={() => selectExternal(agent)}
                        className={`kv-runtime-picker__agent${active ? ' is-active' : ''}`}
                      >
                        <AgentIcon id={agent.id} size={20} />
                        <span className="kv-runtime-picker__agent-name">{agent.name}</span>
                      </button>
                    )
                  })}
                </div>
              )}
            </div>
          </div>
        </>
      )}
    </div>
  )
}

interface ExternalModelSelectorProps {
  agentRuntime: AgentRuntimeConfig
  onModelChange: (model: string, reasoning?: string | null) => void
}

function formatModelLabel(model: {
  id: string
  label: string
  contextWindowTokens?: number | null
  context_window_tokens?: number | null
}): string {
  const tokens = model.contextWindowTokens ?? model.context_window_tokens
  if (!tokens) return model.label
  const window = tokens >= 1_000_000
    ? '1M'
    : tokens >= 1_000
      ? `${Math.round(tokens / 1000)}K`
      : `${tokens}`
  return `${model.label} · ${window}`
}

function ExternalModelSelectorBase({
  agentRuntime,
  onModelChange,
}: ExternalModelSelectorProps) {
  const [open, setOpen] = useState(false)
  const [agents, setAgents] = useState<DetectedExternalAgent[]>([])

  useEffect(() => {
    void chatApi.detectExternalAgents().then(setAgents).catch(() => setAgents([]))
  }, [])

  const agent = agents.find((item) => item.id === agentRuntime.externalAgentId)
  const models = useMemo(
    () => agent?.models ?? [{ id: 'default', label: 'Default' }],
    [agent],
  )
  const reasoningOptions = useMemo(() => agent?.reasoningOptions ?? [], [agent])
  const currentReasoning = agentRuntime.externalReasoning ?? 'default'
  const displayName = useMemo(() => {
    const selected = models.find((item) => item.id === (agentRuntime.externalModel || 'default'))
    // On the pill, show the model name only — drop the provider prefix (e.g.
    // "zmfooogreencloud/mimo-v2.5-pro" → "mimo-v2.5-pro") so the meaningful tail isn't truncated.
    // The dropdown keeps the full id.
    const rawLabel = selected?.label ?? (agentRuntime.externalModel || 'default')
    const slash = rawLabel.lastIndexOf('/')
    const base = slash >= 0 ? rawLabel.slice(slash + 1) : rawLabel
    // Append the active thinking level (when set to a non-default level) so it's visible on the pill.
    const reasoning = reasoningOptions.find((o) => o.id === currentReasoning)
    if (reasoning && currentReasoning !== 'default') {
      return `${base} · ${reasoning.label}`
    }
    return base
  }, [agentRuntime.externalModel, models, reasoningOptions, currentReasoning])

  if (agentRuntime.kind !== 'external' || !agentRuntime.externalAgentId) {
    return null
  }

  return (
    <div className="relative max-w-full min-w-0" data-tauri-drag-region="false">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className={`${chatTitlebarPillButtonClass} max-w-full min-w-0`}
      >
        <span className="max-w-[140px] truncate font-medium text-neutral-800 dark:text-neutral-200">
          {displayName}
        </span>
        <ChevronDown
          size={15}
          className={`shrink-0 text-neutral-400 transition-transform ${open ? 'rotate-180' : ''}`}
        />
      </button>
      {open && (
        <>
          <div className="fixed inset-0 z-10" onClick={() => setOpen(false)} aria-hidden />
          <div className="chat-model-selector-menu chat-motion-popover absolute left-0 top-full z-20 mt-2 max-h-[min(320px,50vh)] min-w-[200px] overflow-y-auto rounded-2xl border border-neutral-200/90 bg-white py-1 shadow-lg dark:border-neutral-700 dark:bg-neutral-900">
            {models.map((model) => (
              <button
                key={model.id}
                type="button"
                onClick={() => {
                  onModelChange(model.id)
                  setOpen(false)
                }}
                className={`block w-full px-3 py-2 text-left text-sm hover:bg-neutral-100 dark:hover:bg-neutral-800 ${
                  displayName === model.id ? 'font-semibold' : ''
                }`}
              >
                {formatModelLabel(model)}
              </button>
            ))}
            {reasoningOptions.length > 0 && (
              <>
                <div className="my-1 border-t border-neutral-100 dark:border-neutral-800" />
                <div className="px-3 py-1 text-[11px] font-semibold uppercase tracking-wide text-neutral-400">
                  Reasoning
                </div>
                {reasoningOptions.map((option) => (
                  <button
                    key={option.id}
                    type="button"
                    onClick={() => {
                      onModelChange(agentRuntime.externalModel ?? 'default', option.id)
                      setOpen(false)
                    }}
                    className="flex w-full items-center justify-between px-3 py-2 text-left text-sm hover:bg-neutral-100 dark:hover:bg-neutral-800"
                  >
                    <span>{option.label}</span>
                    {option.id === currentReasoning && (
                      <Check size={15} className="shrink-0 text-neutral-500" />
                    )}
                  </button>
                ))}
              </>
            )}
          </div>
        </>
      )}
    </div>
  )
}

// memo：頂欄選擇器，僅在 props 變化時重渲。
export const RuntimePicker = memo(RuntimePickerBase)
export const ExternalModelSelector = memo(ExternalModelSelectorBase)
