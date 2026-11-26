import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { chatApi, type DetectedExternalAgent } from './api'
import { useT } from '../components/i18n'
import { modelIncludesEffort, type ExternalModelChange } from './externalModelEffort'
import type { AgentRuntimeConfig } from './types'
import type { ExternalAgentModels } from '../api/externalCliSettings'
import { concreteCliModelId } from '../api/cliModelId'
import { matchModelExact } from '../data/modelMatching'

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
export function useExternalAgents(conversationId?: string | null, enabled = true) {
  const [agents, setAgents] = useState<DetectedExternalAgent[]>([])

  useEffect(() => {
    if (!enabled) return
    let active = true
    chatApi
      .detectExternalAgents(false, conversationId)
      .then((list) => {
        if (active) setAgents(list)
      })
      .catch((err) => {
        console.error('Failed to detect external agents:', err)
        if (active) setAgents([])
      })
    return () => { active = false }
  }, [conversationId, enabled])

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
  /** 实际生效的档位：显式选择优先，否则是 CLI 当前配置；都没有为 null（Auto）。 */
  reasoningDisplayId: string | null
  reasoningLabel: string | null
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

type CatalogProbe = {
  scope: string
  result: ExternalAgentModels | null
  loading: boolean
  error: string | null
}

const EMPTY_MODELS: ExternalModel[] = []
const EMPTY_REASONING_OPTIONS: ReasoningOption[] = []
const EMPTY_REASONING_BY_MODEL: Record<string, ReasoningOption[]> = {}

/**
 * 外部 CLI 的模型与思考档目录。懒查当前代理（cwd-scoped），用户未显式选择时把 CLI 当前配置
 * 同步回会话。必须常驻挂载：胶囊名字和自动同步都不依赖菜单是否打开。
 */
export function useExternalModelCatalog({
  agentRuntime,
  conversationId,
  readOnly = false,
  onModelChange,
}: {
  agentRuntime: AgentRuntimeConfig
  conversationId?: string | null
  readOnly?: boolean
  onModelChange: ExternalModelChange
}): ExternalModelCatalog {
  const t = useT()
  const agentId = agentRuntime.kind === 'external' ? agentRuntime.externalAgentId ?? null : null
  const scope = agentId ? JSON.stringify([agentId, conversationId ?? null, agentRuntime.externalAgentPreset ?? null]) : null
  const [probe, setProbe] = useState<CatalogProbe | null>(null)
  const activeProbe = probe?.scope === scope ? probe : null
  const result = activeProbe?.result
  const models = result?.models ?? EMPTY_MODELS
  const reasoningOptions = result?.reasoningOptions ?? EMPTY_REASONING_OPTIONS
  const reasoningByModel = result?.reasoningByModel ?? EMPTY_REASONING_BY_MODEL
  const loading = !!scope && !readOnly && (activeProbe?.loading ?? true)
  const probeError = activeProbe?.error ?? result?.probeError ?? null
  const source = activeProbe?.error != null ? 'fallback' : result?.source ?? 'probed'
  const modelsReqIdRef = useRef(0)
  const modelSelectionRef = useRef(0)
  const runtimeRef = useRef(agentRuntime)
  const onModelChangeRef = useRef(onModelChange)
  useLayoutEffect(() => {
    runtimeRef.current = agentRuntime
    onModelChangeRef.current = onModelChange
  }, [agentRuntime, onModelChange])

  const loadModels = useCallback(
    (force?: boolean) => {
      if (!agentId || !scope || readOnly) return
      const reqId = ++modelsReqIdRef.current
      const selection = modelSelectionRef.current
      setProbe((prev) => ({ scope, result: prev?.scope === scope ? prev.result : null, loading: true, error: null }))
      const applyResult = (result: ExternalAgentModels) => {
        if (modelsReqIdRef.current !== reqId) return
        setProbe({ scope, result, loading: !!result.refreshing, error: null })
        // A saved catalog may be displayed, but cannot change the conversation's configuration.
        if (result.refreshing || result.source === 'fallback' || selection !== modelSelectionRef.current) return
        // 自动同步 CLI 当前配置：仅当用户未显式选择（externalModel 空 / 'default'）时。
        const rt = runtimeRef.current
        const explicitModel = !!concreteCliModelId(rt.externalModel)
        const effectiveModel = explicitModel ? rt.externalModel : result.currentModel
        const effectiveLabel = result.models.find((m) => m.id === effectiveModel)?.label
        if (modelIncludesEffort(agentId, effectiveModel, effectiveLabel)) {
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
        .detectExternalAgentModels(agentId, conversationId, force, applyResult)
        .then(applyResult)
        .catch((error: unknown) => {
          if (modelsReqIdRef.current !== reqId) return
          setProbe((prev) => ({
            scope, result: prev?.scope === scope ? prev.result : null, loading: false,
            error: error instanceof Error ? error.message : String(error),
          }))
        })
    },
    [agentId, conversationId, readOnly, scope],
  )

  useLayoutEffect(() => {
    void loadModels()
    const pendingRequests = modelsReqIdRef
    return () => { pendingRequests.current++ }
  }, [loadModels])

  const explicitModelId = concreteCliModelId(agentRuntime.externalModel)
  const activeModelId = explicitModelId ?? result?.currentModel ?? null
  const selectedModel = models.find(model => model.id === activeModelId)
  const effortInModel = modelIncludesEffort(agentId, activeModelId, selectedModel?.label)

  // 当前展示用的 effort 列表：优先按所选/当前模型从 reasoningByModel 取（kimi 按模型变档）。
  const activeReasoningOptions = useMemo(() => {
    if (effortInModel) return EMPTY_REASONING_OPTIONS
    if (activeModelId && Object.prototype.hasOwnProperty.call(reasoningByModel, activeModelId)) {
      return reasoningByModel[activeModelId] ?? EMPTY_REASONING_OPTIONS
    }
    // 整表都是 ACP 开关才藏档位。不要从混有 high/max 的表里单独抠掉 off——dsh 的 off 是真档位。
    if (isAcpSwitchOnlyList(reasoningOptions)) return EMPTY_REASONING_OPTIONS
    return reasoningOptions
  }, [activeModelId, effortInModel, reasoningByModel, reasoningOptions])

  const fastTier = (selectedModel?.capabilities?.serviceTiers ?? []).find(tier => tier.id !== 'default')
  const fastOn = !!fastTier && agentRuntime.externalServiceTier === fastTier.id

  const reasoningValue = agentRuntime.externalReasoning
  const explicitReasoning = reasoningValue && reasoningValue.toLowerCase() !== 'default' ? reasoningValue : null
  const effortId = explicitReasoning ?? result?.currentReasoning ?? 'default'
  const effortOption = activeReasoningOptions.find((option) => option.id === effortId)
  const reasoningDisplayId = effortId !== 'default' ? effortOption?.id ?? null : null
  const resolved = source === 'probed' && !!result && !result.refreshing
  const pendingEffort = !resolved && explicitReasoning && !effortInModel
    && (!isAcpSwitchId(explicitReasoning) || (agentId === 'dsh' && explicitReasoning === 'off'))
  const effortLabel = effortOption
    ? formatEffortLabel(effortOption.id, effortOption.label)
    : pendingEffort ? formatEffortLabel(effortId, effortId)
      : activeReasoningOptions.length ? (isAcpSwitchId(effortId) ? 'Auto' : effortId) : null
  const reasoningLabel = effortLabel?.toLowerCase() === 'default' ? 'Auto' : effortLabel

  const catalogModelLabel = selectedModel?.label
  const modelLabel = activeModelId && (
    catalogModelLabel && (resolved || catalogModelLabel !== activeModelId)
      ? catalogModelLabel
      : matchModelExact(activeModelId)?.displayName ?? activeModelId
  )
  const displayName = modelLabel
    ? agentId === 'antigravity' ? modelLabel : stripProviderPrefix(stripParenthetical(modelLabel))
    : loading && models.length === 0 ? t.chatRuntimeFetching : t.chatSelectModel

  const refreshOnOpen = useCallback(() => {
    if (!loading && source === 'probed') void loadModels()
  }, [loadModels, loading, source])

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
    reasoningDisplayId,
    reasoningLabel,
    fastTier,
    fastOn,
    reload: loadModels,
    refreshOnOpen,
    selectModel,
    selectReasoning,
    toggleFast,
  }
}
