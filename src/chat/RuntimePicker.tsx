import { memo, useEffect, useMemo, useState } from 'react'
import { ChevronDown, Check, Brain } from 'lucide-react'
import { AgentIcon } from './AgentIcon'
import { chatApi, type DetectedExternalAgent } from './api'
import { chatTitlebarPillButtonClass } from './platform'
import type { AgentRuntimeConfig } from './types'
import './runtimePicker.css'

const KIVIO_LOGO_SRC = '/logo-mark.png'

interface RuntimePickerProps {
  agentRuntime: AgentRuntimeConfig
  onRuntimeChange: (runtime: AgentRuntimeConfig) => void
  conversationId?: string | null
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

function RuntimePickerBase({ agentRuntime, onRuntimeChange, conversationId }: RuntimePickerProps) {
  const [open, setOpen] = useState(false)
  const [agents, setAgents] = useState<DetectedExternalAgent[]>([])

  useEffect(() => {
    let active = true
    void chatApi.detectExternalAgents(false, conversationId)
      .then((list) => {
        if (active) setAgents(list)
      })
      .catch((err) => {
        if (!active) return
        console.error('Failed to detect external agents:', err)
        setAgents([])
      })
    return () => {
      active = false
    }
  }, [conversationId])

  const usesExternal = agentRuntime.kind === 'external' && !!agentRuntime.externalAgentId
  const availableAgents = useMemo(
    () => agents.filter((agent) => agent.available),
    [agents],
  )
  const currentAgent = agents.find((item) => item.id === agentRuntime.externalAgentId)

  const label = useMemo(() => {
    if (!usesExternal) return '内置 Agent'
    return currentAgent?.name ?? agentRuntime.externalAgentId ?? '本地 CLI'
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
        {/* Icon keys off externalAgentId directly (not the detection result) so the agent
            icon shows immediately — detection is async and the list resets per conversation,
            which used to flash the Kivio logo until the first probe finished. */}
        {usesExternal && agentRuntime.externalAgentId ? (
          <AgentIcon id={agentRuntime.externalAgentId} size={18} />
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
                  内置 Agent
                </button>
                <button
                  type="button"
                  role="tab"
                  aria-selected={usesExternal}
                  disabled={availableAgents.length === 0}
                  className={`kv-runtime-picker__seg-btn${usesExternal ? ' is-active' : ''}`}
                  onClick={selectLocalCliMode}
                >
                  本地 CLI
                </button>
              </div>
            </div>

            <div className="kv-runtime-picker__row">
              <span className="kv-runtime-picker__label">代理</span>
              {agents.length === 0 ? (
                <span className="kv-runtime-picker__hint">正在检测本机 CLI…</span>
              ) : availableAgents.length === 0 ? (
                <span className="kv-runtime-picker__hint">PATH 中未发现可用 CLI</span>
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
  conversationId?: string | null
}

function ExternalModelSelectorBase({
  agentRuntime,
  onModelChange,
  conversationId,
}: ExternalModelSelectorProps) {
  const [open, setOpen] = useState(false)
  const [reasoningOpen, setReasoningOpen] = useState(false)
  // 懒查：只探选中 agent 的模型（cwd-scoped），不再拉全量列表。保留上次结果，不清空闪。
  const [models, setModels] = useState<DetectedExternalAgent['models']>([])
  const [reasoningOptions, setReasoningOptions] = useState<
    NonNullable<DetectedExternalAgent['reasoningOptions']>
  >([])

  useEffect(() => {
    const agentId = agentRuntime.externalAgentId
    if (!agentId) {
      setModels([])
      setReasoningOptions([])
      return
    }
    let active = true
    void chatApi
      .detectExternalAgentModels(agentId, conversationId)
      .then((result) => {
        if (!active) return
        setModels(result.models)
        setReasoningOptions(result.reasoningOptions)
      })
      .catch(() => {
        /* 保留上次结果，不清空 */
      })
    return () => {
      active = false
    }
  }, [agentRuntime.externalAgentId, conversationId])

  const currentReasoning = agentRuntime.externalReasoning ?? 'default'
  const currentReasoningLabel =
    reasoningOptions.find((o) => o.id === currentReasoning)?.label ?? currentReasoning
  const displayName = useMemo(() => {
    const currentId = agentRuntime.externalModel
    const selected = currentId ? models.find((item) => item.id === currentId) : undefined
    // On the pill, show the model name only — drop the provider prefix (e.g.
    // "zmfooogreencloud/mimo-v2.5-pro" → "mimo-v2.5-pro") so the meaningful tail isn't truncated.
    // The dropdown keeps the full id.
    const rawLabel = selected?.label ?? currentId ?? '选择模型'
    const slash = rawLabel.lastIndexOf('/')
    return slash >= 0 ? rawLabel.slice(slash + 1) : rawLabel
  }, [agentRuntime.externalModel, models])

  if (agentRuntime.kind !== 'external' || !agentRuntime.externalAgentId) {
    return null
  }

  return (
    <div className="flex min-w-0 max-w-full items-center gap-1">
      <div className="relative min-w-0" data-tauri-drag-region="false">
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
              {models.length === 0 ? (
                <div className="px-3 py-2 text-sm text-neutral-500 dark:text-neutral-400">
                  该 CLI 未上报可用模型
                </div>
              ) : (
                models.map((model) => (
                  <button
                    key={model.id}
                    type="button"
                    onClick={() => {
                      onModelChange(model.id)
                      setOpen(false)
                    }}
                    className={`block w-full px-3 py-2 text-left text-sm text-neutral-700 hover:bg-neutral-100 dark:text-neutral-200 dark:hover:bg-neutral-800 ${
                      agentRuntime.externalModel === model.id ? 'font-semibold' : ''
                    }`}
                  >
                    {model.label}
                  </button>
                ))
              )}
            </div>
          </>
        )}
      </div>

      {/* Standalone thinking-level pill, mirroring the builtin ThinkingLevelSelector. */}
      {reasoningOptions.length > 0 && (
        <div className="relative shrink-0" data-tauri-drag-region="false">
          <button
            type="button"
            onClick={() => setReasoningOpen(!reasoningOpen)}
            className={`${chatTitlebarPillButtonClass} max-w-full min-w-0`}
            title={`思考等级：${currentReasoningLabel}`}
            aria-label={`思考等级：${currentReasoningLabel}`}
          >
            <Brain size={15} className="shrink-0 text-neutral-500 dark:text-neutral-400" />
            <span className="chat-thinking-level-label max-w-[64px] truncate font-medium text-neutral-800 dark:text-neutral-200">
              {currentReasoningLabel}
            </span>
            <ChevronDown
              size={15}
              className={`shrink-0 text-neutral-400 transition-transform ${reasoningOpen ? 'rotate-180' : ''}`}
            />
          </button>
          {reasoningOpen && (
            <>
              <div
                className="fixed inset-0 z-10"
                onClick={() => setReasoningOpen(false)}
                aria-hidden
              />
              <div className="chat-model-selector-menu chat-motion-popover absolute left-0 top-full z-20 mt-2 min-w-[160px] overflow-y-auto rounded-2xl border border-neutral-200/90 bg-white py-1 shadow-lg dark:border-neutral-700 dark:bg-neutral-900">
                {reasoningOptions.map((option) => {
                  const active = option.id === currentReasoning
                  return (
                    <button
                      key={option.id}
                      type="button"
                      onClick={() => {
                        onModelChange(agentRuntime.externalModel ?? 'default', option.id)
                        setReasoningOpen(false)
                      }}
                      className={`flex w-full items-center justify-between gap-2 px-3 py-2 text-left text-[13px] transition-colors ${
                        active
                          ? 'bg-neutral-100 font-medium text-neutral-900 dark:bg-neutral-800 dark:text-neutral-100'
                          : 'text-neutral-700 hover:bg-neutral-50 dark:text-neutral-300 dark:hover:bg-neutral-800/80'
                      }`}
                    >
                      <span className="min-w-0 truncate">{option.label}</span>
                      {active && <Check size={15} className="shrink-0 text-neutral-500" />}
                    </button>
                  )
                })}
              </div>
            </>
          )}
        </div>
      )}
    </div>
  )
}

// memo：顶栏选择器，仅在 props 变化时重渲。
export const RuntimePicker = memo(RuntimePickerBase)
export const ExternalModelSelector = memo(ExternalModelSelectorBase)
