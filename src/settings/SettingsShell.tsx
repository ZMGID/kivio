import { forwardRef, useImperativeHandle, useState, useEffect, useCallback, useMemo, useRef } from 'react'
import {
  X, Check, Plus, Minus, Trash2, RefreshCw,
  ExternalLink, Download, Upload, ChevronRight, Wrench, Sparkles, FolderOpen, Eye, EyeOff, Info,
} from 'lucide-react'
import { open, save } from '@tauri-apps/plugin-dialog'
import { ChatMarkdown } from '../chat/ChatMarkdown'
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
import { i18n, normalizeLang, promptLangKey } from './i18n'
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
import { RequestDebugPanel } from './RequestDebugPanel'
import { KivioCodeSettings } from './KivioCodeSettings'
import { ExternalAgentsSettings } from './ExternalAgentsSettings'
import { ModelDetailDrawer } from '../components/ModelDetailDrawer'
import { resolveModelInfo } from '../data/modelMatching'
import { useWindowInteractionFocus } from '../utils/windowFocus'
import { hasEnabledNativeBuiltinTool, hasEnabledSkillRuntime } from '../utils/chatTools'
import {
  THEME_COLOR_PRESETS,
  normalizeThemeColorId,
  type ThemeColorPreset,
} from '../themeColors'
import {
  Toggle, Select, Input, TextArea,
  SettingRow, PermissionItem, HotkeyInput,
  SettingsGroup,
} from './components'
import { ConnectorsPanel } from './ConnectorsPanel'
import { KnowledgeBasePanel } from './KnowledgeBasePanel'
import { WebSearchPanel } from './WebSearchPanel'

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
// MCP 持久連線空閒超時預設（ms）。後端鉗制範圍 60s..24h，預設 10 分鐘。
const MCP_IDLE_TIMEOUT_PRESETS_MS = [60_000, 300_000, 600_000, 1_800_000, 3_600_000]
const MCP_IDLE_TIMEOUT_MIN_MS = 60_000
const MCP_IDLE_TIMEOUT_MAX_MS = 24 * 60 * 60 * 1_000
// 子 agent 併發預設。後端鉗制範圍 1..64，預設 12。
const SUB_AGENT_CONCURRENCY_PRESETS = [3, 6, 12, 24, 48]
const SUB_AGENT_CONCURRENCY_MIN = 1
const SUB_AGENT_CONCURRENCY_MAX = 64
const textEncoder = new TextEncoder()

function resolveThemePreviewIsDark(theme: SettingsData['theme'] | undefined): boolean {
  return theme === 'dark'
    || (theme !== 'light' && window.matchMedia('(prefers-color-scheme: dark)').matches)
}

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
  return lang.startsWith('zh') ? `${rounds} 輪` : `${rounds} rounds`
}

function formatToolTimeoutLabel(ms: number, lang: string): string {
  if (ms % 60_000 === 0) {
    const minutes = ms / 60_000
    return lang.startsWith('zh') ? `${minutes} 分鐘` : `${minutes} min`
  }
  if (ms % 1000 === 0) {
    const seconds = ms / 1000
    return lang.startsWith('zh') ? `${seconds} 秒` : `${seconds} sec`
  }
  return `${ms} ms`
}

export interface SettingsShellProps {
  variant: 'standalone' | 'embedded'
  onClose: () => void
  onSettingsChange: () => void
  onReady?: () => void
  reserveTrafficLightSpace?: boolean
  /** 開啟設定面板時選中的側欄項（如 Chat 內嵌設定預設 AI 客戶端） */
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
            {lang.startsWith('zh') ? '過載' : 'Reload'}
          </button>
          <button
            type="button"
            className="kv-btn primary sm"
            onClick={onSave}
            disabled={loading || saving || !dirty || overLimit}
            data-tauri-drag-region="false"
          >
            {saving ? (lang.startsWith('zh') ? '儲存中' : 'Saving') : (lang.startsWith('zh') ? '儲存' : 'Save')}
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
          {lang.startsWith('zh')
            ? `${layer.toUpperCase()} 超出位元組上限，儲存前需要精簡。`
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
    requestDebugEnabled: false,
    nativeTools: defaultNativeTools(),
  }
}

function isBuiltinSkill(skill: SkillMeta): boolean {
  return skill.source === 'builtin'
}

function skillSourceLabel(skill: SkillMeta, lang: string): string {
  if (skill.source === 'builtin') {
    return lang.startsWith('zh') ? '內建' : 'Built-in'
  }
  if (skill.source === 'external') {
    return lang.startsWith('zh') ? '外部' : 'External'
  }
  return lang.startsWith('zh') ? '使用者' : 'User'
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
          {enabled ? (lang.startsWith('zh') ? '啟用' : 'On') : (lang.startsWith('zh') ? '關閉' : 'Off')}
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
                {fileCount} {lang.startsWith('zh') ? '個附屬檔案' : 'files'}
              </span>
            )}
            {skill.disableModelInvocation && (
              <span className="kv-chip">{lang.startsWith('zh') ? '僅手動觸發' : 'Manual only'}</span>
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
            {lang.startsWith('zh') ? '檢視完整內容' : 'View details'}
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
 * 設定面板主元件（standalone / embedded 雙宿主）
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
  // 用量統計頁內的二級檢視：用量統計 / 請求除錯（請求除錯原為獨立導航項，現併入用量統計）
  const [usageView, setUsageView] = useState<'stats' | 'debug'>('stats')
  useEffect(() => {
    if (initialTab) setActiveTab(initialTab)
  }, [initialTab])
  const [saveError, setSaveError] = useState('')
  const [saveSuccess, setSaveSuccess] = useState(false)
  const [closeConfirmOpen, setCloseConfirmOpen] = useState(false)
  const [confirmDeleteProviderId, setConfirmDeleteProviderId] = useState<string | null>(null)
  const [recordingTarget, setRecordingTarget] = useState<null | 'main' | 'screenshotTranslation' | 'screenshotTranslationText' | 'screenshotTranslationReplace' | 'lens'>(null)
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
  // 持久連線狀態：serverId → 最近一次 mcp-server-state 事件的狀態。
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
  // 更新檢查狀態：'idle' / 'checking' / 'up-to-date' / 'available'
  const [updateStatus, setUpdateStatus] = useState<'idle' | 'checking' | 'up-to-date' | 'available'>('idle')
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null)
  // 下載/安裝兩段式狀態機:idle → downloading(進度條) → downloaded(顯示安裝按鈕) → 使用者點選 → 應用退出
  // failed 時顯示錯誤 + 重試 + 跳 GitHub 兜底
  const [downloadState, setDownloadState] = useState<'idle' | 'downloading' | 'downloaded' | 'failed'>('idle')
  const [downloadPercent, setDownloadPercent] = useState(0)
  const requestWindowFocus = useWindowInteractionFocus()
  const [downloadedPath, setDownloadedPath] = useState('')
  const [downloadError, setDownloadError] = useState('')
  // RapidOCR 離線 OCR 狀態:檢查 app data 目錄裡 dylib + 模型 4 個檔案齊不齊。
  const [rapidOcrStatus, setRapidOcrStatus] = useState<import('../api/tauri').RapidOcrStatus | null>(null)
  // 下載臨時狀態:'idle' / 'downloading' / 'failed'(success 後自動 refresh status 到已就緒,
  // 沒有專門的 success 終態)
  const [rapidOcrDownloadState, setRapidOcrDownloadState] = useState<'idle' | 'downloading' | 'failed'>('idle')
  const [rapidOcrDownloadError, setRapidOcrDownloadError] = useState('')
  const platform = getPlatform()
  const isMac = platform === 'macos'
  const hasSystemOcr = isMac || platform === 'windows'
  // 載入失敗時的錯誤資訊；非空則渲染錯誤 UI 而不是用合成預設值進入正常檢視
  // （否則使用者可能沒察覺就 Save 把磁碟真實資料覆蓋掉）
  const [loadError, setLoadError] = useState('')
  const [reloadKey, setReloadKey] = useState(0)
  const saveSuccessTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const readyEmittedRef = useRef(false)
  const onSettingsChangeRef = useRef(onSettingsChange)

  const lang = normalizeLang(settings?.settingsLanguage)
  const t = i18n[lang]
  const themeColor = normalizeThemeColorId(settings?.themeColor)
  const hasLoadedSettings = settings !== null
  const previewTheme = settings?.theme
  const previewThemeColor = settings?.themeColor
  const previewThemeIsDark = hasLoadedSettings ? resolveThemePreviewIsDark(previewTheme) : false
  const chatTools = settings?.chatTools || defaultChatTools()
  const nativeBuiltinToolsEnabled = hasEnabledNativeBuiltinTool(chatTools.nativeTools)
  const skillRuntimeEnabled = hasEnabledSkillRuntime(chatTools.nativeTools)
  // 判斷是否有未儲存的更改
  const hasUnsavedChanges = settings ? stableStringify(settings) !== initialSettingsSnapshot : false

  useEffect(() => {
    onSettingsChangeRef.current = onSettingsChange
  }, [onSettingsChange])

  useEffect(() => {
    if (!hasLoadedSettings) return
    const nextThemeColor = normalizeThemeColorId(previewThemeColor)
    const isDark = resolveThemePreviewIsDark(previewTheme)
    document.documentElement.classList.toggle('dark', isDark)
    document.documentElement.dataset.themeColor = nextThemeColor
    void api.setChatWindowBackground(isDark)
  }, [hasLoadedSettings, previewTheme, previewThemeColor])

  useEffect(() => {
    return () => {
      onSettingsChangeRef.current()
    }
  }, [])

  // 客戶端熱鍵衝突偵測:在儲存前發現"兩個啟用功能用了同一個組合"。
  // OS 層面的衝突(Spotlight 佔用 Cmd+Space 等)仍需儲存後從後端拿到結果。
  // 返回每個 scope 對應的"和誰衝突"——前端各 HotkeyInput 拿到對應 scope 的夥伴名後,
  // 用 hotkeyScope* 範本自己拼本機化字串。
  type HotkeyScopeKey = 'main' | 'screenshotTranslation' | 'screenshotTranslationText' | 'screenshotTranslationReplace' | 'lens'
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
      {
        scope: 'screenshotTranslationReplace',
        hotkey: settings.screenshotTranslation?.replaceHotkey || '',
        enabled: settings.screenshotTranslation?.enabled !== false
          && settings.screenshotTranslation?.replaceEnabled !== false,
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

  const SCOPE_I18N_KEY: Record<HotkeyScopeKey, 'hotkeyScopeTranslator' | 'hotkeyScopeScreenshot' | 'hotkeyScopeScreenshotText' | 'hotkeyScopeScreenshotReplace' | 'hotkeyScopeLens'> = {
    main: 'hotkeyScopeTranslator',
    screenshotTranslation: 'hotkeyScopeScreenshot',
    screenshotTranslationText: 'hotkeyScopeScreenshotText',
    screenshotTranslationReplace: 'hotkeyScopeScreenshotReplace',
    lens: 'hotkeyScopeLens',
  }
  const conflictMessageFor = (scope: HotkeyScopeKey): string | undefined => {
    const partner = hotkeyConflicts[scope]
    if (!partner) return undefined
    return t.hotkeyConflictWith.replace('{partner}', t[SCOPE_I18N_KEY[partner]])
  }

  // 初始化：載入設定、版本號、預設提示詞
  // 重試通過遞增 reloadKey 觸發本 effect 重跑
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
        // 不合成預設值：避免使用者在錯誤狀態下 Save 把磁碟真實資料覆蓋掉
        // 渲染分支會根據 loadError 顯示重試 UI
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
    // resizeWindow 已在 App.tsx 中處理，此處不再重複呼叫
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
   * 重新整理許可權狀態（macOS）
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

  // 監聽後端啟動時的 update-available 事件，發現新版立即在 About 區塊展開提示
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

  // Settings 開啟時靜默 check 一次（覆蓋啟動事件使用者當時沒開 Settings 的場景）
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

  /** 使用者點 "檢查更新" 按鈕 */
  const handleCheckUpdate = useCallback(async () => {
    setUpdateStatus('checking')
    try {
      const info = await api.checkUpdate()
      if (info.available) {
        setUpdateInfo(info)
        setUpdateStatus('available')
      } else {
        setUpdateStatus('up-to-date')
        // 5s 後自動復位回 idle，避免"已是最新"標籤長期佔位
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

  /** 下載新版安裝包到 temp dir,期間監聽 update-download-progress 事件重新整理進度條 */
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

  /** 啟動 installer 並退出當前應用。Rust 端會在 macOS 上 cp 新 .app + open,在 Windows spawn NSIS exe */
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

  /** 拉一次 RapidOCR 狀態(app data 裡 dylib + 模型 4 個檔案齊不齊)。
   *  掛載時 + 切換到 RapidOCR 引擎時調一下。 */
  const refreshRapidOcrStatus = useCallback(async () => {
    if (!hasSystemOcr) return
    try {
      const status = await api.rapidOcrStatus()
      setRapidOcrStatus(status)
    } catch (err) {
      console.error('rapidOcrStatus failed:', err)
    }
  }, [hasSystemOcr])

  /** 下載 RapidOCR 包(dylib + 模型,~30-50MB):阻塞 ~15-30s,完成後 refresh status。 */
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

  // 掛載時拉一次狀態
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
   * 儲存設定
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
      const prefix = lang.startsWith('zh') ? '儲存失敗:' : 'Save failed: '
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
   * 請求關閉設定頁（檢查未儲存更改）
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

  // 放棄更改並關閉
  const handleDiscardAndClose = () => {
    setCloseConfirmOpen(false)
    onClose()
  }

  // 儲存並關閉
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

  // 全域鍵盤：Esc 關閉、Cmd/Ctrl+S 儲存；彈窗開啟時優先處理彈窗內的 Esc/Enter
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

      // 刪除供應商彈窗：Esc 取消；不繫結 Enter，避免誤觸發破壞性刪除
      if (confirmDeleteProviderId) {
        if (e.key === 'Escape') {
          e.preventDefault()
          e.stopPropagation()
          setConfirmDeleteProviderId(null)
        }
        return
      }

      // 未儲存確認彈窗：Esc = 繼續編輯（關彈窗）；Enter = 儲存並關閉
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
   * 測試供應商連線
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
   * 開啟 macOS 系統許可權設定
   */
  const handleOpenPermissionSettings = async (kind: 'accessibility' | 'screen-recording') => {
    try {
      await api.openPermissionSettings(kind)
    } catch (err) {
      console.error('Failed to open permission settings:', err)
    }
  }

  // 重試次數輸入處理
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
   * 更新設定欄位
   */
  const updateSettings = useCallback((updates: Partial<SettingsData>) => {
    setSettings((prev) => {
      if (!prev) return prev
      return { ...prev, ...updates }
    })
  }, [])

  // 設定備份：匯出/匯入 JSON。匯入會覆蓋全部設定並立即生效。
  const [backupStatus, setBackupStatus] = useState<{ kind: 'ok' | 'err'; msg: string } | null>(null)

  // 哪些 API Key 輸入框處於明文顯示（按 `${providerId}-${idx}` 記），預設全部隱藏。
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
      setBackupStatus({ kind: 'ok', msg: lang.startsWith('zh') ? '設定已匯出。' : 'Settings exported.' })
    } catch (err) {
      setBackupStatus({ kind: 'err', msg: `${lang.startsWith('zh') ? '匯出失敗：' : 'Export failed: '}${err}` })
    }
  }, [lang])

  const handleImportSettings = useCallback(async () => {
    try {
      const selected = await open({ multiple: false, filters: [{ name: 'JSON', extensions: ['json'] }] })
      if (!selected || typeof selected !== 'string') return
      const imported = await api.importSettings(selected)
      setSettings(imported)
      setBackupStatus({ kind: 'ok', msg: lang.startsWith('zh') ? '設定已匯入並生效。' : 'Settings imported and applied.' })
    } catch (err) {
      setBackupStatus({ kind: 'err', msg: `${lang.startsWith('zh') ? '匯入失敗：' : 'Import failed: '}${err}` })
    }
  }, [lang])

  const handleRestartOnboarding = useCallback(async () => {
    if (!settings) return
    try {
      const saved = await api.saveSettings({
        ...settings,
        onboardingStatus: 'pending',
      })
      setSettings(saved)
      setInitialSettingsSnapshot(stableStringify(saved))
      onSettingsChange()
      window.location.hash = '#chat/onboarding'
    } catch (err) {
      console.error('Failed to restart onboarding:', err)
    }
  }, [onSettingsChange, settings])

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
        setSkillError(result.error || (lang.startsWith('zh') ? 'Skill 列表載入失敗' : 'Failed to load skills'))
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
            message: lang.startsWith('zh') ? `連線成功，發現 ${result.tools.length} 個工具。` : `Connected. ${result.tools.length} tools found.`,
            tools: result.tools,
          },
        }))
      } else {
        setMcpTestFeedback((prev) => ({
          ...prev,
          [server.id]: {
            ok: false,
            message: result.error || (lang.startsWith('zh') ? '連線失敗' : 'Connection failed'),
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
      // 重連後立即拉一次狀態快照（Disconnected → 下次呼叫透明重連）。
      const status = await api.chatMcpServerStatus(server.id)
      setMcpServerStates((prev) => ({ ...prev, [server.id]: status.state }))
      setMcpStderrTails((prev) => ({ ...prev, [server.id]: status.stderrTail }))
    } catch (err) {
      setSaveError(err instanceof Error ? err.message : String(err))
    } finally {
      setReloadingMcpServerId(null)
    }
  }, [])

  // 訂閱持久連線狀態事件（連線/斷開/錯誤），即時更新狀態點。
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

  // 進入 MCP 標籤頁時拉一次各 server 的狀態快照（含 stderr 尾巴）。
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
        setSaveError(result.error || (lang.startsWith('zh') ? '匯入 mcp.json 失敗' : 'Failed to import mcp.json'))
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
        setSkillError(result.error || (lang.startsWith('zh') ? '匯入 Skill 失敗' : 'Failed to import skill'))
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
        setSkillError(result.error || (lang.startsWith('zh') ? '匯入 Skill 失敗' : 'Failed to import skill'))
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
        setSkillError(result.error || (lang.startsWith('zh') ? '開啟 Skill 資料夾失敗' : 'Failed to open skill folder'))
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
        setSkillError(result.error || (lang.startsWith('zh') ? '讀取 Skill 失敗' : 'Failed to read skill'))
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
   * 更新指定供應商配置
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
   * 新增新供應商
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

  /** 用預設一鍵新增 provider —— baseUrl 和預設模型已填好，使用者只需填 API key */
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
   * 根據 ID 查詢已啟用的供應商（找不到或已停用時返回第一個已啟用的）
   */
  const resolveProvider = (providers: ModelProvider[], providerId: string) => {
    const matched = providers.find(p => p.id === providerId)
    if (matched && isProviderEnabled(matched)) return matched
    return providers.find(p => isProviderEnabled(p)) ?? providers[0]
  }

  /**
   * 確保當前模型在已啟用模型列表中
   */
  const resolveModel = (provider: ModelProvider | undefined, currentModel: string) => {
    if (!provider) return currentModel
    if (provider.enabledModels.includes(currentModel)) return currentModel
    return provider.enabledModels[0] || currentModel
  }

  /**
   * 刪除供應商
   * 刪除後會自動將使用該供應商的功能切換到第一個可用供應商
   */
  const deleteProvider = (id: string) => {
    if (!settings) return
    if (modelPickerProviderId === id) setModelPickerProviderId(null)
    const nextProviders = settings.providers.filter(p => p.id !== id)
    const translatorProvider = resolveProvider(nextProviders, settings.translatorProviderId)
    const screenshotProvider = resolveProvider(nextProviders, settings.screenshotTranslation?.providerId || '')
    // lens providerId 為空表示 fallback 到 translator，刪除時若已設定自身 provider 才需要級聯
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
   * 新增已啟用模型
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
   * 移除已啟用模型
   * 移除後會自動更新使用該模型的功能到新的預設模型
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
   * 儲存模型自訂引數
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
   * 重置模型引數為資料庫預設值
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
   * 從供應商 API 取得可用模型列表
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
   * 更新截圖翻譯配置
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
      setMemoryError(lang.startsWith('zh')
        ? `L1 超過 ${MEMORY_L1_MAX_BYTES} 位元組，請先精簡或歸檔到 L2。`
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
      setMemorySuccess(lang.startsWith('zh')
        ? `${layer.toUpperCase()} 已儲存`
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
        setMemoryError(result.error || (lang.startsWith('zh') ? '開啟記憶資料夾失敗' : 'Failed to open memory folder'))
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
   * 切換快捷鍵錄製狀態
   */
  const toggleRecording = (target: 'main' | 'screenshotTranslation' | 'screenshotTranslationText' | 'screenshotTranslationReplace' | 'lens') => {
    setRecordingTarget((current) => (current === target ? null : target))
  }

  // 當前語言對應的預設 lens 提示詞
  const inheritedPromptLanguage = settings?.targetLang === 'zh-Hant'
    ? 'zh-TW'
    : settings?.targetLang === 'en'
      ? 'en'
      : settings?.settingsLanguage
  const lensPromptKey = promptLangKey(settings?.lens?.defaultLanguage || inheritedPromptLanguage)
  const lensDefaults = defaultPrompts?.lensPrompts?.[lensPromptKey] || defaultPrompts?.lensPrompts?.zh
  const chatLangKey = promptLangKey(settings?.chat?.defaultLanguage || settings?.lens?.defaultLanguage || inheritedPromptLanguage)
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
    ? (lang.startsWith('zh') ? '模型引數' : 'Model override')
    : effectiveChatMaxOutput.source === 'database'
      ? (lang.startsWith('zh') ? '內建模型庫' : 'Model database')
      : (lang.startsWith('zh') ? '兜底設定' : 'Fallback setting')
  const chatMaxOutputModelLabel = effectiveChatMaxOutput.model
    ? (effectiveChatMaxOutput.provider?.name
      ? `${effectiveChatMaxOutput.provider.name} / ${effectiveChatMaxOutput.model}`
      : effectiveChatMaxOutput.model)
    : (lang.startsWith('zh') ? '未配置聊天模型' : 'No chat model configured')

  // 快捷鍵錄製監聽
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
      } else if (recordingTarget === 'screenshotTranslationReplace') {
        updateScreenshotTranslation({ replaceHotkey: hotkey })
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
    // 載入失敗：顯示錯誤 + 重試按鈕，禁止使用者在不知情的情況下用合成預設值 Save 覆蓋磁碟
    return (
      <div className={`${loadingShellClass} p-6`}>
        <div className="max-w-sm w-full bg-white dark:bg-[#1C1C1E] rounded-xl shadow-sm border border-black/5 dark:border-white/5 p-5 text-center">
          <div className="text-[14px] font-semibold text-neutral-900 dark:text-neutral-100 mb-1">
            {lang.startsWith('zh') ? '載入設定失敗' : 'Failed to load settings'}
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
              {lang.startsWith('zh') ? '重試' : 'Retry'}
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
    { id: 'knowledge' as const, label: lang.startsWith('zh') ? '知識庫' : 'Knowledge', icon: KnowledgeIcon },
    { id: 'skill' as const, label: 'Skill', icon: SkillIcon },
    { id: 'webSearch' as const, label: t.tabWebSearch, icon: WebSearchIcon },
    { id: 'usage' as const, label: lang.startsWith('zh') ? '用量統計' : 'Usage', icon: UsageIcon },
    { id: 'providers' as const, label: t.tabModels, icon: ProvidersIcon },
  ]
  const pageMeta: Record<typeof activeTab, { title: string; subtitle: string; right?: string }> = {
    general: {
      title: t.tabGeneral,
      subtitle: lang.startsWith('zh') ? '外觀、行為、歸檔和許可權。' : 'Appearance, behavior, archive, and permissions.',
    },
    translate: {
      title: t.tabTranslate,
      subtitle: lang.startsWith('zh') ? '輸入翻譯的快捷鍵、語言、模型和提示詞。' : 'Shortcut, language, model, and prompt for input translation.',
    },
    screenshot: {
      title: t.tabScreenshot,
      subtitle: lang.startsWith('zh') ? '截圖選擇、OCR、輸出和翻譯模型。' : 'Capture selection, OCR, output, and translation model.',
    },
    lens: {
      title: t.lensTabLabel,
      subtitle: lang.startsWith('zh') ? '視覺問答的快捷鍵、回應方式和提示詞。' : 'Shortcut, response behavior, and prompts for visual Q&A.',
    },
    chat: {
      title: t.tabChatClient,
      subtitle: lang.startsWith('zh')
        ? '主對話模型、流式/思考行為、系統提示詞；副任務模型見混音器。'
        : 'Main chat model, streaming/thinking, and system prompt; side-task models live under Mixer.',
    },
    memory: {
      title: t.tabMemory,
      subtitle: lang.startsWith('zh')
        ? 'L1 線上記憶常駐注入；L2 長期記憶只通過工具讀取。'
        : 'L1 is always injected when enabled; L2 is read only through tools.',
    },
    mixer: {
      title: t.tabMixer,
      subtitle: lang.startsWith('zh')
        ? '按副任務路由模型：視覺、標題總結、上下文壓縮、生圖。'
        : 'Route models by side task: vision, title summaries, context compression, and image generation.',
    },
    kivioCode: {
      title: 'Kivio Code',
      subtitle: lang.startsWith('zh')
        ? '終端編碼代理的預設模型、工具審批策略與上下文讀取。'
        : 'Default model, tool approval policy, and context reading for the terminal coding agent.',
    },
    externalAgents: {
      title: t.tabExternalAgents,
      subtitle: lang.startsWith('zh')
        ? '偵測並啟用外部 CLI 編碼代理。'
        : 'Detect and enable external CLI coding agents.',
    },
    mcp: {
      title: 'MCP',
      subtitle: lang.startsWith('zh') ? '管理 MCP 伺服器與工具審批策略。' : 'Manage MCP servers and tool approval policy.',
    },
    connectors: {
      title: t.tabConnectors,
      subtitle: lang.startsWith('zh')
        ? '連線 Notion、GitHub 等外部資料來源；token 存在本機，資料預設直連。'
        : 'Connect external data sources like Notion and GitHub; tokens stay on your machine, data goes direct by default.',
    },
    skill: {
      title: 'Skill',
      subtitle: lang.startsWith('zh') ? '管理內建與使用者 Skill。' : 'Manage built-in and user Skills.',
    },
    knowledge: {
      title: lang.startsWith('zh') ? '知識庫' : 'Knowledge',
      subtitle: lang.startsWith('zh')
        ? '匯入檔案建立向量知識庫，對話中由 agent 檢索並標註出處。'
        : 'Import documents into vector libraries the agent retrieves and cites in chat.',
    },
    webSearch: {
      title: t.tabWebSearch,
      subtitle: lang.startsWith('zh')
        ? 'Tavily/Exa 金鑰與引數；分別開啟 Lens 與 Chat 的網路搜尋。'
        : 'Tavily/Exa keys and parameters; enable web search for Lens and Chat separately.',
    },
    usage: {
      title: lang.startsWith('zh') ? '用量統計' : 'Usage',
      subtitle: lang.startsWith('zh')
        ? '檢視本機模型請求、Token、成本估算和來源分佈。'
        : 'Inspect local model requests, tokens, estimated cost, and usage distribution.',
    },
    providers: {
      title: t.tabModels,
      subtitle: lang.startsWith('zh') ? '管理 OpenAI 相容供應商、金鑰和啟用模型。' : 'Manage OpenAI-compatible providers, keys, and enabled models.',
    },
    about: {
      title: lang.startsWith('zh') ? '關於' : 'About',
      subtitle: lang.startsWith('zh') ? '版本、更新和應用資訊。' : 'Version, updates, and application details.',
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
            <span>{lang.startsWith('zh') ? '關於' : 'About'}</span>
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
            <span>{lang.startsWith('zh') ? '關於' : 'About'}</span>
          </button>
        </nav>
      </>
    )

  const settingsMain = (
        <main className={`kv-content ${variant === 'embedded' ? 'settings-embedded-main' : ''}`}>
          <header
            className={`kv-page-header ${variant === 'embedded' ? 'settings-embedded-header' : ''}`}
            onMouseDown={handleSettingsDragMouseDown}
          >
            <div>
              <div className="kv-page-title">{pageMeta[activeTab].title}</div>
              <div className="kv-page-sub">{pageMeta[activeTab].subtitle}</div>
            </div>
            <div className="kv-page-header-right">{pageMeta[activeTab].right}</div>
          </header>

          <div className={`kv-scroll ${variant === 'embedded' ? 'settings-embedded-scroll' : ''}`}>
            {/* ===== 基礎設定標籤頁 ===== */}
            {activeTab === 'general' && (
              <>
                <SettingsGroup title={lang.startsWith('zh') ? '外觀' : 'Appearance'}>
                  <SettingRow label={t.language} description={lang.startsWith('zh') ? '設定 Kivio 介面語言。' : 'Used for the Kivio interface.'}>
                    <Select
                      className="w-36"
                      value={normalizeLang(settings.settingsLanguage)}
                      onChange={(v) => updateSettings({ settingsLanguage: v as 'zh' | 'zh-TW' | 'en' })}
                      options={[
                        { value: 'zh', label: '簡體中文' },
                        { value: 'zh-TW', label: '繁體中文' },
                        { value: 'en', label: 'English' },
                      ]}
                    />
                  </SettingRow>
                  <SettingRow label={t.theme} description={lang.startsWith('zh') ? '跟隨系統外觀，或固定淺色/深色。' : 'Match system appearance or pick a mode.'}>
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
                  <SettingRow
                    label={t.themeColor}
                    description={
                      lang.startsWith('zh')
                        ? '選擇 Kivio 的色彩主題；會依目前外觀自動使用淺色或深色設定。'
                        : 'Choose Kivio color theme. It adapts to the current light or dark appearance.'
                    }
                  >
                    <div className="kv-theme-colors" role="radiogroup" aria-label={t.themeColor}>
                      {THEME_COLOR_PRESETS.map((preset: ThemeColorPreset) => {
                        const active = themeColor === preset.id
                        const label = preset.labels[lang]
                        const accentHex = previewThemeIsDark ? preset.darkAccentHex : preset.lightAccentHex
                        return (
                          <button
                            key={preset.id}
                            type="button"
                            className={active ? 'active' : ''}
                            onClick={() => updateSettings({ themeColor: preset.id })}
                            role="radio"
                            aria-checked={active}
                            aria-label={label}
                            title={label}
                            data-tauri-drag-region="false"
                          >
                            <span
                              style={{
                                background: `linear-gradient(135deg, ${preset.lightHex} 0%, ${preset.lightHex} 50%, ${preset.darkHex} 50%, ${preset.darkHex} 100%)`,
                              }}
                            >
                              <i style={{ background: accentHex }} />
                            </span>
                          </button>
                        )
                      })}
                    </div>
                  </SettingRow>
                </SettingsGroup>

                <SettingsGroup title={lang.startsWith('zh') ? '行為' : 'Behavior'}>
                  <SettingRow label={t.launchAtStartup} description={lang.startsWith('zh') ? '登入後在背景啟動 Kivio。' : 'Open Kivio in the background when you sign in.'}>
                    <Toggle
                      checked={settings.launchAtStartup ?? false}
                      onChange={(v) => updateSettings({ launchAtStartup: v })}
                    />
                  </SettingRow>
                  <SettingRow label={t.retryEnabled}>
                    <Toggle
                      checked={settings.retryEnabled ?? true}
                      onChange={(v) => updateSettings({ retryEnabled: v })}
                    />
                  </SettingRow>
                  {settings.retryEnabled !== false && (
                    <SettingRow label={t.retryAttempts} description={lang.startsWith('zh') ? '範圍 1-5 次。' : 'Range: 1-5 attempts.'}>
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
                  <SettingRow label={t.onboardingRestart} description={t.onboardingRestartDesc}>
                    <button
                      type="button"
                      className="kv-btn sm"
                      onClick={handleRestartOnboarding}
                      data-tauri-drag-region="false"
                    >
                      {t.onboardingRestart}
                    </button>
                  </SettingRow>
                </SettingsGroup>

                <SettingsGroup title={lang.startsWith('zh') ? '備份與恢復' : 'Backup & Restore'}>
                  <FieldBlock
                    label={lang.startsWith('zh') ? '設定備份' : 'Settings backup'}
                    description={lang.startsWith('zh')
                      ? '匯出全部設定（含供應商、模型與 API Key）為 JSON 檔案。匯入會覆蓋當前全部設定並立即生效。'
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
                        {lang.startsWith('zh') ? '匯出設定' : 'Export'}
                      </button>
                      <button
                        type="button"
                        className="kv-btn sm"
                        onClick={handleImportSettings}
                        data-tauri-drag-region="false"
                      >
                        <Upload size={11} />
                        {lang.startsWith('zh') ? '匯入設定' : 'Import'}
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

            {/* ===== 翻譯設定標籤頁 ===== */}
            {activeTab === 'translate' && (
              <>
                <SettingsGroup title={t.hotkey}>
                  <SettingRow label={t.hotkey} description={lang.startsWith('zh') ? '翻譯當前選中文字或剪貼簿內容。' : 'Translates the current selection or clipboard.'} stack>
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

                <SettingsGroup title={lang.startsWith('zh') ? '輸出' : 'Output'}>
                  <SettingRow label={t.targetLang} description={lang.startsWith('zh') ? '自動模式會在中英文之間切換。' : 'Auto switches between Chinese and English.'}>
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
                  <SettingRow label={t.autoPaste} description={lang.startsWith('zh') ? '翻譯完成後自動貼上到當前應用。' : 'Paste translated text into the foreground app after translation completes.'}>
                    <Toggle
                      checked={settings.autoPaste ?? true}
                      onChange={(v) => updateSettings({ autoPaste: v })}
                    />
                  </SettingRow>
                </SettingsGroup>

                <SettingsGroup title={t.engine}>
                  <SettingRow label={t.selectModelPair} description={lang.startsWith('zh') ? '選擇輸入翻譯使用的供應商和模型。' : 'Choose the provider and model used for input translation.'}>
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

            {/* ===== 截圖設定標籤頁 ===== */}
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
                replaceHotkeyError={conflictMessageFor('screenshotTranslationReplace')}
                hotkeyClearLabel={t.hotkeyClear}
              />
            )}

            {/* ===== Lens 標籤頁 ===== */}
            {activeTab === 'lens' && (
              <>
                <SettingsGroup title={t.lensSection}>
                  <SettingRow label={t.enabled} description={lang.startsWith('zh') ? '啟用 Lens 截圖問答入口。' : 'Enable the Lens screenshot Q&A entry point.'}>
                    <Toggle
                      checked={settings.lens?.enabled !== false}
                      onChange={(v) => updateLens({ enabled: v })}
                    />
                  </SettingRow>

                  {settings.lens?.enabled !== false && (
                    <>
                      <SettingRow label={t.hotkey} description={lang.startsWith('zh') ? '進入 Lens 截圖選擇模式。' : 'Enter Lens screenshot selection mode.'} stack>
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
                      <SettingRow label={t.lensResponseLanguage} description={lang.startsWith('zh') ? '預設繼承輸入翻譯語言設定。' : 'Defaults to the input translation language setting.'}>
                        <Select
                          className="w-44"
                          value={settings.lens?.defaultLanguage || ''}
                          onChange={(v) => updateLens({ defaultLanguage: v })}
                          options={[
                            { value: '', label: t.lensLanguageInherit },
                            { value: 'zh', label: '中文' },
                            { value: 'zh-TW', label: '繁體中文' },
                            { value: 'en', label: 'English' },
                          ]}
                        />
                      </SettingRow>
                      <SettingRow label={t.lensStreamEnabled} description={lang.startsWith('zh') ? '模型返回時逐步顯示答案。' : 'Show answers progressively as the model responds.'}>
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
                    <SettingsGroup title={lang.startsWith('zh') ? '對話' : 'Conversation'}>
                      <SettingRow label={t.lensSendToChat} description={t.lensSendToChatHint}>
                        <Toggle
                          checked={settings.lens?.sendToChat !== false}
                          onChange={(v) => updateLens({ sendToChat: v })}
                        />
                      </SettingRow>
                      <SettingRow label={t.lensMessageOrder} description={lang.startsWith('zh') ? '控制 Lens 歷史訊息的排列順序。' : 'Controls the order of Lens history messages.'}>
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
                      <SettingRow label={t.lensShowCaptureHint}>
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
                      <SettingRow label={t.selectModelPair} description={lang.startsWith('zh') ? '留空時繼承輸入翻譯模型。' : 'Leave empty to inherit the input translation model.'}>
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
                      <SettingRow label={t.imageArchive}>
                        <Toggle
                          checked={settings.imageArchiveEnabled ?? false}
                          onChange={(v) => updateSettings({ imageArchiveEnabled: v })}
                        />
                      </SettingRow>
                      {settings.imageArchiveEnabled && (
                        <SettingRow label={t.imageArchivePath} stack>
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

            {/* ===== AI 客戶端標籤頁 ===== */}
            {activeTab === 'chat' && (
              <>
                <SettingsGroup title={lang.startsWith('zh') ? '個人資料' : 'Profile'}>
                  <SettingRow
                    label={lang.startsWith('zh') ? '使用者名稱' : 'Display name'}
                    description={lang.startsWith('zh') ? '顯示在 Chat 側欄底部；留空則不顯示。' : 'Shown at the bottom of the Chat sidebar; leave empty to hide.'}
                  >
                    <Input
                      value={chatConfig.userDisplayName || ''}
                      onChange={(userDisplayName) => updateChat({ userDisplayName })}
                      placeholder={lang.startsWith('zh') ? '選填' : 'Optional'}
                    />
                  </SettingRow>
                  <SettingRow
                    label={lang.startsWith('zh') ? '頭像' : 'Avatar'}
                    description={lang.startsWith('zh') ? '圖片連結或 data URL；留空則使用應用圖示。' : 'Image URL or data URL; leave empty to use the app icon.'}
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
                      {lang.startsWith('zh') ? '請先在「模型」中新增並配置供應商。' : 'Add and configure a provider under Models first.'}
                    </p>
                  )}
                  {chatProvider && chatProviderSupportsTools === false && (
                    <p className="kv-row-desc px-0 pb-2 text-amber-700 dark:text-amber-400">
                      {lang.startsWith('zh')
                        ? '當前預設供應商未啟用工具呼叫；MCP / Skill 工具可能不可用。'
                        : 'The default provider is marked as not supporting tools; MCP / Skill may be unavailable.'}
                    </p>
                  )}
                </SettingsGroup>

                <SettingsGroup title={lang.startsWith('zh') ? '回應' : 'Response'}>
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
                  <SettingRow label={t.chatMaxOutputTokens} stack>
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
                          {lang.startsWith('zh') ? '當前聊天模型：' : 'Current chat model: '}
                          {chatMaxOutputModelLabel}
                        </p>
                      </div>
                      <div className="flex shrink-0 items-center gap-2">
                        <span className="kv-row-desc whitespace-nowrap">
                          {lang.startsWith('zh') ? '兜底' : 'Fallback'}
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
                  <SettingRow label={t.chatDefaultLanguage}>
                    <Select
                      className="w-44"
                      value={chatConfig.defaultLanguage || ''}
                      onChange={(defaultLanguage) => updateChat({ defaultLanguage })}
                      options={[
                        { value: '', label: t.lensLanguageInherit },
                        { value: 'zh', label: '中文' },
                        { value: 'zh-TW', label: '繁體中文' },
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
                    label={lang.startsWith('zh') ? 'MCP 工具' : 'MCP tools'}
                    description={lang.startsWith('zh') ? `已配置 ${chatTools.servers.length} 個伺服器` : `${chatTools.servers.length} server(s) configured`}
                  >
                    <span className={`kv-tag ${chatTools.enabled ? 'ok' : ''}`}>
                      {chatTools.enabled
                        ? (lang.startsWith('zh') ? '已啟用' : 'On')
                        : (lang.startsWith('zh') ? '未啟用' : 'Off')}
                    </span>
                  </SettingRow>
                  <SettingRow
                    label={lang.startsWith('zh') ? 'Skill 執行時' : 'Skill runtime'}
                    description={lang.startsWith('zh') ? '內建 skill_activate / read_file / run_script' : 'Built-in skill_activate / read_file / run_script'}
                  >
                    <span className={`kv-tag ${skillRuntimeEnabled ? 'ok' : ''}`}>
                      {skillRuntimeEnabled
                        ? (lang.startsWith('zh') ? '已啟用' : 'On')
                        : (lang.startsWith('zh') ? '未啟用' : 'Off')}
                    </span>
                  </SettingRow>
                  <SettingRow
                    label={lang.startsWith('zh') ? '內建工具' : 'Native tools'}
                    description={lang.startsWith('zh') ? '讀寫檔案、命令、Python、網頁抓取等 Chat 工具' : 'Chat tools such as files, commands, Python, and web fetch'}
                  >
                    <span className={`kv-tag ${nativeBuiltinToolsEnabled ? 'ok' : ''}`}>
                      {nativeBuiltinToolsEnabled
                        ? (lang.startsWith('zh') ? '已啟用' : 'On')
                        : (lang.startsWith('zh') ? '未啟用' : 'Off')}
                    </span>
                  </SettingRow>
                  <SettingRow
                    label={t.tabMemory}
                    description={lang.startsWith('zh') ? 'L1 常駐注入，L2 通過 memory_read 按需讀取' : 'L1 is injected; L2 is read on demand with memory_read'}
                  >
                    <span className={`kv-tag ${chatMemory.enabled ? 'ok' : ''}`}>
                      {chatMemory.enabled
                        ? (lang.startsWith('zh') ? '已啟用' : 'On')
                        : (lang.startsWith('zh') ? '未啟用' : 'Off')}
                    </span>
                  </SettingRow>
                  <SettingRow
                    label={lang.startsWith('zh') ? '網路搜尋' : 'Web search'}
                    description={lang.startsWith('zh') ? 'Tavily/Exa 與 Lens、Chat 開關' : 'Tavily/Exa plus Lens and Chat toggles'}
                  >
                    <span className={`kv-tag ${(settings.lens?.webSearch?.enabled || chatTools.nativeTools?.webSearch) ? 'ok' : ''}`}>
                      {(settings.lens?.webSearch?.enabled || chatTools.nativeTools?.webSearch)
                        ? (lang.startsWith('zh') ? '部分啟用' : 'Partially on')
                        : (lang.startsWith('zh') ? '未啟用' : 'Off')}
                    </span>
                  </SettingRow>
                </SettingsGroup>
              </>
            )}

            {/* ===== 記憶標籤頁 ===== */}
            {activeTab === 'memory' && (
              <>
                <SettingsGroup title={lang.startsWith('zh') ? '記憶執行' : 'Memory runtime'}>
                  <SettingRow
                    label={lang.startsWith('zh') ? '啟用記憶' : 'Enable memory'}
                    description={lang.startsWith('zh')
                      ? '開啟後每次 Chat 請求自動注入 L1，並暴露 memory_read / memory_search / memory_modify。'
                      : 'When enabled, every Chat request injects L1 and exposes memory_read / memory_search / memory_modify.'}
                  >
                    <Toggle
                      checked={chatMemory.enabled}
                      onChange={(enabled) => updateChatMemory({ enabled })}
                    />
                  </SettingRow>
                  <SettingRow label={lang.startsWith('zh') ? '記憶資料夾' : 'Memory folder'} stack>
                    <div className="flex min-w-0 flex-wrap items-center gap-2">
                      <button
                        type="button"
                        className="kv-btn sm"
                        onClick={() => void refreshChatMemory()}
                        disabled={memoryLoading}
                        data-tauri-drag-region="false"
                      >
                        <RefreshCw size={10} className={memoryLoading ? 'animate-spin' : ''} />
                        {lang.startsWith('zh') ? '重新整理' : 'Refresh'}
                      </button>
                      <button
                        type="button"
                        className="kv-btn sm"
                        onClick={() => void handleOpenMemoryFolder()}
                        data-tauri-drag-region="false"
                      >
                        <FolderOpen size={11} />
                        {lang.startsWith('zh') ? '開啟資料夾' : 'Open folder'}
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
                    title={lang.startsWith('zh') ? 'L1 線上記憶' : 'L1 Online Memory'}
                    description={lang.startsWith('zh')
                      ? '短小、高頻、會影響每次回答的偏好、約束和當前目標。'
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
                    title={lang.startsWith('zh') ? 'L2 長期記憶' : 'L2 Long-Term Memory'}
                    description={lang.startsWith('zh')
                      ? '長期流程、決策、排障記錄和可複用知識；不會自動進入上下文。'
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

            {/* ===== 混音器標籤頁 ===== */}
            {activeTab === 'mixer' && (
              <>
                <SettingsGroup title={t.mixerSection}>
                  <div className="mb-3 flex items-start justify-between gap-3">
                    {t.mixerSectionHint ? (
                      <p className="kv-row-desc max-w-[560px]">{t.mixerSectionHint}</p>
                    ) : <span />}
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
                      {lang.startsWith('zh') ? '請先在「模型」中新增並配置供應商。' : 'Add and configure a provider under Models first.'}
                    </p>
                  )}
                </SettingsGroup>
              </>
            )}

            {/* ===== Kivio Code 標籤頁 ===== */}
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

            {/* ===== MCP 標籤頁 ===== */}
            {activeTab === 'mcp' && (
              <>                <SettingsGroup title={lang.startsWith('zh') ? 'Kivio 內建工具' : 'Kivio built-in tools'}>
                  <p className="kv-row-desc mb-2">
                    {lang.startsWith('zh')
                      ? 'Chat 原生工具。啟用後，本會話首次使用檔案 / 命令工具時會請求一次授權；授權後 Kivio 可讀寫磁碟任意路徑並執行終端命令（不再限制在主目錄內，也不再逐次確認）。授權僅本次會話有效，重啟後重新請求。'
                      : 'Native Chat tools. When first used in a conversation, file/command tools ask for one-time consent; once granted, Kivio can read/write anywhere on disk and run shell commands (no home-directory limit, no per-call prompts). Consent lasts for that conversation only and is re-requested after restart.'}
                  </p>
                  <SettingRow label={lang.startsWith('zh') ? '讀取檔案' : 'Read file'} description={lang.startsWith('zh') ? 'read' : 'read'}>
                    <Toggle
                      checked={chatTools.nativeTools?.readFile === true}
                      onChange={(readFile) => updateNativeTools({ readFile })}
                    />
                  </SettingRow>
                  <SettingRow label={lang.startsWith('zh') ? '寫入檔案' : 'Write file'} description={lang.startsWith('zh') ? 'write' : 'write'}>
                    <Toggle
                      checked={chatTools.nativeTools?.writeFile === true}
                      onChange={(writeFile) => updateNativeTools({ writeFile })}
                    />
                  </SettingRow>
                  <SettingRow label={lang.startsWith('zh') ? '編輯檔案' : 'Edit file'} description={lang.startsWith('zh') ? 'edit' : 'edit'}>
                    <Toggle
                      checked={chatTools.nativeTools?.editFile === true}
                      onChange={(editFile) => updateNativeTools({ editFile })}
                    />
                  </SettingRow>
                  <SettingRow label={lang.startsWith('zh') ? '終端命令' : 'Terminal command'} description={lang.startsWith('zh') ? 'bash' : 'bash'}>
                    <Toggle
                      checked={chatTools.nativeTools?.runCommand === true}
                      onChange={(runCommand) => updateNativeTools({ runCommand })}
                    />
                  </SettingRow>
                  <SettingRow label={lang.startsWith('zh') ? 'Python (Pyodide)' : 'Python (Pyodide)'} description={lang.startsWith('zh') ? 'run_python 沙盒，首次載入較慢' : 'run_python sandbox; first load is slow'}>
                    <Toggle
                      checked={chatTools.nativeTools?.runPython === true}
                      onChange={(runPython) => updateNativeTools({ runPython })}
                    />
                  </SettingRow>
                  <SettingRow label={lang.startsWith('zh') ? 'Skill 執行時' : 'Skill runtime'} description={lang.startsWith('zh') ? 'skill_activate / read_file / run_script' : 'skill_activate / read_file / run_script'}>
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
                          setSaveError(lang.startsWith('zh') ? '當前 Chat 模型供應商不支援 tools，無法啟用網路搜尋。' : 'The current Chat provider does not support tools, so web search cannot be enabled.')
                          return
                        }
                        updateNativeTools({ webSearch })
                      }}
                    />
                  </SettingRow>
                  {/* 搜尋 API（供應商 / Key / 結果數 / 深度）統一在「網路搜尋」分頁設定，
                      這裡只保留啟用開關，避免兩處重複編輯同一份 settings.lens.webSearch。 */}
                  <p className="kv-row-desc px-1 pb-1">
                    {lang.startsWith('zh')
                      ? '搜尋 API（供應商 / Key / 結果數 / 深度）在「網路搜尋」設定裡設定。'
                      : 'Configure the search API (provider / key / results / depth) in Web Search settings.'}
                  </p>
                  <SettingRow label={lang.startsWith('zh') ? '網頁抓取' : 'Web fetch'} description={lang.startsWith('zh') ? 'web_fetch，HTTPS 唯讀' : 'web_fetch, HTTPS read-only'}>
                    <Toggle
                      checked={chatTools.nativeTools?.webFetch === true}
                      onChange={(webFetch) => updateNativeTools({ webFetch })}
                    />
                  </SettingRow>
                  <SettingRow label={lang.startsWith('zh') ? '工作區根目錄（可選）' : 'Workspace roots (optional)'} stack>
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
                        {lang.startsWith('zh') ? '新增工作區目錄' : 'Add workspace folder'}
                      </button>
                    </div>
                  </SettingRow>
                </SettingsGroup>

                <SettingsGroup title={lang.startsWith('zh') ? '工具執行' : 'Tool Runtime'}>
                  {/* 主開關：MCP 啟用與否，單獨成行並垂直居中 */}
                  <div className="flex items-center justify-between gap-4 py-3">
                    <div className="min-w-0">
                      <div className="kv-row-label">{lang.startsWith('zh') ? '啟用 MCP' : 'Enable MCP'}</div>
                      <p className="kv-row-desc">
                        {chatProviderSupportsTools
                          ? (lang.startsWith('zh') ? '向支援 tools 的模型暴露已啟用的 MCP 工具。' : 'Expose enabled MCP tools to models that support tools.')
                          : (lang.startsWith('zh') ? '當前 Chat 模型供應商不支援 tools；Skill 仍會作為提示詞生效。' : 'The current Chat provider does not support tools; Skills still work as prompt injection.')}
                      </p>
                    </div>
                    <Toggle
                      checked={chatTools.enabled}
                      onChange={(enabled) => {
                        if (!chatProviderSupportsTools) {
                          setSaveError(lang.startsWith('zh') ? '當前 Chat 模型供應商不支援 tools，無法啟用 MCP。' : 'The current Chat provider does not support tools, so MCP cannot be enabled.')
                          return
                        }
                        updateChatTools({ enabled })
                      }}
                    />
                  </div>

                  {/* 審批與執行引數：統一柵格，每格 label→描述→控制元件，控制元件用 mt-auto 底部對齊 */}
                  <div className="grid grid-cols-[repeat(auto-fit,minmax(190px,1fr))] items-stretch gap-x-4 gap-y-5 border-t border-[var(--divider)] py-3">
                    {/* 審批策略 */}
                    <div className="flex h-full flex-col">
                      <div className="mb-2">
                        <div className="kv-row-label">{lang.startsWith('zh') ? '審批策略' : 'Approval policy'}</div>
                        <p className="kv-row-desc">{lang.startsWith('zh') ? '檔案/命令工具的授權方式；MCP 工具仍按其只讀/敏感屬性逐次判定。' : 'How file/command tools are authorized; MCP tools still follow their read-only/sensitive hints per call.'}</p>
                      </div>
                      <div className="mt-auto">
                        <Select
                          className="w-full"
                          value={chatTools.approvalPolicy || 'auto'}
                          onChange={(approvalPolicy) => updateChatTools({ approvalPolicy })}
                          options={[
                            {
                              value: 'readonly_auto_sensitive_confirm',
                              label: lang.startsWith('zh') ? '會話授權一次（推薦）' : 'Session consent (once)',
                            },
                            { value: 'always_confirm', label: lang.startsWith('zh') ? '授權後仍逐次確認' : 'Confirm every call' },
                            { value: 'auto', label: lang.startsWith('zh') ? '全部自動（不彈授權）' : 'Auto (no prompt)' },
                          ]}
                        />
                      </div>
                    </div>

                    {/* 最大工具輪次 */}
                    <div className="flex h-full flex-col">
                      <div className="mb-2">
                        <div className="kv-row-label">{lang.startsWith('zh') ? '最大工具輪次' : 'Max tool rounds'}</div>
                        <p className="kv-row-desc">
                          {lang.startsWith('zh')
                            ? '達到上限後停止呼叫工具，並基於已有結果生成最終回覆。'
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
                                  label: lang.startsWith('zh')
                                    ? `當前 ${formatToolRoundsLabel(clampToolRounds(chatTools.maxToolRounds), lang)}`
                                    : `Current ${formatToolRoundsLabel(clampToolRounds(chatTools.maxToolRounds), lang)}`,
                                }]
                              : []),
                            ...CHAT_TOOL_ROUND_PRESETS.map((rounds) => ({
                              value: String(rounds),
                              label: formatToolRoundsLabel(rounds, lang),
                            })),
                            { value: 'unlimited', label: lang.startsWith('zh') ? '無限制' : 'Unlimited' },
                          ]}
                        />
                      </div>
                    </div>

                    {/* 子 agent 併發 */}
                    <div className="flex h-full flex-col">
                      <div className="mb-2">
                        <div className="kv-row-label">{lang.startsWith('zh') ? 'Subagent 併發' : 'Subagent concurrency'}</div>
                        <p className="kv-row-desc">
                          {lang.startsWith('zh')
                            ? '同一時刻最多並行執行的 Subagent 數。調高更快但更吃 API 配額。'
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
                                  label: lang.startsWith('zh')
                                    ? `當前 ${clampSubAgentConcurrency(chatTools.subAgentConcurrency)}`
                                    : `Current ${clampSubAgentConcurrency(chatTools.subAgentConcurrency)}`,
                                }]
                              : []),
                            ...SUB_AGENT_CONCURRENCY_PRESETS.map((n) => ({ value: String(n), label: String(n) })),
                          ]}
                        />
                      </div>
                    </div>

                    {/* 工具超時 */}
                    <div className="flex h-full flex-col">
                      <div className="mb-2">
                        <div className="kv-row-label">{lang.startsWith('zh') ? '工具超時' : 'Tool timeout'}</div>
                        <p className="kv-row-desc">{lang.startsWith('zh') ? '單次工具呼叫的最長等待時間。' : 'Maximum wait time for a single tool call.'}</p>
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
                                  label: lang.startsWith('zh')
                                    ? `當前 ${formatToolTimeoutLabel(clampToolTimeoutMs(chatTools.toolTimeoutMs), lang)}`
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

                    {/* MCP 空閒超時 */}
                    <div className="flex h-full flex-col">
                      <div className="mb-2">
                        <div className="kv-row-label">{lang.startsWith('zh') ? 'MCP 空閒超時' : 'MCP idle timeout'}</div>
                        <p className="kv-row-desc">
                          {lang.startsWith('zh')
                            ? '持久連線空閒超過此值後回收子程式，下次呼叫透明重連。'
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
                                  label: lang.startsWith('zh')
                                    ? `當前 ${formatToolTimeoutLabel(clampMcpIdleTimeoutMs(chatTools.mcpIdleTimeoutMs), lang)}`
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

                    {/* 結果截斷字元 */}
                    <div className="flex h-full flex-col">
                      <div className="mb-2">
                        <div className="kv-row-label">{lang.startsWith('zh') ? '結果截斷字元' : 'Output chars'}</div>
                        <p className="kv-row-desc">{lang.startsWith('zh') ? '工具結果當前不做截斷。' : 'Tool results are not truncated for now.'}</p>
                      </div>
                      <div className="mt-auto">
                        <div className="flex h-[30px] items-center rounded-md border border-[var(--border)] bg-[var(--bg-input-subtle)] px-2.5 text-[12.5px] text-[var(--text-muted)]">
                          {lang.startsWith('zh') ? '無限制輸出' : 'Unlimited output'}
                        </div>
                      </div>
                    </div>
                  </div>
                </SettingsGroup>

                <SettingsGroup title={lang.startsWith('zh') ? 'MCP 伺服器' : 'MCP Servers'}>
                  <div className="flex flex-wrap gap-2 py-2">
                    <button
                      type="button"
                      className="kv-btn sm"
                      onClick={() => updateChatTools({ servers: [...chatTools.servers, newMcpServer()] })}
                      data-tauri-drag-region="false"
                    >
                      <Plus size={11} />
                      {lang.startsWith('zh') ? '新增伺服器' : 'Add server'}
                    </button>
                    <button
                      type="button"
                      className="kv-btn sm"
                      onClick={() => void handleImportMcpJson()}
                      data-tauri-drag-region="false"
                    >
                      <FolderOpen size={11} />
                      {lang.startsWith('zh') ? '匯入 mcp.json' : 'Import mcp.json'}
                    </button>
                  </div>

                  {chatTools.servers.length === 0 && (
                    <div className="kv-panel">
                      <div className="kv-panel-title">{lang.startsWith('zh') ? '暫無 MCP 伺服器' : 'No MCP servers'}</div>
                      <div className="kv-panel-body">
                        {lang.startsWith('zh') ? '新增或匯入伺服器後，需要手動啟用才會暴露給模型。' : 'Added or imported servers stay disabled until you enable them.'}
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
                            description: lang.startsWith('zh') ? '已儲存的工具限制；重新測試連線可重新整理描述。' : 'Saved tool limit; test the server to refresh description.',
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
                          ? (lang.startsWith('zh') ? '已連線' : 'Connected')
                          : stateKind === 'connecting'
                            ? (lang.startsWith('zh') ? '連線中' : 'Connecting')
                            : stateKind === 'error'
                              ? (lang.startsWith('zh') ? '錯誤' : 'Error')
                              : (lang.startsWith('zh') ? '未連線' : 'Disconnected')
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
                              title={lang.startsWith('zh') ? '刪除伺服器' : 'Delete server'}
                              aria-label={lang.startsWith('zh') ? '刪除伺服器' : 'Delete server'}
                              data-tauri-drag-region="false"
                            >
                              <Trash2 size={12} />
                            </button>
                          </div>
                          <FieldBlock label={lang.startsWith('zh') ? '傳輸' : 'Transport'}>
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
                              <FieldBlock label={lang.startsWith('zh') ? 'Endpoint URL' : 'Endpoint URL'}>
                                <Input
                                  mono
                                  value={server.url || ''}
                                  onChange={(url) => updateMcpServer(server.id, { url })}
                                  placeholder="https://example.com/mcp"
                                />
                              </FieldBlock>
                              <FieldBlock
                                label="Headers"
                                description={lang.startsWith('zh') ? '每行 KEY=value；例如 Authorization=Bearer ...，會隨 settings.json 明文儲存。' : 'One KEY=value per line, e.g. Authorization=Bearer ...; stored in settings.json as plain text.'}
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
                                <FieldBlock label={lang.startsWith('zh') ? '命令' : 'Command'}>
                                  <Input
                                    mono
                                    value={server.command}
                                    onChange={(command) => updateMcpServer(server.id, { command })}
                                    placeholder="npx"
                                  />
                                </FieldBlock>
                                <FieldBlock label={lang.startsWith('zh') ? '引數' : 'Arguments'} description={lang.startsWith('zh') ? '每行一個引數；保留引數中的空格和引號。' : 'One argument per line; spaces and quotes inside each argument are preserved.'}>
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
                              <FieldBlock label={lang.startsWith('zh') ? '工作目錄' : 'Working directory'}>
                                <Input
                                  mono
                                  value={server.cwd || ''}
                                  onChange={(cwd) => updateMcpServer(server.id, { cwd: cwd.trim() ? cwd : null })}
                                  placeholder={lang.startsWith('zh') ? '可選' : 'Optional'}
                                />
                              </FieldBlock>
                              <FieldBlock
                                label="Env"
                                description={lang.startsWith('zh') ? '每行 KEY=value；這些值會隨 settings.json 明文儲存。' : 'One KEY=value per line; values are stored in settings.json as plain text.'}
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
                              {testingMcpServerId === server.id ? (lang.startsWith('zh') ? '測試中' : 'Testing') : (lang.startsWith('zh') ? '測試連線' : 'Test')}
                            </button>
                            {feedback && (
                              <span className={`kv-tag ${feedback.ok ? 'ok' : 'warn'}`}>
                                {feedback.message}
                              </span>
                            )}
                            {server.enabledTools.length === 0 && knownTools.length > 0 && (
                              <span className="kv-row-desc">{lang.startsWith('zh') ? '當前暴露全部工具。' : 'All tools are exposed.'}</span>
                            )}
                          </div>
                          {/* 持久連線狀態面板：狀態點 / lastError / 摺疊 stderr / 重連按鈕 */}
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
                              {lang.startsWith('zh') ? '重連' : 'Reconnect'}
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
                                  ? (lang.startsWith('zh') ? '隱藏日誌' : 'Hide log')
                                  : (lang.startsWith('zh') ? '檢視 stderr' : 'View stderr')}
                              </button>
                            )}
                          </div>
                          {stateError && (
                            <p className="mt-1 kv-row-desc break-words whitespace-pre-wrap" style={{ color: 'var(--danger)' }}>
                              {stateError}
                            </p>
                          )}
                          {stderrExpanded && stderrTail.trim() && (
                            <pre className="custom-scrollbar mt-1 max-h-40 overflow-auto rounded bg-black/5 dark:bg-white/5 p-2 text-[11px] leading-snug whitespace-pre-wrap break-words">
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

            {/* ===== 連接器標籤頁 ===== */}
            {activeTab === 'connectors' && (
              <ConnectorsPanel
                servers={chatTools.servers}
                updateChatTools={updateChatTools}
                obsidianVaultPath={settings?.obsidianVaultPath ?? ''}
                onObsidianVaultPathChange={(path) => updateSettings({ obsidianVaultPath: path })}
                emailAccounts={settings?.emailAccounts ?? []}
                onEmailAccountsChange={(accounts) => updateSettings({ emailAccounts: accounts })}
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

            {/* ===== 知識庫標籤頁 ===== */}
            {activeTab === 'knowledge' && (
              <KnowledgeBasePanel
                providers={settings?.providers ?? []}
                lang={lang}
                docProcessing={settings?.documentProcessing}
                onChangeDocProcessing={(dp) => updateSettings({ documentProcessing: dp })}
                kbConfig={settings?.knowledgeBase}
                onChangeKbConfig={(kb) => updateSettings({ knowledgeBase: kb })}
              />
            )}

            {/* ===== Skill 標籤頁 ===== */}
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
                      {lang.startsWith('zh') ? '重新整理列表' : 'Refresh'}
                    </button>
                    <button
                      type="button"
                      className="kv-btn sm"
                      onClick={() => void handleImportSkill()}
                      data-tauri-drag-region="false"
                    >
                      <FolderOpen size={11} />
                      {lang.startsWith('zh') ? '匯入資料夾' : 'Import folder'}
                    </button>
                    <button
                      type="button"
                      className="kv-btn sm"
                      onClick={() => void handleImportSkillZip()}
                      data-tauri-drag-region="false"
                    >
                      <Download size={11} />
                      {lang.startsWith('zh') ? '匯入 zip' : 'Import zip'}
                    </button>
                    <button
                      type="button"
                      className="kv-btn sm"
                      onClick={() => void handleOpenSkillFolder()}
                      data-tauri-drag-region="false"
                    >
                      <ExternalLink size={11} />
                      {lang.startsWith('zh') ? '開啟 Skill 資料夾' : 'Open skill folder'}
                    </button>
                  </div>
                  <SettingRow label={lang.startsWith('zh') ? '額外掃描路徑' : 'Extra scan paths'} stack>
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
                            aria-label={lang.startsWith('zh') ? '移除路徑' : 'Remove path'}
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
                        {lang.startsWith('zh') ? '新增掃描路徑' : 'Add scan path'}
                      </button>
                    </div>
                  </SettingRow>
                  <SettingRow
                    label={lang.startsWith('zh') ? '自動匹配 Skill' : 'Auto-match skills'}
                    description={lang.startsWith('zh') ? '允許模型根據 description 自動 activate skill' : 'Allow the model to activate skills from the catalog automatically'}
                  >
                    <Toggle
                      checked={chatTools.skillAutoMatch !== false}
                      onChange={(skillAutoMatch) => updateChatTools({ skillAutoMatch })}
                    />
                  </SettingRow>
                  <SettingRow label={lang.startsWith('zh') ? '無 Tools 降級模式' : 'Fallback without tools'} stack>
                    <Select
                      value={chatTools.skillFallbackMode || 'progressive'}
                      onChange={(skillFallbackMode) => updateChatTools({ skillFallbackMode })}
                      options={[
                        { value: 'progressive', label: lang.startsWith('zh') ? '漸進式（僅 catalog）' : 'Progressive (catalog only)' },
                        { value: 'skill_md_only', label: lang.startsWith('zh') ? '僅 SKILL.md' : 'SKILL.md only' },
                        { value: 'legacy_full_body', label: lang.startsWith('zh') ? '舊版全量注入' : 'Legacy full body' },
                      ]}
                    />
                  </SettingRow>
                  <SettingRow label={lang.startsWith('zh') ? '指令碼直譯器白名單' : 'Script interpreter allowlist'} stack>
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
                  <SettingRow label={t.enabled}>
                    <span className="kv-tag ok">
                      {enabledSkillCount}
                      {' / '}
                      {skills.length}
                    </span>
                  </SettingRow>
                  {skillsLoading && (
                    <div className="kv-panel chat-motion-fade-up">
                      <div className="kv-panel-body">{lang.startsWith('zh') ? '正在載入 Skill...' : 'Loading skills...'}</div>
                    </div>
                  )}
                  {!skillsLoading && skills.length === 0 && (
                    <div className="kv-panel">
                      <div className="kv-panel-title">{lang.startsWith('zh') ? '暫無 Skill' : 'No skills'}</div>
                      <div className="kv-panel-body">
                        {lang.startsWith('zh') ? '暫無 Skill。可匯入資料夾/zip，或開啟 Skill 資料夾手動新增後重新整理。' : 'No skills yet. Import a folder or zip, or add skills manually and refresh.'}
                      </div>
                    </div>
                  )}
                  {!skillsLoading && skills.length > 0 && (
                    <div className="space-y-3 py-2">
                      <SkillListSection
                        title={lang.startsWith('zh') ? '內建 Skill' : 'Built-in skills'}
                        emptyText={lang.startsWith('zh') ? '當前沒有內建 Skill。' : 'No built-in skills.'}
                        skills={builtinSkills}
                        lang={lang}
                        expandedSkillIds={expandedSkillIds}
                        disabledSkillIds={disabledSkillIds}
                        onToggleExpanded={handleToggleSkillExpanded}
                        onToggleEnabled={handleToggleSkillEnabled}
                        onPreview={handlePreviewSkill}
                      />
                      <SkillListSection
                        title={lang.startsWith('zh') ? '使用者 Skill' : 'User skills'}
                        emptyText={lang.startsWith('zh') ? '當前沒有使用者匯入的 Skill。' : 'No imported user skills.'}
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

            {/* ===== 網路搜尋分頁 ===== */}
            {activeTab === 'webSearch' && (
              <WebSearchPanel
                t={t}
                lang={lang}
                webSearch={settings.lens?.webSearch}
                onChange={updateLensWebSearch}
              />
            )}

            {/* ===== 用量統計標籤頁（內含請求除錯二級檢視） ===== */}
            {activeTab === 'usage' && (
              <div className="space-y-3">
                <div className="kv-seg w-fit">
                  <button
                    type="button"
                    className={usageView === 'stats' ? 'active' : ''}
                    onClick={() => setUsageView('stats')}
                    data-tauri-drag-region="false"
                  >
                    {lang.startsWith('zh') ? '用量統計' : 'Usage'}
                  </button>
                  <button
                    type="button"
                    className={usageView === 'debug' ? 'active' : ''}
                    onClick={() => setUsageView('debug')}
                    data-tauri-drag-region="false"
                  >
                    {lang.startsWith('zh') ? '請求除錯' : 'Request debug'}
                  </button>
                </div>
                {usageView === 'stats' ? (
                  <UsageStatsPanel lang={lang} />
                ) : (
                  <RequestDebugPanel
                    lang={lang}
                    enabled={chatTools.requestDebugEnabled ?? false}
                    onToggleEnabled={(v) => updateChatTools({ requestDebugEnabled: v })}
                  />
                )}
              </div>
            )}

            {/* ===== 模型管理標籤頁 ===== */}
            {activeTab === 'providers' && (
              <div className="kv-providers-root">
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
                    trailing={PROVIDER_PRESETS
                      .filter((preset) => !settings.providers.some((p) => p.baseUrl === preset.baseUrl))
                      .map((preset) => (
                        <button
                          key={preset.name}
                          type="button"
                          onClick={() => addProviderFromPreset(preset)}
                          className="kv-provider-item"
                          title={lang.startsWith('zh') ? `新增 ${preset.name}` : `Add ${preset.name}`}
                          data-tauri-drag-region="false"
                        >
                          <span className="kv-provider-item-select">
                            <span className="kv-provider-dot off" />
                            <span className="kv-provider-name">{preset.name}</span>
                          </span>
                        </button>
                      ))}
                  />
                </div>

                <div className="kv-provider-detail">
                  <SettingsGroup title={lang.startsWith('zh') ? '供應商' : 'Provider'} className="!pt-0 kv-provider-section">
                    {selectedProvider ? (() => {
                      const provider = selectedProvider
                      const configured = provider.apiKeys.some((key) => key.trim())
                      return (
                        <div className="kv-provider-header">
                          <div className="kv-provider-header-toolbar">
                            <span className="kv-row-label">{lang.startsWith('zh') ? '啟用供應商' : 'Enable provider'}</span>
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
                                  ? (lang.startsWith('zh') ? '已停用' : 'Disabled')
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
                        {lang.startsWith('zh') ? '在左側選擇供應商，或點選上方「新增」建立。' : 'Select a provider on the left, or click “Add” above.'}
                      </p>
                    )}
                  </SettingsGroup>

                  {selectedProvider ? (() => {
                    const provider = selectedProvider
                    return (
                        <SettingsGroup title={lang.startsWith('zh') ? '配置' : 'Configuration'}>
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
                                  { value: 'gemini', label: 'Gemini' },
                                ]}
                              />
                            </div>
                          </FieldBlock>

                          <SettingRow
                            label={
                              <span className="flex flex-col gap-1">
                                <span className="flex items-center gap-1">
                                  <span>{lang.startsWith('zh') ? '壓縮請求體 (gzip)' : 'Compress request body (gzip)'}</span>
                                  <button
                                    type="button"
                                    aria-label={lang.startsWith('zh') ? '顯示說明' : 'Show details'}
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
                                    {lang.startsWith('zh')
                                      ? '個別供應商前置的 WAF 會掃描明文請求體，把工具/系統提示裡的 shell 命令、檔案路徑等文字誤判為攻擊而返回 403。開啟後請求體用 gzip 壓縮傳送（多數閘道器可正常解壓）。若該供應商不接受 gzip 請求（如官方 DeepSeek）會返回 400，請保持關閉。'
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
                              {(() => {
                                // 命中快速預設 baseUrl 時，給出「取得 API Key」外鏈引導使用者申請。
                                const preset = PROVIDER_PRESETS.find(
                                  (p) => p.baseUrl === provider.baseUrl && p.apiKeyUrl,
                                )
                                if (!preset?.apiKeyUrl) return null
                                return (
                                  <button
                                    type="button"
                                    onClick={() => void api.openExternal(preset.apiKeyUrl!)}
                                    className="inline-flex w-fit items-center gap-0.5 text-[12px] text-indigo-500 hover:underline dark:text-indigo-300"
                                    data-tauri-drag-region="false"
                                  >
                                    {lang.startsWith('zh') ? `取得 ${preset.name} API Key ↗` : `Get ${preset.name} API key ↗`}
                                  </button>
                                )
                              })()}
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
                                      title={revealed ? (lang.startsWith('zh') ? '隱藏金鑰' : 'Hide key') : (lang.startsWith('zh') ? '顯示金鑰' : 'Show key')}
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
                                  ? (lang.startsWith('zh') ? '管理模型' : 'Models')
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
                                <span>{lang.startsWith('zh') ? '模型' : 'Models'}</span>
                                <span className="kv-tag">{provider.enabledModels.length}</span>
                              </span>
                            )}
                            description={lang.startsWith('zh') ? '這些模型會出現在各功能的模型選擇器中。' : 'These models appear in feature model selectors.'}
                          >
                            <ul className="kv-enabled-model-list">
                              {provider.enabledModels.length === 0 && (
                                <li className="kv-enabled-model-empty">
                                  {lang.startsWith('zh') ? '點選上方「取得模型列表」拉取並新增模型。' : 'Use "Fetch Models" above to load and add models.'}
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
              </div>
            )}

            {/* ===== 關於標籤頁 ===== */}
            {activeTab === 'about' && (
              <>
                <SettingsGroup title={lang.startsWith('zh') ? '應用' : 'Application'}>
                  <div className="kv-panel mb-2">
                    <div className="flex items-center gap-3">
                      <div className="w-12 h-12 rounded-[10px] overflow-hidden shrink-0">
                        <img src="/icon.png" alt="Kivio" className="w-full h-full object-contain" />
                      </div>
                      <div className="min-w-0">
                        <div className="kv-page-title">Kivio</div>
                        <div className="kv-panel-body">{lang.startsWith('zh') ? '螢幕級 AI 助手' : 'Screen-level AI Assistant'}</div>
                      </div>
                    </div>
                  </div>
                  <SettingRow label={t.currentVersion}>
                    <span className="kv-tag">v{appVersion}</span>
                  </SettingRow>
                  <SettingRow label={lang.startsWith('zh') ? '開發者' : 'Developer'}>
                    <span className="kv-row-desc">ZM</span>
                  </SettingRow>
                </SettingsGroup>

                <SettingsGroup title={t.checkUpdate}>
                  <SettingRow label={t.autoCheckUpdate}>
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
                        <div className="custom-scrollbar mb-3 max-h-40 overflow-y-auto text-[12px] leading-relaxed">
                          <ChatMarkdown content={updateInfo.body} />
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
                  <span>{lang.startsWith('zh') ? '有未儲存更改。' : 'You have unsaved changes.'}</span>
                </>
              ) : (
                <>
                  <span className="clean-icon"><Check size={13} strokeWidth={2.4} /></span>
                  <span>{lang.startsWith('zh') ? '所有更改已儲存。' : 'All changes saved.'}</span>
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
            title: lang.startsWith('zh') ? '模型' : 'Models',
            searchPlaceholder: lang.startsWith('zh') ? '搜尋模型 ID 或名稱' : 'Search model ID or name',
            fetchModels: t.fetchModels,
            fetching: t.fetching,
            addModel: t.addModel,
            manualAddModel: t.manualAddModel,
            noModels: lang.startsWith('zh') ? '尚未取得模型，請點選上方按鈕拉取。' : 'No models yet. Click the button above to fetch.',
            noSearchResults: lang.startsWith('zh') ? '沒有匹配的模型' : 'No matching models',
            enabled: lang.startsWith('zh') ? '已啟用' : 'On',
            addAllModels: lang.startsWith('zh') ? '新增當前列表中的全部模型' : 'Add all models in the current list',
            close: lang.startsWith('zh') ? '關閉' : 'Close',
          }}
          fetching={fetchingProviderId === modelPickerProvider.id}
          onClose={() => setModelPickerProviderId(null)}
          onFetch={() => void fetchModels(modelPickerProvider.id)}
          onAdd={(model) => addEnabledModel(modelPickerProvider.id, model)}
          onAddAll={(models) => addAllEnabledModels(modelPickerProvider.id, models)}
          onRemove={(model) => removeEnabledModel(modelPickerProvider.id, model)}
        />
      )}
      {/* 模型詳情抽屜 */}
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
      {/* 未儲存更改確認彈窗 */}
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
      {/* 刪除供應商確認彈窗 */}
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
                aria-label={lang.startsWith('zh') ? '關閉' : 'Close'}
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
            <div className="custom-scrollbar max-h-[52vh] overflow-y-auto rounded-md border border-black/[0.08] bg-black/[0.025] p-3 dark:border-white/[0.08] dark:bg-white/[0.035]">
              <ChatMarkdown content={selectedSkillPreview.body} />
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
