import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { ChevronDown, Check, Brain, RefreshCw, Loader2 } from 'lucide-react'
import { AgentIcon } from './AgentIcon'
import { chatApi, type DetectedExternalAgent } from './api'
import { chatTitlebarPillButtonClass } from './platform'
import { IconButton } from '../components/Button'
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
  const [refreshing, setRefreshing] = useState(false)
  // 请求代际：conversationId 切换 / 手动刷新会并发发起检测，只让最新一次的结果落地
  // （也兜住卸载后 setState）。
  const agentsReqIdRef = useRef(0)

  const loadAgents = useCallback(
    (force: boolean) => {
      const reqId = ++agentsReqIdRef.current
      // 初次检测与手动刷新共用同一 in-flight 态：spinner + 「正在检测本机 CLI…」提示，
      // 避免首探未返回时误显示「PATH 中未发现可用 CLI」。
      setRefreshing(true)
      return chatApi
        .detectExternalAgents(force, conversationId)
        .then((list) => {
          if (agentsReqIdRef.current === reqId) setAgents(list)
          return list
        })
        .catch((err) => {
          console.error('Failed to detect external agents:', err)
          if (agentsReqIdRef.current === reqId && !force) setAgents([])
          return null
        })
        .finally(() => {
          if (agentsReqIdRef.current === reqId) setRefreshing(false)
        })
    },
    [conversationId],
  )

  useEffect(() => {
    // 每次 loadAgents 调用自身会先 ++reqId 使旧在途请求失效；卸载后 setState 在 React 18
    // 是安全 no-op，无需 cleanup 递增（避免 exhaustive-deps 对 cleanup 读 ref 的告警）。
    void loadAgents(false)
  }, [loadAgents])

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
    // 隐式契约（D3）：后端各探测路径都把合成的 "default" 占位放在 models[0]
    // （default_model_option / fallback_models 首项），因此这里取 [0] 即「让 CLI 用自己的默认模型」。
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
              <div className="kv-runtime-picker__agents-head">
                <span className="kv-runtime-picker__label">代理</span>
                <IconButton
                  size="xs"
                  variant="ghost"
                  label="刷新本机 CLI"
                  onClick={() => {
                    void loadAgents(true)
                  }}
                  disabled={refreshing}
                >
                  <RefreshCw size={13} className={refreshing ? 'animate-spin' : undefined} />
                </IconButton>
              </div>
              {agents.length === 0 ? (
                <span className="kv-runtime-picker__hint">
                  {refreshing ? '正在检测本机 CLI…' : 'PATH 中未发现可用 CLI'}
                </span>
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
  const [loading, setLoading] = useState(false)
  // source: probed=真实探测 / fallback=探测失败降级静态表（显示"默认列表"角标 + 重试）。
  const [source, setSource] = useState<'probed' | 'fallback'>('probed')
  // 请求代际：agent 切换/卸载时使在途请求失效，防止旧结果覆盖新 agent 或卸载后 setState。
  const modelsReqIdRef = useRef(0)

  const loadModels = useCallback(
    (agentId: string, force?: boolean) => {
      const reqId = ++modelsReqIdRef.current
      setLoading(true)
      return chatApi
        .detectExternalAgentModels(agentId, conversationId, force)
        .then((result) => {
          if (modelsReqIdRef.current !== reqId) return
          setModels(result.models)
          setReasoningOptions(result.reasoningOptions)
          setSource(result.source)
        })
        .catch(() => {
          if (modelsReqIdRef.current !== reqId) return
          // 不再静默吞错：置为降级态，展示重试。保留上次模型列表不清空。
          setSource('fallback')
        })
        .finally(() => {
          if (modelsReqIdRef.current === reqId) setLoading(false)
        })
    },
    [conversationId],
  )

  useEffect(() => {
    const agentId = agentRuntime.externalAgentId
    if (!agentId) {
      // 失效在途请求，防止旧结果落到已清空的状态上。
      modelsReqIdRef.current++
      setModels([])
      setReasoningOptions([])
      setSource('probed')
      return
    }
    // loadModels 自身先 ++reqId 使旧在途请求失效（agent/conversation 变更时 effect 重跑即覆盖）。
    void loadModels(agentId)
  }, [agentRuntime.externalAgentId, loadModels])

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
          {loading ? (
            <Loader2 size={14} className="shrink-0 animate-spin text-neutral-400" />
          ) : (
            <ChevronDown
              size={15}
              className={`shrink-0 text-neutral-400 transition-transform ${open ? 'rotate-180' : ''}`}
            />
          )}
        </button>
        {open && (
          <>
            <div className="fixed inset-0 z-10" onClick={() => setOpen(false)} aria-hidden />
            <div className="chat-model-selector-menu chat-motion-popover absolute left-0 top-full z-20 mt-2 max-h-[min(320px,50vh)] min-w-[200px] overflow-y-auto rounded-2xl border border-neutral-200/90 bg-white py-1 shadow-lg dark:border-neutral-700 dark:bg-neutral-900">
              {source === 'fallback' && (
                <div className="kv-runtime-picker__fallback mx-1 my-1">
                  <span>探测失败，显示默认列表</span>
                  <button
                    type="button"
                    className="kv-runtime-picker__fallback-retry"
                    disabled={loading}
                    onClick={() => {
                      const agentId = agentRuntime.externalAgentId
                      if (agentId) void loadModels(agentId, true)
                    }}
                  >
                    重试
                  </button>
                </div>
              )}
              {models.length === 0 ? (
                <div className="px-3 py-2 text-sm text-neutral-500 dark:text-neutral-400">
                  {loading ? '正在探测模型…' : '该 CLI 未上报可用模型'}
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
