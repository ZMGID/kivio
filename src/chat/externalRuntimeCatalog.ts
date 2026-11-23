import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { chatApi, type DetectedExternalAgent } from './api'
import { useT } from '../components/i18n'
import { modelIncludesEffort, type ExternalModelChange } from './externalModelEffort'
import type { AgentRuntimeConfig } from './types'
import type { ExternalAgentModels } from '../api/externalCliSettings'
import { concreteCliModelId } from '../api/cliModelId'

type ExternalModel = DetectedExternalAgent['models'][number]
type ReasoningOption = NonNullable<DetectedExternalAgent['reasoningOptions']>[number]

/** ACP 探测常把思考开关报成 on/off，那不是档位。dsh 的 `off` 是真档位，会跟 high/max 一起出现。 */
const ACP_SWITCH_IDS = ['on', 'off', 'true', 'false', 'enabled', 'disabled']

function isAcpSwitchId(id: string): boolean {
  return ACP_SWITCH_IDS.includes(id.toLowerCase())
}

function isAcpSwitchOnlyList(options: { id: string }[]): boolean {
  return options.length > 0 && options.every((option) => isAcpSwitchId(option.id))
}

// 胶囊只显示模型名尾巴，去掉 provider 前缀（"foo/mimo-v2.5-pro" → "mimo-v2.5-pro"），
// 避免有意义的尾部被截断；下拉列表仍保留完整 id。
function stripProviderPrefix(label: string): string {
  const slash = label.lastIndexOf('/')
  return slash >= 0 ? label.slice(slash + 1) : label
}

// 胶囊隐藏模型名里的括号补充（"kimi (kimi-for-coding)" → "kimi"），下拉列表仍保留完整名。
function stripParenthetical(label: string): string {
  return label.replace(/\s*\([^)]*\)\s*$/, '').trim() || label
}

/** Codex `model/list` used to label efforts `high — Greater reasoning depth…`. Keep the name. */
export function stripEffortDescription(id: string, label: string): string {
  const text = (label || id).trim()
  const sep = ' — '
  const cut = text.indexOf(sep)
  if (cut <= 0) return text
  const head = text.slice(0, cut)
  if (head === id || head.toLowerCase() === id.toLowerCase()) return id
  return text
}

// CLI 常把档位报成小写 id（high / xhigh），与内置档位（High / XHigh）写法统一。
const EFFORT_DISPLAY: Record<string, string> = {
  none: 'None', off: 'Off', minimal: 'Minimal', low: 'Low', medium: 'Medium',
  high: 'High', xhigh: 'XHigh', max: 'Max', ultra: 'Ultra',
}

/** 档位显示名：去掉营销说明；名字就是小写 id 时按内置写法统一大小写，自定义名字原样保留。 */
export function formatEffortLabel(id: string, label: string): string {
  const text = stripEffortDescription(id, label)
  if (text !== id && text.toLowerCase() !== id.toLowerCase()) return text
  return EFFORT_DISPLAY[id.toLowerCase()] ?? (/^[a-z]/.test(text) ? text[0].toUpperCase() + text.slice(1) : text)
}

/** 本机 CLI 探测：每个会话一份结果，挂载与换会话时自动检测。 */
export function useExternalAgents(conversationId?: string | null) {
  const [agents, setAgents] = useState<DetectedExternalAgent[]>([])
  // 请求代际：conversationId 切换时并发发起检测，只让最新一次的结果落地。
  const agentsReqIdRef = useRef(0)

  useEffect(() => {
    const reqId = ++agentsReqIdRef.current
    chatApi
      .detectExternalAgents(false, conversationId)
      .then((list) => {
        if (agentsReqIdRef.current === reqId) setAgents(list)
      })
      .catch((err) => {
        console.error('Failed to detect external agents:', err)
        if (agentsReqIdRef.current === reqId) setAgents([])
      })
  }, [conversationId])

  // 设置页停用的不出现在列表里；已经绑定它的旧会话照常（当前代理按全量 agents 查名字）。
  const availableAgents = useMemo(
    () => agents.filter((agent) => agent.available && !agent.disabled),
    [agents],
  )
  return { agents, availableAgents }
}

export interface ExternalModelCatalog {
  models: ExternalModel[]
  loading: boolean
  initialLoading: boolean
  source: 'probed' | 'fallback'
  probeError: string | null
  /** 胶囊上的模型名。 */
  displayName: string
  /** 列表里视为当前的模型 id。 */
  activeModelId: string | null
  reasoningOptions: ReasoningOption[]
  reasoningValue: string
  /** 实际生效的档位：显式选择优先，否则是 CLI 当前配置；都没有为 null（Auto）。 */
  reasoningDisplayId: string | null
  reasoningLabel: string
  fastTier: { id: string; label: string } | undefined
  fastOn: boolean
  reload: (force?: boolean) => void
  /** 打开选择器时用：仅在已有真实探测结果时后台刷新。 */
  refreshOnOpen: () => void
  selectModel: (model: ExternalModel) => void
  /** null = 交回 CLI 默认。 */
  selectReasoning: (id: string | null) => void
  toggleFast: () => void
}

/**
 * 外部 CLI 的模型与思考档目录。懒查当前代理（cwd-scoped），用户未显式选择时把 CLI 当前配置
 * 同步回会话。必须常驻挂载：胶囊名字和自动同步都不依赖菜单是否打开。
 */
export function useExternalModelCatalog({
  agentRuntime,
  conversationId,
  onModelChange,
}: {
  agentRuntime: AgentRuntimeConfig
  conversationId?: string | null
  onModelChange: ExternalModelChange
}): ExternalModelCatalog {
  const t = useT()
  const agentId = agentRuntime.kind === 'external' ? agentRuntime.externalAgentId ?? null : null
  const [models, setModels] = useState<ExternalModel[]>([])
  const [reasoningOptions, setReasoningOptions] = useState<ReasoningOption[]>([])
  // kimi 等：档位随模型变（K3=low/high/max，K2.7 always_thinking=无）。按所选模型取。
  const [reasoningByModel, setReasoningByModel] = useState<Record<string, ReasoningOption[]>>({})
  const [loading, setLoading] = useState(false)
  // source: probed=真实探测 / fallback=探测失败降级静态表（显示提示 + 重试）。
  const [source, setSource] = useState<'probed' | 'fallback'>('probed')
  const [probeError, setProbeError] = useState<string | null>(null)
  // CLI 自己当前配置的模型（codex config.toml / ACP currentModelId / claude resolved）。
  const [currentModel, setCurrentModel] = useState<string | null>(null)
  const [currentReasoning, setCurrentReasoning] = useState<string | null>(null)
  // 请求代际：agent 切换/卸载时使在途请求失效，防止旧结果覆盖新 agent 或卸载后 setState。
  const modelsReqIdRef = useRef(0)
  const modelSelectionRef = useRef(0)
  // Only retain the displayed catalog while refreshing the same CLI and conversation.
  const lastScopeRef = useRef<string | null>(null)
  // 用 ref 读最新 runtime / onModelChange，避免把它们塞进 loadModels 依赖导致每次选模型重探。
  const runtimeRef = useRef(agentRuntime)
  runtimeRef.current = agentRuntime
  const onModelChangeRef = useRef(onModelChange)
  onModelChangeRef.current = onModelChange

  const loadModels = useCallback(
    (id: string, force?: boolean) => {
      const reqId = ++modelsReqIdRef.current
      const selection = modelSelectionRef.current
      setLoading(true)
      const applyResult = (result: ExternalAgentModels) => {
        if (modelsReqIdRef.current !== reqId) return
        setModels(result.models)
        setReasoningOptions(result.reasoningOptions)
        setReasoningByModel(result.reasoningByModel ?? {})
        setSource(result.source)
        setProbeError(result.probeError ?? null)
        setCurrentModel(result.currentModel ?? null)
        setCurrentReasoning(result.currentReasoning ?? null)
        // A saved catalog may be displayed, but cannot change the conversation's configuration.
        if (result.refreshing || result.source === 'fallback' || selection !== modelSelectionRef.current) return
        // 自动同步 CLI 当前配置：仅当用户未显式选择（externalModel 空 / 'default'）时。
        const rt = runtimeRef.current
        const explicitModel = !!concreteCliModelId(rt.externalModel)
        const effectiveModel = explicitModel ? rt.externalModel : result.currentModel
        const effectiveLabel = result.models.find((m) => m.id === effectiveModel)?.label
        if (modelIncludesEffort(id, effectiveModel, effectiveLabel)) {
          const nextModel = explicitModel ? rt.externalModel! : result.models.some((m) => m.id === effectiveModel)
            ? effectiveModel! : rt.externalModel ?? 'default'
          if (nextModel !== (rt.externalModel ?? 'default') || rt.externalReasoning != null) {
            onModelChangeRef.current(nextModel, null, undefined, 'catalog')
          }
          return
        }
        if (explicitModel) return
        const cur = result.currentModel
        // 当前模型是列表里可选的真实 id（grok / codex）才回填 externalModel；claude 的 resolved
        // model 不在别名目录里，不回填（发送仍走 CLI 默认），仅由显示层展示其名字。
        const selectable = !!cur && result.models.some((m) => m.id === cur)
        const nextModel = selectable ? (cur as string) : rt.externalModel ?? 'default'
        const explicitReasoning = !!rt.externalReasoning && rt.externalReasoning !== 'default'
        // 回填当前档位：只认「确实在该模型档位列表里」的值（真相源是 reasoningByModel / reasoningOptions）。
        const probedReasoning = result.currentReasoning
        const activeOpts = (cur && result.reasoningByModel?.[cur]) || result.reasoningOptions
        const saneReasoning = probedReasoning && activeOpts.some((o) => o.id === probedReasoning)
          ? probedReasoning
          : null
        const nextReasoning = !explicitReasoning && saneReasoning ? saneReasoning : rt.externalReasoning ?? null
        if (nextModel !== (rt.externalModel ?? 'default') || nextReasoning !== (rt.externalReasoning ?? null)) {
          onModelChangeRef.current(nextModel, nextReasoning, rt.externalServiceTier ?? undefined, 'catalog')
        }
      }
      return chatApi
        .detectExternalAgentModels(id, conversationId, force, applyResult)
        .then(applyResult)
        .catch((error: unknown) => {
          if (modelsReqIdRef.current !== reqId) return
          // 不再静默吞错：置为降级态，展示重试。保留上次模型列表不清空。
          setSource('fallback')
          setProbeError(error instanceof Error ? error.message : String(error))
        })
        .finally(() => {
          if (modelsReqIdRef.current === reqId) setLoading(false)
        })
    },
    [conversationId],
  )

  useEffect(() => {
    const scope = `${agentId}:${conversationId ?? ''}`
    if (lastScopeRef.current !== scope) {
      lastScopeRef.current = scope
      // 换 agent：旧 CLI 的 currentModel 立刻失效（探测中显示「获取中…」而非上个 CLI 的模型名）。
      setCurrentModel(null)
      setProbeError(null)
      setCurrentReasoning(null)
      setReasoningByModel({})
      setModels([])
      setReasoningOptions([])
    }
    if (!agentId) {
      // 失效在途请求，防止旧结果落到已清空的状态上。
      modelsReqIdRef.current++
      setSource('probed')
      setLoading(false)
      return
    }
    void loadModels(agentId)
    const pendingRequests = modelsReqIdRef
    return () => { pendingRequests.current++ }
  }, [agentId, conversationId, loadModels])

  const explicitModelId = concreteCliModelId(agentRuntime.externalModel)
  const activeModelId = explicitModelId ?? currentModel

  // 当前展示用的 effort 列表：优先按所选/当前模型从 reasoningByModel 取（kimi 按模型变档）。
  const activeReasoningOptions = useMemo(() => {
    if (modelIncludesEffort(agentId, activeModelId, models.find((m) => m.id === activeModelId)?.label)) return []
    if (activeModelId && Object.prototype.hasOwnProperty.call(reasoningByModel, activeModelId)) {
      return reasoningByModel[activeModelId] ?? []
    }
    // 整表都是 ACP 开关才藏档位。不要从混有 high/max 的表里单独抠掉 off——dsh 的 off 是真档位。
    if (isAcpSwitchOnlyList(reasoningOptions)) return []
    return reasoningOptions
  }, [agentId, activeModelId, models, reasoningByModel, reasoningOptions])

  const selectedModel = models.find(model => model.id === activeModelId)
  const fastTier = (selectedModel?.capabilities?.serviceTiers ?? []).find(tier => tier.id !== 'default')
  const fastOn = !!fastTier && agentRuntime.externalServiceTier === fastTier.id

  const reasoningValue = agentRuntime.externalReasoning ?? 'default'
  const explicitReasoning = !!agentRuntime.externalReasoning && agentRuntime.externalReasoning !== 'default'
  // 未显式选择时跟模型胶囊同一口径：优先展示 CLI 实际在用的档位，而不是「Auto」。
  const displayId = explicitReasoning
    ? reasoningValue
    : currentReasoning && currentReasoning !== 'default' ? currentReasoning : reasoningValue
  const reasoningDisplayId = displayId !== 'default' && activeReasoningOptions.some((o) => o.id === displayId)
    ? displayId
    : null
  const reasoningLabel = useMemo(() => {
    const opt = activeReasoningOptions.find((o) => o.id === displayId)
    if (opt) {
      const label = formatEffortLabel(opt.id, opt.label || displayId)
      return label === 'Default' ? 'Auto' : label
    }
    // 未显式选择且探测也没有档位时显示「自动」；残留的 ACP on/off 开关也当自动。
    if (displayId === 'Default' || displayId === 'default' || isAcpSwitchId(String(displayId))) return 'Auto'
    return displayId
  }, [activeReasoningOptions, displayId])

  const displayName = useMemo(() => {
    const formatLabel = (label: string) => agentId === 'antigravity'
      ? label
      : stripProviderPrefix(stripParenthetical(label))
    // 显式选择：显示所选模型 label（探测中列表未到时退回原始 id）。
    if (explicitModelId) {
      return formatLabel(models.find((item) => item.id === explicitModelId)?.label ?? explicitModelId)
    }
    // 旧会话可能把模型交给 CLI。显示已知的实际模型，未知时提示选择。
    if (currentModel) return formatLabel(models.find((item) => item.id === currentModel)?.label ?? currentModel)
    if (loading && models.length === 0) return t.chatRuntimeFetching
    return t.chatSelectModel
  }, [agentId, explicitModelId, models, currentModel, loading, t])

  const reload = useCallback((force?: boolean) => {
    if (agentId) void loadModels(agentId, force)
  }, [agentId, loadModels])

  const refreshOnOpen = useCallback(() => {
    if (!loading && source === 'probed' && agentId) void loadModels(agentId)
  }, [agentId, loadModels, loading, source])

  const selectModel = useCallback((model: ExternalModel) => {
    // 换模型时 effort 可能变（kimi K3 有 low/high/max，K2.7 无）。旧值不在新列表里 → 清掉或落到 high。
    const efforts = reasoningByModel[model.id]
    const cur = agentRuntime.externalReasoning
    const curOk = !!cur && cur !== 'default' && (!efforts || efforts.some((o) => o.id === cur))
    let nextReasoning: string | null = curOk ? (cur as string) : null
    if (!curOk && efforts && efforts.length > 0) {
      nextReasoning = efforts.find((o) => o.id === 'high')?.id ?? efforts[0]?.id ?? null
    }
    if (efforts && efforts.length === 0) nextReasoning = null
    if (modelIncludesEffort(agentId, model.id, model.label)) nextReasoning = null
    modelSelectionRef.current++
    onModelChange(model.id, nextReasoning)
  }, [agentId, agentRuntime.externalReasoning, onModelChange, reasoningByModel])

  const selectReasoning = useCallback((id: string | null) => {
    modelSelectionRef.current++
    onModelChange(agentRuntime.externalModel ?? 'default', id)
  }, [agentRuntime.externalModel, onModelChange])

  const toggleFast = useCallback(() => {
    if (!fastTier) return
    modelSelectionRef.current++
    onModelChange(agentRuntime.externalModel ?? 'default', undefined, fastOn ? null : fastTier.id)
  }, [agentRuntime.externalModel, fastOn, fastTier, onModelChange])

  return {
    models,
    loading,
    initialLoading: loading && models.length === 0,
    source,
    probeError,
    displayName,
    activeModelId,
    reasoningOptions: activeReasoningOptions,
    reasoningValue,
    reasoningDisplayId,
    reasoningLabel,
    fastTier,
    fastOn,
    reload,
    refreshOnOpen,
    selectModel,
    selectReasoning,
    toggleFast,
  }
}
