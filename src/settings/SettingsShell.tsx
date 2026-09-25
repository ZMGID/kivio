import { forwardRef, useImperativeHandle, useState, useEffect, useCallback, useMemo, useRef, useSyncExternalStore, type ReactNode, type SetStateAction } from 'react'
import {
  X, RefreshCw, Monitor,
  Download, Upload, ArrowLeft,
} from 'lucide-react'
import { open, save } from '@tauri-apps/plugin-dialog'
import {
  api,
  type Settings as SettingsType,
  type ModelProvider,
  type ModelInfo,
  type DefaultPromptTemplates,
  type ChatToolsConfig,
  type ChatNativeToolsConfig,
  type ChatMemoryConfig,
} from '../api/tauri'
import {
  getSettingsSnapshotCached,
  importSettingsSnapshotCached,
  peekSettingsSnapshot,
  refreshSettingsSnapshot,
  saveSettingsSnapshotCached,
  subscribeSettingsSnapshot,
  updateSettingsCached,
} from '../api/settingsCache'
import { SettingsEditorController, type SettingsCloseOptions } from './SettingsEditorController'
import { useSettingsUpdateController } from './useSettingsUpdateController'
import { useSettingsOcrDownloads } from './useSettingsOcrDownloads'
import { useSettingsPermissions } from './useSettingsPermissions'
import { applyProviderDraftIntent, type ProviderDraftIntent } from './providerDraftIntents'
import { i18n, type Lang } from '../components/i18n'
import {
  GeneralIcon, HotkeysIcon, TranslateIcon, LensIcon, ChatIcon, MemoryIcon, MixerIcon,
  AgentIcon, WebSearchIcon, PluginsIcon, SessionsIcon, UsageIcon, ProvidersIcon, AboutIcon, HooksIcon,
} from './NavIcons'
import { formatHotkeyError, getPlatform } from './utils'
import { type ProviderPreset } from './providerPresets'
import { ProviderModelsPicker } from './ProviderModelsPicker'
import { ScreenshotTranslationSettings } from './ScreenshotTranslationSettings'
import { UsageStatsPanel } from './UsageStatsPanel'
import { RequestDebugPanel } from './RequestDebugPanel'
import { ExternalAgentsSettings } from './ExternalAgentsSettings'
import { HotkeysTab } from './tabs/HotkeysTab'
import { LensTab } from './tabs/LensTab'
import { MixerTab } from './tabs/MixerTab'
import { TranslateTab } from './tabs/TranslateTab'
import { MemoryTab } from './tabs/MemoryTab'
import { ChatTab } from './tabs/ChatTab'
import { ProvidersTab } from './tabs/ProvidersTab'
import { HooksTab } from './tabs/HooksTab'
import { ComputerControlTab } from './tabs/ComputerControlTab'
import { AppearanceGroup, BehaviorGroup, PermissionsGroup } from './tabs/GeneralTab'
import { AppInfoGroup, UpdateGroup } from './tabs/AboutTab'
import { useSettingsMemoryEditor } from './useSettingsMemoryEditor'
import { useProviderCatalogController } from './useProviderCatalogController'
import { useSettingsBackupController } from './useSettingsBackupController'
import { useSettingsHotkeyRecorder, type HotkeyScopeKey } from './useSettingsHotkeyRecorder'
import { useProviderModalController } from './useProviderModalController'
import { useSettingsOnboardingController } from './useSettingsOnboardingController'
import { ModelDetailDrawer } from './ModelDetailDrawer'
import { ProviderModelTestModal } from './ProviderModelTestModal'
import { Button } from '../components/Button'
import { resolveModelInfo } from '../data/modelMatching'
import { loadLastModel, resolvePreferredChatModel } from '../data/chatModelPreference'
import { useWindowInteractionFocus } from '../api/windowFocus'
import { hasEnabledNativeBuiltinTool, hasEnabledSkillRuntime } from '../api/chatTools'
import { UI_FONT_PX_MIN, UI_FONT_PX_MAX } from './uiFont'
import {
  SettingRow,
  SettingsGroup, FieldBlock,
} from './components'
import { ConnectorsPanel } from './ConnectorsPanel'
import { WebSearchPanel } from './WebSearchPanel'

export type SettingsTab = 'general' | 'hotkeys' | 'translate' | 'lens' | 'chat' | 'memory' | 'mixer' | 'externalAgents' | 'computerControl' | 'hooks' | 'webSearch' | 'connectors' | 'plugins' | 'sessions' | 'usage' | 'providers' | 'about'

type SettingsData = SettingsType
// UI 字号：以 px 展示、以整体缩放（zoom）实现。CSS 全是 px 硬编码，做不了真正的 rem 基准字号，
// 故 14px 锚定为 100%，输入 px → scale = px/14。ponytail: zoom 代理，若将来全量 rem 化可换真基准。
const UI_FONT_BASE_PX = 14

export interface SettingsShellProps {
  variant: 'standalone' | 'embedded'
  onClose: () => void
  onSettingsChange: () => void
  onReady?: () => void
  reserveTrafficLightSpace?: boolean
  /** 打开设置面板时选中的侧栏项（如 Chat 内嵌设置默认 AI 客户端） */
  initialTab?: SettingsTab
  /** embedded 单页模式：隐藏左侧设置导航，只显示 initialTab 对应页（如从扩展点「知识库」进入） */
  hideNav?: boolean
  /** Chat 宿主提供的领域视图；设置只决定它们出现的位置。 */
  renderSessionCenter?: (lang: Lang) => ReactNode
  renderPluginCenter: (input: {
    section: 'plugins' | 'connectors'
    onSectionChange: (section: 'plugins' | 'connectors') => void
    lang: Lang
    connectors: ReactNode
  }) => ReactNode
  renderReleaseNotes: (markdown: string) => ReactNode
}

export interface SettingsShellHandle {
  requestClose: (options?: SettingsCloseOptions) => void
}

/** 快捷键作用域。原本是组件体内的局部 type，抽 HotkeysTab 后需要跨模块共享，提到模块作用域。 */
export type { HotkeyScopeKey } from './useSettingsHotkeyRecorder'

function resolveEffectiveChatModel(settings: SettingsData): { provider?: ModelProvider, model: string } {
  const selected = resolvePreferredChatModel({
    providers: settings.providers,
    last: loadLastModel(),
    storedChat: settings.defaultModels.chat,
    legacyChat: { providerId: settings.chatProviderId, model: settings.chatModel },
    lens: { providerId: settings.lens?.providerId || '', model: settings.lens?.model || '' },
    translator: { providerId: settings.translatorProviderId, model: settings.translatorModel },
  })

  return {
    provider: settings.providers.find((provider) => provider.id === selected.providerId),
    model: selected.model || '',
  }
}

function resolveEffectiveChatMaxOutput(settings: SettingsData, fallbackTokens: number) {
  const { provider, model } = resolveEffectiveChatModel(settings)
  const override = model ? provider?.modelOverrides?.[model]?.maxOutput : undefined
  const modelInfo = model ? resolveModelInfo(model, provider?.modelOverrides, provider) : {}
  const maxOutput = override || modelInfo.maxOutput || fallbackTokens
  const source: 'override' | 'database' | 'fallback' = override
    ? 'override'
    : modelInfo.maxOutput
      ? 'database'
      : 'fallback'

  return { maxOutput, source, model, provider }
}

/**
 * 设置面板主组件（standalone / embedded 双宿主）
 */
export const SettingsShell = forwardRef<SettingsShellHandle, SettingsShellProps>(function SettingsShell(
  { variant, onClose, onSettingsChange, onReady, reserveTrafficLightSpace = false, initialTab, hideNav = false, renderSessionCenter, renderPluginCenter, renderReleaseNotes },
  ref,
) {
  const onSettingsChangeRef = useRef(onSettingsChange)
  onSettingsChangeRef.current = onSettingsChange
  const editorControllerRef = useRef<SettingsEditorController | null>(null)
  if (!editorControllerRef.current) {
    editorControllerRef.current = new SettingsEditorController({
      peek: peekSettingsSnapshot,
      load: getSettingsSnapshotCached,
      refresh: refreshSettingsSnapshot,
      save: saveSettingsSnapshotCached,
      subscribe: subscribeSettingsSnapshot,
      import: importSettingsSnapshotCached,
    }, () => onSettingsChangeRef.current())
  }
  const editorController = editorControllerRef.current
  const editorView = useSyncExternalStore(editorController.subscribe, () => editorController.snapshot)
  const { settings, loading, loadError, hasUnsavedChanges } = editorView
  const setSettings = useCallback((update: SetStateAction<SettingsData | null>) => {
    editorController.edit((current) => {
      const next = typeof update === 'function' ? update(current) : update
      return next ?? current
    })
  }, [editorController])
  const [appVersion, setAppVersion] = useState('')
  const [activeTab, setActiveTab] = useState<Exclude<SettingsTab, 'connectors'>>(initialTab === 'connectors' ? 'plugins' : initialTab ?? 'general')
  const [pluginSection, setPluginSection] = useState<'plugins' | 'connectors'>(initialTab === 'connectors' ? 'connectors' : 'plugins')
  const navigateToSettingsTab = useCallback((tab: SettingsTab) => {
    if (tab === 'connectors') {
      setPluginSection('connectors')
      setActiveTab('plugins')
    } else {
      setActiveTab(tab)
    }
  }, [])
  // 使用统计页内的视图：应用用量 / 调用明细 / 请求调试
  const [usageView, setUsageView] = useState<'app' | 'calls' | 'debug'>('app')
  useEffect(() => {
    if (initialTab) navigateToSettingsTab(initialTab)
  }, [initialTab, navigateToSettingsTab])
  // 热键被占用未能注册的警告（保存已成功，只是提醒，不阻断）。
  const [saveWarning, setSaveWarning] = useState('')
  const hotkeyRecorder = useSettingsHotkeyRecorder((update) => editorController.edit(update))
  const recordingTarget = hotkeyRecorder.target
  const [defaultPrompts, setDefaultPrompts] = useState<DefaultPromptTemplates | null>(null)
  const [retryAttemptsInput, setRetryAttemptsInput] = useState('')
  const [uiFontPxInput, setUiFontPxInput] = useState('')
  const [systemFonts, setSystemFonts] = useState<string[]>([])
  const permissions = useSettingsPermissions()
  const providerModals = useProviderModalController(settings?.providers.map((provider) => provider.id) ?? [])
  const selectedProviderId = providerModals.selectedId
  const modelPickerProviderId = providerModals.pickerId
  const confirmDeleteProviderId = providerModals.deleteId
  const drawerModel = providerModals.drawerModel
  const modelTestProviderId = providerModals.testId
  const { closePicker, cancelDelete } = providerModals
  const requestWindowFocus = useWindowInteractionFocus()
  const updates = useSettingsUpdateController(settings ? settings.autoCheckUpdate : null)
  const platform = getPlatform()
  const isMac = platform === 'macos'
  const hasSystemOcr = isMac || platform === 'windows'
  const ocrDownloads = useSettingsOcrDownloads(
    hasSystemOcr,
    settings?.screenshotTranslation?.rapidOcrTier ?? 'standard',
  )
  // 加载失败时的错误信息；非空则渲染错误 UI 而不是用合成默认值进入正常视图
  // （否则用户可能没察觉就自动保存把磁盘真实数据覆盖掉）
  const [reloadKey, setReloadKey] = useState(0)
  const readyEmittedRef = useRef(false)
  const toastClearTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const lang = settings?.settingsLanguage || 'zh'
  const memoryEditor = useSettingsMemoryEditor(undefined, lang, activeTab === 'memory')
  const providerCatalog = useProviderCatalogController({
    fetch: api.fetchModelCatalog,
    getProvider: (id) => editorController.snapshot.settings?.providers.find((provider) => provider.id === id),
    apply: (id, updates) => editorController.edit((current) => applyProviderDraftIntent(current, { type: 'update', id, updates })),
  })
  const settingsBackup = useSettingsBackupController({
    pickImport: async () => {
      const selected = await open({ multiple: false, filters: [{ name: 'JSON', extensions: ['json'] }] })
      return typeof selected === 'string' ? selected : null
    },
    pickExport: () => save({
      defaultPath: 'kivio-settings-backup.json',
      filters: [{ name: 'JSON', extensions: ['json'] }],
    }),
    import: (path) => editorController.import(path),
    export: (path) => api.exportSettings(path),
  }, lang)
  const onboarding = useSettingsOnboardingController({
    flush: () => editorController.flush(),
    writePending: () => updateSettingsCached((current) => ({ ...current, onboardingStatus: 'pending' })),
    committed: () => onSettingsChangeRef.current(),
    navigate: () => { window.location.hash = '#chat/onboarding' },
  }, lang)
  const t = i18n[lang]
  const controllerSaveError = editorView.saveError.startsWith('Settings conflict: ')
    ? `${lang === 'zh' ? '设置冲突：' : 'Settings conflict: '}${editorView.saveError.slice('Settings conflict: '.length)}`
    : editorView.saveError.startsWith('Save failed: ')
      ? `${lang === 'zh' ? '保存失败：' : 'Save failed: '}${formatHotkeyError(editorView.saveError.slice('Save failed: '.length), lang)}`
      : editorView.saveError
  const visibleSaveError = providerCatalog.error
    ? `${lang === 'zh' ? '获取模型失败：' : 'Could not fetch models: '}${providerCatalog.error}`
    : controllerSaveError
  const chatTools = settings?.chatTools
  const nativeBuiltinToolsEnabled = chatTools ? hasEnabledNativeBuiltinTool(chatTools.nativeTools) : false
  const skillRuntimeEnabled = chatTools ? hasEnabledSkillRuntime(chatTools.nativeTools) : false

  // 客户端热键冲突检测:在保存前发现"两个启用功能用了同一个组合"。
  // OS 层面的冲突(Spotlight 占用 Cmd+Space 等)仍需保存后从后端拿到结果。
  // 返回每个 scope 对应的"和谁冲突"——前端各 HotkeyInput 拿到对应 scope 的伙伴名后,
  // 用 hotkeyScope* 模板自己拼本地化字符串。
  const hotkeyConflicts = useMemo<Partial<Record<HotkeyScopeKey, HotkeyScopeKey>>>(() => {
    if (!settings) return {}
    const slots: Array<{ scope: HotkeyScopeKey; hotkey: string; enabled: boolean }> = [
      { scope: 'main', hotkey: settings.hotkey || '', enabled: !!(settings.hotkey || '').trim() },
      { scope: 'chat', hotkey: settings.chatHotkey || '', enabled: !!(settings.chatHotkey || '').trim() },
      {
        scope: 'closeChat',
        hotkey: settings.closeChatHotkey || '',
        enabled: !!(settings.closeChatHotkey || '').trim(),
      },
      {
        scope: 'screenshotTranslation',
        hotkey: settings.screenshotTranslation.hotkey,
        enabled: settings.screenshotTranslation.enabled,
      },
      {
        scope: 'screenshotTranslationText',
        hotkey: settings.screenshotTranslation.textHotkey,
        enabled: settings.screenshotTranslation.enabled,
      },
      {
        scope: 'screenshotTranslationReplace',
        hotkey: settings.screenshotTranslation.replaceHotkey || '',
        enabled: settings.screenshotTranslation.enabled
          && settings.screenshotTranslation.replaceEnabled !== false,
      },
      { scope: 'screenshotAnnotate', hotkey: settings.screenshotAnnotate.hotkey, enabled: settings.screenshotAnnotate.enabled },
      { scope: 'lens', hotkey: settings.lens.hotkey, enabled: settings.lens.enabled },
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

  const SCOPE_I18N_KEY: Record<HotkeyScopeKey, 'hotkeyScopeTranslator' | 'hotkeyScopeChat' | 'hotkeyScopeCloseChat' | 'hotkeyScopeScreenshot' | 'hotkeyScopeScreenshotText' | 'hotkeyScopeScreenshotReplace' | 'annotateHotkeyLabel' | 'hotkeyScopeLens'> = {
    main: 'hotkeyScopeTranslator',
    chat: 'hotkeyScopeChat',
    closeChat: 'hotkeyScopeCloseChat',
    screenshotTranslation: 'hotkeyScopeScreenshot',
    screenshotTranslationText: 'hotkeyScopeScreenshotText',
    screenshotTranslationReplace: 'hotkeyScopeScreenshotReplace',
    screenshotAnnotate: 'annotateHotkeyLabel',
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
    editorController.start()
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
  }, [editorController, reloadKey])

  useEffect(() => () => editorController.dispose(), [editorController])

  // 首屏内容就绪信号：settings 数据就绪或错误态就绪时触发一次。用于让宿主（Chat→App）
  // 把窗口 show 推迟到设置页可渲染之后，避免“窗口已弹出但在转圈”。传了 onReady 就触发，
  // 不再限定 standalone（旧独立设置窗时代的遗留条件）。
  useEffect(() => {
    if (!onReady) return
    if (!loading && !readyEmittedRef.current && (settings || loadError)) {
      readyEmittedRef.current = true
      onReady()
    }
  }, [loadError, loading, onReady, settings])

  const retryAttempts = settings?.retryAttempts

  useEffect(() => {
    if (retryAttempts === undefined) return
    setRetryAttemptsInput(String(retryAttempts))
  }, [retryAttempts])

  const uiFontScale = settings?.uiFontScale
  useEffect(() => {
    if (uiFontScale === undefined) return
    setUiFontPxInput(String(Math.round((uiFontScale ?? 1) * UI_FONT_BASE_PX)))
  }, [uiFontScale])

  const commitUiFontPx = (raw: string, commit: boolean) => {
    if (!settings) return
    const fallback = String(Math.round((settings.uiFontScale ?? 1) * UI_FONT_BASE_PX))
    if (raw.trim() === '') {
      if (commit) setUiFontPxInput(fallback)
      return
    }
    const parsed = Number.parseInt(raw, 10)
    if (Number.isNaN(parsed)) {
      if (commit) setUiFontPxInput(fallback)
      return
    }
    const clamped = Math.min(UI_FONT_PX_MAX, Math.max(UI_FONT_PX_MIN, parsed))
    if (commit) setUiFontPxInput(String(clamped))
    updateSettings({ uiFontScale: clamped / UI_FONT_BASE_PX })
  }

  // 系统已装字体列表，仅拉取一次（前端无法枚举，走后端 CoreText/GDI）。
  useEffect(() => {
    let alive = true
    api.listSystemFonts().then((fonts) => {
      if (alive) setSystemFonts(fonts)
    })
    return () => {
      alive = false
    }
  }, [])

  /**
   * 立即把当前草稿写盘。自动保存与关闭前 flush 共用。
   * 保存中若草稿又变了，收尾后会再跑一轮，避免丢字。
   */
  const persistSettingsNow = useCallback(() => editorController.flush(), [editorController])

  useEffect(() => {
    return () => {
      if (toastClearTimerRef.current) clearTimeout(toastClearTimerRef.current)
    }
  }, [])

  // 错误 / 热键警告：短暂 toast，几秒后自动消失
  useEffect(() => {
    if (!saveWarning) return
    if (toastClearTimerRef.current) clearTimeout(toastClearTimerRef.current)
    toastClearTimerRef.current = setTimeout(() => {
      setSaveWarning('')
      toastClearTimerRef.current = null
    }, 5000)
    return () => {
      if (toastClearTimerRef.current) {
        clearTimeout(toastClearTimerRef.current)
        toastClearTimerRef.current = null
      }
    }
  }, [saveWarning])

  /**
   * 关闭设置页：默认等待 flush 成功；显式 waitForSave=false 才立即退场。
   */
  const handleCloseRequest = useCallback((options?: SettingsCloseOptions) => {
    if (recordingTarget) return
    void editorController.requestClose(onClose, options)
  }, [editorController, onClose, recordingTarget])

  useImperativeHandle(ref, () => ({ requestClose: handleCloseRequest }), [handleCloseRequest])

  const handleSettingsDragMouseDown = useCallback((event: React.MouseEvent<HTMLElement>) => {
    if (event.button !== 0) return
    const target = event.target as HTMLElement | null
    if (target?.closest('button, input, textarea, select, [data-tauri-drag-region="false"]')) return
    event.preventDefault()
    void api.startDragging().catch((err) => {
      console.error('[settings-drag] startDragging failed:', err)
    })
  }, [])

  // 全局键盘：Esc 关闭、Cmd/Ctrl+S 立即 flush；弹窗打开时优先处理弹窗内的 Esc
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (recordingTarget) return

      if (modelPickerProviderId) {
        if (e.key === 'Escape') {
          e.preventDefault()
          e.stopPropagation()
          closePicker()
        }
        return
      }

      // 删除供应商弹窗：Esc 取消；不绑定 Enter，避免误触发破坏性删除
      if (confirmDeleteProviderId) {
        if (e.key === 'Escape') {
          e.preventDefault()
          e.stopPropagation()
          cancelDelete()
        }
        return
      }

      if (e.key === 'Escape') {
        handleCloseRequest({ waitForSave: false })
      } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 's') {
        e.preventDefault()
        if (hasUnsavedChanges) void persistSettingsNow()
      }
    }
    window.addEventListener('keydown', handler, true)
    return () => window.removeEventListener('keydown', handler, true)
  }, [
    handleCloseRequest,
    recordingTarget,
    confirmDeleteProviderId,
    modelPickerProviderId,
    closePicker,
    cancelDelete,
    hasUnsavedChanges,
    persistSettingsNow,
  ])

  // 重试次数输入处理
  const handleRetryAttemptsChange = (value: string) => {
    if (!settings) return
    setRetryAttemptsInput(value)
    if (value.trim() === '') return
    const parsed = Number.parseInt(value, 10)
    if (Number.isNaN(parsed)) return
    const clamped = Math.min(8, Math.max(1, parsed))
    updateSettings({ retryAttempts: clamped })
  }

  const handleRetryAttemptsBlur = () => {
    if (!settings) return
    if (retryAttemptsInput.trim() === '') {
      setRetryAttemptsInput(String(settings.retryAttempts))
      return
    }
    const parsed = Number.parseInt(retryAttemptsInput, 10)
    if (Number.isNaN(parsed)) {
      setRetryAttemptsInput(String(settings.retryAttempts))
      return
    }
    const clamped = Math.min(8, Math.max(1, parsed))
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
  }, [setSettings])

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

  const updateDefaultModel = useCallback((
    key: keyof SettingsData['defaultModels'],
    providerId: string,
    model: string,
  ) => {
    setSettings((prev) => {
      if (!prev) return prev
      const defaultModels = {
        ...prev.defaultModels,
        [key]: { providerId, model },
      }
      return {
        ...prev,
        defaultModels,
        ...(key === 'chat' ? { chatProviderId: providerId, chatModel: model } : {}),
      }
    })
  }, [setSettings])

  const updateChatTools = useCallback((updates: Partial<ChatToolsConfig>) => {
    setSettings((prev) => {
      if (!prev) return prev
      return { ...prev, chatTools: { ...prev.chatTools, ...updates } }
    })
  }, [setSettings])

  const updateNativeTools = useCallback((updates: Partial<ChatNativeToolsConfig>) => {
    setSettings((prev) => {
      if (!prev) return prev
      const chatTools = prev.chatTools
      return {
        ...prev,
        chatTools: {
          ...chatTools,
          nativeTools: {
            ...chatTools.nativeTools,
            ...updates,
          },
        },
      }
    })
  }, [setSettings])

  // 保存后若有热键被系统/其他应用占用未注册，后端推 hotkey-warning——提示但不视为保存失败。
  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | null = null
    void api.onHotkeyWarning((raw) => {
      if (cancelled) return
      const detail = formatHotkeyError(raw, lang).replace(/\n/g, ' / ')
      setSaveWarning(lang === 'zh' ? `已保存,但部分热键未注册:${detail}` : `Saved, but some hotkeys were not registered: ${detail}`)
    }).then((fn) => {
      if (cancelled) fn()
      else unlisten = fn
    })
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [lang])

  const dispatchProviderIntent = useCallback((intent: ProviderDraftIntent) => {
    setSettings((current) => current ? applyProviderDraftIntent(current, intent) : current)
  }, [setSettings])

  const updateProvider = useCallback((id: string, updates: Partial<ModelProvider>) => {
    dispatchProviderIntent({ type: 'update', id, updates })
  }, [dispatchProviderIntent])

  const setProviderIcon = useCallback((id: string, iconKey: string) => {
    dispatchProviderIntent({ type: 'icon', id, iconKey })
  }, [dispatchProviderIntent])

  const reorderProviders = useCallback((fromId: string, toId: string) => {
    dispatchProviderIntent({ type: 'reorder', fromId, toId })
  }, [dispatchProviderIntent])

  /**
   * 添加新提供商
   */
  const addProvider = () => {
    if (!settings) return
    const newId = `provider-${Date.now()}`
    dispatchProviderIntent({ type: 'add', id: newId })
    providerModals.select(newId)
  }

  /** 用预设一键添加 provider —— baseUrl 和默认模型已填好，用户只需填 API key */
  const addProviderFromPreset = (preset: ProviderPreset) => {
    if (!settings) return
    const newId = `provider-${Date.now()}`
    dispatchProviderIntent({ type: 'add', id: newId, preset })
    providerModals.select(newId)
  }

  const deleteProvider = (id: string) => {
    if (!settings) return
    providerModals.providerDeleted(id)
    dispatchProviderIntent({ type: 'delete', id })
  }

  /**
   * 添加已启用模型
   */
  const addEnabledModel = (providerId: string, model: string) => {
    dispatchProviderIntent({ type: 'add-model', id: providerId, model })
  }

  const addAllEnabledModels = (providerId: string, models: string[]) => {
    dispatchProviderIntent({ type: 'add-models', id: providerId, models })
  }

  /**
   * 移除已启用模型
   * 移除后会自动更新使用该模型的功能到新的默认模型
   */
  const removeEnabledModel = (providerId: string, model: string) => {
    dispatchProviderIntent({ type: 'remove-model', id: providerId, model })
  }

  /**
   * 保存模型自定义参数
   */
  const saveModelOverride = useCallback((providerId: string, modelName: string, info: ModelInfo) => {
    dispatchProviderIntent({ type: 'save-override', id: providerId, model: modelName, info })
  }, [dispatchProviderIntent])

  /**
   * 重置模型参数为数据库默认值
   */
  const resetModelOverride = useCallback((providerId: string, modelName: string) => {
    dispatchProviderIntent({ type: 'reset-override', id: providerId, model: modelName })
  }, [dispatchProviderIntent])

  const openModelPicker = (providerId: string) => {
    providerModals.openPicker(providerId)
  }

  /**
   * 更新截图翻译配置
   */
  const updateScreenshotTranslation = useCallback((updates: Partial<SettingsData['screenshotTranslation']>) => {
    setSettings((prev) => {
      if (!prev) return prev
      return { ...prev, screenshotTranslation: { ...prev.screenshotTranslation, ...updates } }
    })
  }, [setSettings])

  /**
   * 更新截图标注配置
   */
  const updateScreenshotAnnotate = useCallback((updates: Partial<NonNullable<SettingsData['screenshotAnnotate']>>) => {
    setSettings((prev) => {
      if (!prev) return prev
      return { ...prev, screenshotAnnotate: { ...prev.screenshotAnnotate, ...updates } }
    })
  }, [setSettings])

  /**
   * 更新 Lens 配置
   */
  const updateLens = useCallback((updates: Partial<SettingsData['lens']>) => {
    setSettings((prev) => {
      if (!prev) return prev
      return { ...prev, lens: { ...prev.lens, ...updates } }
    })
  }, [setSettings])

  const updateChat = useCallback((updates: Partial<NonNullable<SettingsData['chat']>>) => {
    setSettings((prev) => {
      if (!prev) return prev
      return { ...prev, chat: { ...prev.chat, ...updates } }
    })
  }, [setSettings])

  const updateChatMemory = useCallback((updates: Partial<ChatMemoryConfig>) => {
    setSettings((prev) => {
      if (!prev) return prev
      return { ...prev, chatMemory: { ...prev.chatMemory, ...updates } }
    })
  }, [setSettings])

  const updateLensWebSearch = useCallback((updates: Partial<NonNullable<SettingsData['lens']['webSearch']>>) => {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        lens: {
          ...prev.lens,
          webSearch: {
            ...prev.lens.webSearch!,
            ...updates,
          },
        },
      }
    })
  }, [setSettings])

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
              className="flex items-center gap-1.5 text-[12px] font-medium px-3 py-1.5 rounded-md bg-neutral-900 dark:bg-white text-white dark:text-neutral-900 hover:bg-neutral-800 dark:hover:bg-neutral-100 transition-colors duration-[var(--kv-dur-fast)]"
              data-tauri-drag-region="false"
            >
              <RefreshCw size={12} />
              {lang === 'zh' ? '重试' : 'Retry'}
            </button>
            <button
              type="button"
              onClick={onClose}
              className="text-[12px] font-medium px-3 py-1.5 rounded-md text-neutral-600 dark:text-neutral-400 hover:bg-black/5 dark:hover:bg-white/5 transition-colors duration-[var(--kv-dur-fast)]"
              data-tauri-drag-region="false"
            >
              {t.cancel}
            </button>
          </div>
        </div>
      </div>
    )
  }

  // These values come from the canonical backend response. Do not synthesize persisted defaults
  // in the webview: legacy omissions are migrated before get_settings returns.
  const lensDefaults = defaultPrompts?.lensPrompts?.[settings.lens.defaultLanguage === 'en' ? 'en' : 'zh']
  const chatLangKey = settings.chat.defaultLanguage === 'en' ? 'en' : 'zh'
  const chatDefaults = defaultPrompts?.chatPrompts?.[chatLangKey]
  const chatRuntimeDefaults = defaultPrompts?.chatRuntimePrompt
  const chatConfig = settings.chat
  const themeColor = settings.themeColor
  const chatMemory = settings.chatMemory
  const effectiveChatMaxOutput = resolveEffectiveChatMaxOutput(settings, chatConfig.maxOutputTokens)
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

  const navItems = [
    { id: 'general' as const, label: t.tabGeneral, icon: GeneralIcon },
    { id: 'providers' as const, label: t.tabModels, icon: ProvidersIcon },
    { id: 'hotkeys' as const, label: t.tabHotkeys, icon: HotkeysIcon },
    { id: 'translate' as const, label: t.tabTranslation, icon: TranslateIcon },
    { id: 'lens' as const, label: t.lensTabLabel, icon: LensIcon },
    { id: 'chat' as const, label: t.tabChatClient, icon: ChatIcon },
    { id: 'memory' as const, label: t.tabMemory, icon: MemoryIcon },
    { id: 'mixer' as const, label: t.tabMixer, icon: MixerIcon },
    { id: 'externalAgents' as const, label: t.tabExternalAgents, icon: AgentIcon },
    { id: 'computerControl' as const, label: lang === 'zh' ? '电脑操控' : 'Computer control', icon: Monitor },
    { id: 'hooks' as const, label: t.tabHooks, icon: HooksIcon },
    { id: 'plugins' as const, label: t.tabPlugins, icon: PluginsIcon },
    { id: 'sessions' as const, label: t.tabSessions, icon: SessionsIcon },
    { id: 'webSearch' as const, label: t.tabWebSearch, icon: WebSearchIcon },
    { id: 'usage' as const, label: lang === 'zh' ? '使用统计' : 'Usage', icon: UsageIcon },
    // 关于固定在分类列表最末
    { id: 'about' as const, label: lang === 'zh' ? '关于' : 'About', icon: AboutIcon },
  ]
  const pageMeta: Record<typeof activeTab, { title: string; subtitle: string; right?: string }> = {
    general: {
      title: t.tabGeneral,
      subtitle: lang === 'zh' ? '外观、行为、归档和权限。' : 'Appearance, behavior, archive, and permissions.',
    },
    translate: {
      title: t.tabTranslation,
      subtitle: lang === 'zh' ? '输入翻译与快速翻译的语言、模型、OCR 和提示词。' : 'Language, model, OCR, and prompts for input and quick translation.',
    },
    hotkeys: {
      title: t.tabHotkeys,
      subtitle: lang === 'zh' ? '集中管理所有全局快捷键。' : 'Manage all global hotkeys in one place.',
    },
    lens: {
      title: t.lensTabLabel,
      subtitle: lang === 'zh' ? '视觉问答的快捷键、响应方式和提示词。' : 'Shortcut, response behavior, and prompts for visual Q&A.',
    },
    chat: {
      title: t.tabChatClient,
      subtitle: lang === 'zh'
        ? '主对话模型、流式/思考、系统提示词。'
        : 'Main chat model, streaming/thinking, and system prompt.',
    },
    memory: {
      title: t.tabMemory,
      subtitle: lang === 'zh'
        ? 'L1 注入；L2 按需读取。'
        : 'L1 injected; L2 read on demand.',
    },
    mixer: {
      title: t.tabMixer,
      subtitle: lang === 'zh'
        ? '按副任务路由模型：视觉、标题总结、上下文压缩、生图。'
        : 'Route models by side task: vision, title summaries, context compression, and image generation.',
    },
    computerControl: {
      title: lang === 'zh' ? '电脑操控' : 'Computer control',
      subtitle: lang === 'zh' ? '管理桌面、浏览器和文档操作工具。' : 'Manage desktop, browser, and document control tools.',
    },
    externalAgents: {
      title: t.tabExternalAgents,
      subtitle: lang === 'zh'
        ? '检测外部 CLI 编码代理，管理版本、路径、模型与环境变量。'
        : 'Detect external CLI coding agents; manage versions, paths, models, and env vars.',
    },
    hooks: {
      title: t.tabHooks,
      subtitle: t.hooksPageSubtitle,
    },
    plugins: {
      title: pluginSection === 'plugins' ? t.tabPlugins : t.tabConnectors,
      subtitle: pluginSection === 'plugins' ? t.pluginCenterPluginsSubtitle : t.pluginCenterConnectorsSubtitle,
    },
    sessions: {
      title: t.tabSessions,
      subtitle: t.chatSessionSubtitle,
    },
    webSearch: {
      title: t.tabWebSearch,
      subtitle: lang === 'zh'
        ? 'Tavily/Exa 密钥；分别开启 Lens 与 Chat 联网。'
        : 'Tavily/Exa keys; enable web search for Lens and Chat.',
    },
    usage: {
      title: lang === 'zh' ? '使用统计' : 'Usage',
      subtitle: '',
    },
    providers: {
      title: t.tabModels,
      subtitle: lang === 'zh' ? '管理 OpenAI 兼容供应商、密钥和启用模型。' : 'Manage OpenAI-compatible providers, keys, and enabled models.',
    },
    about: {
      title: lang === 'zh' ? '关于' : 'About',
      subtitle: lang === 'zh' ? '应用、版本和更新信息。' : 'Application, version, and update details.',
    },
  }
  const selectedProvider = settings.providers.find((provider) => provider.id === selectedProviderId) ?? settings.providers[0]
  const chatProvider = settings.providers.find((provider) => provider.id === settings.chatProviderId)
    ?? settings.providers.find((provider) => provider.id === settings.lens?.providerId)
    ?? settings.providers.find((provider) => provider.id === settings.translatorProviderId)

  const categoryNav =
    variant === 'embedded' ? (
      <>
        <nav className="settings-embedded-nav-list custom-scrollbar">
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
            onClick={() => handleCloseRequest({ waitForSave: false })}
            className="settings-embedded-back"
            title={lang === 'zh' ? '返回对话' : 'Back to chat'}
            data-tauri-drag-region="false"
          >
            <span className="settings-embedded-nav-icon">
              <ArrowLeft size={17} strokeWidth={1.75} />
            </span>
            <span>{lang === 'zh' ? '返回对话' : 'Back to chat'}</span>
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
      </>
    )

  const settingsMain = (
        <main className={`kv-content ${variant === 'embedded' ? 'settings-embedded-main' : ''}${activeTab === 'sessions' ? ' kv-content--wide' : ''}`}>
          <header
            className={`kv-page-header ${variant === 'embedded' ? 'settings-embedded-header' : ''}`}
            onMouseDown={handleSettingsDragMouseDown}
          >
            <div key={activeTab} className={`settings-section-title-enter${activeTab === 'usage' ? ' settings-usage-title' : ''}`}>
              <div className="flex items-center gap-3">
                <div className="kv-page-title">{pageMeta[activeTab].title}</div>
                {activeTab === 'usage' && (
                  <div className="inline-flex items-center gap-0.5 rounded-full bg-[var(--bg-input-subtle)] p-0.5" data-tauri-drag-region="false">
                    {([
                      { id: 'app' as const, label: lang === 'zh' ? '应用用量' : 'App usage' },
                      { id: 'calls' as const, label: lang === 'zh' ? '调用明细' : 'Call details' },
                      { id: 'debug' as const, label: lang === 'zh' ? '请求调试' : 'Request debug' },
                    ]).map(option => (
                      <button
                        key={option.id}
                        type="button"
                        className={`rounded-full px-2.5 py-0.5 text-[12px] leading-5 ${
                          usageView === option.id
                            ? 'bg-[var(--bg)] text-[var(--text)] shadow-sm'
                            : 'text-[var(--text-muted)]'
                        }`}
                        onClick={() => setUsageView(option.id)}
                        data-tauri-drag-region="false"
                      >
                        {option.label}
                      </button>
                    ))}
                  </div>
                )}
              </div>
              {pageMeta[activeTab].subtitle ? (
                <div className="kv-page-sub">{pageMeta[activeTab].subtitle}</div>
              ) : null}
            </div>
            <div className="kv-page-header-right">{pageMeta[activeTab].right}</div>
          </header>

          <div
            key={activeTab}
            className={`kv-scroll custom-scrollbar settings-section-enter ${variant === 'embedded' ? 'settings-embedded-scroll' : ''}${activeTab === 'sessions' ? ' kv-scroll--fill' : ''}`}
          >
            {/* ===== 基础设置标签页 ===== */}
            {activeTab === 'general' && (
              <>
                <AppearanceGroup
                  settings={settings}
                  t={t}
                  lang={lang}
                  themeColor={themeColor}
                  systemFonts={systemFonts}
                  uiFontPxInput={uiFontPxInput}
                  onUpdateSettings={updateSettings}
                  onUiFontPxInputChange={setUiFontPxInput}
                  onCommitUiFontPx={commitUiFontPx}
                />

                <BehaviorGroup
                  settings={settings}
                  t={t}
                  lang={lang}
                  retryAttemptsInput={retryAttemptsInput}
                  onUpdateSettings={updateSettings}
                  onRetryAttemptsChange={handleRetryAttemptsChange}
                  onRetryAttemptsBlur={handleRetryAttemptsBlur}
                />

                <SettingsGroup title={lang === 'zh' ? '首次使用' : 'First-time setup'}>
                  <SettingRow
                    label={lang === 'zh' ? '首次使用引导' : 'Setup wizard'}
                    description={t.onboardingRestartDesc}
                  >
                    <Button
                      size="sm"
                      onClick={() => void onboarding.restart()}
                      disabled={onboarding.busy}
                      data-tauri-drag-region="false"
                    >
                      {t.onboardingRestart}
                    </Button>
                    {onboarding.error && <span className="text-[12px] text-red-500" role="status">{onboarding.error}</span>}
                  </SettingRow>
                </SettingsGroup>

                <SettingsGroup title={lang === 'zh' ? '备份与恢复' : 'Backup & Restore'}>
                  <FieldBlock
                    label={lang === 'zh' ? '设置备份' : 'Settings backup'}
                    description={lang === 'zh'
                      ? '导出/导入全部设置（含 API Key）。'
                      : 'Export/import all settings (incl. API keys).'}
                  >
                    <div className="flex flex-wrap items-center gap-2">
                      <Button
                        size="sm"
                        onClick={settingsBackup.exportBackup}
                        disabled={settingsBackup.busy}
                        data-tauri-drag-region="false"
                      >
                        <Download size={11} />
                        {lang === 'zh' ? '导出设置' : 'Export'}
                      </Button>
                      <Button
                        size="sm"
                        onClick={settingsBackup.importBackup}
                        disabled={settingsBackup.busy}
                        data-tauri-drag-region="false"
                      >
                        <Upload size={11} />
                        {lang === 'zh' ? '导入设置' : 'Import'}
                      </Button>
                      {settingsBackup.status && (
                        <span className={`text-[12px] ${settingsBackup.status.kind === 'ok' ? 'text-emerald-600 dark:text-emerald-400' : 'text-red-500'}`}>
                          {settingsBackup.status.msg}
                        </span>
                      )}
                    </div>
                  </FieldBlock>
                </SettingsGroup>

                {permissions.status?.platform === 'macos' && (
                  <PermissionsGroup
                    t={t}
                    permissionStatus={permissions.status}
                    permissionsLoading={permissions.loading}
                    onOpenPermissionSettings={permissions.open}
                    onRefreshPermissions={permissions.refresh}
                  />
                )}
              </>
            )}

            {/* ===== 翻译设置标签页 ===== */}
            {activeTab === 'translate' && (
              <>
                <TranslateTab
                  settings={settings}
                  t={t}
                  lang={lang}
                  defaultPrompts={defaultPrompts}
                  onUpdateSettings={updateSettings}
                />

                <div className="kv-section-title">{t.tabScreenshot}</div>
                <ScreenshotTranslationSettings
                  settings={settings}
                  isMac={isMac}
                  hasSystemOcr={hasSystemOcr}
                  defaultPrompts={defaultPrompts}
                  rapidOcrStatus={ocrDownloads.rapidStatus}
                  rapidOcrDownloadState={ocrDownloads.rapidDownloadState}
                  rapidOcrDownloadError={ocrDownloads.rapidDownloadError}
                  replacePackStatus={ocrDownloads.replaceStatus}
                  replacePackDownloadState={ocrDownloads.replaceDownload.downloadState}
                  replacePackDownloadError={ocrDownloads.replaceDownload.error}
                  replacePackProgress={ocrDownloads.replaceDownload.progress}
                  t={t}
                  onUpdate={updateScreenshotTranslation}
                  onRefreshRapidOcrStatus={ocrDownloads.refreshRapid}
                  onDownloadRapidOcr={ocrDownloads.downloadRapid}
                  onRefreshReplacePack={ocrDownloads.refreshReplace}
                  onDownloadReplacePack={ocrDownloads.downloadReplace}
                />
              </>
            )}

            {/* ===== 快捷键标签页：集中所有全局热键 ===== */}
            {activeTab === 'hotkeys' && (
              <HotkeysTab
                settings={settings}
                t={t}
                recordingTarget={recordingTarget}
                onToggleRecording={hotkeyRecorder.toggle}
                conflictMessageFor={conflictMessageFor}
                hotkeyConflicts={hotkeyConflicts}
                onUpdateSettings={updateSettings}
                onUpdateScreenshotTranslation={updateScreenshotTranslation}
                onUpdateScreenshotAnnotate={updateScreenshotAnnotate}
                onUpdateLens={updateLens}
              />
            )}

            {/* ===== Lens 标签页 ===== */}
            {activeTab === 'lens' && (
              <LensTab
                settings={settings}
                t={t}
                lang={lang}
                lensDefaults={lensDefaults}
                onUpdateSettings={updateSettings}
                onUpdateLens={updateLens}
              />
            )}

            {/* ===== AI 客户端标签页 ===== */}
            {activeTab === 'chat' && (
              <ChatTab
                settings={settings}
                t={t}
                lang={lang}
                chatConfig={chatConfig}
                chatTools={settings.chatTools}
                chatMemory={chatMemory}
                chatDefaults={chatDefaults}
                chatRuntimeDefaults={chatRuntimeDefaults}
                effectiveChatMaxOutput={effectiveChatMaxOutput}
                chatMaxOutputSourceLabel={chatMaxOutputSourceLabel}
                chatMaxOutputModelLabel={chatMaxOutputModelLabel}
                skillRuntimeEnabled={skillRuntimeEnabled}
                nativeBuiltinToolsEnabled={nativeBuiltinToolsEnabled}
                onUpdateChat={updateChat}
                onUpdateNativeTools={updateNativeTools}
                onNavigateTab={navigateToSettingsTab}
              />
            )}

            {/* ===== 记忆标签页 ===== */}
            {activeTab === 'memory' && (
              <MemoryTab
                lang={lang}
                chatMemory={chatMemory}
                memoryDir={memoryEditor.view.dir}
                memoryError={memoryEditor.view.error}
                memorySuccess={memoryEditor.view.success}
                memoryLoading={memoryEditor.view.loading}
                memorySavingLayer={memoryEditor.view.savingLayer}
                memoryDrafts={memoryEditor.view.drafts}
                memorySnapshots={memoryEditor.view.snapshots}
                onUpdateChatMemory={updateChatMemory}
                onRefresh={() => void memoryEditor.refresh()}
                onOpenFolder={() => void memoryEditor.openFolder()}
                onDraftChange={memoryEditor.edit}
                onSaveLayer={(layer) => void memoryEditor.save(layer)}
              />
            )}

            {/* ===== 混音器标签页 ===== */}
            {activeTab === 'mixer' && (
              <MixerTab
                settings={settings}
                t={t}
                lang={lang}
                chatTools={settings.chatTools}
                hasChatProvider={Boolean(chatProvider)}
                defaultPromptOptimize={defaultPrompts?.promptOptimizePrompts?.[lang] ?? ''}
                onUpdateDefaultModel={updateDefaultModel}
                onUpdateChatTools={updateChatTools}
                onUpdateChat={updateChat}
              />
            )}

            {activeTab === 'externalAgents' && (
              <ExternalAgentsSettings
                lang={lang}
                settings={settings}
                updateChat={updateChat}
              />
            )}

            {activeTab === 'computerControl' && (
              <ComputerControlTab lang={lang} tools={settings.chatTools} onChange={updateChatTools} />
            )}

            {/* ===== Hooks 标签页（对话生命周期） ===== */}
            {activeTab === 'hooks' && (
              <HooksTab
                lang={lang}
                hooks={settings.chatTools.hooks ?? []}
                onChange={(hooks) => updateChatTools({ hooks })}
              />
            )}

            {/* ===== 插件与连接器；第三方应用入口已删除 ===== */}
            {activeTab === 'plugins' && (
              renderPluginCenter({
                section: pluginSection,
                onSectionChange: setPluginSection,
                lang,
                connectors:
                  <ConnectorsPanel
                    servers={settings.chatTools.servers}
                    updateChatTools={updateChatTools}
                    obsidianVaultPath={settings?.obsidianVaultPath ?? ''}
                    onObsidianVaultPathChange={(path) => updateSettings({ obsidianVaultPath: path })}
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
                  />,
              })
            )}

            {activeTab === 'sessions' && renderSessionCenter?.(lang)}

            {/* ===== 网络搜索标签页 ===== */}
            {activeTab === 'webSearch' && (
              <WebSearchPanel
                t={t}
                lang={lang}
                webSearch={settings.lens?.webSearch}
                onChange={updateLensWebSearch}
              />
            )}

            {/* ===== 使用统计：应用用量 / 调用明细 / 请求调试 ===== */}
            {activeTab === 'usage' && (
              usageView === 'debug' ? (
                <RequestDebugPanel
                  lang={lang}
                  enabled={settings.chatTools.requestDebugEnabled ?? false}
                  onToggleEnabled={(v) => updateChatTools({ requestDebugEnabled: v })}
                />
              ) : (
                <UsageStatsPanel lang={lang} view={usageView} />
              )
            )}

            {/* ===== 模型管理标签页 ===== */}
            {activeTab === 'providers' && (
              <ProvidersTab
                settings={settings}
                t={t}
                lang={lang}
                selectedProvider={selectedProvider}
                revealedKeys={revealedKeys}
                gzipInfoOpen={gzipInfoOpen}
                onSelectProvider={providerModals.select}
                onReorderProviders={reorderProviders}
                onAddProvider={addProvider}
                onAddProviderFromPreset={addProviderFromPreset}
                onUpdateProvider={updateProvider}
                onSetProviderIcon={setProviderIcon}
                onRequestDeleteProvider={providerModals.requestDelete}
                onToggleGzipInfo={(id) => setGzipInfoOpen((prev) => {
                  const next = new Set(prev)
                  if (next.has(id)) next.delete(id)
                  else next.add(id)
                  return next
                })}
                onToggleKeyReveal={toggleKeyReveal}
                onOpenModelPicker={openModelPicker}
                onOpenModelTest={providerModals.openTest}
                onOpenModelDrawer={providerModals.openDrawer}
                onRemoveEnabledModel={removeEnabledModel}
              />
            )}

            {/* ===== 关于标签页 ===== */}
            {activeTab === 'about' && (
              <>
                <AppInfoGroup t={t} lang={lang} appVersion={appVersion} />

                <UpdateGroup
                  settings={settings}
                  t={t}
                  update={{
                    status: updates.status,
                    info: updates.info,
                    downloadState: updates.downloadState,
                    downloadPercent: updates.downloadPercent,
                    downloadError: updates.downloadError,
                  }}
                  onUpdateSettings={updateSettings}
                  onCheck={updates.check}
                  onDownloadAndInstall={updates.downloadAndInstall}
                  onInstall={updates.install}
                  onOpenReleasePage={updates.openReleasePage}
                  onOpenGithubReleases={updates.openGithubReleases}
                  onDismiss={updates.dismiss}
                  renderReleaseNotes={renderReleaseNotes}
                />
              </>
            )}
          </div>

          {(visibleSaveError || saveWarning) && (
            <div
              className={`settings-autosave-toast ${visibleSaveError ? 'error' : 'warn'}`}
              role="status"
              title={visibleSaveError || saveWarning}
              data-tauri-drag-region="false"
            >
              {visibleSaveError || saveWarning}
            </div>
          )}
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
            noModels: lang === 'zh' ? '没有可用模型。点刷新重试，或手动添加。' : 'No models yet. Refresh or add one manually.',
            noSearchResults: lang === 'zh' ? '没有匹配的模型' : 'No matching models',
            enabled: lang === 'zh' ? '已启用' : 'On',
            addAllModels: lang === 'zh' ? '添加当前列表中的全部模型' : 'Add all models in the current list',
            close: lang === 'zh' ? '关闭' : 'Close',
          }}
          fetching={providerCatalog.fetchingProviderId === modelPickerProvider.id}
          onClose={providerModals.closePicker}
          onFetch={() => void providerCatalog.fetchModels(modelPickerProvider.id)}
          onAdd={(model) => addEnabledModel(modelPickerProvider.id, model)}
          onAddAll={(models) => addAllEnabledModels(modelPickerProvider.id, models)}
          onRemove={(model) => removeEnabledModel(modelPickerProvider.id, model)}
        />
      )}
      {/* 模型详情抽屉 */}
      {drawerModel && settings && (
        <ModelDetailDrawer
          modelName={drawerModel.model}
          provider={settings.providers.find(p => p.id === drawerModel.providerId)}
          overrides={settings.providers.find(p => p.id === drawerModel.providerId)?.modelOverrides}
          lang={lang}
          onClose={providerModals.closeDrawer}
          onSave={(modelName, info) => {
            saveModelOverride(drawerModel.providerId, modelName, info)
            providerModals.closeDrawer()
          }}
          onReset={(modelName) => resetModelOverride(drawerModel.providerId, modelName)}
        />
      )}
      {modelTestProviderId && settings && (() => {
        const p = settings.providers.find(pv => pv.id === modelTestProviderId)
        if (!p) return null
        return (
          <ProviderModelTestModal
            providerId={p.id}
            baseUrl={p.baseUrl}
            apiKeys={p.apiKeys}
            activeKeyIndex={p.activeKeyIndex}
            apiFormat={p.apiFormat}
            request={p.request}
            models={p.enabledModels}
            lang={lang}
            onClose={providerModals.closeTest}
          />
        )
      })()}
      {/* 删除提供商确认弹窗 */}
      {confirmDeleteProviderId && (
        <div className="kv-modal-backdrop" data-tauri-drag-region="false">
          <div className="kv-modal space-y-3">
            <h3 className="text-[14px] font-semibold">{t.confirmDeleteProvider}</h3>
            <p className="kv-panel-body">{t.confirmDeleteProviderDesc}</p>
            <div className="flex justify-end gap-2 pt-1">
              <Button
                onClick={providerModals.cancelDelete}
                data-tauri-drag-region="false"
              >
                {t.cancel}
              </Button>
              <Button
                variant="danger"
                onClick={() => {
                  if (confirmDeleteProviderId) deleteProvider(confirmDeleteProviderId)
                  providerModals.cancelDelete()
                }}
                data-tauri-drag-region="false"
              >
                {t.deleteProvider}
              </Button>
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
        {!hideNav && (
          <aside className="settings-embedded-nav">
            <h2 className="settings-embedded-nav-title" onMouseDown={handleSettingsDragMouseDown}>
              {t.settings}
            </h2>
            {categoryNav}
          </aside>
        )}
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
          onClick={() => handleCloseRequest()}
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
            <div className="kv-sidebar-brand-name">Kivio Desktop</div>
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
