import type { ExternalModelChange } from './externalModelEffort'
import { providerModelLabel } from '../data/modelCatalog'
import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { Loader2, Lock, Zap } from 'lucide-react'
import { AgentIcon } from '../components/AgentIcon'
import { ModelIcon, hasModelIcon } from '../components/ModelIcon'
import { IconButton } from '../components/Button'
import { useLang, useT } from '../components/i18n'
import { chatTitlebarPillButtonClass } from './platform'
import { usePopoverMenu } from './usePopoverMenu'
import { initialExternalRuntime } from './lastAgentRuntime'
import { useThinkingLevel } from './useThinkingLevel'
import {
  formatEffortLabel,
  useExternalAgents,
  useExternalModelCatalog,
  type ExternalModelCatalog,
} from './externalRuntimeCatalog'
import { BuiltinModelList, ModelRowCheck, ModelSearchField } from './ModelSelector'
import {
  modelRowButtonClass,
  modelRowClass,
  useAutoFocusSearch,
  useBuiltinModelCatalog,
  useRevealCurrentRow,
} from './builtinModelCatalog'
import type { AgentRuntimeConfig, ThinkingLevel } from './types'
import './agentModelPicker.css'

const KIVIO_LOGO_SRC = '/logo-mark.png'

interface ModelPresentation {
  modelLabel: string
  modelId: string | null
  effortLabel: string | null
  fastOn: boolean
}

const recentModelPresentations = new Map<string, ModelPresentation>()
const MAX_MODEL_PRESENTATIONS = 32

const BUILTIN: AgentRuntimeConfig = {
  kind: 'builtin',
  externalAgentId: null,
  externalModel: null,
  externalReasoning: null,
}

const CHAT: AgentRuntimeConfig = {
  kind: 'chat',
  externalAgentId: null,
  externalModel: null,
  externalReasoning: null,
}

/** Same brand mark as Agent; `variant` only changes color treatment so the shape stays identical. */
function KivioMark({ size = 18, variant = 'agent' }: { size?: number; variant?: 'agent' | 'chat' }) {
  return (
    <img
      src={KIVIO_LOGO_SRC}
      alt=""
      aria-hidden="true"
      className={variant === 'chat' ? 'kv-agent-picker__logo kv-agent-picker__logo--chat' : 'kv-agent-picker__logo'}
      width={size}
      height={size}
      draggable={false}
    />
  )
}

function pickerPosition(trigger: HTMLButtonElement) {
  const anchor = trigger.getBoundingClientRect()
  const edge = 8
  const width = Math.min(420, window.innerWidth - 2 * edge)
  const top = anchor.bottom + edge
  return {
    top,
    left: Math.max(edge, Math.min(anchor.left, window.innerWidth - width - edge)),
    width,
    height: Math.max(0, Math.min(460, window.innerHeight - top - edge)),
  }
}

export interface AgentModelPickerProps {
  agentRuntime: AgentRuntimeConfig
  conversationId?: string | null
  /** Imported native sessions remain bound to their original CLI. */
  locked?: boolean
  busy?: boolean
  /** Keeps the titlebar visible while the conversation replay is being verified. */
  readOnly?: boolean
  /** 内置运行时的服务商与模型。 */
  providerId: string
  model: string
  thinkingLevel: ThinkingLevel | null
  onRuntimeChange: (runtime: AgentRuntimeConfig) => void
  onModelChange: (providerId: string, model: string) => void
  onExternalModelChange: ExternalModelChange
  /** null = 清掉显式档位，按模型默认档处理。 */
  onThinkingLevelChange: (level: ThinkingLevel | null) => void
}

/**
 * 顶栏的「代理 · 模型 · 思考强度」合一选择器：左栏选代理，右侧选模型，底部调思考强度。
 * 选模型后关闭；换代理、调强度不关闭，方便连续设置。
 */
function AgentModelPickerBase({
  agentRuntime,
  conversationId,
  locked = false,
  busy = false,
  readOnly = false,
  providerId,
  model,
  thinkingLevel,
  onRuntimeChange,
  onModelChange,
  onExternalModelChange,
  onThinkingLevelChange,
}: AgentModelPickerProps) {
  const t = useT()
  const lang = useLang()
  const triggerRef = useRef<HTMLButtonElement>(null)
  const [position, setPosition] = useState<ReturnType<typeof pickerPosition> | null>(null)
  const open = !readOnly && position !== null
  const menuRef = useRef<HTMLDivElement>(null)
  const close = useCallback(() => setPosition(null), [])
  useLayoutEffect(() => {
    if (readOnly) close()
  }, [readOnly, close])
  usePopoverMenu(open, close, menuRef)
  useLayoutEffect(() => {
    const trigger = triggerRef.current
    if (!open || !trigger) return
    const update = () => setPosition(pickerPosition(trigger))
    const observer = new ResizeObserver(update)
    observer.observe(trigger)
    const pane = trigger.closest('.chat-main-pane, .chat-titlebar-strip-content')
    if (pane) observer.observe(pane)
    window.addEventListener('resize', update)
    return () => {
      observer.disconnect()
      window.removeEventListener('resize', update)
    }
  }, [open])

  const usesExternal = agentRuntime.kind === 'external' && !!agentRuntime.externalAgentId
  const usesChat = agentRuntime.kind === 'chat'
  const usesBuiltinAgent = !usesExternal && !usesChat

  const { agents, availableAgents } = useExternalAgents(conversationId, !readOnly)
  const builtinCatalog = useBuiltinModelCatalog(providerId, model)
  const external = useExternalModelCatalog({
    agentRuntime,
    conversationId,
    readOnly,
    onModelChange: onExternalModelChange,
  })
  const thinking = useThinkingLevel({
    enabled: !usesExternal && !readOnly,
    value: thinkingLevel,
    currentProviderId: providerId,
    currentModel: model,
    onChange: onThinkingLevelChange,
    lang: lang === 'zh' ? 'zh' : 'en',
  })

  const currentAgent = agents.find((item) => item.id === agentRuntime.externalAgentId)
  const agentLabel = usesExternal
    ? currentAgent?.name ?? agentRuntime.externalAgentId ?? t.chatRuntimeLocalCli
    : usesChat ? 'Kivio Chat' : 'Kivio Agent'
  const liveModelLabel = usesExternal
    ? external.displayName
    : providerModelLabel(model || builtinCatalog.currentProvider?.enabledModels[0] || t.chatSelectModel, builtinCatalog.currentProvider)
  const liveEffortLabel = usesExternal
    ? external.reasoningLabel
    : !thinking.loaded && thinkingLevel ? formatEffortLabel(thinkingLevel, thinkingLevel)
      : thinking.hidden ? null : thinking.effectiveLabel
  const liveFastOn = usesExternal && external.fastOn
  const liveModelId = usesExternal
    ? external.activeModelId
    : model || null
  const presentationKey = conversationId ? JSON.stringify(usesExternal
    ? [conversationId, 'external', agentRuntime.externalAgentId, agentRuntime.externalAgentPreset ?? null,
        agentRuntime.externalModel ?? null, agentRuntime.externalReasoning ?? null, agentRuntime.externalServiceTier ?? null]
    : [conversationId, agentRuntime.kind, providerId, model, thinkingLevel, lang]) : null
  const presentationReady = !readOnly && (usesExternal ? !external.loading : thinking.loaded)
  const remembered = presentationKey && !presentationReady ? recentModelPresentations.get(presentationKey) : undefined
  const { modelLabel, modelId, effortLabel, fastOn } = remembered ?? {
    modelLabel: liveModelLabel, modelId: liveModelId, effortLabel: liveEffortLabel, fastOn: liveFastOn,
  }
  useLayoutEffect(() => {
    if (!presentationKey || !presentationReady) return
    recentModelPresentations.delete(presentationKey)
    recentModelPresentations.set(presentationKey, {
      modelLabel: liveModelLabel, modelId: liveModelId, effortLabel: liveEffortLabel, fastOn: liveFastOn,
    })
    const oldest = recentModelPresentations.keys().next().value
    if (recentModelPresentations.size > MAX_MODEL_PRESENTATIONS && oldest !== undefined) {
      recentModelPresentations.delete(oldest)
    }
  }, [presentationKey, presentationReady, liveModelLabel, liveModelId, liveEffortLabel, liveFastOn])

  // 认不出品牌的模型不画首字母方块，也不带「/」。
  const pillModelId = modelId && hasModelIcon(modelId) ? modelId : null
  const providerName = !usesExternal && model ? builtinCatalog.currentProvider?.name : undefined
  const summary = [
    agentLabel,
    providerName ? `${providerName}：${modelLabel}` : modelLabel,
    effortLabel ? t.chatThinkingLevel.replace('{level}', effortLabel) : null,
    fastOn ? t.chatFastMode : null,
    locked ? t.chatRuntimeBoundHint : null,
  ].filter(Boolean).join(' · ')

  const selectionRequest = useRef<symbol | null>(null)
  const [pendingAgent, setPendingAgent] = useState<string | null>(null)
  useLayoutEffect(() => {
    setPendingAgent(null)
    return () => { selectionRequest.current = null }
  }, [conversationId, agentRuntime.kind, agentRuntime.externalAgentId, busy, locked, open])

  const switchRuntime = async (target: AgentRuntimeConfig | string, active: boolean) => {
    const request = Symbol()
    selectionRequest.current = request
    setPendingAgent(null)
    if (locked || busy || active) return
    if (typeof target !== 'string') {
      onRuntimeChange(target)
      return
    }
    setPendingAgent(target)
    const next = await initialExternalRuntime(target, conversationId)
    if (request !== selectionRequest.current) return
    setPendingAgent(null)
    onRuntimeChange(next)
  }

  const boundBadge = locked
    ? (
      <span className="kv-agent-picker__badge" title={t.chatRuntimeBoundHint}>
        <Lock size={11} aria-hidden="true" />
        {t.chatRuntimeBound}
      </span>
    )
    : null
  const searchPlaceholder = t.chatSearchAgentModels.replace('{agent}', agentLabel)
  // 档位落点：CLI 的 default 不占轨道位置；未显式选择时滑杆显示 CLI 实际在用的档。
  const effortSteps: EffortStep[] = usesExternal
    ? external.reasoningOptions
      .filter((option) => option.id !== 'default')
      .map((option) => ({ value: option.id, label: formatEffortLabel(option.id, option.label), disabled: false }))
    : thinking.hidden
      ? []
      : thinking.options.map((option) => ({
        ...option,
        // 「是否支持关闭思考」类说明只挂在 Off 上，不再常驻一段提示文字。
        title: option.value === 'off' && thinking.notice ? `${option.label} · ${thinking.notice}` : undefined,
      }))

  return (
    <div className="kv-agent-picker" data-tauri-drag-region="false">
      <button
        type="button"
        ref={triggerRef}
        onClick={(event) => {
          if (open) {
            close()
            return
          }
          // 常用入口：无论鼠标还是键盘打开，焦点都进搜索框，直接打字、方向键下移、回车选中。
          if (usesExternal) external.refreshOnOpen()
          setPosition(pickerPosition(event.currentTarget))
        }}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={summary}
        disabled={readOnly}
        title={summary}
        className={`${chatTitlebarPillButtonClass} max-w-full min-w-0`}
      >
        {/* 胶囊像面包屑：代理图标 / 模型图标 模型名 强度。代理图标直接按 externalAgentId 取，不等检测结果；
            CLI 未定具体模型（Auto）时只留代理图标。 */}
        {usesExternal && agentRuntime.externalAgentId
          ? <AgentIcon id={agentRuntime.externalAgentId} size={16} />
          : <KivioMark size={16} variant={usesChat ? 'chat' : 'agent'} />}
        {pillModelId && (
          <>
            <span className="kv-agent-picker__crumb-sep" aria-hidden="true">/</span>
            <ModelIcon model={pillModelId} size={16} />
          </>
        )}
        <span
          className={`chat-model-selector-label max-w-[200px] truncate font-medium ${
            usesExternal && external.initialLoading && !remembered && !modelId ? 'reasoning-shimmer-text' : 'text-neutral-800'
          }`}
        >
          {modelLabel}
        </span>
        {effortLabel && (
          <span className="chat-thinking-level-label kv-agent-picker__effort-label">{effortLabel}</span>
        )}
        {fastOn && <Zap size={13} fill="currentColor" className="shrink-0 text-[var(--accent)]" aria-hidden="true" />}
      </button>

      {open && position && createPortal(
        <>
          <div className="fixed inset-0 z-[1000]" onClick={close} aria-hidden />
          <div
            ref={menuRef}
            role="dialog"
            aria-label={summary}
            // 定高：换代理时列表长短、底栏有无都不该让弹层伸缩。
            style={position}
            className="kv-agent-picker__popover kv-menu chat-motion-popover"
          >
            {/* 左栏只放代理图标（名字在悬停提示和右侧搜索框里），把宽度留给模型列表。
                本机 CLI 在挂载与换会话时自动检测，这里不再放手动刷新。 */}
            <div className="kv-agent-picker__rail custom-scrollbar" role="radiogroup" aria-label={t.chatRuntimeAgent}>
              <AgentOption
                label="Kivio Agent"
                icon={<KivioMark size={18} variant="agent" />}
                active={usesBuiltinAgent}
                disabled={(locked || busy) && !usesBuiltinAgent}
                onSelect={() => switchRuntime(BUILTIN, usesBuiltinAgent)}
              />
              <AgentOption
                label="Kivio Chat"
                icon={<KivioMark size={18} variant="chat" />}
                active={usesChat}
                disabled={(locked || busy) && !usesChat}
                onSelect={() => switchRuntime(CHAT, usesChat)}
              />
              {availableAgents.length > 0 && <span className="kv-agent-picker__rail-sep" aria-hidden="true" />}
              {availableAgents.map((agent) => {
                const active = usesExternal && agentRuntime.externalAgentId === agent.id
                return (
                  <AgentOption
                    key={agent.id}
                    label={agent.name}
                    title={agent.version ? `${agent.name} · ${agent.version}` : agent.name}
                    icon={pendingAgent === agent.id
                      ? <Loader2 size={18} role="status" aria-label={t.chatRuntimeFetching} className="animate-spin" />
                      : <AgentIcon id={agent.id} size={18} />}
                    active={active}
                    disabled={(locked || busy) && !active}
                    onSelect={() => switchRuntime(agent.id, active)}
                  />
                )
              })}
            </div>

            <div className="kv-agent-picker__pane">
              {usesExternal ? (
                <ExternalModelList
                  catalog={external}
                  agentId={agentRuntime.externalAgentId ?? ''}
                  autoFocusSearch
                  searchPlaceholder={searchPlaceholder}
                  searchTrailing={boundBadge}
                  onSelected={close}
                />
              ) : (
                <BuiltinModelList
                  catalog={builtinCatalog}
                  currentProviderId={providerId}
                  providerLocked={locked || busy}
                  currentModel={model}
                  autoFocusSearch
                  alwaysShowSearch
                  searchPlaceholder={searchPlaceholder}
                  searchTrailing={boundBadge}
                  onSelect={(nextProviderId, nextModel) => {
                    onModelChange(nextProviderId, nextModel)
                    close()
                  }}
                />
              )}
              <EffortSlider
                steps={effortSteps}
                value={usesExternal ? external.reasoningDisplayId : thinking.effective}
                title={liveEffortLabel ?? ''}
                disabled={!usesExternal && !thinking.loaded}
                onSelect={(value) => {
                  if (usesExternal) external.selectReasoning(value)
                  else onThinkingLevelChange(value as ThinkingLevel)
                }}
                pending={usesExternal && external.initialLoading}
                fastTier={usesExternal ? external.fastTier : undefined}
                fastOn={liveFastOn}
                onToggleFast={external.toggleFast}
              />
            </div>
          </div>
        </>,
        document.body,
      )}
    </div>
  )
}

function AgentOption({
  label,
  title,
  icon,
  active,
  disabled,
  onSelect,
}: {
  label: string
  title?: string
  icon: ReactNode
  active: boolean
  disabled: boolean
  onSelect: () => void
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={active}
      aria-label={label}
      disabled={disabled}
      title={title ?? label}
      onClick={onSelect}
      className={`kv-agent-picker__agent${active ? ' is-active' : ''}`}
    >
      <span className="kv-agent-picker__agent-icon">{icon}</span>
    </button>
  )
}

/** `active` 持续超过 `delayMs` 才变 true；变回 false 立即生效。用来压掉一闪而过的加载提示。 */
function useDelayedFlag(active: boolean, delayMs: number): boolean {
  const [shown, setShown] = useState(false)
  useEffect(() => {
    if (!active) {
      setShown(false)
      return
    }
    const timer = window.setTimeout(() => setShown(true), delayMs)
    return () => window.clearTimeout(timer)
  }, [active, delayMs])
  return active && shown
}

function ExternalModelList({
  catalog,
  agentId,
  autoFocusSearch,
  searchPlaceholder,
  searchTrailing,
  onSelected,
}: {
  catalog: ExternalModelCatalog
  agentId: string
  autoFocusSearch: boolean
  searchPlaceholder: string
  searchTrailing: ReactNode
  onSelected: () => void
}) {
  const t = useT()
  const zh = useLang() === 'zh'
  const [query, setQuery] = useState('')
  const searchRef = useRef<HTMLInputElement>(null)
  const listRef = useRef<HTMLDivElement>(null)
  const { models, loading, source, probeError } = catalog
  // 换代理时先有缓存快照（通常几十毫秒内到）：空窗口先留白，超过一小段时间仍在探测才显示提示，
  // 避免「正在探测」一闪而过。
  const probing = loading && models.length === 0
  const showProbing = useDelayedFlag(probing, 300)
  useAutoFocusSearch(autoFocusSearch, searchRef)
  useRevealCurrentRow(listRef)
  const normalizedQuery = query.trim().toLowerCase()
  const visibleModels = useMemo(() => normalizedQuery
    ? models.filter((item) => item.id.toLowerCase().includes(normalizedQuery) || item.label.toLowerCase().includes(normalizedQuery))
    : models, [models, normalizedQuery])

  const select = (item: (typeof models)[number]) => {
    catalog.selectModel(item)
    onSelected()
  }

  return (
    <>
      <ModelSearchField
        value={query}
        onChange={setQuery}
        inputRef={searchRef}
        placeholder={searchPlaceholder}
        trailing={searchTrailing}
        onSubmit={() => { if (visibleModels[0]) select(visibleModels[0]) }}
      />
      {source === 'fallback' && !loading && (
        <div className="kv-agent-picker__fallback">
          <span>{t.chatRuntimeProbeFailed}</span>
          <button type="button" className="kv-agent-picker__fallback-retry" onClick={() => catalog.reload(true)}>
            {t.chatRetry}
          </button>
        </div>
      )}
      {source === 'fallback' && !loading && probeError && (
        <p className="mx-2 mb-2 break-words text-[11px] text-neutral-500" role="status">{probeError}</p>
      )}
      <div ref={listRef} className="custom-scrollbar min-h-0 flex-1 overflow-y-auto px-1 pb-1" key={agentId}>
        {models.length === 0 ? (
          probing && !showProbing ? null : (
            <div className={`kv-menu-row ${loading ? 'reasoning-shimmer-text' : 'text-neutral-500'}`}>
              {loading ? t.chatRuntimeProbingModels : t.chatRuntimeNoModels}
            </div>
          )
        ) : visibleModels.length === 0 ? (
          <div className="px-4 py-6 text-center text-[12px] text-neutral-500">{t.chatNoMatchingModels}</div>
        ) : (
          visibleModels.map((item, index) => {
            const selected = catalog.activeModelId === item.id
            const textOnly = item.capabilities?.inputModalities && !item.capabilities.inputModalities.includes('image')
            return (
              <div key={item.id} className={modelRowClass(Boolean(normalizedQuery) && index === 0)}>
                <button
                  type="button"
                  aria-current={selected ? 'true' : undefined}
                  onClick={() => select(item)}
                  className={modelRowButtonClass(selected)}
                >
                  <ModelIcon model={item.id} size={16} />
                  <span className="min-w-0 truncate">{item.label}</span>
                  {textOnly && <span className="ml-auto shrink-0 text-[11px] font-normal text-neutral-400">{zh ? '仅文本' : 'Text only'}</span>}
                </button>
                <ModelRowCheck selected={selected} />
              </div>
            )
          })
        )}
      </div>
    </>
  )
}

export interface EffortStep {
  value: string
  label: string
  title?: string
  disabled: boolean
}

// 轨道两端留出旋钮半径，首末档的旋钮不会探出轨道。
const TRACK_INSET = 12

function stepRatio(index: number, count: number): number {
  return count > 1 ? index / (count - 1) : 1
}

function stepPosition(ratio: number): string {
  return `calc(${TRACK_INSET}px + (100% - ${TRACK_INSET * 2}px) * ${ratio})`
}

/**
 * 思考强度滑杆（一行）：左侧档位名，中间轨道按档位等距落点，右侧快速模式；
 * 拖动或点击吸附到最近的可用档，松手才提交。档位列表由模型库决定，这里只负责呈现。
 */
function EffortSlider({
  steps,
  value,
  title,
  disabled,
  onSelect,
  pending,
  fastTier,
  fastOn,
  onToggleFast,
}: {
  steps: EffortStep[]
  /** 当前所在档；null = 未落在任何档（Auto / CLI 默认）。 */
  value: string | null
  title: string
  disabled: boolean
  onSelect: (value: string) => void
  /** 外部 CLI 首次探测中：档位未到，先占住底栏（空凹槽），到了再填，弹层不跳。 */
  pending: boolean
  fastTier: { id: string; label: string } | undefined
  fastOn: boolean
  onToggleFast: () => void
}) {
  const t = useT()
  const zh = useLang() === 'zh'
  const trackRef = useRef<HTMLDivElement>(null)
  const dotRefs = useRef<(HTMLButtonElement | null)[]>([])
  const [dragIndex, setDragIndex] = useState<number | null>(null)
  if (steps.length === 0 && !fastTier) {
    if (!pending) return null
    return (
      <div className="kv-agent-picker__effort" aria-hidden="true">
        <span className="kv-effort__value" />
        <div className="kv-effort__track is-placeholder" />
      </div>
    )
  }

  const count = steps.length
  const valueIndex = value == null ? -1 : steps.findIndex((step) => step.value === value)
  const shownIndex = dragIndex ?? valueIndex
  const shownTitle = dragIndex != null ? steps[dragIndex]?.label ?? title : title
  const ratio = shownIndex >= 0 ? stepRatio(shownIndex, count) : 0

  const nearestEnabled = (target: number): number | null => {
    let best: number | null = null
    steps.forEach((step, index) => {
      if (step.disabled) return
      if (best == null || Math.abs(index - target) < Math.abs(best - target)) best = index
    })
    return best
  }

  const indexFromPointer = (clientX: number): number | null => {
    const rect = trackRef.current?.getBoundingClientRect()
    if (!rect || count === 0) return null
    const usable = Math.max(1, rect.width - TRACK_INSET * 2)
    const r = Math.min(1, Math.max(0, (clientX - rect.left - TRACK_INSET) / usable))
    return nearestEnabled(Math.round(r * (count - 1)))
  }

  const commit = (index: number | null) => {
    if (index == null) return
    const step = steps[index]
    if (step && !step.disabled && step.value !== value) onSelect(step.value)
  }

  const moveBy = (delta: number) => {
    let index = (dragIndex ?? valueIndex) < 0 ? (delta > 0 ? -1 : count) : dragIndex ?? valueIndex
    do { index += delta } while (index >= 0 && index < count && steps[index].disabled)
    if (index < 0 || index >= count) return
    commit(index)
    dotRefs.current[index]?.focus()
  }

  const fastButton = fastTier && (
    <IconButton
      size="sm"
      variant="ghost"
      aria-pressed={fastOn}
      label={t.chatFastMode}
      title={zh
        ? `${t.chatFastMode}（${fastTier.label}）：由当前模型提供，计费与可用性取决于 CLI 账号`
        : `${t.chatFastMode} (${fastTier.label}): provided by the model catalog; pricing and availability depend on the CLI account`}
      onClick={onToggleFast}
    >
      <Zap size={14} fill={fastOn ? 'currentColor' : 'none'} />
    </IconButton>
  )

  return (
    <div className="kv-agent-picker__effort">
      {/* 一行：档位名（定宽，换档时轨道不跳）→ 滑杆 → 快速模式。 */}
      <span className={`kv-effort__value${count > 0 ? '' : ' kv-effort__value--solo'}`} title={t.chatThinkingEffort} aria-live="polite">
        {/* 换档时文字重新淡入（按文字换 key 重挂）。 */}
        <span key={count > 0 ? shownTitle : 'fast'} className="kv-effort__value-text">{count > 0 ? shownTitle : t.chatFastMode}</span>
      </span>
      {count > 0 && (
        <div
          ref={trackRef}
          role="group"
          aria-label={t.chatThinkingEffort}
          aria-disabled={disabled || undefined}
          className={`kv-effort__track${dragIndex != null ? ' is-dragging' : ''}${disabled ? ' is-disabled' : ''}`}
          onPointerDown={(event) => {
            if (disabled || event.button !== 0) return
            const index = indexFromPointer(event.clientX)
            if (index == null) return
            event.preventDefault()
            event.currentTarget.setPointerCapture?.(event.pointerId)
            setDragIndex(index)
          }}
          onPointerMove={(event) => {
            if (dragIndex == null) return
            const index = indexFromPointer(event.clientX)
            if (index != null && index !== dragIndex) setDragIndex(index)
          }}
          onPointerUp={() => {
            if (dragIndex == null) return
            commit(dragIndex)
            dotRefs.current[dragIndex]?.focus({ preventScroll: true })
            setDragIndex(null)
          }}
          onPointerCancel={() => setDragIndex(null)}
          onKeyDown={(event) => {
            if (disabled) return
            if (event.key === 'ArrowRight') { event.preventDefault(); moveBy(1) }
            if (event.key === 'ArrowLeft') { event.preventDefault(); moveBy(-1) }
          }}
        >
          {shownIndex >= 0 && (
            // 外层管随档位增强的光晕（滤镜要在裁切之外，否则光被裁掉）；
            // 内层是铺满整条轨道的渐变，用 clip-path 露出到当前档：越往右颜色越深，档位越高越「热」。
            <span
              className="kv-effort__glow"
              style={{ '--kv-effort-ratio': ratio } as CSSProperties}
              aria-hidden="true"
            >
              <span
                className="kv-effort__fill"
                style={{ clipPath: `inset(0 calc(100% - ${stepPosition(ratio)}) 0 0 round 999px)` }}
              >
                {/* 打开面板与每次换档时扫过一道光（按已提交的档位换 key 重挂）。 */}
                <span key={value ?? 'none'} className="kv-effort__sheen" />
              </span>
            </span>
          )}
          {steps.map((step, index) => (
            <button
              key={step.value}
              ref={(node) => { dotRefs.current[index] = node }}
              type="button"
              role="menuitemradio"
              aria-checked={index === valueIndex}
              aria-label={step.label}
              title={step.title ?? step.label}
              disabled={disabled || step.disabled}
              tabIndex={index === (valueIndex >= 0 ? valueIndex : 0) ? 0 : -1}
              // 指针操作由轨道统一处理（拖动吸附、松手提交）；这里只接键盘触发的 click。
              onClick={(event) => { if (event.detail === 0) commit(index) }}
              className={`kv-effort__dot${shownIndex >= 0 && index <= shownIndex ? ' is-filled' : ''}`}
              style={{ left: stepPosition(stepRatio(index, count)) }}
            />
          ))}
          {shownIndex >= 0 && <span className="kv-effort__knob" style={{ left: stepPosition(ratio) }} aria-hidden="true" />}
        </div>
      )}
      {fastButton}
    </div>
  )
}

export const AgentModelPicker = memo(AgentModelPickerBase)
