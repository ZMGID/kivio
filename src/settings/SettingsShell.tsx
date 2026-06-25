import { forwardRef, useImperativeHandle, useState, useEffect, useCallback, useMemo, useRef } from 'react'
import {
  X, Check, Plus, Minus, Trash2, RefreshCw,
  ExternalLink, Download, Upload, ChevronRight, Wrench, Sparkles, FolderOpen, Eye, EyeOff, Info,
} from 'lucide-react'
import { open, save } from '@tauri-apps/plugin-dialog'
import ReactMarkdown from 'react-markdown'
import {
  api,
  type Settings as SettingsType,
  type ModelProvider,
  type ModelInfo,
  type DefaultPromptTemplates,
  type PermissionStatus,
  type UpdateInfo,
  type ChatMcpServer,
  type ChatToolsConfig,
  type ChatNativeToolsConfig,
  type ChatMemoryConfig,
  type ChatToolDefinition,
  type McpServerState,
  type McpServerStatePayload,
  defaultNativeTools,
  normalizeProviderApiFormat,
  type SkillMeta,
  type SkillDetail,
} from '../api/tauri'
import { i18n } from './i18n'
import {
  GeneralIcon, TranslateIcon, ScreenshotIcon, LensIcon, ChatIcon, MemoryIcon, MixerIcon,
  CodeIcon, AgentIcon, McpIcon, SkillIcon, WebSearchIcon, ConnectorsIcon, UsageIcon, ProvidersIcon, AboutIcon, KnowledgeIcon,
} from './NavIcons'
import { buildHotkey, formatHotkeyError, getPlatform, isProviderEnabled, stableStringify } from './utils'
import { PROVIDER_PRESETS, type ProviderPreset } from './providerPresets'
import { ModelPairSelect } from './ModelPairSelect'
import { ProviderModelsPicker } from './ProviderModelsPicker'
import { ModelIcon } from '../chat/ModelIcon'
import { ProviderSortableList } from './ProviderSortableList'
import { PromptField, ScreenshotTranslationSettings } from './ScreenshotTranslationSettings'
import { UsageStatsPanel } from './UsageStatsPanel'
import { KivioCodeSettings } from './KivioCodeSettings'
import { ExternalAgentsSettings } from './ExternalAgentsSettings'
import { ModelDetailDrawer } from '../components/ModelDetailDrawer'
import { resolveModelInfo } from '../data/modelMatching'
import { useWindowInteractionFocus } from '../utils/windowFocus'
import { hasEnabledNativeBuiltinTool, hasEnabledSkillRuntime } from '../utils/chatTools'
import { THEME_COLOR_PRESETS, normalizeThemeColorId } from '../themeColors'
import {
  Toggle, Select, Input, TextArea,
  SettingRow, PermissionItem, HotkeyInput,
  SettingsGroup,
} from './components'
import { ConnectorsPanel } from './ConnectorsPanel'
import { KnowledgeBasePanel } from './KnowledgeBasePanel'

export type SettingsTab = 'general' | 'translate' | 'screenshot' | 'lens' | 'chat' | 'memory' | 'mixer' | 'kivioCode' | 'externalAgents' | 'mcp' | 'skill' | 'webSearch' | 'connectors' | 'knowledge' | 'usage' | 'providers' | 'about'

type SettingsData = SettingsType
type MemoryLayerKey = 'l1' | 'l2'

const MEMORY_L1_MAX_BYTES = 5_000
const CHAT_MAX_OUTPUT_TOKEN_OPTIONS = [2048, 8192, 16384, 32768]
const CHAT_TOOL_DEFAULT_ROUNDS = 20
const CHAT_TOOL_MIN_ROUNDS = 1
const CHAT_TOOL_MAX_ROUNDS = 100
const CHAT_TOOL_ROUND_PRESETS = [5, 10, 20, 50, 100]
const CHAT_TOOL_TIMEOUT_PRESETS_MS = [30_000, 60_000, 120_000, 300_000]
// MCP 持久连接空闲超时预设（ms）。后端钳制范围 60s..24h，默认 10 分钟。
const MCP_IDLE_TIMEOUT_PRESETS_MS = [60_000, 300_000, 600_000, 1_800_000, 3_600_000]
const MCP_IDLE_TIMEOUT_MIN_MS = 60_000
const MCP_IDLE_TIMEOUT_MAX_MS = 24 * 60 * 60 * 1_000
// 子 agent 并发预设。后端钳制范围 1..64，默认 12。
const SUB_AGENT_CONCURRENCY_PRESETS = [3, 6, 12, 24, 48]
const SUB_AGENT_CONCURRENCY_MIN = 1
const SUB_AGENT_CONCURRENCY_MAX = 64
const textEncoder = new TextEncoder()

function utf8ByteLength(value: string): number {
  return textEncoder.encode(value).length
}

function clampToolRounds(value: string | number | null | undefined): number {
  const parsed = Number(value ?? CHAT_TOOL_DEFAULT_ROUNDS)
  if (!Number.isFinite(parsed)) return CHAT_TOOL_DEFAULT_ROUNDS
  return Math.min(CHAT_TOOL_MAX_ROUNDS, Math.max(CHAT_TOOL_MIN_ROUNDS, Math.round(parsed)))
}

function clampToolTimeoutMs(value: string | number | null | undefined): number {
  const parsed = Number(value ?? 60_000)
  if (!Number.isFinite(parsed)) return 60_000
  return Math.min(300_000, Math.max(1_000, Math.round(parsed)))
}

function clampMcpIdleTimeoutMs(value: string | number | null | undefined): number {
  const parsed = Number(value ?? 600_000)
  if (!Number.isFinite(parsed)) return 600_000
  return Math.min(MCP_IDLE_TIMEOUT_MAX_MS, Math.max(MCP_IDLE_TIMEOUT_MIN_MS, Math.round(parsed)))
}

function clampSubAgentConcurrency(value: string | number | null | undefined): number {
  const parsed = Number(value ?? 12)
  if (!Number.isFinite(parsed)) return 12
  return Math.min(SUB_AGENT_CONCURRENCY_MAX, Math.max(SUB_AGENT_CONCURRENCY_MIN, Math.round(parsed)))
}

function formatToolRoundsLabel(rounds: number, lang: string): string {
  return lang === 'zh' ? `${rounds} 轮` : `${rounds} rounds`
}

function formatToolTimeoutLabel(ms: number, lang: string): string {
  if (ms % 60_000 === 0) {
    const minutes = ms / 60_000
    return lang === 'zh' ? `${minutes} 分钟` : `${minutes} min`
  }
  if (ms % 1000 === 0) {
    const seconds = ms / 1000
    return lang === 'zh' ? `${seconds} 秒` : `${seconds} sec`
  }
  return `${ms} ms`
}

export interface SettingsShellProps {
  variant: 'standalone' | 'embedded'
  onClose: () => void
  onSettingsChange: () => void
  onReady?: () => void
  reserveTrafficLightSpace?: boolean
  /** 打开设置面板时选中的侧栏项（如 Chat 内嵌设置默认 AI 客户端） */
  initialTab?: SettingsTab
}

export interface SettingsShellHandle {
  requestClose: () => void
}

function FieldBlock({
  label,
  description,
  children,
  className = '',
}: {
  label: React.ReactNode
  description?: string
  children: React.ReactNode
  className?: string
}) {
  return (
    <div className={`py-2 ${className}`}>
      <div className="mb-2">
        <div className="kv-row-label">{label}</div>
        {description && <p className="kv-row-desc">{description}</p>}
      </div>
      {children}
    </div>
  )
}

function MemoryEditor({
  layer,
  title,
  description,
  value,
  savedValue,
  maxBytes,
  rows,
  loading,
  saving,
  lang,
  onChange,
  onSave,
  onReload,
}: {
  layer: MemoryLayerKey
  title: string
  description: string
  value: string
  savedValue: string
  maxBytes?: number
  rows: number
  loading: boolean
  saving: boolean
  lang: string
  onChange: (value: string) => void
  onSave: () => void
  onReload: () => void
}) {
  const bytes = utf8ByteLength(value)
  const overLimit = maxBytes !== undefined && bytes > maxBytes
  const dirty = value !== savedValue
  return (
    <div className="kv-panel">
      <div className="mb-2 flex flex-wrap items-start justify-between gap-2">
        <div className="min-w-0">
          <div className="kv-panel-title !mb-1">
            {title}
            <span className={`kv-tag ${overLimit ? 'danger' : dirty ? 'warn' : 'ok'}`}>
              {maxBytes ? `${bytes} / ${maxBytes} bytes` : `${bytes} bytes`}
            </span>
          </div>
          <div className="kv-panel-body">{description}</div>
        </div>
        <div className="flex shrink-0 items-center gap-1.5">
          <button
            type="button"
            className="kv-btn sm"
            onClick={onReload}
            disabled={loading || saving}
            data-tauri-drag-region="false"
          >
            <RefreshCw size={10} className={loading ? 'animate-spin' : ''} />
            {lang === 'zh' ? '重载' : 'Reload'}
          </button>
          <button
            type="button"
            className="kv-btn primary sm"
            onClick={onSave}
            disabled={loading || saving || !dirty || overLimit}
            data-tauri-drag-region="false"
          >
            {saving ? (lang === 'zh' ? '保存中' : 'Saving') : (lang === 'zh' ? '保存' : 'Save')}
          </button>
        </div>
      </div>
      <textarea
        value={value}
        onChange={(event) => onChange(event.target.value)}
        rows={rows}
        className="kv-textarea mono custom-scrollbar min-h-[160px]"
        spellCheck={false}
        data-tauri-drag-region="false"
        aria-label={title}
      />
      {overLimit && (
        <p className="mt-1.5 text-[11px] leading-snug text-red-500 dark:text-red-400">
          {lang === 'zh'
            ? `${layer.toUpperCase()} 超出字节上限，保存前需要精简。`
            : `${layer.toUpperCase()} is over its byte limit.`}
        </p>
      )}
    </div>
  )
}

function defaultChatConfig(): NonNullable<SettingsData['chat']> {
  return {
    streamEnabled: true,
    thinkingEnabled: true,
    maxOutputTokens: 8192,
    defaultLanguage: '',
    systemPrompt: '',
    userDisplayName: '',
    userAvatar: '',
    defaultAgentRuntime: {
      kind: 'builtin',
      externalAgentId: null,
      externalModel: null,
      externalReasoning: null,
    },
  }
}

function defaultChatMemory(): ChatMemoryConfig {
  return {
    enabled: false,
    toolWriteConfirm: false,
  }
}

function formatTokenCount(tokens?: number): string {
  if (!tokens || !Number.isFinite(tokens)) return ''
  return `${tokens.toLocaleString()} tokens`
}

function resolveEffectiveChatModel(settings: SettingsData): { provider?: ModelProvider, model: string } {
  const configuredChat = settings.defaultModels.chat.providerId
    ? settings.defaultModels.chat
    : settings.chatProviderId
      ? { providerId: settings.chatProviderId, model: settings.chatModel }
      : settings.lens?.providerId
        ? { providerId: settings.lens.providerId, model: settings.lens.model || '' }
        : { providerId: settings.translatorProviderId, model: settings.translatorModel }

  return {
    provider: settings.providers.find((provider) => provider.id === configuredChat.providerId),
    model: configuredChat.model || '',
  }
}

function resolveEffectiveChatMaxOutput(settings: SettingsData, fallbackTokens: number) {
  const { provider, model } = resolveEffectiveChatModel(settings)
  const override = model ? provider?.modelOverrides?.[model]?.maxOutput : undefined
  const modelInfo = model ? resolveModelInfo(model, provider?.modelOverrides) : {}
  const maxOutput = override || modelInfo.maxOutput || fallbackTokens
  const source: 'override' | 'database' | 'fallback' = override
    ? 'override'
    : modelInfo.maxOutput
      ? 'database'
      : 'fallback'

  return { maxOutput, source, model, provider }
}

function defaultChatTools(): ChatToolsConfig {
  return {
    enabled: false,
    servers: [],
    skillScanPaths: [],
    skillAutoMatch: true,
    skillFallbackMode: 'progressive',
    skillScriptAllowlist: ['python3', 'bash', 'sh', 'node'],
    disabledSkillIds: [],
    maxToolRounds: CHAT_TOOL_DEFAULT_ROUNDS,
    toolTimeoutMs: 60_000,
    mcpIdleTimeoutMs: 600_000,
    maxToolOutputChars: null,
    approvalPolicy: 'readonly_auto_sensitive_confirm',
    subAgentConcurrency: 12,
    nativeTools: defaultNativeTools(),
  }
}

function isBuiltinSkill(skill: SkillMeta): boolean {
  return skill.source === 'builtin'
}

function skillSourceLabel(skill: SkillMeta, lang: string): string {
  if (skill.source === 'builtin') {
    return lang === 'zh' ? '内置' : 'Built-in'
  }
  if (skill.source === 'external') {
    return lang === 'zh' ? '外部' : 'External'
  }
  return lang === 'zh' ? '用户' : 'User'
}

function SkillRow({
  skill,
  lang,
  expanded,
  enabled,
  onToggleExpanded,
  onToggleEnabled,
  onPreview,
}: {
  skill: SkillMeta
  lang: string
  expanded: boolean
  enabled: boolean
  onToggleExpanded: (skillId: string) => void
  onToggleEnabled: (skillId: string, enabled: boolean) => void
  onPreview: (skillId: string) => void
}) {
  const fileCount = skill.files?.length ?? 0
  return (
    <div className={`chat-motion-row bg-white dark:bg-neutral-950/40 ${enabled ? '' : 'opacity-70'}`}>
      <div className="flex h-9 items-center gap-2 px-2.5">
        <button
          type="button"
          className="flex h-8 min-w-0 flex-1 items-center gap-2 rounded-md text-left transition-colors hover:bg-black/[0.035] dark:hover:bg-white/[0.045]"
          onClick={() => onToggleExpanded(skill.id)}
          aria-expanded={expanded}
          data-tauri-drag-region="false"
        >
          <ChevronRight
            size={13}
            className={`shrink-0 text-neutral-400 transition-transform ${expanded ? 'rotate-90' : ''}`}
            strokeWidth={2.25}
          />
          <span className="min-w-0 flex-1 truncate text-[13px] font-semibold text-neutral-900 dark:text-neutral-100">
            {skill.name}
          </span>
        </button>
        <span
          className={`inline-flex h-5 shrink-0 items-center gap-1 rounded-full px-2 text-[11px] font-medium ${
            enabled
              ? 'bg-emerald-500/10 text-emerald-700 dark:bg-emerald-400/15 dark:text-emerald-300'
              : 'bg-neutral-200/70 text-neutral-500 dark:bg-neutral-800 dark:text-neutral-400'
          }`}
        >
          <span className={`size-1.5 rounded-full ${enabled ? 'bg-emerald-500' : 'bg-neutral-400'}`} />
          {enabled ? (lang === 'zh' ? '启用' : 'On') : (lang === 'zh' ? '关闭' : 'Off')}
        </span>
        <Toggle checked={enabled} onChange={(nextEnabled) => onToggleEnabled(skill.id, nextEnabled)} />
      </div>
      <div className={`chat-motion-reveal ${expanded ? 'is-open' : ''}`}>
        <div className="px-3 pb-3 pl-8">
          <p className="kv-panel-body">{skill.description}</p>
          <div className="mt-2 flex flex-wrap items-center gap-1.5">
            <span className="kv-chip">{skillSourceLabel(skill, lang)}</span>
            {fileCount > 0 && (
              <span className="kv-chip">
                {fileCount} {lang === 'zh' ? '个附属文件' : 'files'}
              </span>
            )}
            {skill.disableModelInvocation && (
              <span className="kv-chip">{lang === 'zh' ? '仅手动触发' : 'Manual only'}</span>
            )}
            {skill.recommendedTools.map((tool) => (
              <span key={tool} className="kv-chip">{tool}</span>
            ))}
          </div>
          <button
            type="button"
            className="kv-btn sm mt-2"
            onClick={() => onPreview(skill.id)}
            data-tauri-drag-region="false"
          >
            <ExternalLink size={10} />
            {lang === 'zh' ? '查看完整内容' : 'View details'}
          </button>
        </div>
      </div>
    </div>
  )
}

function SkillListSection({
  title,
  emptyText,
  skills,
  lang,
  expandedSkillIds,
  disabledSkillIds,
  onToggleExpanded,
  onToggleEnabled,
  onPreview,
}: {
  title: string
  emptyText: string
  skills: SkillMeta[]
  lang: string
  expandedSkillIds: string[]
  disabledSkillIds: string[]
  onToggleExpanded: (skillId: string) => void
  onToggleEnabled: (skillId: string, enabled: boolean) => void
  onPreview: (skillId: string) => void
}) {
  const enabledCount = skills.filter((skill) => !disabledSkillIds.includes(skill.id)).length
  return (
    <div className="w-full max-w-[680px] space-y-2 py-2">
      <div className="flex items-center justify-between px-1">
        <div className="text-[12px] font-semibold text-neutral-800 dark:text-neutral-100">{title}</div>
        <span className="kv-tag ok">{enabledCount} / {skills.length}</span>
      </div>
      {skills.length > 0 ? (
        <div className="overflow-hidden rounded-lg border border-neutral-200 bg-white shadow-sm [&>*+*]:border-t [&>*+*]:border-neutral-200 dark:border-neutral-800 dark:bg-neutral-950/40 dark:[&>*+*]:border-neutral-800">
          {skills.map((skill) => (
            <SkillRow
              key={skill.id}
              skill={skill}
              lang={lang}
              expanded={expandedSkillIds.includes(skill.id)}
              enabled={!disabledSkillIds.includes(skill.id)}
              onToggleExpanded={onToggleExpanded}
              onToggleEnabled={onToggleEnabled}
              onPreview={onPreview}
            />
          ))}
        </div>
      ) : (
        <div className="kv-panel">
          <div className="kv-panel-body">{emptyText}</div>
        </div>
      )}
    </div>
  )
}

function defaultDefaultModels(chatProviderId = '', chatModel = ''): SettingsData['defaultModels'] {
  return {
    chat: { providerId: chatProviderId, model: chatModel },
    vision: { providerId: '', model: '' },
    titleSummary: { providerId: '', model: '' },
    compression: { providerId: '', model: '' },
    imageGeneration: { providerId: '', model: '' },
  }
}

function clearDefaultModelProvider(
  defaultModels: SettingsData['defaultModels'],
  providerId: string,
): SettingsData['defaultModels'] {
  return {
    chat: defaultModels.chat.providerId === providerId ? { providerId: '', model: '' } : defaultModels.chat,
    vision: defaultModels.vision.providerId === providerId
      ? { providerId: '', model: '' }
      : defaultModels.vision,
    titleSummary: defaultModels.titleSummary.providerId === providerId
      ? { providerId: '', model: '' }
      : defaultModels.titleSummary,
    compression: defaultModels.compression.providerId === providerId
      ? { providerId: '', model: '' }
      : defaultModels.compression,
    imageGeneration: defaultModels.imageGeneration.providerId === providerId
      ? { providerId: '', model: '' }
      : defaultModels.imageGeneration,
  }
}

function resolveDefaultModelsAfterModelRemoval(
  defaultModels: SettingsData['defaultModels'],
  providerId: string,
  resolveAfterRemoval: (currentModel: string) => string,
): SettingsData['defaultModels'] {
  return {
    chat: defaultModels.chat.providerId === providerId
      ? { ...defaultModels.chat, model: resolveAfterRemoval(defaultModels.chat.model) }
      : defaultModels.chat,
    vision: defaultModels.vision.providerId === providerId
      ? { ...defaultModels.vision, model: resolveAfterRemoval(defaultModels.vision.model) }
      : defaultModels.vision,
    titleSummary: defaultModels.titleSummary.providerId === providerId
      ? { ...defaultModels.titleSummary, model: resolveAfterRemoval(defaultModels.titleSummary.model) }
      : defaultModels.titleSummary,
    compression: defaultModels.compression.providerId === providerId
      ? { ...defaultModels.compression, model: resolveAfterRemoval(defaultModels.compression.model) }
      : defaultModels.compression,
    imageGeneration: defaultModels.imageGeneration.providerId === providerId
      ? { ...defaultModels.imageGeneration, model: resolveAfterRemoval(defaultModels.imageGeneration.model) }
      : defaultModels.imageGeneration,
  }
}

function newMcpServer(): ChatMcpServer {
  return {
    id: `mcp-${Date.now()}`,
    name: 'New MCP Server',
    enabled: false,
    transport: 'stdio',
    url: '',
    command: '',
    args: [],
    env: {},
    headers: {},
    cwd: null,
    enabledTools: [],
  }
}

function envToText(env: Record<string, string>): string {
  return Object.entries(env)
    .map(([key, value]) => `${key}=${value}`)
    .join('\n')
}

function textToEnv(text: string): Record<string, string> {
  const env: Record<string, string> = {}
  for (const line of text.split('\n')) {
    const normalized = line.replace(/\r$/, '')
    if (!normalized.trim()) continue
    const separator = normalized.indexOf('=')
    const key = (separator >= 0 ? normalized.slice(0, separator) : normalized).trim()
    if (!key) continue
    env[key] = separator >= 0 ? normalized.slice(separator + 1) : ''
  }
  return env
}

function argsToText(args: string[]): string {
  return args.join('\n')
}

function textToArgs(text: string): string[] {
  return text
    .split('\n')
    .map((arg) => arg.replace(/\r$/, ''))
    .filter((arg) => arg !== '')
}

/**
 * 设置面板主组件（standalone / embedded 双宿主）
 */
export const SettingsShell = forwardRef<SettingsShellHandle, SettingsShellProps>(function SettingsShell(
  { variant, onClose, onSettingsChange, onReady, reserveTrafficLightSpace = false, initialTab },
  ref,
) {
  const [settings, setSettings] = useState<SettingsData | null>(null)
  const [initialSettingsSnapshot, setInitialSettingsSnapshot] = useState('')
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [appVersion, setAppVersion] = useState('')
  const [activeTab, setActiveTab] = useState<SettingsTab>(initialTab ?? 'general')
  useEffect(() => {
    if (initialTab) setActiveTab(initialTab)
  }, [initialTab])
  const [saveError, setSaveError] = useState('')
  const [saveSuccess, setSaveSuccess] = useState(false)
  const [closeConfirmOpen, setCloseConfirmOpen] = useState(false)
  const [confirmDeleteProviderId, setConfirmDeleteProviderId] = useState<string | null>(null)
  const [recordingTarget, setRecordingTarget] = useState<null | 'main' | 'screenshotTranslation' | 'screenshotTranslationText' | 'lens'>(null)
  const [defaultPrompts, setDefaultPrompts] = useState<DefaultPromptTemplates | null>(null)
  const [chatSystemPromptInteracted, setChatSystemPromptInteracted] = useState(false)
  const [retryAttemptsInput, setRetryAttemptsInput] = useState('')
  const [permissionStatus, setPermissionStatus] = useState<PermissionStatus | null>(null)
  const [permissionsLoading, setPermissionsLoading] = useState(false)
  const [testingProviderId, setTestingProviderId] = useState<string | null>(null)
  const [fetchingProviderId, setFetchingProviderId] = useState<string | null>(null)
  const [modelPickerProviderId, setModelPickerProviderId] = useState<string | null>(null)
  const [drawerModel, setDrawerModel] = useState<{ providerId: string; model: string } | null>(null)
  const [providerTestFeedback, setProviderTestFeedback] = useState<Record<string, { ok: boolean; message: string }>>({})
  const [selectedProviderId, setSelectedProviderId] = useState('')
  const [testingMcpServerId, setTestingMcpServerId] = useState<string | null>(null)
  const [mcpTestFeedback, setMcpTestFeedback] = useState<Record<string, { ok: boolean; message: string; tools: ChatToolDefinition[] }>>({})
  // 持久连接状态：serverId → 最近一次 mcp-server-state 事件的状态。
  const [mcpServerStates, setMcpServerStates] = useState<Record<string, McpServerState>>({})
  const [reloadingMcpServerId, setReloadingMcpServerId] = useState<string | null>(null)
  const [expandedMcpStderrIds, setExpandedMcpStderrIds] = useState<string[]>([])
  const [mcpStderrTails, setMcpStderrTails] = useState<Record<string, string>>({})
  const [skillsLoading, setSkillsLoading] = useState(false)
  const [skills, setSkills] = useState<SkillMeta[]>([])
  const [expandedSkillIds, setExpandedSkillIds] = useState<string[]>([])
  const [selectedSkillPreview, setSelectedSkillPreview] = useState<SkillDetail | null>(null)
  const [skillError, setSkillError] = useState('')
  const [memoryDrafts, setMemoryDrafts] = useState<Record<MemoryLayerKey, string>>({ l1: '', l2: '' })
  const [memorySnapshots, setMemorySnapshots] = useState<Record<MemoryLayerKey, string>>({ l1: '', l2: '' })
  const [memoryDir, setMemoryDir] = useState('')
  const [memoryLoading, setMemoryLoading] = useState(false)
  const [memorySavingLayer, setMemorySavingLayer] = useState<MemoryLayerKey | null>(null)
  const [memoryError, setMemoryError] = useState('')
  const [memorySuccess, setMemorySuccess] = useState('')
  // 更新检查状态：'idle' / 'checking' / 'up-to-date' / 'available'
  const [updateStatus, setUpdateStatus] = useState<'idle' | 'checking' | 'up-to-date' | 'available'>('idle')
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null)
  // 下载/安装两段式状态机:idle → downloading(进度条) → downloaded(显示安装按钮) → 用户点击 → 应用退出
  // failed 时显示错误 + 重试 + 跳 GitHub 兜底
  const [downloadState, setDownloadState] = useState<'idle' | 'downloading' | 'downloaded' | 'failed'>('idle')
  const [downloadPercent, setDownloadPercent] = useState(0)
  const requestWindowFocus = useWindowInteractionFocus()
  const [downloadedPath, setDownloadedPath] = useState('')
  const [downloadError, setDownloadError] = useState('')
  // RapidOCR 离线 OCR 状态:检查 app data 目录里 dylib + 模型 4 个文件齐不齐。
  const [rapidOcrStatus, setRapidOcrStatus] = useState<import('../api/tauri').RapidOcrStatus | null>(null)
  // 下载临时状态:'idle' / 'downloading' / 'failed'(success 后自动 refresh status 到已就绪,
  // 没有专门的 success 终态)
  const [rapidOcrDownloadState, setRapidOcrDownloadState] = useState<'idle' | 'downloading' | 'failed'>('idle')
  const [rapidOcrDownloadError, setRapidOcrDownloadError] = useState('')
  const platform = getPlatform()
  const isMac = platform === 'macos'
  const hasSystemOcr = isMac || platform === 'windows'
  // 加载失败时的错误信息；非空则渲染错误 UI 而不是用合成默认值进入正常视图
  // （否则用户可能没察觉就 Save 把磁盘真实数据覆盖掉）
  const [loadError, setLoadError] = useState('')
  const [reloadKey, setReloadKey] = useState(0)
  const saveSuccessTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const readyEmittedRef = useRef(false)

  const lang = settings?.settingsLanguage || 'zh'
  const t = i18n[lang]
  const themeColor = normalizeThemeColorId(settings?.themeColor)
  const chatTools = settings?.chatTools || defaultChatTools()
  const nativeBuiltinToolsEnabled = hasEnabledNativeBuiltinTool(chatTools.nativeTools)
  const skillRuntimeEnabled = hasEnabledSkillRuntime(chatTools.nativeTools)
  // 判断是否有未保存的更改
  const hasUnsavedChanges = settings ? stableStringify(settings) !== initialSettingsSnapshot : false

  // 客户端热键冲突检测:在保存前发现"两个启用功能用了同一个组合"。
  // OS 层面的冲突(Spotlight 占用 Cmd+Space 等)仍需保存后从后端拿到结果。
  // 返回每个 scope 对应的"和谁冲突"——前端各 HotkeyInput 拿到对应 scope 的伙伴名后,
  // 用 hotkeyScope* 模板自己拼本地化字符串。
  type HotkeyScopeKey = 'main' | 'screenshotTranslation' | 'screenshotTranslationText' | 'lens'
  const hotkeyConflicts = useMemo<Partial<Record<HotkeyScopeKey, HotkeyScopeKey>>>(() => {
    if (!settings) return {}
    const slots: Array<{ scope: HotkeyScopeKey; hotkey: string; enabled: boolean }> = [
      { scope: 'main', hotkey: settings.hotkey || '', enabled: !!(settings.hotkey || '').trim() },
      {
        scope: 'screenshotTranslation',
        hotkey: settings.screenshotTranslation?.hotkey || '',
        enabled: settings.screenshotTranslation?.enabled !== false,
      },
      {
        scope: 'screenshotTranslationText',
        hotkey: settings.screenshotTranslation?.textHotkey || '',
        enabled: settings.screenshotTranslation?.enabled !== false,
      },
      { scope: 'lens', hotkey: settings.lens?.hotkey || '', enabled: settings.lens?.enabled !== false },
    ]
    const groups = new Map<string, HotkeyScopeKey[]>()
    for (const s of slots) {
      const key = s.hotkey.trim().toLowerCase()
      if (!key || !s.enabled) continue
      const list = groups.get(key) ?? []
      list.push(s.scope)
      groups.set(key, list)
    }
    const out: Partial<Record<HotkeyScopeKey, HotkeyScopeKey>> = {}
    for (const list of groups.values()) {
      if (list.length < 2) continue
      for (const scope of list) {
        const partner = list.find(other => other !== scope)
        if (partner) out[scope] = partner
      }
    }
    return out
  }, [settings])

  const SCOPE_I18N_KEY: Record<HotkeyScopeKey, 'hotkeyScopeTranslator' | 'hotkeyScopeScreenshot' | 'hotkeyScopeScreenshotText' | 'hotkeyScopeLens'> = {
    main: 'hotkeyScopeTranslator',
    screenshotTranslation: 'hotkeyScopeScreenshot',
    screenshotTranslationText: 'hotkeyScopeScreenshotText',
    lens: 'hotkeyScopeLens',
  }
  const conflictMessageFor = (scope: HotkeyScopeKey): string | undefined => {
    const partner = hotkeyConflicts[scope]
    if (!partner) return undefined
    return t.hotkeyConflictWith.replace('{partner}', t[SCOPE_I18N_KEY[partner]])
  }

  // 初始化：加载设置、版本号、默认提示词
  // 重试通过递增 reloadKey 触发本 effect 重跑
  useEffect(() => {
    let active = true
    readyEmittedRef.current = false
    setLoading(true)
    setLoadError('')
    api.getSettings()
      .then((data: SettingsData) => {
        if (!active) return
        setSettings(data)
        setInitialSettingsSnapshot(stableStringify(data))
        setChatSystemPromptInteracted(false)
        setLoading(false)
      })
      .catch((err) => {
        if (!active) return
        console.error('Failed to load settings:', err)
        // 不合成默认值：避免用户在错误状态下 Save 把磁盘真实数据覆盖掉
        // 渲染分支会根据 loadError 显示重试 UI
        const message = err instanceof Error ? err.message : String(err)
        setLoadError(message || 'Unknown error')
        setLoading(false)
      })
    api.getAppVersion()
      .then((ver: string) => {
        if (active) setAppVersion(ver)
      })
      .catch(() => {
        if (active) setAppVersion('unknown')
      })
    api.getDefaultPromptTemplates()
      .then((templates) => {
        if (active) setDefaultPrompts(templates)
      })
      .catch((err) => {
        console.error('Failed to load default prompt templates:', err)
      })
    // resizeWindow 已在 App.tsx 中处理，此处不再重复调用
    return () => {
      active = false
    }
  }, [reloadKey])

  useEffect(() => {
    if (variant !== 'standalone') return
    if (!loading && !readyEmittedRef.current && (settings || loadError)) {
      readyEmittedRef.current = true
      onReady?.()
    }
  }, [loadError, loading, onReady, settings, variant])

  /**
   * 刷新权限状态（macOS）
   */
  const refreshPermissions = useCallback(async () => {
    setPermissionsLoading(true)
    try {
      const status = await api.getPermissionStatus()
      setPermissionStatus(status)
    } catch (err) {
      console.error('Failed to get permission status:', err)
    } finally {
      setPermissionsLoading(false)
    }
  }, [])

  useEffect(() => {
    refreshPermissions()
  }, [refreshPermissions])

  // 监听后端启动时的 update-available 事件，发现新版立即在 About 区块展开提示
  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined
    api.onUpdateAvailable((info) => {
      if (cancelled) return
      setUpdateInfo(info)
      setUpdateStatus('available')
    }).then((u) => {
      if (cancelled) u()
      else unlisten = u
    })
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [])

  // Settings 打开时静默 check 一次（覆盖启动事件用户当时没开 Settings 的场景）
  useEffect(() => {
    if (!settings) return
    if (settings.autoCheckUpdate === false) return
    if (updateStatus === 'available' || updateStatus === 'checking') return
    let cancelled = false
    api.checkUpdate().then((info) => {
      if (cancelled) return
      if (info.available) {
        setUpdateInfo(info)
        setUpdateStatus('available')
      }
    }).catch(() => {})
    return () => { cancelled = true }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings?.autoCheckUpdate, !!settings])

  /** 用户点 "检查更新" 按钮 */
  const handleCheckUpdate = useCallback(async () => {
    setUpdateStatus('checking')
    try {
      const info = await api.checkUpdate()
      if (info.available) {
        setUpdateInfo(info)
        setUpdateStatus('available')
      } else {
        setUpdateStatus('up-to-date')
        // 5s 后自动复位回 idle，避免"已是最新"标签长期占位
        setTimeout(() => setUpdateStatus((s) => (s === 'up-to-date' ? 'idle' : s)), 5000)
      }
    } catch (err) {
      console.error('Check update failed:', err)
      setUpdateStatus('idle')
    }
  }, [])

  const handleOpenReleasePage = useCallback(async () => {
    if (!updateInfo?.htmlUrl) return
    try {
      await api.openExternal(updateInfo.htmlUrl)
    } catch (err) {
      console.error('Open release page failed:', err)
    }
  }, [updateInfo])

  /** 下载新版安装包到 temp dir,期间监听 update-download-progress 事件刷新进度条 */
  const handleDownloadAndInstall = useCallback(async () => {
    if (!updateInfo?.version) return
    setDownloadState('downloading')
    setDownloadPercent(0)
    setDownloadError('')
    let unlisten: (() => void) | undefined
    try {
      unlisten = await api.onUpdateDownloadProgress((p) => {
        setDownloadPercent(Math.max(0, Math.min(100, Math.round(p.percent))))
      })
      const path = await api.downloadUpdate(updateInfo.version)
      setDownloadedPath(path)
      setDownloadState('downloaded')
    } catch (err) {
      console.error('Download update failed:', err)
      setDownloadError(typeof err === 'string' ? err : (err instanceof Error ? err.message : String(err)))
      setDownloadState('failed')
    } finally {
      unlisten?.()
    }
  }, [updateInfo])

  /** 启动 installer 并退出当前应用。Rust 端会在 macOS 上 cp 新 .app + open,在 Windows spawn NSIS exe */
  const handleInstall = useCallback(async () => {
    if (!downloadedPath) return
    try {
      await api.installUpdate(downloadedPath)
    } catch (err) {
      console.error('Install update failed:', err)
      setDownloadError(typeof err === 'string' ? err : (err instanceof Error ? err.message : String(err)))
      setDownloadState('failed')
    }
  }, [downloadedPath])

  /** 拉一次 RapidOCR 状态(app data 里 dylib + 模型 4 个文件齐不齐)。
   *  挂载时 + 切换到 RapidOCR 引擎时调一下。 */
  const refreshRapidOcrStatus = useCallback(async () => {
    if (!hasSystemOcr) return
    try {
      const status = await api.rapidOcrStatus()
      setRapidOcrStatus(status)
    } catch (err) {
      console.error('rapidOcrStatus failed:', err)
    }
  }, [hasSystemOcr])

  /** 下载 RapidOCR 包(dylib + 模型,~30-50MB):阻塞 ~15-30s,完成后 refresh status。 */
  const handleDownloadRapidOcr = useCallback(async () => {
    setRapidOcrDownloadState('downloading')
    setRapidOcrDownloadError('')
    try {
      const result = await api.rapidOcrInstall()
      if (result.success) {
        setRapidOcrDownloadState('idle')
        await refreshRapidOcrStatus()
      } else {
        setRapidOcrDownloadError(result.message)
        setRapidOcrDownloadState('failed')
      }
    } catch (err) {
      const msg = typeof err === 'string' ? err : err instanceof Error ? err.message : String(err)
      setRapidOcrDownloadError(msg)
      setRapidOcrDownloadState('failed')
    }
  }, [refreshRapidOcrStatus])

  // 挂载时拉一次状态
  useEffect(() => {
    refreshRapidOcrStatus()
  }, [refreshRapidOcrStatus])

  useEffect(() => {
    setProviderTestFeedback({})
  }, [lang])

  const retryAttempts = settings?.retryAttempts

  useEffect(() => {
    if (retryAttempts === undefined) return
    setRetryAttemptsInput(String(retryAttempts ?? 3))
  }, [retryAttempts])

  useEffect(() => {
    if (!settings?.providers.length) {
      setSelectedProviderId('')
      return
    }
    if (!selectedProviderId || !settings.providers.some((provider) => provider.id === selectedProviderId)) {
      setSelectedProviderId(settings.providers[0].id)
    }
  }, [selectedProviderId, settings?.providers])

  /**
   * 保存设置
   */
  const handleSave = useCallback(async () => {
    if (!settings) return false
    try {
      setSaving(true)
      setSaveError('')
      setSaveSuccess(false)
      if (saveSuccessTimerRef.current) {
        clearTimeout(saveSuccessTimerRef.current)
        saveSuccessTimerRef.current = null
      }
      const savedSettings = await api.saveSettings(settings)
      setSettings(savedSettings)
      setInitialSettingsSnapshot(stableStringify(savedSettings))
      onSettingsChange()
      setSaveSuccess(true)
      saveSuccessTimerRef.current = setTimeout(() => {
        setSaveSuccess(false)
        saveSuccessTimerRef.current = null
      }, 2200)
      return true
    } catch (err) {
      console.error('Failed to save settings:', err)
      const message = err instanceof Error ? err.message : String(err)
      const translated = formatHotkeyError(message, lang)
      const prefix = lang === 'zh' ? '保存失败:' : 'Save failed: '
      setSaveError(`${prefix}${translated.replace(/\n/g, ' / ')}`)
      setSaveSuccess(false)
      return false
    } finally {
      setSaving(false)
    }
  }, [lang, onSettingsChange, settings])

  useEffect(() => {
    return () => {
      if (saveSuccessTimerRef.current) {
        clearTimeout(saveSuccessTimerRef.current)
      }
    }
  }, [])

  /**
   * 请求关闭设置页（检查未保存更改）
   */
  const handleCloseRequest = useCallback(() => {
    if (recordingTarget) return
    if (hasUnsavedChanges) {
      setCloseConfirmOpen(true)
      return
    }
    onClose()
  }, [hasUnsavedChanges, onClose, recordingTarget])

  useImperativeHandle(ref, () => ({ requestClose: handleCloseRequest }), [handleCloseRequest])

  // 放弃更改并关闭
  const handleDiscardAndClose = () => {
    setCloseConfirmOpen(false)
    onClose()
  }

  // 保存并关闭
  const handleSaveAndClose = useCallback(async () => {
    const saved = await handleSave()
    if (saved) {
      setCloseConfirmOpen(false)
      onClose()
    }
  }, [handleSave, onClose])

  const handleSettingsDragMouseDown = useCallback((event: React.MouseEvent<HTMLElement>) => {
    if (event.button !== 0) return
    const target = event.target as HTMLElement | null
    if (target?.closest('button, input, textarea, select, [data-tauri-drag-region="false"]')) return
    event.preventDefault()
    void api.startDragging().catch((err) => {
      console.error('[settings-drag] startDragging failed:', err)
    })
  }, [])

  // 全局键盘：Esc 关闭、Cmd/Ctrl+S 保存；弹窗打开时优先处理弹窗内的 Esc/Enter
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (recordingTarget) return

      if (modelPickerProviderId) {
        if (e.key === 'Escape') {
          e.preventDefault()
          e.stopPropagation()
          setModelPickerProviderId(null)
        }
        return
      }

      // 删除供应商弹窗：Esc 取消；不绑定 Enter，避免误触发破坏性删除
      if (confirmDeleteProviderId) {
        if (e.key === 'Escape') {
          e.preventDefault()
          e.stopPropagation()
          setConfirmDeleteProviderId(null)
        }
        return
      }

      // 未保存确认弹窗：Esc = 继续编辑（关弹窗）；Enter = 保存并关闭
      if (closeConfirmOpen) {
        if (e.key === 'Escape') {
          e.preventDefault()
          e.stopPropagation()
          setCloseConfirmOpen(false)
        } else if (e.key === 'Enter') {
          e.preventDefault()
          e.stopPropagation()
          if (!saving) void handleSaveAndClose()
        }
        return
      }

      if (e.key === 'Escape') {
        handleCloseRequest()
      } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 's') {
        e.preventDefault()
        if (hasUnsavedChanges && !saving) void handleSave()
      }
    }
    window.addEventListener('keydown', handler, true)
    return () => window.removeEventListener('keydown', handler, true)
  }, [
    handleCloseRequest,
    recordingTarget,
    closeConfirmOpen,
    confirmDeleteProviderId,
    modelPickerProviderId,
    saving,
    hasUnsavedChanges,
    handleSave,
    handleSaveAndClose,
  ])

  /**
   * 测试提供商连接
   */
  const handleTestConnection = async (providerId: string) => {
    setTestingProviderId(providerId)
    setProviderTestFeedback((prev) => {
      const next = { ...prev }
      delete next[providerId]
      return next
    })
    try {
      const provider = settings?.providers.find((p) => p.id === providerId)
      const result = await api.testProviderConnection(providerId, provider
        ? {
          id: provider.id,
          baseUrl: provider.baseUrl,
          apiKeys: provider.apiKeys,
        }
        : undefined)
      if (result.success) {
        setProviderTestFeedback((prev) => ({ ...prev, [providerId]: { ok: true, message: t.connectionOk } }))
      } else {
        setProviderTestFeedback((prev) => ({
          ...prev,
          [providerId]: { ok: false, message: `${t.connectionFailed}${result.error || 'Unknown error'}` },
        }))
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err)
      setProviderTestFeedback((prev) => ({
        ...prev,
        [providerId]: { ok: false, message: `${t.connectionFailed}${message}` },
      }))
    } finally {
      setTestingProviderId(null)
    }
  }

  /**
   * 打开 macOS 系统权限设置
   */
  const handleOpenPermissionSettings = async (kind: 'accessibility' | 'screen-recording') => {
    try {
      await api.openPermissionSettings(kind)
    } catch (err) {
      console.error('Failed to open permission settings:', err)
    }
  }

  // 重试次数输入处理
  const handleRetryAttemptsChange = (value: string) => {
    if (!settings) return
    setRetryAttemptsInput(value)
    if (value.trim() === '') return
    const parsed = Number.parseInt(value, 10)
    if (Number.isNaN(parsed)) return
    const clamped = Math.min(5, Math.max(1, parsed))
    updateSettings({ retryAttempts: clamped })
  }

  const handleRetryAttemptsBlur = () => {
    if (!settings) return
    if (retryAttemptsInput.trim() === '') {
      setRetryAttemptsInput(String(settings.retryAttempts ?? 3))
      return
    }
    const parsed = Number.parseInt(retryAttemptsInput, 10)
    if (Number.isNaN(parsed)) {
      setRetryAttemptsInput(String(settings.retryAttempts ?? 3))
      return
    }
    const clamped = Math.min(5, Math.max(1, parsed))
    setRetryAttemptsInput(String(clamped))
    if (clamped !== settings.retryAttempts) {
      updateSettings({ retryAttempts: clamped })
    }
  }

  /**
   * 更新设置字段
   */
  const updateSettings = useCallback((updates: Partial<SettingsData>) => {
    setSettings((prev) => {
      if (!prev) return prev
      return { ...prev, ...updates }
    })
  }, [])

  // 设置备份：导出/导入 JSON。导入会覆盖全部设置并立即生效。
  const [backupStatus, setBackupStatus] = useState<{ kind: 'ok' | 'err'; msg: string } | null>(null)

  // 哪些 API Key 输入框处于明文显示（按 `${providerId}-${idx}` 记），默认全部隐藏。
  const [revealedKeys, setRevealedKeys] = useState<Set<string>>(new Set())
  const [gzipInfoOpen, setGzipInfoOpen] = useState<Set<string>>(new Set())
  const toggleKeyReveal = useCallback((keyId: string) => {
    setRevealedKeys((prev) => {
      const next = new Set(prev)
      if (next.has(keyId)) next.delete(keyId)
      else next.add(keyId)
      return next
    })
  }, [])

  const handleExportSettings = useCallback(async () => {
    try {
      const path = await save({
        defaultPath: 'kivio-settings-backup.json',
        filters: [{ name: 'JSON', extensions: ['json'] }],
      })
      if (!path) return
      await api.exportSettings(path)
      setBackupStatus({ kind: 'ok', msg: lang === 'zh' ? '设置已导出。' : 'Settings exported.' })
    } catch (err) {
      setBackupStatus({ kind: 'err', msg: `${lang === 'zh' ? '导出失败：' : 'Export failed: '}${err}` })
    }
  }, [lang])

  const handleImportSettings = useCallback(async () => {
    try {
      const selected = await open({ multiple: false, filters: [{ name: 'JSON', extensions: ['json'] }] })
      if (!selected || typeof selected !== 'string') return
      const imported = await api.importSettings(selected)
      setSettings(imported)
      setBackupStatus({ kind: 'ok', msg: lang === 'zh' ? '设置已导入并生效。' : 'Settings imported and applied.' })
    } catch (err) {
      setBackupStatus({ kind: 'err', msg: `${lang === 'zh' ? '导入失败：' : 'Import failed: '}${err}` })
    }
  }, [lang])

  const updateDefaultModel = useCallback((
    key: keyof SettingsData['defaultModels'],
    providerId: string,
    model: string,
  ) => {
    setSettings((prev) => {
      if (!prev) return prev
      const current = prev.defaultModels || defaultDefaultModels(prev.chatProviderId, prev.chatModel)
      const defaultModels = {
        ...current,
        [key]: { providerId, model },
      }
      return {
        ...prev,
        defaultModels,
        ...(key === 'chat' ? { chatProviderId: providerId, chatModel: model } : {}),
      }
    })
  }, [])

  const updateChatTools = useCallback((updates: Partial<ChatToolsConfig>) => {
    setSettings((prev) => {
      if (!prev) return prev
      const current = prev.chatTools || defaultChatTools()
      return { ...prev, chatTools: { ...current, ...updates } }
    })
  }, [])

  const updateNativeTools = useCallback((updates: Partial<ChatNativeToolsConfig>) => {
    setSettings((prev) => {
      if (!prev) return prev
      const chatTools = prev.chatTools || defaultChatTools()
      return {
        ...prev,
        chatTools: {
          ...chatTools,
          nativeTools: {
            ...defaultNativeTools(),
            ...chatTools.nativeTools,
            ...updates,
          },
        },
      }
    })
  }, [])

  const updateMcpServer = useCallback((serverId: string, updates: Partial<ChatMcpServer>) => {
    setSettings((prev) => {
      if (!prev) return prev
      const chatTools = prev.chatTools || defaultChatTools()
      return {
        ...prev,
        chatTools: {
          ...chatTools,
          servers: chatTools.servers.map((server) =>
            server.id === serverId ? { ...server, ...updates } : server,
          ),
        },
      }
    })
  }, [])

  const refreshChatSkills = useCallback(async () => {
    setSkillsLoading(true)
    setSkillError('')
    try {
      const result = await api.chatSkillsList(settings?.chatTools?.skillScanPaths)
      if (result.success) {
        setSkills(result.skills)
        if (result.error) {
          setSkillError(result.error)
        }
      } else {
        setSkillError(result.error || (lang === 'zh' ? 'Skill 列表加载失败' : 'Failed to load skills'))
      }
    } catch (err) {
      setSkillError(err instanceof Error ? err.message : String(err))
    } finally {
      setSkillsLoading(false)
    }
  }, [lang, settings?.chatTools?.skillScanPaths])

  const handleTestMcpServer = useCallback(async (server: ChatMcpServer) => {
    setTestingMcpServerId(server.id)
    setMcpTestFeedback((prev) => {
      const next = { ...prev }
      delete next[server.id]
      return next
    })
    try {
      const result = await api.chatMcpTestServer(server, settings?.chatTools?.toolTimeoutMs)
      if (result.success) {
        setMcpTestFeedback((prev) => ({
          ...prev,
          [server.id]: {
            ok: true,
            message: lang === 'zh' ? `连接成功，发现 ${result.tools.length} 个工具。` : `Connected. ${result.tools.length} tools found.`,
            tools: result.tools,
          },
        }))
      } else {
        setMcpTestFeedback((prev) => ({
          ...prev,
          [server.id]: {
            ok: false,
            message: result.error || (lang === 'zh' ? '连接失败' : 'Connection failed'),
            tools: [],
          },
        }))
      }
    } catch (err) {
      setMcpTestFeedback((prev) => ({
        ...prev,
        [server.id]: {
          ok: false,
          message: err instanceof Error ? err.message : String(err),
          tools: [],
        },
      }))
    } finally {
      setTestingMcpServerId(null)
    }
  }, [lang, settings?.chatTools?.toolTimeoutMs])

  const handleReloadMcpServer = useCallback(async (server: ChatMcpServer) => {
    setReloadingMcpServerId(server.id)
    try {
      await api.chatMcpReloadServer(server.id)
      // 重连后立即拉一次状态快照（Disconnected → 下次调用透明重连）。
      const status = await api.chatMcpServerStatus(server.id)
      setMcpServerStates((prev) => ({ ...prev, [server.id]: status.state }))
      setMcpStderrTails((prev) => ({ ...prev, [server.id]: status.stderrTail }))
    } catch (err) {
      setSaveError(err instanceof Error ? err.message : String(err))
    } finally {
      setReloadingMcpServerId(null)
    }
  }, [])

  // 订阅持久连接状态事件（连接/断开/错误），实时更新状态点。
  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | null = null
    void api.onMcpServerState((payload: McpServerStatePayload) => {
      if (cancelled) return
      setMcpServerStates((prev) => ({ ...prev, [payload.serverId]: payload.state }))
    }).then((fn) => {
      if (cancelled) fn()
      else unlisten = fn
    })
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [])

  // 进入 MCP 标签页时拉一次各 server 的状态快照（含 stderr 尾巴）。
  useEffect(() => {
    if (activeTab !== 'mcp' || !settings) return
    let cancelled = false
    const servers = settings.chatTools?.servers || []
    void Promise.all(
      servers.map(async (server) => {
        try {
          const status = await api.chatMcpServerStatus(server.id)
          return { id: server.id, status }
        } catch {
          return null
        }
      }),
    ).then((results) => {
      if (cancelled) return
      const states: Record<string, McpServerState> = {}
      const tails: Record<string, string> = {}
      for (const entry of results) {
        if (!entry) continue
        states[entry.id] = entry.status.state
        tails[entry.id] = entry.status.stderrTail
      }
      setMcpServerStates((prev) => ({ ...prev, ...states }))
      setMcpStderrTails((prev) => ({ ...prev, ...tails }))
    })
    return () => {
      cancelled = true
    }
  }, [activeTab, settings])

  const handleImportMcpJson = useCallback(async () => {
    if (!settings) return
    try {
      const selected = await open({
        directory: false,
        multiple: false,
        filters: [{ name: 'MCP JSON', extensions: ['json'] }],
      })
      if (typeof selected !== 'string') return
      const result = await api.chatMcpImportJson(selected)
      if (!result.success) {
        setSaveError(result.error || (lang === 'zh' ? '导入 mcp.json 失败' : 'Failed to import mcp.json'))
        return
      }
      const chatTools = settings.chatTools || defaultChatTools()
      updateChatTools({ servers: [...chatTools.servers, ...result.servers] })
    } catch (err) {
      setSaveError(err instanceof Error ? err.message : String(err))
    }
  }, [lang, settings, updateChatTools])

  const handleImportSkill = useCallback(async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
      })
      if (typeof selected !== 'string') return
      const result = await api.chatSkillsImport(selected)
      if (!result.success) {
        setSkillError(result.error || (lang === 'zh' ? '导入 Skill 失败' : 'Failed to import skill'))
        return
      }
      await refreshChatSkills()
      onSettingsChange()
    } catch (err) {
      setSkillError(err instanceof Error ? err.message : String(err))
    }
  }, [lang, onSettingsChange, refreshChatSkills])

  const handleImportSkillZip = useCallback(async () => {
    try {
      const selected = await open({
        directory: false,
        multiple: false,
        filters: [{ name: 'Skill Zip', extensions: ['zip'] }],
      })
      if (typeof selected !== 'string') return
      const result = await api.chatSkillsImport(selected)
      if (!result.success) {
        setSkillError(result.error || (lang === 'zh' ? '导入 Skill 失败' : 'Failed to import skill'))
        return
      }
      await refreshChatSkills()
      onSettingsChange()
    } catch (err) {
      setSkillError(err instanceof Error ? err.message : String(err))
    }
  }, [lang, onSettingsChange, refreshChatSkills])

  const handleOpenSkillFolder = useCallback(async () => {
    setSkillError('')
    try {
      const result = await api.chatSkillsOpenFolder()
      if (!result.success) {
        setSkillError(result.error || (lang === 'zh' ? '打开 Skill 文件夹失败' : 'Failed to open skill folder'))
      }
    } catch (err) {
      setSkillError(err instanceof Error ? err.message : String(err))
    }
  }, [lang])

  const handlePreviewSkill = useCallback(async (skillId: string) => {
    setSkillError('')
    try {
      const result = await api.chatSkillsRead(skillId)
      if (result.success && result.skill) {
        setSelectedSkillPreview(result.skill)
      } else {
        setSkillError(result.error || (lang === 'zh' ? '读取 Skill 失败' : 'Failed to read skill'))
      }
    } catch (err) {
      setSkillError(err instanceof Error ? err.message : String(err))
    }
  }, [lang])

  const handleToggleSkillExpanded = useCallback((skillId: string) => {
    setExpandedSkillIds((current) => (
      current.includes(skillId)
        ? current.filter((id) => id !== skillId)
        : [...current, skillId]
    ))
  }, [])

  const handleToggleSkillEnabled = useCallback((skillId: string, enabled: boolean) => {
    const disabled = chatTools.disabledSkillIds ?? []
    const next = enabled
      ? disabled.filter((id) => id !== skillId)
      : disabled.includes(skillId)
        ? disabled
        : [...disabled, skillId]
    updateChatTools({ disabledSkillIds: next })
  }, [chatTools.disabledSkillIds, updateChatTools])

  useEffect(() => {
    if (activeTab === 'skill') {
      void refreshChatSkills()
    }
  }, [activeTab, refreshChatSkills])

  /**
   * 更新指定提供商配置
   */
  const updateProvider = useCallback((id: string, updates: Partial<ModelProvider>) => {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        providers: prev.providers.map(p => p.id === id ? { ...p, ...updates } : p)
      }
    })
  }, [])

  const reorderProviders = useCallback((fromId: string, toId: string) => {
    if (fromId === toId) return
    setSettings((prev) => {
      if (!prev) return prev
      const fromIndex = prev.providers.findIndex((p) => p.id === fromId)
      const toIndex = prev.providers.findIndex((p) => p.id === toId)
      if (fromIndex < 0 || toIndex < 0) return prev
      const nextProviders = [...prev.providers]
      const [moved] = nextProviders.splice(fromIndex, 1)
      nextProviders.splice(toIndex, 0, moved)
      return { ...prev, providers: nextProviders }
    })
  }, [])

  /**
   * 添加新提供商
   */
  const addProvider = () => {
    if (!settings) return
    const newId = `provider-${Date.now()}`
    const newProvider: ModelProvider = {
      id: newId,
      name: 'New Provider',
      apiKeys: [],
      baseUrl: 'https://api.openai.com/v1',
      availableModels: [],
      enabledModels: [],
      supportsTools: true,
      enabled: true,
      apiFormat: 'openai_chat',
    }
    setSettings({
      ...settings,
      providers: [...settings.providers, newProvider]
    })
    setSelectedProviderId(newId)
  }

  /** 用预设一键添加 provider —— baseUrl 和默认模型已填好，用户只需填 API key */
  const addProviderFromPreset = (preset: ProviderPreset) => {
    if (!settings) return
    const newId = `provider-${Date.now()}`
    const newProvider: ModelProvider = {
      id: newId,
      name: preset.name,
      apiKeys: [],
      baseUrl: preset.baseUrl,
      availableModels: [],
      enabledModels: [],
      supportsTools: true,
      enabled: true,
      apiFormat: 'openai_chat',
    }
    setSettings({
      ...settings,
      providers: [...settings.providers, newProvider]
    })
    setSelectedProviderId(newId)
  }

  /**
   * 根据 ID 查找已启用的提供商（找不到或已禁用时返回第一个已启用的）
   */
  const resolveProvider = (providers: ModelProvider[], providerId: string) => {
    const matched = providers.find(p => p.id === providerId)
    if (matched && isProviderEnabled(matched)) return matched
    return providers.find(p => isProviderEnabled(p)) ?? providers[0]
  }

  /**
   * 确保当前模型在已启用模型列表中
   */
  const resolveModel = (provider: ModelProvider | undefined, currentModel: string) => {
    if (!provider) return currentModel
    if (provider.enabledModels.includes(currentModel)) return currentModel
    return provider.enabledModels[0] || currentModel
  }

  /**
   * 删除提供商
   * 删除后会自动将使用该提供商的功能切换到第一个可用提供商
   */
  const deleteProvider = (id: string) => {
    if (!settings) return
    if (modelPickerProviderId === id) setModelPickerProviderId(null)
    const nextProviders = settings.providers.filter(p => p.id !== id)
    const translatorProvider = resolveProvider(nextProviders, settings.translatorProviderId)
    const screenshotProvider = resolveProvider(nextProviders, settings.screenshotTranslation?.providerId || '')
    // lens providerId 为空表示 fallback 到 translator，删除时若已设置自身 provider 才需要级联
    const lensHadOwnProvider = !!settings.lens?.providerId
    const lensProvider = lensHadOwnProvider
      ? resolveProvider(nextProviders, settings.lens?.providerId || '')
      : undefined
    const deletedProviderWasChatModel =
      settings.defaultModels.chat.providerId === id || settings.chatProviderId === id

    const defaultModels = clearDefaultModelProvider(settings.defaultModels, id)
    const nextSettings: SettingsData = {
      ...settings,
      providers: nextProviders,
      translatorProviderId: translatorProvider ? translatorProvider.id : '',
      translatorModel: resolveModel(translatorProvider, settings.translatorModel),
      defaultModels,
      screenshotTranslation: {
        ...settings.screenshotTranslation,
        providerId: screenshotProvider ? screenshotProvider.id : '',
        model: resolveModel(screenshotProvider, settings.screenshotTranslation?.model || '')
      },
      ...(lensHadOwnProvider ? {
        lens: {
          ...settings.lens,
          providerId: lensProvider ? lensProvider.id : '',
          model: resolveModel(lensProvider, settings.lens?.model || '')
        }
      } : {})
    }
    setSettings({
      ...nextSettings,
      chatProviderId: deletedProviderWasChatModel ? '' : settings.chatProviderId,
      chatModel: deletedProviderWasChatModel ? '' : settings.chatModel,
    })
  }

  /**
   * 添加已启用模型
   */
  const addEnabledModel = (providerId: string, model: string) => {
    if (!settings || !model.trim()) return
    const provider = settings.providers.find(p => p.id === providerId)
    if (!provider || provider.enabledModels.includes(model)) return
    updateProvider(providerId, {
      enabledModels: [...provider.enabledModels, model.trim()]
    })
  }

  const addAllEnabledModels = (providerId: string, models: string[]) => {
    if (!settings || models.length === 0) return
    const provider = settings.providers.find((p) => p.id === providerId)
    if (!provider) return

    const enabledKeys = new Set(provider.enabledModels.map((model) => model.toLowerCase()))
    const nextModels: string[] = []
    const seen = new Set<string>()
    for (const model of models) {
      const trimmed = model.trim()
      if (!trimmed) continue
      const key = trimmed.toLowerCase()
      if (enabledKeys.has(key) || seen.has(key)) continue
      seen.add(key)
      nextModels.push(trimmed)
    }
    if (nextModels.length === 0) return

    updateProvider(providerId, {
      enabledModels: [...provider.enabledModels, ...nextModels],
    })
  }

  /**
   * 移除已启用模型
   * 移除后会自动更新使用该模型的功能到新的默认模型
   */
  const removeEnabledModel = (providerId: string, model: string) => {
    if (!settings) return
    const provider = settings.providers.find((p) => p.id === providerId)
    if (!provider) return

    const nextEnabledModels = provider.enabledModels.filter((m) => m !== model)
    const resolveAfterRemoval = (currentModel: string) => {
      if (currentModel !== model) return currentModel
      return nextEnabledModels[0] || ''
    }

    setSettings((prev) => {
      if (!prev) return prev

      const nextProviders = prev.providers.map((p) =>
        p.id === providerId ? { ...p, enabledModels: nextEnabledModels } : p,
      )

      const next = {
        ...prev,
        providers: nextProviders,
      }
      const defaultModels = resolveDefaultModelsAfterModelRemoval(
        prev.defaultModels,
        providerId,
        resolveAfterRemoval,
      )

      if (prev.translatorProviderId === providerId) {
        next.translatorModel = resolveAfterRemoval(prev.translatorModel)
      }
      next.defaultModels = defaultModels
      next.chatProviderId = defaultModels.chat.providerId
      next.chatModel = defaultModels.chat.model

      if (prev.screenshotTranslation.providerId === providerId) {
        next.screenshotTranslation = {
          ...prev.screenshotTranslation,
          model: resolveAfterRemoval(prev.screenshotTranslation.model),
        }
      }

      if (prev.lens?.providerId === providerId) {
        next.lens = {
          ...prev.lens,
          model: resolveAfterRemoval(prev.lens.model || ''),
        }
      }

      return next
    })
  }

  /**
   * 保存模型自定义参数
   */
  const saveModelOverride = useCallback((providerId: string, modelName: string, info: ModelInfo) => {
    if (!settings) return
    const provider = settings.providers.find(p => p.id === providerId)
    if (!provider) return
    updateProvider(providerId, {
      modelOverrides: {
        ...provider.modelOverrides,
        [modelName]: info,
      },
    })
  }, [settings, updateProvider])

  /**
   * 重置模型参数为数据库默认值
   */
  const resetModelOverride = useCallback((providerId: string, modelName: string) => {
    if (!settings) return
    const provider = settings.providers.find(p => p.id === providerId)
    if (!provider?.modelOverrides?.[modelName]) return
    // eslint-disable-next-line @typescript-eslint/no-unused-vars
    const { [modelName]: _removed, ...rest } = provider.modelOverrides
    updateProvider(providerId, { modelOverrides: rest })
  }, [settings, updateProvider])

  /**
   * 从提供商 API 获取可用模型列表
   */
  const fetchModels = async (providerId: string) => {
    if (!settings || fetchingProviderId) return
    setFetchingProviderId(providerId)
    try {
      const currentProvider = settings.providers.find(p => p.id === providerId)
      const models = await api.fetchModels(providerId, currentProvider
        ? {
          id: currentProvider.id,
          baseUrl: currentProvider.baseUrl,
          apiKeys: currentProvider.apiKeys,
        }
        : undefined)
      if (currentProvider) {
        updateProvider(providerId, { availableModels: models })
      }
    } catch (err) {
      console.error('Failed to fetch models:', err)
    } finally {
      setFetchingProviderId(null)
    }
  }

  const openModelPicker = (providerId: string) => {
    if (!settings) return
    const provider = settings.providers.find((p) => p.id === providerId)
    if (!provider) return
    setModelPickerProviderId(providerId)
    if (provider.availableModels.length === 0 && fetchingProviderId !== providerId) {
      void fetchModels(providerId)
    }
  }

  /**
   * 更新截图翻译配置
   */
  const updateScreenshotTranslation = useCallback((updates: Partial<SettingsData['screenshotTranslation']>) => {
    setSettings((prev) => {
      if (!prev) return prev
      const current = prev.screenshotTranslation || {
        enabled: true,
        hotkey: 'CommandOrControl+Shift+A',
        textHotkey: 'CommandOrControl+Shift+T',
        providerId: 'default-ocr',
        model: '',
        directTranslate: false,
        thinkingEnabled: false,
        streamEnabled: true,
        ocrMode: 'cloud_vision',
        prompt: ''
      }
      return { ...prev, screenshotTranslation: { ...current, ...updates } }
    })
  }, [])

  /**
   * 更新 Lens 配置
   */
  const updateLens = useCallback((updates: Partial<SettingsData['lens']>) => {
    setSettings((prev) => {
      if (!prev) return prev
      const current = prev.lens || {
        enabled: true,
        hotkey: 'CommandOrControl+Shift+G',
        providerId: '',
        model: '',
        defaultLanguage: '',
        streamEnabled: true,
        thinkingEnabled: true,
        systemPrompt: '',
        questionPrompt: '',
        sendToChat: true,
        messageOrder: 'asc' as const,
        showCaptureHint: true,
        windowsFreezeFrameSelection: getPlatform() === 'windows',
        webSearch: {
          enabled: false,
          provider: 'tavily' as const,
          tavilyApiKey: '',
          exaApiKey: '',
          maxResults: 5,
          searchDepth: 'basic' as const,
        },
      }
      return { ...prev, lens: { ...current, ...updates } }
    })
  }, [])

  const updateChat = useCallback((updates: Partial<NonNullable<SettingsData['chat']>>) => {
    setSettings((prev) => {
      if (!prev) return prev
      const current = prev.chat || defaultChatConfig()
      return { ...prev, chat: { ...current, ...updates } }
    })
  }, [])

  const updateChatMemory = useCallback((updates: Partial<ChatMemoryConfig>) => {
    setSettings((prev) => {
      if (!prev) return prev
      const current = prev.chatMemory || defaultChatMemory()
      return { ...prev, chatMemory: { ...current, ...updates } }
    })
  }, [])

  const updateLensWebSearch = useCallback((updates: Partial<NonNullable<SettingsData['lens']['webSearch']>>) => {
    setSettings((prev) => {
      if (!prev) return prev
      const currentLens = prev.lens || {
        enabled: true,
        hotkey: 'CommandOrControl+Shift+G',
      }
      const currentWebSearch = currentLens.webSearch || {
        enabled: false,
        provider: 'tavily' as const,
        tavilyApiKey: '',
        exaApiKey: '',
        maxResults: 5,
        searchDepth: 'basic' as const,
      }
      return {
        ...prev,
        lens: {
          ...currentLens,
          webSearch: {
            ...currentWebSearch,
            ...updates,
          },
        },
      }
    })
  }, [])

  const refreshChatMemory = useCallback(async () => {
    setMemoryLoading(true)
    setMemoryError('')
    try {
      const result = await api.chatMemoryGet()
      const next = {
        l1: result.l1.content,
        l2: result.l2.content,
      }
      setMemoryDrafts(next)
      setMemorySnapshots(next)
      setMemoryDir(result.dir)
      setMemorySuccess('')
    } catch (err) {
      setMemoryError(err instanceof Error ? err.message : String(err))
    } finally {
      setMemoryLoading(false)
    }
  }, [])

  const handleSaveMemoryLayer = useCallback(async (layer: MemoryLayerKey) => {
    const content = memoryDrafts[layer]
    if (layer === 'l1' && utf8ByteLength(content) > MEMORY_L1_MAX_BYTES) {
      setMemoryError(lang === 'zh'
        ? `L1 超过 ${MEMORY_L1_MAX_BYTES} 字节，请先精简或归档到 L2。`
        : `L1 exceeds ${MEMORY_L1_MAX_BYTES} bytes. Shorten it or archive details into L2.`)
      return
    }
    setMemorySavingLayer(layer)
    setMemoryError('')
    setMemorySuccess('')
    try {
      const saved = await api.chatMemorySave(layer, content)
      setMemoryDrafts((prev) => ({ ...prev, [layer]: saved.content }))
      setMemorySnapshots((prev) => ({ ...prev, [layer]: saved.content }))
      setMemorySuccess(lang === 'zh'
        ? `${layer.toUpperCase()} 已保存`
        : `${layer.toUpperCase()} saved`)
    } catch (err) {
      setMemoryError(err instanceof Error ? err.message : String(err))
    } finally {
      setMemorySavingLayer(null)
    }
  }, [lang, memoryDrafts])

  const handleOpenMemoryFolder = useCallback(async () => {
    setMemoryError('')
    try {
      const result = await api.chatMemoryOpenFolder()
      if (!result.success) {
        setMemoryError(result.error || (lang === 'zh' ? '打开记忆文件夹失败' : 'Failed to open memory folder'))
      } else if (result.path) {
        setMemoryDir(result.path)
      }
    } catch (err) {
      setMemoryError(err instanceof Error ? err.message : String(err))
    }
  }, [lang])

  useEffect(() => {
    if (activeTab === 'memory') {
      void refreshChatMemory()
    }
  }, [activeTab, refreshChatMemory])

  /**
   * 切换快捷键录制状态
   */
  const toggleRecording = (target: 'main' | 'screenshotTranslation' | 'screenshotTranslationText' | 'lens') => {
    setRecordingTarget((current) => (current === target ? null : target))
  }

  // 当前语言对应的默认 lens 提示词
  const lensDefaults = defaultPrompts?.lensPrompts?.[settings?.lens?.defaultLanguage === 'en' ? 'en' : 'zh']
  const chatLangKey = settings?.chat?.defaultLanguage === 'en' ? 'en' : 'zh'
  const chatDefaults = defaultPrompts?.chatPrompts?.[chatLangKey]
  const chatConfig = settings?.chat || defaultChatConfig()
  const chatMemory = settings?.chatMemory || defaultChatMemory()
  const chatSystemPromptValue = chatSystemPromptInteracted
    ? (chatConfig.systemPrompt || '')
    : (chatConfig.systemPrompt || chatDefaults || '')
  const chatFallbackMaxOutputTokens = chatConfig.maxOutputTokens ?? 8192
  const effectiveChatMaxOutput = settings
    ? resolveEffectiveChatMaxOutput(settings, chatFallbackMaxOutputTokens)
    : { maxOutput: chatFallbackMaxOutputTokens, source: 'fallback' as const, model: '', provider: undefined }
  const chatMaxOutputSourceLabel = effectiveChatMaxOutput.source === 'override'
    ? (lang === 'zh' ? '模型参数' : 'Model override')
    : effectiveChatMaxOutput.source === 'database'
      ? (lang === 'zh' ? '内置模型库' : 'Model database')
      : (lang === 'zh' ? '兜底设置' : 'Fallback setting')
  const chatMaxOutputModelLabel = effectiveChatMaxOutput.model
    ? (effectiveChatMaxOutput.provider?.name
      ? `${effectiveChatMaxOutput.provider.name} / ${effectiveChatMaxOutput.model}`
      : effectiveChatMaxOutput.model)
    : (lang === 'zh' ? '未配置聊天模型' : 'No chat model configured')

  // 快捷键录制监听
  useEffect(() => {
    if (!recordingTarget) return
    const handler = (e: KeyboardEvent) => {
      e.preventDefault()
      e.stopPropagation()
      if (e.key === 'Escape') {
        setRecordingTarget(null)
        return
      }
      const hotkey = buildHotkey(e)
      if (!hotkey) return
      if (recordingTarget === 'main') {
        updateSettings({ hotkey })
      } else if (recordingTarget === 'screenshotTranslation') {
        updateScreenshotTranslation({ hotkey })
      } else if (recordingTarget === 'screenshotTranslationText') {
        updateScreenshotTranslation({ textHotkey: hotkey })
      } else if (recordingTarget === 'lens') {
        updateLens({ hotkey })
      }
      setRecordingTarget(null)
    }
    window.addEventListener('keydown', handler, true)
    return () => window.removeEventListener('keydown', handler, true)
  }, [recordingTarget, updateLens, updateScreenshotTranslation, updateSettings])

  const loadingShellClass =
    variant === 'embedded'
      ? 'flex min-h-0 min-w-0 flex-1 items-center justify-center bg-white dark:bg-[#212121]'
      : 'flex items-center justify-center h-full bg-neutral-200 dark:bg-black'

  if (loading) {
    return (
      <div className={loadingShellClass}>
        <div className="w-6 h-6 border-2 border-neutral-300 dark:border-neutral-700 border-t-neutral-800 dark:border-t-neutral-200 rounded-full animate-spin" />
      </div>
    )
  }

  if (loadError || !settings) {
    // 加载失败：显示错误 + 重试按钮，禁止用户在不知情的情况下用合成默认值 Save 覆盖磁盘
    return (
      <div className={`${loadingShellClass} p-6`}>
        <div className="max-w-sm w-full bg-white dark:bg-[#1C1C1E] rounded-xl shadow-sm border border-black/5 dark:border-white/5 p-5 text-center">
          <div className="text-[14px] font-semibold text-neutral-900 dark:text-neutral-100 mb-1">
            {lang === 'zh' ? '加载设置失败' : 'Failed to load settings'}
          </div>
          <div className="text-[11px] text-rose-600 dark:text-rose-400 mb-4 break-all" title={loadError}>
            {loadError}
          </div>
          <div className="flex gap-2 justify-center">
            <button
              type="button"
              onClick={() => setReloadKey((k) => k + 1)}
              className="flex items-center gap-1.5 text-[12px] font-medium px-3 py-1.5 rounded-md bg-neutral-900 dark:bg-white text-white dark:text-neutral-900 hover:bg-neutral-800 dark:hover:bg-neutral-100 transition-all"
              data-tauri-drag-region="false"
            >
              <RefreshCw size={12} />
              {lang === 'zh' ? '重试' : 'Retry'}
            </button>
            <button
              type="button"
              onClick={onClose}
              className="text-[12px] font-medium px-3 py-1.5 rounded-md text-neutral-600 dark:text-neutral-400 hover:bg-black/5 dark:hover:bg-white/5 transition-all"
              data-tauri-drag-region="false"
            >
              {t.cancel}
            </button>
          </div>
        </div>
      </div>
    )
  }

  const navItems = [
    { id: 'general' as const, label: t.tabGeneral, icon: GeneralIcon },
    { id: 'translate' as const, label: t.tabTranslate, icon: TranslateIcon },
    { id: 'screenshot' as const, label: t.tabScreenshot, icon: ScreenshotIcon },
    { id: 'lens' as const, label: t.lensTabLabel, icon: LensIcon },
    { id: 'chat' as const, label: t.tabChatClient, icon: ChatIcon },
    { id: 'memory' as const, label: t.tabMemory, icon: MemoryIcon },
    { id: 'mixer' as const, label: t.tabMixer, icon: MixerIcon },
    { id: 'kivioCode' as const, label: 'Kivio Code', icon: CodeIcon },
    { id: 'externalAgents' as const, label: t.tabExternalAgents, icon: AgentIcon },
    { id: 'mcp' as const, label: 'MCP', icon: McpIcon },
    { id: 'connectors' as const, label: t.tabConnectors, icon: ConnectorsIcon },
    { id: 'knowledge' as const, label: lang === 'zh' ? '知识库' : 'Knowledge', icon: KnowledgeIcon },
    { id: 'skill' as const, label: 'Skill', icon: SkillIcon },
    { id: 'webSearch' as const, label: t.tabWebSearch, icon: WebSearchIcon },
    { id: 'usage' as const, label: lang === 'zh' ? '用量统计' : 'Usage', icon: UsageIcon },
    { id: 'providers' as const, label: t.tabModels, icon: ProvidersIcon },
  ]
  const pageMeta: Record<typeof activeTab, { title: string; subtitle: string; right?: string }> = {
    general: {
      title: t.tabGeneral,
      subtitle: lang === 'zh' ? '外观、行为、归档和权限。' : 'Appearance, behavior, archive, and permissions.',
    },
    translate: {
      title: t.tabTranslate,
      subtitle: lang === 'zh' ? '输入翻译的快捷键、语言、模型和提示词。' : 'Shortcut, language, model, and prompt for input translation.',
    },
    screenshot: {
      title: t.tabScreenshot,
      subtitle: lang === 'zh' ? '截图选择、OCR、输出和翻译模型。' : 'Capture selection, OCR, output, and translation model.',
    },
    lens: {
      title: t.lensTabLabel,
      subtitle: lang === 'zh' ? '视觉问答的快捷键、响应方式和提示词。' : 'Shortcut, response behavior, and prompts for visual Q&A.',
    },
    chat: {
      title: t.tabChatClient,
      subtitle: lang === 'zh'
        ? '主对话模型、流式/思考行为、系统提示词；副任务模型见混音器。'
        : 'Main chat model, streaming/thinking, and system prompt; side-task models live under Mixer.',
    },
    memory: {
      title: t.tabMemory,
      subtitle: lang === 'zh'
        ? 'L1 在线记忆常驻注入；L2 长期记忆只通过工具读取。'
        : 'L1 is always injected when enabled; L2 is read only through tools.',
    },
    mixer: {
      title: t.tabMixer,
      subtitle: lang === 'zh'
        ? '按副任务路由模型：视觉、标题总结、上下文压缩、生图。'
        : 'Route models by side task: vision, title summaries, context compression, and image generation.',
    },
    kivioCode: {
      title: 'Kivio Code',
      subtitle: lang === 'zh'
        ? '终端编码代理的默认模型、工具审批策略与上下文读取。'
        : 'Default model, tool approval policy, and context reading for the terminal coding agent.',
    },
    externalAgents: {
      title: t.tabExternalAgents,
      subtitle: t.externalAgentsPageHint,
    },
    mcp: {
      title: 'MCP',
      subtitle: lang === 'zh' ? '管理 MCP 服务器与工具审批策略。' : 'Manage MCP servers and tool approval policy.',
    },
    connectors: {
      title: t.tabConnectors,
      subtitle: lang === 'zh'
        ? '连接 Notion、GitHub 等外部数据源；token 存在本机，数据默认直连。'
        : 'Connect external data sources like Notion and GitHub; tokens stay on your machine, data goes direct by default.',
    },
    skill: {
      title: 'Skill',
      subtitle: lang === 'zh' ? '管理内置与用户 Skill。' : 'Manage built-in and user Skills.',
    },
    knowledge: {
      title: lang === 'zh' ? '知识库' : 'Knowledge',
      subtitle: lang === 'zh'
        ? '导入文档建立向量知识库，对话中由 agent 检索并标注出处。'
        : 'Import documents into vector libraries the agent retrieves and cites in chat.',
    },
    webSearch: {
      title: t.tabWebSearch,
      subtitle: lang === 'zh'
        ? 'Tavily/Exa 密钥与参数；分别开启 Lens 与 Chat 的联网搜索。'
        : 'Tavily/Exa keys and parameters; enable web search for Lens and Chat separately.',
    },
    usage: {
      title: lang === 'zh' ? '用量统计' : 'Usage',
      subtitle: lang === 'zh'
        ? '查看本地模型请求、Token、成本估算和来源分布。'
        : 'Inspect local model requests, tokens, estimated cost, and usage distribution.',
    },
    providers: {
      title: t.tabModels,
      subtitle: lang === 'zh' ? '管理 OpenAI 兼容供应商、密钥和启用模型。' : 'Manage OpenAI-compatible providers, keys, and enabled models.',
    },
    about: {
      title: lang === 'zh' ? '关于' : 'About',
      subtitle: lang === 'zh' ? '版本、更新和应用信息。' : 'Version, updates, and application details.',
    },
  }
  const selectedProvider = settings.providers.find((provider) => provider.id === selectedProviderId) ?? settings.providers[0]
  const chatProvider = settings.providers.find((provider) => provider.id === settings.chatProviderId)
    ?? settings.providers.find((provider) => provider.id === settings.lens?.providerId)
    ?? settings.providers.find((provider) => provider.id === settings.translatorProviderId)
  const chatProviderSupportsTools = chatProvider?.supportsTools !== false
  const disabledSkillIds = chatTools.disabledSkillIds ?? []
  const builtinSkills = skills.filter(isBuiltinSkill)
  const userSkills = skills.filter((skill) => !isBuiltinSkill(skill))
  const enabledSkillCount = skills.filter((skill) => !disabledSkillIds.includes(skill.id)).length

  const categoryNav =
    variant === 'embedded' ? (
      <>
        <nav className="settings-embedded-nav-list">
          {navItems.map((item) => {
            const Icon = item.icon
            return (
              <button
                key={item.id}
                type="button"
                onClick={() => setActiveTab(item.id)}
                className={`settings-embedded-nav-item ${activeTab === item.id ? 'active' : ''}`}
                data-tauri-drag-region="false"
              >
                <span className="settings-embedded-nav-icon">
                  <Icon size={17} strokeWidth={1.75} />
                </span>
                <span>{item.label}</span>
              </button>
            )
          })}
        </nav>
        <div className="min-h-0 flex-1" />
        <nav className="settings-embedded-nav-list settings-embedded-nav-list--footer">
          <button
            type="button"
            onClick={() => setActiveTab('about')}
            className={`settings-embedded-nav-item ${activeTab === 'about' ? 'active' : ''}`}
            data-tauri-drag-region="false"
          >
            <span className="settings-embedded-nav-icon">
              <AboutIcon size={17} strokeWidth={1.75} />
            </span>
            <span>{lang === 'zh' ? '关于' : 'About'}</span>
          </button>
        </nav>
      </>
    ) : (
      <>
        <nav className="kv-nav">
          {navItems.map((item) => {
            const Icon = item.icon
            return (
              <button
                key={item.id}
                type="button"
                onClick={() => setActiveTab(item.id)}
                className={`kv-nav-item ${activeTab === item.id ? 'active' : ''}`}
                data-tauri-drag-region="false"
              >
                <Icon strokeWidth={1.7} />
                <span>{item.label}</span>
              </button>
            )
          })}
        </nav>

        <div className="kv-nav-spacer" />

        <nav className="kv-nav">
          <button
            type="button"
            onClick={() => setActiveTab('about')}
            className={`kv-nav-item ${activeTab === 'about' ? 'active' : ''}`}
            data-tauri-drag-region="false"
          >
            <AboutIcon strokeWidth={1.7} />
            <span>{lang === 'zh' ? '关于' : 'About'}</span>
          </button>
        </nav>
      </>
    )

  const settingsMain = (
        <main className={`kv-content ${variant === 'embedded' ? 'settings-embedded-main' : ''}`}>
          <header
            className={`kv-page-header ${variant === 'embedded' ? 'settings-embedded-header' : ''}`}
            data-tauri-drag-region={variant === 'embedded' ? true : undefined}
          >
            <div>
              <div className="kv-page-title">{pageMeta[activeTab].title}</div>
              <div className="kv-page-sub">{pageMeta[activeTab].subtitle}</div>
            </div>
            <div className="kv-page-header-right">{pageMeta[activeTab].right}</div>
          </header>

          <div className={`kv-scroll ${variant === 'embedded' ? 'settings-embedded-scroll' : ''}`}>
            {/* ===== 基础设置标签页 ===== */}
            {activeTab === 'general' && (
              <>
                <SettingsGroup title={lang === 'zh' ? '外观' : 'Appearance'}>
                  <SettingRow label={t.language} description={lang === 'zh' ? '设置 Kivio 界面语言。' : 'Used for the Kivio interface.'}>
                    <Select
                      className="w-36"
                      value={settings.settingsLanguage || 'zh'}
                      onChange={(v) => updateSettings({ settingsLanguage: v as 'zh' | 'en' })}
                      options={[
                        { value: 'zh', label: '中文' },
                        { value: 'en', label: 'English' },
                      ]}
                    />
                  </SettingRow>
                  <SettingRow label={t.theme} description={lang === 'zh' ? '跟随系统外观，或固定浅色/深色。' : 'Match system appearance or pick a mode.'}>
                    <div className="kv-seg">
                      {[
                        { value: 'system', label: t.themeSystem },
                        { value: 'light', label: t.themeLight },
                        { value: 'dark', label: t.themeDark },
                      ].map((option) => (
                        <button
                          key={option.value}
                          type="button"
                          className={(settings.theme || 'system') === option.value ? 'active' : ''}
                          onClick={() => updateSettings({ theme: option.value as SettingsData['theme'] })}
                          data-tauri-drag-region="false"
                        >
                          {option.label}
                        </button>
                      ))}
                    </div>
                  </SettingRow>
                  <SettingRow label={t.themeColor} description={lang === 'zh' ? '选择浅色界面的背景色调。' : 'Choose the surface tint for light appearance.'}>
                    <div className="kv-theme-colors" role="radiogroup" aria-label={t.themeColor}>
                      {THEME_COLOR_PRESETS.map((preset) => {
                        const active = themeColor === preset.id
                        return (
                          <button
                            key={preset.id}
                            type="button"
                            className={active ? 'active' : ''}
                            onClick={() => updateSettings({ themeColor: preset.id })}
                            role="radio"
                            aria-checked={active}
                            aria-label={preset.labels[lang]}
                            title={`${preset.labels[lang]} ${preset.hex}`}
                            data-tauri-drag-region="false"
                          >
                            <span style={{ background: preset.hex }} />
                          </button>
                        )
                      })}
                    </div>
                  </SettingRow>
                </SettingsGroup>

                <SettingsGroup title={lang === 'zh' ? '行为' : 'Behavior'}>
                  <SettingRow label={t.launchAtStartup} description={lang === 'zh' ? '登录后在后台启动 Kivio。' : 'Open Kivio in the background when you sign in.'}>
                    <Toggle
                      checked={settings.launchAtStartup ?? false}
                      onChange={(v) => updateSettings({ launchAtStartup: v })}
                    />
                  </SettingRow>
                  <SettingRow label={t.retryEnabled} description={t.retryAttemptsHint}>
                    <Toggle
                      checked={settings.retryEnabled ?? true}
                      onChange={(v) => updateSettings({ retryEnabled: v })}
                    />
                  </SettingRow>
                  {settings.retryEnabled !== false && (
                    <SettingRow label={t.retryAttempts} description={lang === 'zh' ? '范围 1-5 次。' : 'Range: 1-5 attempts.'}>
                      <Input
                        type="number"
                        value={retryAttemptsInput}
                        onChange={handleRetryAttemptsChange}
                        onBlur={handleRetryAttemptsBlur}
                        placeholder="3"
                        min={1}
                        max={5}
                        className="!w-20 text-center"
                      />
                    </SettingRow>
                  )}
                </SettingsGroup>

                <SettingsGroup title={lang === 'zh' ? '备份与恢复' : 'Backup & Restore'}>
                  <FieldBlock
                    label={lang === 'zh' ? '设置备份' : 'Settings backup'}
                    description={lang === 'zh'
                      ? '导出全部设置（含供应商、模型与 API Key）为 JSON 文件。导入会覆盖当前全部设置并立即生效。'
                      : 'Export all settings (providers, models, and API keys) to a JSON file. Importing overwrites all current settings and takes effect immediately.'}
                  >
                    <div className="flex flex-wrap items-center gap-2">
                      <button
                        type="button"
                        className="kv-btn sm"
                        onClick={handleExportSettings}
                        data-tauri-drag-region="false"
                      >
                        <Download size={11} />
                        {lang === 'zh' ? '导出设置' : 'Export'}
                      </button>
                      <button
                        type="button"
                        className="kv-btn sm"
                        onClick={handleImportSettings}
                        data-tauri-drag-region="false"
                      >
                        <Upload size={11} />
                        {lang === 'zh' ? '导入设置' : 'Import'}
                      </button>
                      {backupStatus && (
                        <span className={`text-[12px] ${backupStatus.kind === 'ok' ? 'text-emerald-600 dark:text-emerald-400' : 'text-red-500'}`}>
                          {backupStatus.msg}
                        </span>
                      )}
                    </div>
                  </FieldBlock>
                </SettingsGroup>

                {permissionStatus?.platform === 'macos' && (
                  <SettingsGroup title={t.permissions}>
                    <PermissionItem
                      label={t.accessibilityPermission}
                      granted={permissionStatus.accessibility}
                      grantedText={t.permissionGranted}
                      missingText={t.permissionMissing}
                      actionLabel={t.openSystemSettings}
                      onOpen={() => handleOpenPermissionSettings('accessibility')}
                    />
                    <PermissionItem
                      label={t.screenRecordingPermission}
                      granted={permissionStatus.screenRecording}
                      grantedText={t.permissionGranted}
                      missingText={t.permissionMissing}
                      actionLabel={t.openSystemSettings}
                      onOpen={() => handleOpenPermissionSettings('screen-recording')}
                    />
                    <div className="flex justify-end py-2">
                      <button
                        type="button"
                        onClick={refreshPermissions}
                        disabled={permissionsLoading}
                        className="kv-btn sm"
                        data-tauri-drag-region="false"
                      >
                        <RefreshCw size={10} className={permissionsLoading ? 'animate-spin' : ''} />
                        {t.refreshPermissions}
                      </button>
                    </div>
                  </SettingsGroup>
                )}
              </>
            )}

            {/* ===== 翻译设置标签页 ===== */}
            {activeTab === 'translate' && (
              <>
                <SettingsGroup title={t.hotkey}>
                  <SettingRow label={t.hotkey} description={lang === 'zh' ? '翻译当前选中文本或剪贴板内容。' : 'Translates the current selection or clipboard.'} stack>
                    <HotkeyInput
                      value={settings.hotkey}
                      placeholder={t.hotkeyPlaceholder}
                      recording={recordingTarget === 'main'}
                      onToggleRecording={() => toggleRecording('main')}
                      recordLabel={t.hotkeyRecord}
                      recordingLabel={t.hotkeyRecording}
                      recordingPlaceholder={t.hotkeyRecordingPlaceholder}
                      onClear={() => updateSettings({ hotkey: '' })}
                      clearLabel={t.hotkeyClear}
                      error={conflictMessageFor('main')}
                    />
                  </SettingRow>
                </SettingsGroup>

                <SettingsGroup title={lang === 'zh' ? '输出' : 'Output'}>
                  <SettingRow label={t.targetLang} description={lang === 'zh' ? '自动模式会在中英文之间切换。' : 'Auto switches between Chinese and English.'}>
                    <Select
                      className="w-40"
                      value={settings.targetLang || 'auto'}
                      onChange={(v) => updateSettings({ targetLang: v })}
                      options={[
                        { value: 'auto', label: t.langAuto },
                        { value: 'en', label: t.langEn },
                        { value: 'zh', label: t.langZh },
                        { value: 'zh-Hant', label: t.langZhTw },
                        { value: 'ja', label: t.langJa },
                        { value: 'ko', label: t.langKo },
                        { value: 'fr', label: t.langFr },
                        { value: 'de', label: t.langDe },
                      ]}
                    />
                  </SettingRow>
                  <SettingRow label={t.autoPaste} description={lang === 'zh' ? '翻译完成后自动粘贴到当前应用。' : 'Paste translated text into the foreground app after translation completes.'}>
                    <Toggle
                      checked={settings.autoPaste ?? true}
                      onChange={(v) => updateSettings({ autoPaste: v })}
                    />
                  </SettingRow>
                </SettingsGroup>

                <SettingsGroup title={t.engine}>
                  <SettingRow label={t.selectModelPair} description={lang === 'zh' ? '选择输入翻译使用的供应商和模型。' : 'Choose the provider and model used for input translation.'}>
                    <ModelPairSelect
                      providerId={settings.translatorProviderId}
                      model={settings.translatorModel}
                      providers={settings.providers}
                      onChange={(providerId, model) => {
                        updateSettings({ translatorProviderId: providerId, translatorModel: model })
                      }}
                    />
                  </SettingRow>
                </SettingsGroup>

                <SettingsGroup title={t.translatorPrompt}>
                  <PromptField
                    label={t.translatorPrompt}
                    description={t.translatorPromptHint}
                    value={settings.translatorPrompt || ''}
                    defaultText={defaultPrompts?.translationTemplate || ''}
                    restoreLabel={t.restoreDefaultPrompt}
                    onChange={(v) => updateSettings({ translatorPrompt: v })}
                  />
                </SettingsGroup>
              </>
            )}

            {/* ===== 截图设置标签页 ===== */}
            {activeTab === 'screenshot' && (
              <ScreenshotTranslationSettings
                settings={settings}
                isMac={isMac}
                hasSystemOcr={hasSystemOcr}
                recordingTarget={recordingTarget}
                defaultPrompts={defaultPrompts}
                rapidOcrStatus={rapidOcrStatus}
                rapidOcrDownloadState={rapidOcrDownloadState}
                rapidOcrDownloadError={rapidOcrDownloadError}
                t={t}
                onUpdate={updateScreenshotTranslation}
                onToggleRecording={toggleRecording}
                onRefreshRapidOcrStatus={refreshRapidOcrStatus}
                onDownloadRapidOcr={handleDownloadRapidOcr}
                hotkeyError={conflictMessageFor('screenshotTranslation')}
                textHotkeyError={conflictMessageFor('screenshotTranslationText')}
                hotkeyClearLabel={t.hotkeyClear}
              />
            )}

            {/* ===== Lens 标签页 ===== */}
            {activeTab === 'lens' && (
              <>
                <SettingsGroup title={t.lensSection}>
                  <SettingRow label={t.enabled} description={lang === 'zh' ? '启用 Lens 截图问答入口。' : 'Enable the Lens screenshot Q&A entry point.'}>
                    <Toggle
                      checked={settings.lens?.enabled !== false}
                      onChange={(v) => updateLens({ enabled: v })}
                    />
                  </SettingRow>

                  {settings.lens?.enabled !== false && (
                    <>
                      <SettingRow label={t.hotkey} description={lang === 'zh' ? '进入 Lens 截图选择模式。' : 'Enter Lens screenshot selection mode.'} stack>
                        <HotkeyInput
                          value={settings.lens?.hotkey ?? ''}
                          placeholder="CommandOrControl+Shift+G"
                          recording={recordingTarget === 'lens'}
                          onToggleRecording={() => toggleRecording('lens')}
                          recordLabel={t.hotkeyRecord}
                          recordingLabel={t.hotkeyRecording}
                          recordingPlaceholder={t.hotkeyRecordingPlaceholder}
                          onClear={() => updateLens({ hotkey: '' })}
                          clearLabel={t.hotkeyClear}
                          error={conflictMessageFor('lens')}
                        />
                      </SettingRow>
                      <SettingRow label={t.lensResponseLanguage} description={lang === 'zh' ? '默认继承输入翻译语言设置。' : 'Defaults to the input translation language setting.'}>
                        <Select
                          className="w-44"
                          value={settings.lens?.defaultLanguage || ''}
                          onChange={(v) => updateLens({ defaultLanguage: v })}
                          options={[
                            { value: '', label: t.lensLanguageInherit },
                            { value: 'zh', label: '中文' },
                            { value: 'zh-Hant', label: '繁體中文' },
                            { value: 'en', label: 'English' },
                          ]}
                        />
                      </SettingRow>
                      <SettingRow label={t.lensStreamEnabled} description={lang === 'zh' ? '模型返回时逐步显示答案。' : 'Show answers progressively as the model responds.'}>
                        <Toggle
                          checked={settings.lens?.streamEnabled !== false}
                          onChange={(v) => updateLens({ streamEnabled: v })}
                        />
                      </SettingRow>
                      <SettingRow label={t.lensThinkingEnabled} description={t.lensThinkingHint}>
                        <Toggle
                          checked={settings.lens?.thinkingEnabled !== false}
                          onChange={(v) => updateLens({ thinkingEnabled: v })}
                        />
                      </SettingRow>
                    </>
                  )}
                </SettingsGroup>

                {settings.lens?.enabled !== false && (
                  <>
                    <SettingsGroup title={lang === 'zh' ? '对话' : 'Conversation'}>
                      <SettingRow label={t.lensSendToChat} description={t.lensSendToChatHint}>
                        <Toggle
                          checked={settings.lens?.sendToChat !== false}
                          onChange={(v) => updateLens({ sendToChat: v })}
                        />
                      </SettingRow>
                      <SettingRow label={t.lensMessageOrder} description={lang === 'zh' ? '控制 Lens 历史消息的排列顺序。' : 'Controls the order of Lens history messages.'}>
                        <Select
                          className="w-52"
                          value={settings.lens?.messageOrder ?? 'asc'}
                          onChange={(v) => updateLens({ messageOrder: v as 'asc' | 'desc' })}
                          options={[
                            { value: 'asc', label: t.lensMessageOrderAsc },
                            { value: 'desc', label: t.lensMessageOrderDesc },
                          ]}
                        />
                      </SettingRow>
                      <SettingRow label={t.lensShowCaptureHint} description={t.lensShowCaptureHintHint}>
                        <Toggle
                          checked={settings.lens?.showCaptureHint !== false}
                          onChange={(v) => updateLens({ showCaptureHint: v })}
                        />
                      </SettingRow>
                      {platform === 'windows' && (
                        <SettingRow label={t.lensWindowsFreezeFrameSelection} description={t.lensWindowsFreezeFrameSelectionHint}>
                          <Toggle
                            checked={settings.lens?.windowsFreezeFrameSelection === true}
                            onChange={(v) => updateLens({ windowsFreezeFrameSelection: v })}
                          />
                        </SettingRow>
                      )}
                    </SettingsGroup>

                    <SettingsGroup title={t.engine}>
                      <SettingRow label={t.selectModelPair} description={lang === 'zh' ? '留空时继承输入翻译模型。' : 'Leave empty to inherit the input translation model.'}>
                        <ModelPairSelect
                          providerId={settings.lens?.providerId || ''}
                          model={settings.lens?.model || ''}
                          providers={settings.providers}
                          inheritLabel={t.lensLanguageInherit}
                          onChange={(providerId, model) => {
                            updateLens({ providerId, model })
                          }}
                        />
                      </SettingRow>
                    </SettingsGroup>

                    <SettingsGroup title={t.imageArchive}>
                      <SettingRow label={t.imageArchive} description={t.imageArchiveHint}>
                        <Toggle
                          checked={settings.imageArchiveEnabled ?? false}
                          onChange={(v) => updateSettings({ imageArchiveEnabled: v })}
                        />
                      </SettingRow>
                      {settings.imageArchiveEnabled && (
                        <SettingRow label={t.imageArchivePath} description={t.imageArchivePathPlaceholder} stack>
                          <div className="kv-path-row">
                            <Input
                              value={settings.imageArchivePath || ''}
                              onChange={(v) => updateSettings({ imageArchivePath: v })}
                              placeholder={t.imageArchivePathPlaceholder}
                            />
                            <button
                              type="button"
                              onClick={async () => {
                                try {
                                  const selected = await open({ directory: true, multiple: false })
                                  if (typeof selected === 'string') {
                                    updateSettings({ imageArchivePath: selected })
                                  }
                                } catch (err) {
                                  console.error('Failed to pick directory:', err)
                                }
                              }}
                              className="kv-btn"
                              data-tauri-drag-region="false"
                            >
                              {t.imageArchiveBrowse}
                            </button>
                          </div>
                        </SettingRow>
                      )}
                    </SettingsGroup>

                    <SettingsGroup title={t.customPrompts}>
                      <PromptField
                        label={t.lensSystemPrompt}
                        value={settings.lens?.systemPrompt || ''}
                        defaultText={lensDefaults?.system || ''}
                        restoreLabel={t.restoreDefaultPrompt}
                        onChange={(v) => updateLens({ systemPrompt: v })}
                      />
                      <PromptField
                        label={t.lensQuestionPrompt}
                        value={settings.lens?.questionPrompt || ''}
                        defaultText={lensDefaults?.question || ''}
                        restoreLabel={t.restoreDefaultPrompt}
                        onChange={(v) => updateLens({ questionPrompt: v })}
                      />
                    </SettingsGroup>
                  </>
                )}
              </>
            )}

            {/* ===== AI 客户端标签页 ===== */}
            {activeTab === 'chat' && (
              <>
                <SettingsGroup title={lang === 'zh' ? '个人资料' : 'Profile'}>
                  <SettingRow
                    label={lang === 'zh' ? '用户名' : 'Display name'}
                    description={lang === 'zh' ? '显示在 Chat 侧栏底部；留空则不显示。' : 'Shown at the bottom of the Chat sidebar; leave empty to hide.'}
                  >
                    <Input
                      value={chatConfig.userDisplayName || ''}
                      onChange={(userDisplayName) => updateChat({ userDisplayName })}
                      placeholder={lang === 'zh' ? '选填' : 'Optional'}
                    />
                  </SettingRow>
                  <SettingRow
                    label={lang === 'zh' ? '头像' : 'Avatar'}
                    description={lang === 'zh' ? '图片链接或 data URL；留空则使用应用图标。' : 'Image URL or data URL; leave empty to use the app icon.'}
                    stack
                  >
                    <Input
                      value={chatConfig.userAvatar || ''}
                      onChange={(userAvatar) => updateChat({ userAvatar })}
                      placeholder="https://..."
                    />
                  </SettingRow>
                </SettingsGroup>

                <SettingsGroup title={t.defaultModelsSection}>
                  <SettingRow
                    label={t.defaultChatModel}
                    description={t.defaultChatModelHint}
                  >
                    <ModelPairSelect
                      providerId={settings.defaultModels.chat.providerId || ''}
                      model={settings.defaultModels.chat.model || ''}
                      providers={settings.providers}
                      inheritLabel={t.defaultModelsUnset}
                      onChange={(providerId, model) => {
                        updateDefaultModel('chat', providerId, model)
                      }}
                    />
                  </SettingRow>
                  {!chatProvider && (
                    <p className="kv-row-desc px-0 pb-2">
                      {lang === 'zh' ? '请先在「模型」中添加并配置供应商。' : 'Add and configure a provider under Models first.'}
                    </p>
                  )}
                  {chatProvider && chatProviderSupportsTools === false && (
                    <p className="kv-row-desc px-0 pb-2 text-amber-700 dark:text-amber-400">
                      {lang === 'zh'
                        ? '当前默认供应商未启用工具调用；MCP / Skill 工具可能不可用。'
                        : 'The default provider is marked as not supporting tools; MCP / Skill may be unavailable.'}
                    </p>
                  )}
                </SettingsGroup>

                <SettingsGroup title={lang === 'zh' ? '响应' : 'Response'}>
                  <SettingRow label={t.chatStreamEnabled} description={t.chatStreamHint}>
                    <Toggle
                      checked={chatConfig.streamEnabled !== false}
                      onChange={(streamEnabled) => updateChat({ streamEnabled })}
                    />
                  </SettingRow>
                  <SettingRow label={t.chatThinkingEnabled} description={t.chatThinkingHint}>
                    <Toggle
                      checked={chatConfig.thinkingEnabled !== false}
                      onChange={(thinkingEnabled) => updateChat({ thinkingEnabled })}
                    />
                  </SettingRow>
                  <SettingRow label={t.chatMaxOutputTokens} description={t.chatMaxOutputTokensHint} stack>
                    <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
                      <div className="min-w-0">
                        <div className="flex flex-wrap items-center gap-2">
                          <span className="text-[15px] font-medium text-neutral-900 dark:text-neutral-50">
                            {formatTokenCount(effectiveChatMaxOutput.maxOutput)}
                          </span>
                          <span className={`kv-tag ${effectiveChatMaxOutput.source === 'fallback' ? 'warn' : 'ok'}`}>
                            {chatMaxOutputSourceLabel}
                          </span>
                        </div>
                        <p className="kv-row-desc mt-1 min-w-0 break-all">
                          {lang === 'zh' ? '当前聊天模型：' : 'Current chat model: '}
                          {chatMaxOutputModelLabel}
                        </p>
                      </div>
                      <div className="flex shrink-0 items-center gap-2">
                        <span className="kv-row-desc whitespace-nowrap">
                          {lang === 'zh' ? '兜底' : 'Fallback'}
                        </span>
                        <Select
                          className="w-44"
                          value={String(chatFallbackMaxOutputTokens)}
                          onChange={(maxOutputTokens) => updateChat({ maxOutputTokens: Number(maxOutputTokens) })}
                          options={CHAT_MAX_OUTPUT_TOKEN_OPTIONS.map((tokens) => ({
                            value: String(tokens),
                            label: formatTokenCount(tokens),
                          }))}
                        />
                      </div>
                    </div>
                  </SettingRow>
                  <SettingRow label={t.chatDefaultLanguage} description={t.chatDefaultLanguageHint}>
                    <Select
                      className="w-44"
                      value={chatConfig.defaultLanguage || ''}
                      onChange={(defaultLanguage) => updateChat({ defaultLanguage })}
                      options={[
                        { value: '', label: t.lensLanguageInherit },
                        { value: 'zh', label: '中文' },
                        { value: 'zh-Hant', label: '繁體中文' },
                        { value: 'en', label: 'English' },
                      ]}
                    />
                  </SettingRow>
                </SettingsGroup>

                <SettingsGroup title={t.customPrompts}>
                  <FieldBlock label={t.chatSystemPrompt} description={t.chatSystemPromptHint}>
                    <div className="mb-2 flex justify-end">
                      <button
                        type="button"
                        className="kv-btn sm"
                        onClick={() => {
                          setChatSystemPromptInteracted(false)
                          updateChat({ systemPrompt: '' })
                        }}
                        disabled={!chatDefaults || (!chatConfig.systemPrompt && !chatSystemPromptInteracted)}
                        data-tauri-drag-region="false"
                      >
                        <RefreshCw size={10} />
                        {t.restoreDefaultPrompt}
                      </button>
                    </div>
                    <TextArea
                      value={chatSystemPromptValue}
                      onChange={(systemPrompt) => {
                        setChatSystemPromptInteracted(true)
                        updateChat({ systemPrompt })
                      }}
                      rows={4}
                    />
                  </FieldBlock>
                </SettingsGroup>

                <SettingsGroup title={t.chatToolsSection}>
                  <p className="kv-row-desc mb-3">{t.chatToolsSectionHint}</p>
                  <div className="flex flex-wrap gap-2 pb-2">
                    <button
                      type="button"
                      className="kv-btn sm"
                      onClick={() => setActiveTab('mcp')}
                      data-tauri-drag-region="false"
                    >
                      <McpIcon size={11} />
                      {t.chatOpenMcp}
                    </button>
                    <button
                      type="button"
                      className="kv-btn sm"
                      onClick={() => setActiveTab('skill')}
                      data-tauri-drag-region="false"
                    >
                      <SkillIcon size={11} />
                      {t.chatOpenSkill}
                    </button>
                    <button
                      type="button"
                      className="kv-btn sm"
                      onClick={() => setActiveTab('memory')}
                      data-tauri-drag-region="false"
                    >
                      <MemoryIcon size={11} />
                      {t.tabMemory}
                    </button>
                    <button
                      type="button"
                      className="kv-btn sm"
                      onClick={() => setActiveTab('externalAgents')}
                      data-tauri-drag-region="false"
                    >
                      <AgentIcon size={11} />
                      {t.chatOpenExternalAgents}
                    </button>
                    <button
                      type="button"
                      className="kv-btn sm"
                      onClick={() => setActiveTab('providers')}
                      data-tauri-drag-region="false"
                    >
                      <ProvidersIcon size={11} />
                      {t.chatOpenProviders}
                    </button>
                  </div>
                  <SettingRow
                    label={lang === 'zh' ? 'MCP 工具' : 'MCP tools'}
                    description={lang === 'zh' ? `已配置 ${chatTools.servers.length} 个服务器` : `${chatTools.servers.length} server(s) configured`}
                  >
                    <span className={`kv-tag ${chatTools.enabled ? 'ok' : ''}`}>
                      {chatTools.enabled
                        ? (lang === 'zh' ? '已启用' : 'On')
                        : (lang === 'zh' ? '未启用' : 'Off')}
                    </span>
                  </SettingRow>
                  <SettingRow
                    label={lang === 'zh' ? 'Skill 运行时' : 'Skill runtime'}
                    description={lang === 'zh' ? '内置 skill_activate / read_file / run_script' : 'Built-in skill_activate / read_file / run_script'}
                  >
                    <span className={`kv-tag ${skillRuntimeEnabled ? 'ok' : ''}`}>
                      {skillRuntimeEnabled
                        ? (lang === 'zh' ? '已启用' : 'On')
                        : (lang === 'zh' ? '未启用' : 'Off')}
                    </span>
                  </SettingRow>
                  <SettingRow
                    label={lang === 'zh' ? '内置工具' : 'Native tools'}
                    description={lang === 'zh' ? '读写文件、命令、Python、网页抓取等 Chat 工具' : 'Chat tools such as files, commands, Python, and web fetch'}
                  >
                    <span className={`kv-tag ${nativeBuiltinToolsEnabled ? 'ok' : ''}`}>
                      {nativeBuiltinToolsEnabled
                        ? (lang === 'zh' ? '已启用' : 'On')
                        : (lang === 'zh' ? '未启用' : 'Off')}
                    </span>
                  </SettingRow>
                  <SettingRow
                    label={t.tabMemory}
                    description={lang === 'zh' ? 'L1 常驻注入，L2 通过 memory_read 按需读取' : 'L1 is injected; L2 is read on demand with memory_read'}
                  >
                    <span className={`kv-tag ${chatMemory.enabled ? 'ok' : ''}`}>
                      {chatMemory.enabled
                        ? (lang === 'zh' ? '已启用' : 'On')
                        : (lang === 'zh' ? '未启用' : 'Off')}
                    </span>
                  </SettingRow>
                  <SettingRow
                    label={lang === 'zh' ? '联网搜索' : 'Web search'}
                    description={lang === 'zh' ? 'Tavily/Exa 与 Lens、Chat 开关' : 'Tavily/Exa plus Lens and Chat toggles'}
                  >
                    <span className={`kv-tag ${(settings.lens?.webSearch?.enabled || chatTools.nativeTools?.webSearch) ? 'ok' : ''}`}>
                      {(settings.lens?.webSearch?.enabled || chatTools.nativeTools?.webSearch)
                        ? (lang === 'zh' ? '部分启用' : 'Partially on')
                        : (lang === 'zh' ? '未启用' : 'Off')}
                    </span>
                  </SettingRow>
                  <p className="kv-row-desc">{t.chatRetryNote}</p>
                </SettingsGroup>
              </>
            )}

            {/* ===== 记忆标签页 ===== */}
            {activeTab === 'memory' && (
              <>
                <SettingsGroup title={lang === 'zh' ? '记忆运行' : 'Memory runtime'}>
                  <SettingRow
                    label={lang === 'zh' ? '启用记忆' : 'Enable memory'}
                    description={lang === 'zh'
                      ? '开启后每次 Chat 请求自动注入 L1，并暴露 memory_read / memory_search / memory_modify。'
                      : 'When enabled, every Chat request injects L1 and exposes memory_read / memory_search / memory_modify.'}
                  >
                    <Toggle
                      checked={chatMemory.enabled}
                      onChange={(enabled) => updateChatMemory({ enabled })}
                    />
                  </SettingRow>
                  <SettingRow label={lang === 'zh' ? '记忆文件夹' : 'Memory folder'} stack>
                    <div className="flex min-w-0 flex-wrap items-center gap-2">
                      <button
                        type="button"
                        className="kv-btn sm"
                        onClick={() => void refreshChatMemory()}
                        disabled={memoryLoading}
                        data-tauri-drag-region="false"
                      >
                        <RefreshCw size={10} className={memoryLoading ? 'animate-spin' : ''} />
                        {lang === 'zh' ? '刷新' : 'Refresh'}
                      </button>
                      <button
                        type="button"
                        className="kv-btn sm"
                        onClick={() => void handleOpenMemoryFolder()}
                        data-tauri-drag-region="false"
                      >
                        <FolderOpen size={11} />
                        {lang === 'zh' ? '打开文件夹' : 'Open folder'}
                      </button>
                      {memoryDir && <span className="kv-row-desc min-w-0 break-all">{memoryDir}</span>}
                    </div>
                  </SettingRow>
                  {memoryError && <div className="kv-inline-error">{memoryError}</div>}
                  {memorySuccess && (
                    <div className="kv-panel info">
                      <div className="kv-panel-body">{memorySuccess}</div>
                    </div>
                  )}
                </SettingsGroup>

                <SettingsGroup title="L1">
                  <MemoryEditor
                    layer="l1"
                    title={lang === 'zh' ? 'L1 在线记忆' : 'L1 Online Memory'}
                    description={lang === 'zh'
                      ? '短小、高频、会影响每次回答的偏好、约束和当前目标。'
                      : 'Short active preferences, constraints, and current goals that should affect every reply.'}
                    value={memoryDrafts.l1}
                    savedValue={memorySnapshots.l1}
                    maxBytes={MEMORY_L1_MAX_BYTES}
                    rows={9}
                    loading={memoryLoading}
                    saving={memorySavingLayer === 'l1'}
                    lang={lang}
                    onChange={(value) => {
                      setMemoryDrafts((prev) => ({ ...prev, l1: value }))
                      setMemorySuccess('')
                    }}
                    onSave={() => void handleSaveMemoryLayer('l1')}
                    onReload={() => void refreshChatMemory()}
                  />
                </SettingsGroup>

                <SettingsGroup title="L2">
                  <MemoryEditor
                    layer="l2"
                    title={lang === 'zh' ? 'L2 长期记忆' : 'L2 Long-Term Memory'}
                    description={lang === 'zh'
                      ? '长期流程、决策、排障记录和可复用知识；不会自动进入上下文。'
                      : 'Long-term workflows, decisions, troubleshooting notes, and reusable knowledge; never auto-loaded.'}
                    value={memoryDrafts.l2}
                    savedValue={memorySnapshots.l2}
                    rows={13}
                    loading={memoryLoading}
                    saving={memorySavingLayer === 'l2'}
                    lang={lang}
                    onChange={(value) => {
                      setMemoryDrafts((prev) => ({ ...prev, l2: value }))
                      setMemorySuccess('')
                    }}
                    onSave={() => void handleSaveMemoryLayer('l2')}
                    onReload={() => void refreshChatMemory()}
                  />
                </SettingsGroup>
              </>
            )}

            {/* ===== 混音器标签页 ===== */}
            {activeTab === 'mixer' && (
              <>
                <SettingsGroup title={t.mixerSection}>
                  <div className="mb-3 flex items-start justify-between gap-3">
                    <p className="kv-row-desc max-w-[560px]">{t.mixerSectionHint}</p>
                    <button
                      type="button"
                      className="kv-btn sm shrink-0"
                      onClick={() => {
                        updateDefaultModel('vision', '', '')
                        updateDefaultModel('titleSummary', '', '')
                        updateDefaultModel('compression', '', '')
                        updateDefaultModel('imageGeneration', '', '')
                      }}
                      data-tauri-drag-region="false"
                    >
                      {t.mixerResetAuto}
                    </button>
                  </div>
                  <SettingRow
                    label={t.auxiliaryVisionModel}
                    description={t.auxiliaryVisionModelHint}
                  >
                    <ModelPairSelect
                      providerId={settings.defaultModels.vision.providerId || ''}
                      model={settings.defaultModels.vision.model || ''}
                      providers={settings.providers}
                      inheritLabel={t.mixerAutoVisionModel}
                      onChange={(providerId, model) => {
                        updateDefaultModel('vision', providerId, model)
                      }}
                    />
                  </SettingRow>
                  <SettingRow
                    label={t.defaultTitleSummaryModel}
                    description={t.defaultTitleSummaryModelHint}
                  >
                    <ModelPairSelect
                      providerId={settings.defaultModels.titleSummary.providerId || ''}
                      model={settings.defaultModels.titleSummary.model || ''}
                      providers={settings.providers}
                      inheritLabel={t.mixerAutoModel}
                      onChange={(providerId, model) => {
                        updateDefaultModel('titleSummary', providerId, model)
                      }}
                    />
                  </SettingRow>
                  <SettingRow
                    label={t.defaultCompressionModel}
                    description={t.defaultCompressionModelHint}
                  >
                    <ModelPairSelect
                      providerId={settings.defaultModels.compression.providerId || ''}
                      model={settings.defaultModels.compression.model || ''}
                      providers={settings.providers}
                      inheritLabel={t.mixerAutoModel}
                      onChange={(providerId, model) => {
                        updateDefaultModel('compression', providerId, model)
                      }}
                    />
                  </SettingRow>
                  <SettingRow
                    label={t.defaultImageGenerationModel}
                    description={t.defaultImageGenerationModelHint}
                  >
                    <ModelPairSelect
                      providerId={settings.defaultModels.imageGeneration.providerId || ''}
                      model={settings.defaultModels.imageGeneration.model || ''}
                      providers={settings.providers}
                      inheritLabel={t.mixerNoImageGenerationModel}
                      onChange={(providerId, model) => {
                        updateDefaultModel('imageGeneration', providerId, model)
                      }}
                    />
                  </SettingRow>
                  {!chatProvider && (
                    <p className="kv-row-desc px-0 pb-2">
                      {lang === 'zh' ? '请先在「模型」中添加并配置供应商。' : 'Add and configure a provider under Models first.'}
                    </p>
                  )}
                </SettingsGroup>
              </>
            )}

            {/* ===== Kivio Code 标签页 ===== */}
            {activeTab === 'kivioCode' && (
              <KivioCodeSettings lang={lang} providers={settings.providers} />
            )}

            {activeTab === 'externalAgents' && (
              <ExternalAgentsSettings
                lang={lang}
                chatConfig={chatConfig}
                onChatChange={updateChat}
                onNavigateTab={setActiveTab}
              />
            )}

            {/* ===== MCP 标签页 ===== */}
            {activeTab === 'mcp' && (
              <>                <SettingsGroup title={lang === 'zh' ? 'Kivio 内置工具' : 'Kivio built-in tools'}>
                  <p className="kv-row-desc mb-2">
                    {lang === 'zh'
                      ? 'Chat 原生工具。启用后，本会话首次使用文件 / 命令工具时会请求一次授权；授权后 Kivio 可读写磁盘任意路径并执行终端命令（不再限制在主目录内，也不再逐次确认）。授权仅本次会话有效，重启后重新请求。'
                      : 'Native Chat tools. When first used in a conversation, file/command tools ask for one-time consent; once granted, Kivio can read/write anywhere on disk and run shell commands (no home-directory limit, no per-call prompts). Consent lasts for that conversation only and is re-requested after restart.'}
                  </p>
                  <SettingRow label={lang === 'zh' ? '读取文件' : 'Read file'} description={lang === 'zh' ? 'read' : 'read'}>
                    <Toggle
                      checked={chatTools.nativeTools?.readFile === true}
                      onChange={(readFile) => updateNativeTools({ readFile })}
                    />
                  </SettingRow>
                  <SettingRow label={lang === 'zh' ? '写入文件' : 'Write file'} description={lang === 'zh' ? 'write' : 'write'}>
                    <Toggle
                      checked={chatTools.nativeTools?.writeFile === true}
                      onChange={(writeFile) => updateNativeTools({ writeFile })}
                    />
                  </SettingRow>
                  <SettingRow label={lang === 'zh' ? '编辑文件' : 'Edit file'} description={lang === 'zh' ? 'edit' : 'edit'}>
                    <Toggle
                      checked={chatTools.nativeTools?.editFile === true}
                      onChange={(editFile) => updateNativeTools({ editFile })}
                    />
                  </SettingRow>
                  <SettingRow label={lang === 'zh' ? '终端命令' : 'Terminal command'} description={lang === 'zh' ? 'bash' : 'bash'}>
                    <Toggle
                      checked={chatTools.nativeTools?.runCommand === true}
                      onChange={(runCommand) => updateNativeTools({ runCommand })}
                    />
                  </SettingRow>
                  <SettingRow label={lang === 'zh' ? 'Python (Pyodide)' : 'Python (Pyodide)'} description={lang === 'zh' ? 'run_python 沙盒，首次加载较慢' : 'run_python sandbox; first load is slow'}>
                    <Toggle
                      checked={chatTools.nativeTools?.runPython === true}
                      onChange={(runPython) => updateNativeTools({ runPython })}
                    />
                  </SettingRow>
                  <SettingRow label={lang === 'zh' ? 'Skill 运行时' : 'Skill runtime'} description={lang === 'zh' ? 'skill_activate / read_file / run_script' : 'skill_activate / read_file / run_script'}>
                    <Toggle
                      checked={chatTools.nativeTools?.skillRuntime !== false}
                      onChange={(skillRuntime) => updateNativeTools({ skillRuntime })}
                    />
                  </SettingRow>
                  <SettingRow label={t.webSearchChatToggle} description={t.webSearchChatHint}>
                    <Toggle
                      checked={chatTools.nativeTools?.webSearch === true}
                      onChange={(webSearch) => {
                        if (!chatProviderSupportsTools) {
                          setSaveError(lang === 'zh' ? '当前 Chat 模型供应商不支持 tools，无法启用联网搜索。' : 'The current Chat provider does not support tools, so web search cannot be enabled.')
                          return
                        }
                        updateNativeTools({ webSearch })
                      }}
                    />
                  </SettingRow>
                  <SettingRow label={lang === 'zh' ? '网页抓取' : 'Web fetch'} description={lang === 'zh' ? 'web_fetch，HTTPS 只读' : 'web_fetch, HTTPS read-only'}>
                    <Toggle
                      checked={chatTools.nativeTools?.webFetch === true}
                      onChange={(webFetch) => updateNativeTools({ webFetch })}
                    />
                  </SettingRow>
                  <SettingRow label={t.webSearchApiSection} stack>
                    <div className="flex w-full flex-col gap-2">
                      <Select
                        className="w-full"
                        value={settings.lens?.webSearch?.provider || 'tavily'}
                        onChange={(provider) => updateLensWebSearch({ provider: provider as 'tavily' | 'exa' })}
                        options={[
                          { value: 'tavily', label: 'Tavily' },
                          { value: 'exa', label: 'Exa' },
                        ]}
                      />
                      <Input
                        type="password"
                        value={settings.lens?.webSearch?.provider === 'exa'
                          ? settings.lens?.webSearch?.exaApiKey || ''
                          : settings.lens?.webSearch?.tavilyApiKey || ''}
                        onChange={(value) => {
                          if (settings.lens?.webSearch?.provider === 'exa') {
                            updateLensWebSearch({ exaApiKey: value })
                          } else {
                            updateLensWebSearch({ tavilyApiKey: value })
                          }
                        }}
                        placeholder={settings.lens?.webSearch?.provider === 'exa' ? 'exa-...' : 'tvly-...'}
                      />
                      <div className="grid grid-cols-2 gap-2">
                        <FieldBlock label={lang === 'zh' ? '最大结果数' : 'Max results'}>
                          <Input
                            type="number"
                            min={1}
                            max={10}
                            value={String(settings.lens?.webSearch?.maxResults ?? 5)}
                            onChange={(value) => updateLensWebSearch({
                              maxResults: Math.min(10, Math.max(1, Number.parseInt(value, 10) || 5)),
                            })}
                          />
                        </FieldBlock>
                        {settings.lens?.webSearch?.provider !== 'exa' && (
                          <FieldBlock label={lang === 'zh' ? '搜索深度' : 'Search depth'}>
                            <Select
                              value={settings.lens?.webSearch?.searchDepth || 'basic'}
                              onChange={(searchDepth) => updateLensWebSearch({
                                searchDepth: searchDepth as 'basic' | 'advanced',
                              })}
                              options={[
                                { value: 'basic', label: 'basic' },
                                { value: 'advanced', label: 'advanced' },
                              ]}
                            />
                          </FieldBlock>
                        )}
                      </div>
                    </div>
                  </SettingRow>
                  <SettingRow label={lang === 'zh' ? '工作区根目录（可选）' : 'Workspace roots (optional)'} stack>
                    <div className="flex w-full flex-col gap-2">
                      {(chatTools.nativeTools?.workspaceRoots ?? []).map((path, index) => (
                        <div key={`${path}-${index}`} className="flex gap-2">
                          <Input
                            className="min-w-0 flex-1"
                            value={path}
                            onChange={(value) => {
                              const roots = [...(chatTools.nativeTools?.workspaceRoots ?? [])]
                              roots[index] = value
                              updateNativeTools({ workspaceRoots: roots })
                            }}
                          />
                          <button
                            type="button"
                            className="kv-btn sm shrink-0"
                            onClick={() => {
                              const roots = (chatTools.nativeTools?.workspaceRoots ?? []).filter((_, i) => i !== index)
                              updateNativeTools({ workspaceRoots: roots })
                            }}
                            data-tauri-drag-region="false"
                          >
                            <Minus size={11} />
                          </button>
                        </div>
                      ))}
                      <button
                        type="button"
                        className="kv-btn sm self-start"
                        onClick={async () => {
                          const selected = await open({ directory: true, multiple: false })
                          if (!selected || typeof selected !== 'string') return
                          const roots = [...(chatTools.nativeTools?.workspaceRoots ?? []), selected]
                          updateNativeTools({ workspaceRoots: roots })
                        }}
                        data-tauri-drag-region="false"
                      >
                        <FolderOpen size={11} />
                        {lang === 'zh' ? '添加工作区目录' : 'Add workspace folder'}
                      </button>
                    </div>
                  </SettingRow>
                </SettingsGroup>

                <SettingsGroup title={lang === 'zh' ? '工具运行' : 'Tool Runtime'}>
                  {/* 主开关：MCP 启用与否，单独成行并垂直居中 */}
                  <div className="flex items-center justify-between gap-4 py-3">
                    <div className="min-w-0">
                      <div className="kv-row-label">{lang === 'zh' ? '启用 MCP' : 'Enable MCP'}</div>
                      <p className="kv-row-desc">
                        {chatProviderSupportsTools
                          ? (lang === 'zh' ? '向支持 tools 的模型暴露已启用的 MCP 工具。' : 'Expose enabled MCP tools to models that support tools.')
                          : (lang === 'zh' ? '当前 Chat 模型供应商不支持 tools；Skill 仍会作为提示词生效。' : 'The current Chat provider does not support tools; Skills still work as prompt injection.')}
                      </p>
                    </div>
                    <Toggle
                      checked={chatTools.enabled}
                      onChange={(enabled) => {
                        if (!chatProviderSupportsTools) {
                          setSaveError(lang === 'zh' ? '当前 Chat 模型供应商不支持 tools，无法启用 MCP。' : 'The current Chat provider does not support tools, so MCP cannot be enabled.')
                          return
                        }
                        updateChatTools({ enabled })
                      }}
                    />
                  </div>

                  {/* 审批与运行参数：统一栅格，每格 label→描述→控件，控件用 mt-auto 底部对齐 */}
                  <div className="grid grid-cols-[repeat(auto-fit,minmax(190px,1fr))] items-stretch gap-x-4 gap-y-5 border-t border-[var(--divider)] py-3">
                    {/* 审批策略 */}
                    <div className="flex h-full flex-col">
                      <div className="mb-2">
                        <div className="kv-row-label">{lang === 'zh' ? '审批策略' : 'Approval policy'}</div>
                        <p className="kv-row-desc">{lang === 'zh' ? '文件/命令工具的授权方式；MCP 工具仍按其只读/敏感属性逐次判定。' : 'How file/command tools are authorized; MCP tools still follow their read-only/sensitive hints per call.'}</p>
                      </div>
                      <div className="mt-auto">
                        <Select
                          className="w-full"
                          value={chatTools.approvalPolicy || 'auto'}
                          onChange={(approvalPolicy) => updateChatTools({ approvalPolicy })}
                          options={[
                            {
                              value: 'readonly_auto_sensitive_confirm',
                              label: lang === 'zh' ? '会话授权一次（推荐）' : 'Session consent (once)',
                            },
                            { value: 'always_confirm', label: lang === 'zh' ? '授权后仍逐次确认' : 'Confirm every call' },
                            { value: 'auto', label: lang === 'zh' ? '全部自动（不弹授权）' : 'Auto (no prompt)' },
                          ]}
                        />
                      </div>
                    </div>

                    {/* 最大工具轮次 */}
                    <div className="flex h-full flex-col">
                      <div className="mb-2">
                        <div className="kv-row-label">{lang === 'zh' ? '最大工具轮次' : 'Max tool rounds'}</div>
                        <p className="kv-row-desc">
                          {lang === 'zh'
                            ? '达到上限后停止调用工具，并基于已有结果生成最终回复。'
                            : 'After the limit, Chat stops calling tools and synthesizes a final answer from existing tool results.'}
                        </p>
                      </div>
                      <div className="mt-auto">
                        <Select
                          className="w-full"
                          value={chatTools.maxToolRounds === null ? 'unlimited' : String(clampToolRounds(chatTools.maxToolRounds))}
                          onChange={(value) => updateChatTools({
                            maxToolRounds: value === 'unlimited' ? null : clampToolRounds(value),
                          })}
                          options={[
                            ...(chatTools.maxToolRounds !== null && !CHAT_TOOL_ROUND_PRESETS.includes(clampToolRounds(chatTools.maxToolRounds))
                              ? [{
                                  value: String(clampToolRounds(chatTools.maxToolRounds)),
                                  label: lang === 'zh'
                                    ? `当前 ${formatToolRoundsLabel(clampToolRounds(chatTools.maxToolRounds), lang)}`
                                    : `Current ${formatToolRoundsLabel(clampToolRounds(chatTools.maxToolRounds), lang)}`,
                                }]
                              : []),
                            ...CHAT_TOOL_ROUND_PRESETS.map((rounds) => ({
                              value: String(rounds),
                              label: formatToolRoundsLabel(rounds, lang),
                            })),
                            { value: 'unlimited', label: lang === 'zh' ? '无限制' : 'Unlimited' },
                          ]}
                        />
                      </div>
                    </div>

                    {/* 子 agent 并发 */}
                    <div className="flex h-full flex-col">
                      <div className="mb-2">
                        <div className="kv-row-label">{lang === 'zh' ? 'Subagent 并发' : 'Subagent concurrency'}</div>
                        <p className="kv-row-desc">
                          {lang === 'zh'
                            ? '同一时刻最多并行运行的 Subagent 数。调高更快但更吃 API 配额。'
                            : 'Max subagents running at once. Higher is faster but more API-quota intensive.'}
                        </p>
                      </div>
                      <div className="mt-auto">
                        <Select
                          className="w-full"
                          value={String(clampSubAgentConcurrency(chatTools.subAgentConcurrency))}
                          onChange={(value) => updateChatTools({ subAgentConcurrency: clampSubAgentConcurrency(value) })}
                          options={[
                            ...(!SUB_AGENT_CONCURRENCY_PRESETS.includes(clampSubAgentConcurrency(chatTools.subAgentConcurrency))
                              ? [{
                                  value: String(clampSubAgentConcurrency(chatTools.subAgentConcurrency)),
                                  label: lang === 'zh'
                                    ? `当前 ${clampSubAgentConcurrency(chatTools.subAgentConcurrency)}`
                                    : `Current ${clampSubAgentConcurrency(chatTools.subAgentConcurrency)}`,
                                }]
                              : []),
                            ...SUB_AGENT_CONCURRENCY_PRESETS.map((n) => ({ value: String(n), label: String(n) })),
                          ]}
                        />
                      </div>
                    </div>

                    {/* 工具超时 */}
                    <div className="flex h-full flex-col">
                      <div className="mb-2">
                        <div className="kv-row-label">{lang === 'zh' ? '工具超时' : 'Tool timeout'}</div>
                        <p className="kv-row-desc">{lang === 'zh' ? '单次工具调用的最长等待时间。' : 'Maximum wait time for a single tool call.'}</p>
                      </div>
                      <div className="mt-auto">
                        <Select
                          className="w-full"
                          value={String(clampToolTimeoutMs(chatTools.toolTimeoutMs))}
                          onChange={(value) => updateChatTools({ toolTimeoutMs: clampToolTimeoutMs(value) })}
                          options={[
                            ...(!CHAT_TOOL_TIMEOUT_PRESETS_MS.includes(clampToolTimeoutMs(chatTools.toolTimeoutMs))
                              ? [{
                                  value: String(clampToolTimeoutMs(chatTools.toolTimeoutMs)),
                                  label: lang === 'zh'
                                    ? `当前 ${formatToolTimeoutLabel(clampToolTimeoutMs(chatTools.toolTimeoutMs), lang)}`
                                    : `Current ${formatToolTimeoutLabel(clampToolTimeoutMs(chatTools.toolTimeoutMs), lang)}`,
                                }]
                              : []),
                            ...CHAT_TOOL_TIMEOUT_PRESETS_MS.map((ms) => ({
                              value: String(ms),
                              label: formatToolTimeoutLabel(ms, lang),
                            })),
                          ]}
                        />
                      </div>
                    </div>

                    {/* MCP 空闲超时 */}
                    <div className="flex h-full flex-col">
                      <div className="mb-2">
                        <div className="kv-row-label">{lang === 'zh' ? 'MCP 空闲超时' : 'MCP idle timeout'}</div>
                        <p className="kv-row-desc">
                          {lang === 'zh'
                            ? '持久连接空闲超过此值后回收子进程，下次调用透明重连。'
                            : 'Persistent MCP connections idle beyond this are recycled; the next call reconnects transparently.'}
                        </p>
                      </div>
                      <div className="mt-auto">
                        <Select
                          className="w-full"
                          value={String(clampMcpIdleTimeoutMs(chatTools.mcpIdleTimeoutMs))}
                          onChange={(value) => updateChatTools({ mcpIdleTimeoutMs: clampMcpIdleTimeoutMs(value) })}
                          options={[
                            ...(!MCP_IDLE_TIMEOUT_PRESETS_MS.includes(clampMcpIdleTimeoutMs(chatTools.mcpIdleTimeoutMs))
                              ? [{
                                  value: String(clampMcpIdleTimeoutMs(chatTools.mcpIdleTimeoutMs)),
                                  label: lang === 'zh'
                                    ? `当前 ${formatToolTimeoutLabel(clampMcpIdleTimeoutMs(chatTools.mcpIdleTimeoutMs), lang)}`
                                    : `Current ${formatToolTimeoutLabel(clampMcpIdleTimeoutMs(chatTools.mcpIdleTimeoutMs), lang)}`,
                                }]
                              : []),
                            ...MCP_IDLE_TIMEOUT_PRESETS_MS.map((ms) => ({
                              value: String(ms),
                              label: formatToolTimeoutLabel(ms, lang),
                            })),
                          ]}
                        />
                      </div>
                    </div>

                    {/* 结果截断字符 */}
                    <div className="flex h-full flex-col">
                      <div className="mb-2">
                        <div className="kv-row-label">{lang === 'zh' ? '结果截断字符' : 'Output chars'}</div>
                        <p className="kv-row-desc">{lang === 'zh' ? '工具结果当前不做截断。' : 'Tool results are not truncated for now.'}</p>
                      </div>
                      <div className="mt-auto">
                        <div className="flex h-[30px] items-center rounded-md border border-[var(--border)] bg-[var(--bg-input-subtle)] px-2.5 text-[12.5px] text-[var(--text-muted)]">
                          {lang === 'zh' ? '无限制输出' : 'Unlimited output'}
                        </div>
                      </div>
                    </div>
                  </div>
                </SettingsGroup>

                <SettingsGroup title={lang === 'zh' ? 'MCP 服务器' : 'MCP Servers'}>
                  <div className="flex flex-wrap gap-2 py-2">
                    <button
                      type="button"
                      className="kv-btn sm"
                      onClick={() => updateChatTools({ servers: [...chatTools.servers, newMcpServer()] })}
                      data-tauri-drag-region="false"
                    >
                      <Plus size={11} />
                      {lang === 'zh' ? '添加服务器' : 'Add server'}
                    </button>
                    <button
                      type="button"
                      className="kv-btn sm"
                      onClick={() => void handleImportMcpJson()}
                      data-tauri-drag-region="false"
                    >
                      <FolderOpen size={11} />
                      {lang === 'zh' ? '导入 mcp.json' : 'Import mcp.json'}
                    </button>
                  </div>

                  {chatTools.servers.length === 0 && (
                    <div className="kv-panel">
                      <div className="kv-panel-title">{lang === 'zh' ? '暂无 MCP 服务器' : 'No MCP servers'}</div>
                      <div className="kv-panel-body">
                        {lang === 'zh' ? '添加或导入服务器后，需要手动启用才会暴露给模型。' : 'Added or imported servers stay disabled until you enable them.'}
                      </div>
                    </div>
                  )}

                  <div className="space-y-3 py-2">
                    {chatTools.servers.filter((s) => !s.connectorId).map((server) => {
                      const feedback = mcpTestFeedback[server.id]
                      const knownTools = [
                        ...(feedback?.tools ?? []),
                        ...server.enabledTools
                          .filter((toolName) => !(feedback?.tools ?? []).some((tool) => tool.name === toolName))
                          .map((toolName) => ({
                            id: `${server.id}-${toolName}`,
                            name: toolName,
                            description: lang === 'zh' ? '已保存的工具限制；重新测试连接可刷新描述。' : 'Saved tool limit; test the server to refresh description.',
                            source: 'mcp',
                            serverId: server.id,
                            serverName: server.name,
                            inputSchema: {},
                            sensitive: false,
                          } satisfies ChatToolDefinition)),
                      ]
                      const isHttpTransport = server.transport === 'streamable_http'
                      const liveState = mcpServerStates[server.id]
                      const stateKind = liveState?.kind
                      const stateDotClass =
                        stateKind === 'connected'
                          ? 'on'
                          : stateKind === 'connecting'
                            ? 'warn'
                            : stateKind === 'error'
                              ? 'err'
                              : 'off'
                      const stateLabel =
                        stateKind === 'connected'
                          ? (lang === 'zh' ? '已连接' : 'Connected')
                          : stateKind === 'connecting'
                            ? (lang === 'zh' ? '连接中' : 'Connecting')
                            : stateKind === 'error'
                              ? (lang === 'zh' ? '错误' : 'Error')
                              : (lang === 'zh' ? '未连接' : 'Disconnected')
                      const stateError = liveState?.kind === 'error' ? liveState.message : ''
                      const stderrTail = mcpStderrTails[server.id] || ''
                      const stderrExpanded = expandedMcpStderrIds.includes(server.id)
                      return (
                        <div key={server.id} className="kv-panel">
                          <div className="mb-2 flex items-center gap-2">
                            <span className={`kv-provider-dot ${server.enabled ? 'on' : 'warn'}`} />
                            <Input
                              value={server.name}
                              onChange={(name) => updateMcpServer(server.id, { name })}
                              placeholder="Server name"
                            />
                            <Toggle
                              checked={server.enabled}
                              onChange={(enabled) => updateMcpServer(server.id, { enabled })}
                            />
                            <button
                              type="button"
                              className="kv-icon-btn danger"
                              onClick={() => updateChatTools({
                                servers: chatTools.servers.filter((item) => item.id !== server.id),
                              })}
                              title={lang === 'zh' ? '删除服务器' : 'Delete server'}
                              aria-label={lang === 'zh' ? '删除服务器' : 'Delete server'}
                              data-tauri-drag-region="false"
                            >
                              <Trash2 size={12} />
                            </button>
                          </div>
                          <FieldBlock label={lang === 'zh' ? '传输' : 'Transport'}>
                            <Select
                              value={isHttpTransport ? 'streamable_http' : 'stdio'}
                              onChange={(transport) => updateMcpServer(server.id, { transport })}
                              options={[
                                { value: 'stdio', label: 'stdio' },
                                { value: 'streamable_http', label: 'Streamable HTTP' },
                              ]}
                            />
                          </FieldBlock>
                          {isHttpTransport ? (
                            <>
                              <FieldBlock label={lang === 'zh' ? 'Endpoint URL' : 'Endpoint URL'}>
                                <Input
                                  mono
                                  value={server.url || ''}
                                  onChange={(url) => updateMcpServer(server.id, { url })}
                                  placeholder="https://example.com/mcp"
                                />
                              </FieldBlock>
                              <FieldBlock
                                label="Headers"
                                description={lang === 'zh' ? '每行 KEY=value；例如 Authorization=Bearer ...，会随 settings.json 明文保存。' : 'One KEY=value per line, e.g. Authorization=Bearer ...; stored in settings.json as plain text.'}
                              >
                                <TextArea
                                  mono
                                  rows={2}
                                  value={envToText(server.headers || {})}
                                  onChange={(value) => updateMcpServer(server.id, { headers: textToEnv(value) })}
                                  placeholder="Authorization=Bearer ..."
                                />
                              </FieldBlock>
                            </>
                          ) : (
                            <>
                              <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                                <FieldBlock label={lang === 'zh' ? '命令' : 'Command'}>
                                  <Input
                                    mono
                                    value={server.command}
                                    onChange={(command) => updateMcpServer(server.id, { command })}
                                    placeholder="npx"
                                  />
                                </FieldBlock>
                                <FieldBlock label={lang === 'zh' ? '参数' : 'Arguments'} description={lang === 'zh' ? '每行一个参数；保留参数中的空格和引号。' : 'One argument per line; spaces and quotes inside each argument are preserved.'}>
                                  <TextArea
                                    mono
                                    rows={2}
                                    value={argsToText(server.args)}
                                    onChange={(value) => updateMcpServer(server.id, {
                                      args: textToArgs(value),
                                    })}
                                    placeholder={'-y\n@modelcontextprotocol/server-fetch'}
                                  />
                                </FieldBlock>
                              </div>
                              <FieldBlock label={lang === 'zh' ? '工作目录' : 'Working directory'}>
                                <Input
                                  mono
                                  value={server.cwd || ''}
                                  onChange={(cwd) => updateMcpServer(server.id, { cwd: cwd.trim() ? cwd : null })}
                                  placeholder={lang === 'zh' ? '可选' : 'Optional'}
                                />
                              </FieldBlock>
                              <FieldBlock
                                label="Env"
                                description={lang === 'zh' ? '每行 KEY=value；这些值会随 settings.json 明文保存。' : 'One KEY=value per line; values are stored in settings.json as plain text.'}
                              >
                                <TextArea
                                  mono
                                  rows={2}
                                  value={envToText(server.env || {})}
                                  onChange={(value) => updateMcpServer(server.id, { env: textToEnv(value) })}
                                  placeholder="API_KEY=..."
                                />
                              </FieldBlock>
                            </>
                          )}
                          <div className="flex flex-wrap items-center gap-2 pt-1">
                            <button
                              type="button"
                              className="kv-btn sm"
                              disabled={testingMcpServerId === server.id || (isHttpTransport ? !server.url.trim() : !server.command.trim())}
                              onClick={() => void handleTestMcpServer(server)}
                              data-tauri-drag-region="false"
                            >
                              <RefreshCw size={10} className={testingMcpServerId === server.id ? 'animate-spin' : ''} />
                              {testingMcpServerId === server.id ? (lang === 'zh' ? '测试中' : 'Testing') : (lang === 'zh' ? '测试连接' : 'Test')}
                            </button>
                            {feedback && (
                              <span className={`kv-tag ${feedback.ok ? 'ok' : 'warn'}`}>
                                {feedback.message}
                              </span>
                            )}
                            {server.enabledTools.length === 0 && knownTools.length > 0 && (
                              <span className="kv-row-desc">{lang === 'zh' ? '当前暴露全部工具。' : 'All tools are exposed.'}</span>
                            )}
                          </div>
                          {/* 持久连接状态面板：状态点 / lastError / 折叠 stderr / 重连按钮 */}
                          <div className="mt-2 flex flex-wrap items-center gap-2">
                            <span className="inline-flex items-center gap-1.5 kv-row-desc">
                              <span className={`kv-provider-dot ${stateDotClass}`} />
                              {stateLabel}
                            </span>
                            <button
                              type="button"
                              className="kv-btn sm"
                              disabled={reloadingMcpServerId === server.id}
                              onClick={() => void handleReloadMcpServer(server)}
                              data-tauri-drag-region="false"
                            >
                              <RefreshCw size={10} className={reloadingMcpServerId === server.id ? 'animate-spin' : ''} />
                              {lang === 'zh' ? '重连' : 'Reconnect'}
                            </button>
                            {stderrTail.trim() && (
                              <button
                                type="button"
                                className="kv-btn sm ghost"
                                onClick={() => setExpandedMcpStderrIds((prev) => (
                                  prev.includes(server.id)
                                    ? prev.filter((id) => id !== server.id)
                                    : [...prev, server.id]
                                ))}
                                data-tauri-drag-region="false"
                              >
                                {stderrExpanded
                                  ? (lang === 'zh' ? '隐藏日志' : 'Hide log')
                                  : (lang === 'zh' ? '查看 stderr' : 'View stderr')}
                              </button>
                            )}
                          </div>
                          {stateError && (
                            <p className="mt-1 kv-row-desc break-words whitespace-pre-wrap" style={{ color: 'var(--danger)' }}>
                              {stateError}
                            </p>
                          )}
                          {stderrExpanded && stderrTail.trim() && (
                            <pre className="mt-1 max-h-40 overflow-auto rounded bg-black/5 dark:bg-white/5 p-2 text-[11px] leading-snug whitespace-pre-wrap break-words">
                              {stderrTail}
                            </pre>
                          )}
                          {knownTools.length > 0 && (
                            <div className="mt-2 flex flex-wrap gap-1.5">
                              {knownTools.map((tool) => {
                                const checked = server.enabledTools.length === 0 || server.enabledTools.includes(tool.name)
                                return (
                                  <button
                                    key={tool.id}
                                    type="button"
                                    className={`kv-chip ${checked ? '' : 'opacity-45'}`}
                                    title={tool.description}
                                    onClick={() => {
                                      if (checked) {
                                        const next = server.enabledTools.length === 0
                                          ? knownTools.map((item) => item.name).filter((name) => name !== tool.name)
                                          : server.enabledTools.filter((name) => name !== tool.name)
                                        updateMcpServer(server.id, { enabledTools: next })
                                      } else {
                                        updateMcpServer(server.id, {
                                          enabledTools: Array.from(new Set([...server.enabledTools, tool.name])),
                                        })
                                      }
                                    }}
                                    data-tauri-drag-region="false"
                                  >
                                    {tool.sensitive && <Wrench size={10} />}
                                    {tool.name}
                                  </button>
                                )
                              })}
                            </div>
                          )}
                        </div>
                      )
                    })}
                  </div>
                </SettingsGroup>
              </>
            )}

            {/* ===== 连接器标签页 ===== */}
            {activeTab === 'connectors' && (
              <ConnectorsPanel
                servers={chatTools.servers}
                updateChatTools={updateChatTools}
                lang={lang}
                testServer={async (server) => {
                  try {
                    const result = await api.chatMcpTestServer(server, settings?.chatTools?.toolTimeoutMs)
                    return {
                      ok: result.success,
                      message: result.error || '',
                      tools: result.tools,
                    }
                  } catch {
                    return null
                  }
                }}
              />
            )}

            {/* ===== 知识库标签页 ===== */}
            {activeTab === 'knowledge' && (
              <KnowledgeBasePanel providers={settings?.providers ?? []} lang={lang} />
            )}

            {/* ===== Skill 标签页 ===== */}
            {activeTab === 'skill' && (
              <>
                <SettingsGroup title="Skill">
                  <div className="flex flex-wrap gap-2 py-2">
                    <button
                      type="button"
                      className="kv-btn sm"
                      onClick={() => void refreshChatSkills()}
                      disabled={skillsLoading}
                      data-tauri-drag-region="false"
                    >
                      <RefreshCw size={10} className={skillsLoading ? 'animate-spin' : ''} />
                      {lang === 'zh' ? '刷新列表' : 'Refresh'}
                    </button>
                    <button
                      type="button"
                      className="kv-btn sm"
                      onClick={() => void handleImportSkill()}
                      data-tauri-drag-region="false"
                    >
                      <FolderOpen size={11} />
                      {lang === 'zh' ? '导入文件夹' : 'Import folder'}
                    </button>
                    <button
                      type="button"
                      className="kv-btn sm"
                      onClick={() => void handleImportSkillZip()}
                      data-tauri-drag-region="false"
                    >
                      <Download size={11} />
                      {lang === 'zh' ? '导入 zip' : 'Import zip'}
                    </button>
                    <button
                      type="button"
                      className="kv-btn sm"
                      onClick={() => void handleOpenSkillFolder()}
                      data-tauri-drag-region="false"
                    >
                      <ExternalLink size={11} />
                      {lang === 'zh' ? '打开 Skill 文件夹' : 'Open skill folder'}
                    </button>
                  </div>
                  <SettingRow label={lang === 'zh' ? '额外扫描路径' : 'Extra scan paths'} stack>
                    <div className="space-y-1.5">
                      {chatTools.skillScanPaths.map((path, index) => (
                        <div key={`${path}-${index}`} className="flex items-center gap-1.5">
                          <Input
                            mono
                            value={path}
                            onChange={(value) => {
                              const next = [...chatTools.skillScanPaths]
                              next[index] = value
                              updateChatTools({ skillScanPaths: next })
                            }}
                            placeholder="/path/to/skills"
                          />
                          <button
                            type="button"
                            className="kv-icon-btn danger"
                            onClick={() => updateChatTools({
                              skillScanPaths: chatTools.skillScanPaths.filter((_, i) => i !== index),
                            })}
                            data-tauri-drag-region="false"
                            aria-label={lang === 'zh' ? '移除路径' : 'Remove path'}
                          >
                            <Trash2 size={12} />
                          </button>
                        </div>
                      ))}
                      <button
                        type="button"
                        className="kv-btn sm"
                        onClick={async () => {
                          const selected = await open({ directory: true, multiple: false })
                          if (typeof selected === 'string') {
                            updateChatTools({ skillScanPaths: [...chatTools.skillScanPaths, selected] })
                          }
                        }}
                        data-tauri-drag-region="false"
                      >
                        <Plus size={11} />
                        {lang === 'zh' ? '添加扫描路径' : 'Add scan path'}
                      </button>
                    </div>
                  </SettingRow>
                  <SettingRow
                    label={lang === 'zh' ? '自动匹配 Skill' : 'Auto-match skills'}
                    description={lang === 'zh' ? '允许模型根据 description 自动 activate skill' : 'Allow the model to activate skills from the catalog automatically'}
                  >
                    <Toggle
                      checked={chatTools.skillAutoMatch !== false}
                      onChange={(skillAutoMatch) => updateChatTools({ skillAutoMatch })}
                    />
                  </SettingRow>
                  <SettingRow label={lang === 'zh' ? '无 Tools 降级模式' : 'Fallback without tools'} stack>
                    <Select
                      value={chatTools.skillFallbackMode || 'progressive'}
                      onChange={(skillFallbackMode) => updateChatTools({ skillFallbackMode })}
                      options={[
                        { value: 'progressive', label: lang === 'zh' ? '渐进式（仅 catalog）' : 'Progressive (catalog only)' },
                        { value: 'skill_md_only', label: lang === 'zh' ? '仅 SKILL.md' : 'SKILL.md only' },
                        { value: 'legacy_full_body', label: lang === 'zh' ? '旧版全量注入' : 'Legacy full body' },
                      ]}
                    />
                  </SettingRow>
                  <SettingRow label={lang === 'zh' ? '脚本解释器白名单' : 'Script interpreter allowlist'} stack>
                    <Input
                      mono
                      value={(chatTools.skillScriptAllowlist || []).join(', ')}
                      onChange={(value) => updateChatTools({
                        skillScriptAllowlist: value.split(',').map((item) => item.trim()).filter(Boolean),
                      })}
                      placeholder="python3, bash, sh, node"
                    />
                  </SettingRow>
                  {skillError && <div className="kv-inline-error">{skillError}</div>}
                  <SettingRow label={t.enabled} description={t.skillCatalogEnableHint}>
                    <span className="kv-tag ok">
                      {enabledSkillCount}
                      {' / '}
                      {skills.length}
                    </span>
                  </SettingRow>
                  {skillsLoading && (
                    <div className="kv-panel chat-motion-fade-up">
                      <div className="kv-panel-body">{lang === 'zh' ? '正在加载 Skill...' : 'Loading skills...'}</div>
                    </div>
                  )}
                  {!skillsLoading && skills.length === 0 && (
                    <div className="kv-panel">
                      <div className="kv-panel-title">{lang === 'zh' ? '暂无 Skill' : 'No skills'}</div>
                      <div className="kv-panel-body">
                        {lang === 'zh' ? '暂无 Skill。可导入文件夹/zip，或打开 Skill 文件夹手动添加后刷新。' : 'No skills yet. Import a folder or zip, or add skills manually and refresh.'}
                      </div>
                    </div>
                  )}
                  {!skillsLoading && skills.length > 0 && (
                    <div className="space-y-3 py-2">
                      <SkillListSection
                        title={lang === 'zh' ? '内置 Skill' : 'Built-in skills'}
                        emptyText={lang === 'zh' ? '当前没有内置 Skill。' : 'No built-in skills.'}
                        skills={builtinSkills}
                        lang={lang}
                        expandedSkillIds={expandedSkillIds}
                        disabledSkillIds={disabledSkillIds}
                        onToggleExpanded={handleToggleSkillExpanded}
                        onToggleEnabled={handleToggleSkillEnabled}
                        onPreview={handlePreviewSkill}
                      />
                      <SkillListSection
                        title={lang === 'zh' ? '用户 Skill' : 'User skills'}
                        emptyText={lang === 'zh' ? '当前没有用户导入的 Skill。' : 'No imported user skills.'}
                        skills={userSkills}
                        lang={lang}
                        expandedSkillIds={expandedSkillIds}
                        disabledSkillIds={disabledSkillIds}
                        onToggleExpanded={handleToggleSkillExpanded}
                        onToggleEnabled={handleToggleSkillEnabled}
                        onPreview={handlePreviewSkill}
                      />
                    </div>
                  )}
                </SettingsGroup>
              </>
            )}

            {/* ===== 网络搜索标签页 ===== */}
            {activeTab === 'webSearch' && (
              <>
                <SettingsGroup title={t.webSearchApiSection}>
                  <SettingRow label={t.lensWebSearchProvider}>
                    <Select
                      className="w-44"
                      value={settings.lens?.webSearch?.provider || 'tavily'}
                      onChange={(v) => updateLensWebSearch({ provider: v as 'tavily' | 'exa' })}
                      options={[
                        { value: 'tavily', label: 'Tavily' },
                        { value: 'exa', label: 'Exa' },
                      ]}
                    />
                  </SettingRow>
                  <SettingRow label={t.lensWebSearchApiKey}>
                    <Input
                      type="password"
                      value={settings.lens?.webSearch?.provider === 'exa'
                        ? settings.lens?.webSearch?.exaApiKey || ''
                        : settings.lens?.webSearch?.tavilyApiKey || ''}
                      onChange={(value) => {
                        if (settings.lens?.webSearch?.provider === 'exa') {
                          updateLensWebSearch({ exaApiKey: value })
                        } else {
                          updateLensWebSearch({ tavilyApiKey: value })
                        }
                      }}
                      placeholder={settings.lens?.webSearch?.provider === 'exa' ? 'exa-...' : 'tvly-...'}
                      mono
                    />
                  </SettingRow>
                  <SettingRow label={t.lensWebSearchMaxResults}>
                    <Input
                      type="number"
                      min={1}
                      max={10}
                      className="w-24"
                      value={String(settings.lens?.webSearch?.maxResults ?? 5)}
                      onChange={(value) => updateLensWebSearch({
                        maxResults: Math.min(10, Math.max(1, Number.parseInt(value, 10) || 1)),
                      })}
                    />
                  </SettingRow>
                  {settings.lens?.webSearch?.provider !== 'exa' && (
                    <SettingRow label={t.lensWebSearchDepth}>
                      <Select
                        className="w-44"
                        value={settings.lens?.webSearch?.searchDepth || 'basic'}
                        onChange={(v) => updateLensWebSearch({ searchDepth: v as 'ultra-fast' | 'fast' | 'basic' | 'advanced' })}
                        options={[
                          { value: 'ultra-fast', label: 'Ultra fast' },
                          { value: 'fast', label: 'Fast' },
                          { value: 'basic', label: 'Basic' },
                          { value: 'advanced', label: 'Advanced' },
                        ]}
                      />
                    </SettingRow>
                  )}
                </SettingsGroup>

                <SettingsGroup title={t.webSearchLensSection}>
                  <SettingRow label={t.enabled} description={t.lensWebSearchHint}>
                    <Toggle
                      checked={settings.lens?.webSearch?.enabled === true}
                      onChange={(v) => updateLensWebSearch({ enabled: v })}
                    />
                  </SettingRow>
                </SettingsGroup>

                <p className="kv-row-desc px-1 py-2">
                  {lang === 'zh'
                    ? 'Chat 的 web_search / web_fetch 开关在「MCP」→「Kivio 内置工具」中配置。'
                    : 'Chat web_search / web_fetch toggles live under MCP → Kivio built-in tools.'}
                </p>
              </>
            )}

            {/* ===== 用量统计标签页 ===== */}
            {activeTab === 'usage' && (
              <UsageStatsPanel lang={lang} />
            )}

            {/* ===== 模型管理标签页 ===== */}
            {activeTab === 'providers' && (
              <div className="kv-providers">
                <div className="kv-provider-list">
                  <button
                    type="button"
                    onClick={addProvider}
                    className="kv-provider-add"
                    data-tauri-drag-region="false"
                  >
                    <Plus />
                    {t.addProvider}
                  </button>

                  <ProviderSortableList
                    providers={settings.providers}
                    selectedId={selectedProvider?.id}
                    lang={lang}
                    providerNameLabel={t.providerName}
                    onSelect={setSelectedProviderId}
                    onReorder={reorderProviders}
                  />

                  <div className="kv-provider-list-presets">
                    <div className="kv-provider-list-section-label">
                      {lang === 'zh' ? '快速预设' : 'Quick presets'}
                    </div>
                    <div className="kv-provider-list-preset-grid">
                      {PROVIDER_PRESETS.map(preset => (
                        <button
                          key={preset.name}
                          type="button"
                          onClick={() => addProviderFromPreset(preset)}
                          className="kv-provider-preset-btn"
                          data-tauri-drag-region="false"
                        >
                          <Plus size={11} strokeWidth={2.25} />
                          <span className="min-w-0 truncate">{preset.name}</span>
                        </button>
                      ))}
                    </div>
                  </div>
                </div>

                <div className="kv-provider-detail">
                  <SettingsGroup title={lang === 'zh' ? '供应商' : 'Provider'} className="!pt-0 kv-provider-section">
                    {selectedProvider ? (() => {
                      const provider = selectedProvider
                      const configured = provider.apiKeys.some((key) => key.trim())
                      return (
                        <div className="kv-provider-header">
                          <div className="kv-provider-header-toolbar">
                            <span className="kv-row-label">{lang === 'zh' ? '启用供应商' : 'Enable provider'}</span>
                            <Toggle
                              checked={isProviderEnabled(provider)}
                              onChange={(enabled) => updateProvider(provider.id, { enabled })}
                            />
                          </div>
                          <div className="kv-provider-header-toolbar">
                            <span className="kv-row-label">{t.providerName}</span>
                            <div className="kv-provider-header-actions">
                              <span className={`kv-tag ${!isProviderEnabled(provider) ? 'warn' : configured ? 'ok' : 'warn'}`}>
                                {!isProviderEnabled(provider)
                                  ? (lang === 'zh' ? '已禁用' : 'Disabled')
                                  : configured ? t.connectionOk : t.permissionMissing}
                              </span>
                              <button
                                type="button"
                                onClick={() => setConfirmDeleteProviderId(provider.id)}
                                className="kv-icon-btn danger"
                                data-tauri-drag-region="false"
                                title={t.deleteProvider}
                                aria-label={t.deleteProvider}
                              >
                                <Trash2 size={12} />
                              </button>
                            </div>
                          </div>
                          <Input
                            value={provider.name}
                            onChange={(v) => updateProvider(provider.id, { name: v })}
                            placeholder="Provider name"
                          />
                        </div>
                      )
                    })() : (
                      <p className="kv-provider-empty-hint">
                        {lang === 'zh' ? '在左侧选择供应商，或使用快速预设添加。' : 'Select a provider on the left, or add one from quick presets.'}
                      </p>
                    )}
                  </SettingsGroup>

                  {selectedProvider ? (() => {
                    const provider = selectedProvider
                    return (
                        <SettingsGroup title={lang === 'zh' ? '配置' : 'Configuration'}>
                          <FieldBlock label={t.baseUrl}>
                            <div className="kv-provider-endpoint-row">
                              <Input
                                className="min-w-0 flex-1"
                                value={provider.baseUrl}
                                onChange={(v) => updateProvider(provider.id, { baseUrl: v })}
                                placeholder="https://api.openai.com/v1"
                                mono
                              />
                              <Select
                                className="w-[11.5rem] shrink-0"
                                value={normalizeProviderApiFormat(provider.apiFormat)}
                                onChange={(apiFormat) => updateProvider(provider.id, { apiFormat })}
                                options={[
                                  { value: 'openai_chat', label: 'OpenAI Chat' },
                                  { value: 'openai_responses', label: 'OpenAI Responses' },
                                  { value: 'anthropic_messages', label: 'Anthropic' },
                                ]}
                              />
                            </div>
                          </FieldBlock>

                          <SettingRow
                            label={
                              <span className="flex flex-col gap-1">
                                <span className="flex items-center gap-1">
                                  <span>{lang === 'zh' ? '压缩请求体 (gzip)' : 'Compress request body (gzip)'}</span>
                                  <button
                                    type="button"
                                    aria-label={lang === 'zh' ? '显示说明' : 'Show details'}
                                    className="kv-icon-btn"
                                    onClick={() => setGzipInfoOpen((prev) => {
                                      const next = new Set(prev)
                                      if (next.has(provider.id)) next.delete(provider.id)
                                      else next.add(provider.id)
                                      return next
                                    })}
                                  >
                                    <Info size={12} />
                                  </button>
                                </span>
                                {gzipInfoOpen.has(provider.id) && (
                                  <span className="kv-row-desc block mt-1">
                                    {lang === 'zh'
                                      ? '个别供应商前置的 WAF 会扫描明文请求体，把工具/系统提示里的 shell 命令、文件路径等文本误判为攻击而返回 403。开启后请求体用 gzip 压缩发送（多数网关可正常解压）。若该供应商不接受 gzip 请求（如官方 DeepSeek）会返回 400，请保持关闭。'
                                      : 'Some providers sit behind a WAF that scans the plaintext request body and returns 403 for shell/path text inside tool or system-prompt content. Enable to gzip the request body (most gateways accept it). Keep off for providers that reject gzip requests (e.g. official DeepSeek), which would return 400.'}
                                  </span>
                                )}
                              </span>
                            }
                          >
                            <Toggle
                              checked={provider.compressRequestBody === true}
                              onChange={(v) => updateProvider(provider.id, { compressRequestBody: v })}
                            />
                          </SettingRow>

                          <FieldBlock label={t.apiKey} description={t.apiKeysHint}>
                            <div className="space-y-1.5">
                              {(provider.apiKeys.length > 0 ? provider.apiKeys : ['']).map((key, idx) => {
                                const total = Math.max(provider.apiKeys.length, 1)
                                const keyId = `${provider.id}-${idx}`
                                const revealed = revealedKeys.has(keyId)
                                return (
                                  <div key={`${provider.id}-${total}-${idx}`} className="flex items-center gap-1.5">
                                    <Input
                                      type={revealed ? 'text' : 'password'}
                                      value={key}
                                      mono
                                      onChange={(v) => {
                                        const base = provider.apiKeys.length > 0 ? [...provider.apiKeys] : ['']
                                        base[idx] = v
                                        updateProvider(provider.id, { apiKeys: base })
                                      }}
                                      placeholder={idx === 0 ? `sk-... (${t.apiKeyPrimary})` : `sk-... (${t.apiKeyBackup})`}
                                    />
                                    <button
                                      type="button"
                                      onClick={() => toggleKeyReveal(keyId)}
                                      className="kv-icon-btn"
                                      title={revealed ? (lang === 'zh' ? '隐藏密钥' : 'Hide key') : (lang === 'zh' ? '显示密钥' : 'Show key')}
                                      data-tauri-drag-region="false"
                                    >
                                      {revealed ? <EyeOff size={12} /> : <Eye size={12} />}
                                    </button>
                                    {total > 1 && (
                                      <button
                                        type="button"
                                        onClick={() => {
                                          const next = provider.apiKeys.filter((_, i) => i !== idx)
                                          updateProvider(provider.id, { apiKeys: next })
                                        }}
                                        className="kv-icon-btn danger"
                                        title={t.removeKey}
                                        data-tauri-drag-region="false"
                                      >
                                        <Trash2 size={12} />
                                      </button>
                                    )}
                                  </div>
                                )
                              })}
                            </div>
                            <button
                              type="button"
                              onClick={() => {
                                const base = provider.apiKeys.length > 0 ? provider.apiKeys : ['']
                                updateProvider(provider.id, { apiKeys: [...base, ''] })
                              }}
                              className="kv-btn sm mt-2"
                              data-tauri-drag-region="false"
                            >
                              <Plus size={11} />
                              {t.addKey}
                            </button>
                          </FieldBlock>

                          <div className="kv-row">
                            <div className="kv-row-text">
                              <span className="kv-row-label">{t.testConnection}</span>
                              {providerTestFeedback[provider.id]?.message && (
                                <p className="kv-row-desc">{providerTestFeedback[provider.id]?.message}</p>
                              )}
                            </div>
                            <div className="kv-row-control kv-row-control-cluster">
                              <button
                                type="button"
                                onClick={() => openModelPicker(provider.id)}
                                className="kv-btn sm"
                                data-tauri-drag-region="false"
                              >
                                <RefreshCw size={10} className={fetchingProviderId === provider.id ? 'animate-spin' : ''} />
                                {provider.availableModels.length > 0
                                  ? (lang === 'zh' ? '管理模型' : 'Models')
                                  : t.fetchModels}
                              </button>
                              <button
                                type="button"
                                onClick={() => handleTestConnection(provider.id)}
                                disabled={testingProviderId === provider.id}
                                className="kv-btn sm"
                                data-tauri-drag-region="false"
                              >
                                <RefreshCw size={10} className={testingProviderId === provider.id ? 'animate-spin' : ''} />
                                {testingProviderId === provider.id ? t.testingConnection : t.testConnection}
                              </button>
                            </div>
                          </div>

                          <FieldBlock
                            label={(
                              <span className="inline-flex items-center gap-2">
                                <span>{lang === 'zh' ? '模型' : 'Models'}</span>
                                <span className="kv-tag">{provider.enabledModels.length}</span>
                              </span>
                            )}
                            description={lang === 'zh' ? '这些模型会出现在各功能的模型选择器中。' : 'These models appear in feature model selectors.'}
                          >
                            <ul className="kv-enabled-model-list">
                              {provider.enabledModels.length === 0 && (
                                <li className="kv-enabled-model-empty">
                                  {lang === 'zh' ? '点击上方「获取模型列表」拉取并添加模型。' : 'Use "Fetch Models" above to load and add models.'}
                                </li>
                              )}
                              {provider.enabledModels.map(model => {
                                const modelInfo = resolveModelInfo(model, provider.modelOverrides)
                                const caps = modelInfo.capabilities
                                return (
                                  <li key={model} className="kv-enabled-model-row" onClick={() => setDrawerModel({ providerId: provider.id, model })}>
                                    <ModelIcon model={model} size={16} />
                                    <span className="kv-enabled-model-name" title={model}>{modelInfo.displayName || model}</span>
                                    <span className="kv-enabled-model-badges">
                                      {caps?.vision && <span className="kv-badge-mini">V</span>}
                                      {caps?.functionCalling && <span className="kv-badge-mini">T</span>}
                                      {caps?.reasoning && <span className="kv-badge-mini">R</span>}
                                      {caps?.imageGeneration && <span className="kv-badge-mini">G</span>}
                                    </span>
                                    <button
                                      type="button"
                                      onClick={(e) => { e.stopPropagation(); removeEnabledModel(provider.id, model) }}
                                      className="kv-enabled-model-remove"
                                      data-tauri-drag-region="false"
                                      aria-label={t.removeModel}
                                    >
                                      <Minus size={14} />
                                    </button>
                                  </li>
                                )
                              })}
                            </ul>
                          </FieldBlock>
                        </SettingsGroup>
                    )
                  })() : null}
                </div>
              </div>
            )}

            {/* ===== 关于标签页 ===== */}
            {activeTab === 'about' && (
              <>
                <SettingsGroup title={lang === 'zh' ? '应用' : 'Application'}>
                  <div className="kv-panel mb-2">
                    <div className="flex items-center gap-3">
                      <div className="w-12 h-12 rounded-[10px] overflow-hidden shrink-0">
                        <img src="/icon.png" alt="Kivio" className="w-full h-full object-contain" />
                      </div>
                      <div className="min-w-0">
                        <div className="kv-page-title">Kivio</div>
                        <div className="kv-panel-body">{lang === 'zh' ? '屏幕级 AI 助手' : 'Screen-level AI Assistant'}</div>
                      </div>
                    </div>
                  </div>
                  <SettingRow label={t.currentVersion}>
                    <span className="kv-tag">v{appVersion}</span>
                  </SettingRow>
                  <SettingRow label={lang === 'zh' ? '开发者' : 'Developer'}>
                    <span className="kv-row-desc">ZM</span>
                  </SettingRow>
                </SettingsGroup>

                <SettingsGroup title={t.checkUpdate}>
                  <SettingRow label={t.autoCheckUpdate} description={t.autoCheckUpdateHint}>
                    <Toggle
                      checked={settings?.autoCheckUpdate ?? true}
                      onChange={(v) => updateSettings({ autoCheckUpdate: v })}
                    />
                  </SettingRow>
                  <SettingRow
                    label={t.checkUpdate}
                    description={updateStatus === 'up-to-date' ? t.upToDate : undefined}
                  >
                    <button
                      type="button"
                      onClick={handleCheckUpdate}
                      disabled={updateStatus === 'checking'}
                      className="kv-btn sm"
                      data-tauri-drag-region="false"
                    >
                      <RefreshCw size={11} className={updateStatus === 'checking' ? 'animate-spin' : ''} />
                      {updateStatus === 'checking' ? t.checkingUpdate : t.checkUpdate}
                    </button>
                  </SettingRow>

                  {updateStatus === 'available' && updateInfo && (
                    <div className="kv-panel info mt-2">
                      <div className="kv-panel-title">
                        {t.updateAvailable}
                        <span className="kv-tag accent ml-auto">v{updateInfo.version}</span>
                      </div>
                      {updateInfo.body && (
                        <div className="prose prose-sm dark:prose-invert max-w-none text-[12px] leading-relaxed max-h-40 overflow-y-auto mb-3">
                          <ReactMarkdown>{updateInfo.body}</ReactMarkdown>
                        </div>
                      )}

                      {downloadState === 'downloading' && (
                        <div className="mb-3">
                          <div className="flex items-center justify-between kv-panel-body mb-1">
                            <span>{t.downloading}</span>
                            <span className="font-mono tabular-nums">{downloadPercent}%</span>
                          </div>
                          <div className="kv-progress">
                            <div style={{ width: `${downloadPercent}%` }} />
                          </div>
                        </div>
                      )}

                      {downloadState === 'failed' && downloadError && (
                        <div className="kv-inline-error mb-3">
                          {t.downloadFailed}: {downloadError}
                        </div>
                      )}

                      <div className="flex gap-2 flex-wrap">
                        {downloadState === 'idle' && (
                          <>
                            <button
                              type="button"
                              onClick={handleDownloadAndInstall}
                              className="kv-btn primary"
                              data-tauri-drag-region="false"
                            >
                              <Download size={12} />
                              {t.downloadAndInstall}
                            </button>
                            <button
                              type="button"
                              onClick={handleOpenReleasePage}
                              className="kv-btn"
                              data-tauri-drag-region="false"
                            >
                              <ExternalLink size={12} />
                              {t.downloadFromGithub}
                            </button>
                          </>
                        )}
                        {downloadState === 'downloading' && (
                          <button type="button" disabled className="kv-btn">
                            <RefreshCw size={12} className="animate-spin" />
                            {t.downloading}
                          </button>
                        )}
                        {downloadState === 'downloaded' && (
                          <button
                            type="button"
                            onClick={handleInstall}
                            className="kv-btn primary"
                            data-tauri-drag-region="false"
                          >
                            <Download size={12} />
                            {t.installAndRestart}
                          </button>
                        )}
                        {downloadState === 'failed' && (
                          <>
                            <button
                              type="button"
                              onClick={handleDownloadAndInstall}
                              className="kv-btn primary"
                              data-tauri-drag-region="false"
                            >
                              <RefreshCw size={12} />
                              {t.retryDownload}
                            </button>
                            <button
                              type="button"
                              onClick={handleOpenReleasePage}
                              className="kv-btn"
                              data-tauri-drag-region="false"
                            >
                              <ExternalLink size={12} />
                              {t.downloadFromGithub}
                            </button>
                          </>
                        )}
                        <button
                          type="button"
                          onClick={() => {
                            setUpdateStatus('idle')
                            setDownloadState('idle')
                            setDownloadPercent(0)
                            setDownloadError('')
                          }}
                          className="kv-btn ghost"
                          data-tauri-drag-region="false"
                        >
                          {t.updateLater}
                        </button>
                      </div>
                    </div>
                  )}
                </SettingsGroup>
              </>
            )}
          </div>

          <div className={`kv-savebar ${variant === 'embedded' ? 'settings-embedded-savebar' : ''}`}>
            <div className={`kv-savebar-hint ${saveError ? 'error' : hasUnsavedChanges ? 'dirty' : ''}`}>
              {saveError ? (
                <>
                  <span className="dot" />
                  <span title={saveError}>{saveError}</span>
                </>
              ) : saveSuccess ? (
                <>
                  <span className="clean-icon"><Check size={13} strokeWidth={2.4} /></span>
                  <span>{t.saved}</span>
                </>
              ) : hasUnsavedChanges ? (
                <>
                  <span className="dot" />
                  <span>{lang === 'zh' ? '有未保存更改。' : 'You have unsaved changes.'}</span>
                </>
              ) : (
                <>
                  <span className="clean-icon"><Check size={13} strokeWidth={2.4} /></span>
                  <span>{lang === 'zh' ? '所有更改已保存。' : 'All changes saved.'}</span>
                </>
              )}
            </div>
            <button
              type="button"
              onClick={handleCloseRequest}
              className="kv-btn"
              data-tauri-drag-region="false"
            >
              {t.cancel}
            </button>
            <button
              type="button"
              onClick={handleSave}
              disabled={saving || !hasUnsavedChanges}
              className="kv-btn primary"
              data-tauri-drag-region="false"
            >
              {saving ? t.saving : t.save}
            </button>
          </div>
        </main>
  )

  const modelPickerProvider =
    modelPickerProviderId && settings
      ? settings.providers.find((p) => p.id === modelPickerProviderId)
      : undefined

  const settingsModals = (
    <>
      {modelPickerProvider && (
        <ProviderModelsPicker
          provider={modelPickerProvider}
          lang={lang}
          labels={{
            title: lang === 'zh' ? '模型' : 'Models',
            searchPlaceholder: lang === 'zh' ? '搜索模型 ID 或名称' : 'Search model ID or name',
            fetchModels: t.fetchModels,
            fetching: t.fetching,
            addModel: t.addModel,
            manualAddModel: t.manualAddModel,
            noModels: lang === 'zh' ? '尚未获取模型，请点击上方按钮拉取。' : 'No models yet. Click the button above to fetch.',
            noSearchResults: lang === 'zh' ? '没有匹配的模型' : 'No matching models',
            enabled: lang === 'zh' ? '已启用' : 'On',
            addAllModels: lang === 'zh' ? '添加当前列表中的全部模型' : 'Add all models in the current list',
            close: lang === 'zh' ? '关闭' : 'Close',
          }}
          fetching={fetchingProviderId === modelPickerProvider.id}
          onClose={() => setModelPickerProviderId(null)}
          onFetch={() => void fetchModels(modelPickerProvider.id)}
          onAdd={(model) => addEnabledModel(modelPickerProvider.id, model)}
          onAddAll={(models) => addAllEnabledModels(modelPickerProvider.id, models)}
          onRemove={(model) => removeEnabledModel(modelPickerProvider.id, model)}
        />
      )}
      {/* 模型详情抽屉 */}
      {drawerModel && settings && (
        <ModelDetailDrawer
          modelName={drawerModel.model}
          overrides={settings.providers.find(p => p.id === drawerModel.providerId)?.modelOverrides}
          lang={lang}
          onClose={() => setDrawerModel(null)}
          onSave={(modelName, info) => {
            saveModelOverride(drawerModel.providerId, modelName, info)
            setDrawerModel(null)
          }}
          onReset={(modelName) => resetModelOverride(drawerModel.providerId, modelName)}
        />
      )}
      {/* 未保存更改确认弹窗 */}
      {closeConfirmOpen && (
        <div className="kv-modal-backdrop" data-tauri-drag-region="false">
          <div className="kv-modal space-y-3">
            <h3 className="text-[14px] font-semibold">{t.unsavedChanges}</h3>
            <p className="kv-panel-body">{t.unsavedChangesDesc}</p>
            <div className="flex justify-end gap-2 pt-1">
              <button
                type="button"
                onClick={() => setCloseConfirmOpen(false)}
                className="kv-btn ghost"
              >
                {t.continueEditing}
              </button>
              <button
                type="button"
                onClick={handleDiscardAndClose}
                className="kv-btn"
              >
                {t.discardAndClose}
              </button>
              <button
                type="button"
                onClick={handleSaveAndClose}
                disabled={saving}
                className="kv-btn primary"
                autoFocus
              >
                {saving ? t.saving : t.saveAndClose}
              </button>
            </div>
          </div>
        </div>
      )}
      {/* 删除提供商确认弹窗 */}
      {confirmDeleteProviderId && (
        <div className="kv-modal-backdrop" data-tauri-drag-region="false">
          <div className="kv-modal space-y-3">
            <h3 className="text-[14px] font-semibold">{t.confirmDeleteProvider}</h3>
            <p className="kv-panel-body">{t.confirmDeleteProviderDesc}</p>
            <div className="flex justify-end gap-2 pt-1">
              <button
                type="button"
                onClick={() => setConfirmDeleteProviderId(null)}
                className="kv-btn"
                data-tauri-drag-region="false"
              >
                {t.cancel}
              </button>
              <button
                type="button"
                onClick={() => {
                  if (confirmDeleteProviderId) deleteProvider(confirmDeleteProviderId)
                  setConfirmDeleteProviderId(null)
                }}
                className="kv-btn danger"
                data-tauri-drag-region="false"
              >
                {t.deleteProvider}
              </button>
            </div>
          </div>
        </div>
      )}
      {selectedSkillPreview && (
        <div className="kv-modal-backdrop" data-tauri-drag-region="false">
          <div className="kv-modal max-h-[80vh] space-y-3 overflow-hidden">
            <div className="flex items-start gap-2">
              <Sparkles size={16} className="mt-0.5 shrink-0 text-[#C56646] dark:text-[#E39A78]" />
              <div className="min-w-0 flex-1">
                <h3 className="truncate text-[14px] font-semibold">{selectedSkillPreview.name}</h3>
                <p className="kv-panel-body">{selectedSkillPreview.description}</p>
              </div>
              <button
                type="button"
                className="kv-icon-btn"
                onClick={() => setSelectedSkillPreview(null)}
                data-tauri-drag-region="false"
                aria-label={lang === 'zh' ? '关闭' : 'Close'}
              >
                <X size={12} />
              </button>
            </div>
            {selectedSkillPreview.recommendedTools.length > 0 && (
              <div className="flex flex-wrap gap-1">
                {selectedSkillPreview.recommendedTools.map((tool) => (
                  <span key={tool} className="kv-chip">{tool}</span>
                ))}
              </div>
            )}
            <div className="custom-scrollbar max-h-[52vh] overflow-y-auto rounded-md border border-black/[0.08] bg-black/[0.025] p-3 text-[12px] leading-relaxed dark:border-white/[0.08] dark:bg-white/[0.035]">
              <ReactMarkdown>{selectedSkillPreview.body}</ReactMarkdown>
            </div>
          </div>
        </div>
      )}
    </>
  )

  const focusHandlers = {
    onPointerEnter: requestWindowFocus,
    onPointerMove: requestWindowFocus,
    onPointerDownCapture: requestWindowFocus,
  }

  if (variant === 'embedded') {
    return (
      <div
        className={`settings-embedded kv flex min-h-0 min-w-0 flex-1 ${
          reserveTrafficLightSpace ? 'settings-embedded--traffic-safe' : ''
        }`}
        data-theme-color={themeColor}
      >
        <aside className="settings-embedded-nav">
          <h2 className="settings-embedded-nav-title">{t.settings}</h2>
          {categoryNav}
        </aside>
        {settingsMain}
        {settingsModals}
      </div>
    )
  }

  return (
    <div className="kv kv-window" data-theme-color={themeColor} {...focusHandlers}>
      <div className="kv-titlebar" onMouseDown={handleSettingsDragMouseDown}>
        <div className="kv-titlebar-spacer" aria-hidden="true" />
        <div className="kv-title">{t.settings}</div>
        <button
          type="button"
          onClick={handleCloseRequest}
          className="kv-titlebar-close"
          data-tauri-drag-region="false"
          aria-label={t.cancel}
        >
          <X size={13} strokeWidth={2.2} />
        </button>
      </div>

      <div className="kv-body">
        <aside className="kv-sidebar">
          <div className="kv-sidebar-brand" onMouseDown={handleSettingsDragMouseDown}>
            <div className="kv-sidebar-brand-mark">
              <img src="/icon.png" alt="" aria-hidden="true" />
            </div>
            <div className="kv-sidebar-brand-name">Kivio</div>
            <div className="kv-sidebar-brand-ver">v{appVersion}</div>
          </div>
          {categoryNav}
        </aside>
        {settingsMain}
      </div>
      {settingsModals}
    </div>
  )
})
