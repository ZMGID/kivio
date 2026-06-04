import { useState, useEffect, useCallback, useMemo, useRef } from 'react'
import {
  X, Check, Plus, Trash2, RefreshCw,
  Settings as SettingsIcon, Languages, Camera,
  Cloud, Info, Aperture, ExternalLink, Download, ChevronRight
} from 'lucide-react'
import { open } from '@tauri-apps/plugin-dialog'
import ReactMarkdown from 'react-markdown'
import { api, type Settings as SettingsType, type ModelProvider, type DefaultPromptTemplates, type PermissionStatus, type UpdateInfo, type LensQuickAction } from './api/tauri'
import { i18n } from './settings/i18n'
import { buildHotkey, formatHotkeyError, getPlatform, stableStringify } from './settings/utils'
import { PROVIDER_PRESETS, type ProviderPreset } from './settings/providerPresets'
import { ModelPairSelect } from './settings/ModelPairSelect'
import { ScreenshotTranslationSettings } from './settings/ScreenshotTranslationSettings'
import { useWindowInteractionFocus } from './utils/windowFocus'
import {
  Toggle, Select, Input, TextArea, Label,
  SettingRow, PermissionItem, HotkeyInput, DefaultPrompt,
  SettingsGroup,
} from './settings/components'

type SettingsData = SettingsType

interface SettingsProps {
  onClose: () => void
  onSettingsChange: () => void
  onReady?: () => void
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

/**
 * 设置面板主组件
 * 提供基础设置、翻译设置、截图设置、模型管理四大标签页
 */
export default function Settings({ onClose, onSettingsChange, onReady }: SettingsProps) {
  const [settings, setSettings] = useState<SettingsData | null>(null)
  const [initialSettingsSnapshot, setInitialSettingsSnapshot] = useState('')
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [appVersion, setAppVersion] = useState('')
  const [activeTab, setActiveTab] = useState<'general' | 'translate' | 'screenshot' | 'lens' | 'providers' | 'about'>('general')
  const [saveError, setSaveError] = useState('')
  const [saveSuccess, setSaveSuccess] = useState(false)
  const [closeConfirmOpen, setCloseConfirmOpen] = useState(false)
  const [confirmDeleteProviderId, setConfirmDeleteProviderId] = useState<string | null>(null)
  const [recordingTarget, setRecordingTarget] = useState<null | 'main' | 'screenshotTranslation' | 'screenshotTranslationText' | 'lens'>(null)
  const [defaultPrompts, setDefaultPrompts] = useState<DefaultPromptTemplates | null>(null)
  const [retryAttemptsInput, setRetryAttemptsInput] = useState('')
  const [permissionStatus, setPermissionStatus] = useState<PermissionStatus | null>(null)
  const [permissionsLoading, setPermissionsLoading] = useState(false)
  const [testingProviderId, setTestingProviderId] = useState<string | null>(null)
  const [providerTestFeedback, setProviderTestFeedback] = useState<Record<string, { ok: boolean; message: string }>>({})
  const [selectedProviderId, setSelectedProviderId] = useState('')
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
  const [rapidOcrStatus, setRapidOcrStatus] = useState<import('./api/tauri').RapidOcrStatus | null>(null)
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
    if (!loading && !readyEmittedRef.current && (settings || loadError)) {
      readyEmittedRef.current = true
      onReady?.()
    }
  }, [loadError, loading, onReady, settings])

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

  /**
   * 更新指定提供商配置
   */
  const updateProvider = (id: string, updates: Partial<ModelProvider>) => {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        providers: prev.providers.map(p => p.id === id ? { ...p, ...updates } : p)
      }
    })
  }

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
      enabledModels: []
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
      // 端上 provider(Apple Intelligence)不需 API key,填一个哨兵字符串绕开"Missing API Key"检查
      apiKeys: preset.onDevice ? ['__on_device__'] : [],
      baseUrl: preset.baseUrl,
      availableModels: [],
      enabledModels: [],
    }
    setSettings({
      ...settings,
      providers: [...settings.providers, newProvider]
    })
    setSelectedProviderId(newId)
  }

  /**
   * 根据 ID 查找提供商（找不到则返回第一个）
   */
  const resolveProvider = (providers: ModelProvider[], providerId: string) => {
    return providers.find(p => p.id === providerId) ?? providers[0]
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
    const nextProviders = settings.providers.filter(p => p.id !== id)
    const translatorProvider = resolveProvider(nextProviders, settings.translatorProviderId)
    const screenshotProvider = resolveProvider(nextProviders, settings.screenshotTranslation?.providerId || '')
    // lens providerId 为空表示 fallback 到 translator，删除时若已设置自身 provider 才需要级联
    const lensHadOwnProvider = !!settings.lens?.providerId
    const lensProvider = lensHadOwnProvider
      ? resolveProvider(nextProviders, settings.lens?.providerId || '')
      : undefined

    setSettings({
      ...settings,
      providers: nextProviders,
      translatorProviderId: translatorProvider ? translatorProvider.id : '',
      translatorModel: resolveModel(translatorProvider, settings.translatorModel),
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

      if (prev.translatorProviderId === providerId) {
        next.translatorModel = resolveAfterRemoval(prev.translatorModel)
      }

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

  const [fetchingProviderId, setFetchingProviderId] = useState<string | null>(null)
  const [manualInputs, setManualInputs] = useState<Record<string, string>>({})

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
        messageOrder: 'asc' as const,
        showCaptureHint: true,
        windowsFreezeFrameSelection: false,
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

  const updateLensQuickActions = useCallback((actions: LensQuickAction[]) => {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        lens: {
          ...prev.lens,
          quickActions: actions,
        },
      }
    })
  }, [])

  /**
   * 切换快捷键录制状态
   */
  const toggleRecording = (target: 'main' | 'screenshotTranslation' | 'screenshotTranslationText' | 'lens') => {
    setRecordingTarget((current) => (current === target ? null : target))
  }

  // 当前语言对应的默认 lens 提示词
  const lensDefaults = defaultPrompts?.lensPrompts?.[settings?.lens?.defaultLanguage === 'en' ? 'en' : 'zh']

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

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full bg-neutral-200 dark:bg-black">
        <div className="w-6 h-6 border-2 border-neutral-300 dark:border-neutral-700 border-t-neutral-800 dark:border-t-neutral-200 rounded-full animate-spin" />
      </div>
    )
  }

  if (loadError || !settings) {
    // 加载失败：显示错误 + 重试按钮，禁止用户在不知情的情况下用合成默认值 Save 覆盖磁盘
    return (
      <div className="flex items-center justify-center h-full bg-neutral-200 dark:bg-black p-6">
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
    { id: 'general' as const, label: t.tabGeneral, icon: SettingsIcon },
    { id: 'translate' as const, label: t.tabTranslate, icon: Languages },
    { id: 'screenshot' as const, label: t.tabScreenshot, icon: Camera },
    { id: 'lens' as const, label: t.lensTabLabel, icon: Aperture },
    { id: 'providers' as const, label: t.tabModels, icon: Cloud },
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

  return (
    <div
      className="kv kv-window"
      onPointerEnter={requestWindowFocus}
      onPointerMove={requestWindowFocus}
      onPointerDownCapture={requestWindowFocus}
    >
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
              <Info strokeWidth={1.7} />
              <span>{lang === 'zh' ? '关于' : 'About'}</span>
            </button>
          </nav>
        </aside>

        <main className="kv-content">
          <header className="kv-page-header">
            <div>
              <div className="kv-page-title">{pageMeta[activeTab].title}</div>
              <div className="kv-page-sub">{pageMeta[activeTab].subtitle}</div>
            </div>
            <div className="kv-page-header-right">{pageMeta[activeTab].right}</div>
          </header>

          <div className="kv-scroll">
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
                </SettingsGroup>

                <SettingsGroup title={lang === 'zh' ? '行为' : 'Behavior'}>
                  <SettingRow label={t.launchAtStartup} description={lang === 'zh' ? '登录后在后台启动 Kivio。' : 'Open Kivio in the background when you sign in.'}>
                    <Toggle
                      checked={settings.launchAtStartup ?? false}
                      onChange={(v) => updateSettings({ launchAtStartup: v })}
                    />
                  </SettingRow>
                  <SettingRow label={t.autoPaste} description={lang === 'zh' ? '翻译完成后自动粘贴到当前应用。' : 'Paste translated text into the foreground app after translation completes.'}>
                    <Toggle
                      checked={settings.autoPaste ?? true}
                      onChange={(v) => updateSettings({ autoPaste: v })}
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
                </SettingsGroup>

                <SettingsGroup title={t.engine}>
                  <SettingRow label={t.selectModelPair} description={lang === 'zh' ? '选择输入翻译使用的供应商和模型。' : 'Choose the provider and model used for input translation.'}>
                    <ModelPairSelect
                      providerId={settings.translatorProviderId}
                      model={settings.translatorModel}
                      providers={settings.providers}
                      platform={platform}
                      onChange={(providerId, model) => {
                        updateSettings({ translatorProviderId: providerId, translatorModel: model })
                      }}
                    />
                  </SettingRow>
                </SettingsGroup>

                <SettingsGroup title={t.translatorPrompt}>
                  <SettingRow label={t.translatorPrompt} description={t.translatorPromptHint} stack>
                    <TextArea
                      value={settings.translatorPrompt || ''}
                      onChange={(v) => updateSettings({ translatorPrompt: v })}
                      placeholder={t.translatorPromptHint}
                      rows={3}
                    />
                    {!settings.translatorPrompt?.trim() && defaultPrompts?.translationTemplate && (
                      <DefaultPrompt label={t.defaultTemplate} content={defaultPrompts.translationTemplate} />
                    )}
                  </SettingRow>
                </SettingsGroup>
              </>
            )}

            {/* ===== 截图设置标签页 ===== */}
            {activeTab === 'screenshot' && (
              <ScreenshotTranslationSettings
                settings={settings}
                platform={platform}
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

                    <SettingsGroup title={t.lensWebSearch}>
                      <SettingRow label={t.enabled} description={t.lensWebSearchHint}>
                        <Toggle
                          checked={settings.lens?.webSearch?.enabled === true}
                          onChange={(v) => updateLensWebSearch({ enabled: v })}
                        />
                      </SettingRow>
                      {settings.lens?.webSearch?.enabled === true && (
                        <>
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
                        </>
                      )}
                    </SettingsGroup>

                    <SettingsGroup title={t.engine}>
                      <SettingRow label={t.selectModelPair} description={lang === 'zh' ? '留空时继承输入翻译模型。' : 'Leave empty to inherit the input translation model.'}>
                        <ModelPairSelect
                          providerId={settings.lens?.providerId || ''}
                          model={settings.lens?.model || ''}
                          providers={settings.providers}
                          platform={platform}
                          inheritLabel={t.lensLanguageInherit}
                          onChange={(providerId, model) => {
                            updateLens({ providerId, model })
                          }}
                        />
                      </SettingRow>
                    </SettingsGroup>

                    <SettingsGroup title={t.customPrompts}>
                      <details className="group">
                        <summary className="kv-row cursor-pointer list-none">
                          <div className="kv-row-text">
                            <div className="kv-row-label flex items-center gap-1.5">
                              <ChevronRight size={13} className="text-neutral-400 dark:text-neutral-500 group-open:rotate-90 transition-transform duration-200" strokeWidth={2.25} />
                              {t.customPrompts}
                            </div>
                            <div className="kv-row-desc">{t.customPromptsHint}</div>
                          </div>
                        </summary>
                        <div className="space-y-4 pb-2">
                          <div>
                            <Label>{t.lensSystemPrompt}</Label>
                            <TextArea
                              value={settings.lens?.systemPrompt || ''}
                              onChange={(v) => updateLens({ systemPrompt: v })}
                              placeholder={t.lensPromptHint}
                              rows={2}
                            />
                            {!settings.lens?.systemPrompt?.trim() && lensDefaults?.system && (
                              <DefaultPrompt label={t.defaultTemplate} content={lensDefaults.system} />
                            )}
                          </div>
                          <div>
                            <Label>{t.lensQuestionPrompt}</Label>
                            <TextArea
                              value={settings.lens?.questionPrompt || ''}
                              onChange={(v) => updateLens({ questionPrompt: v })}
                              placeholder={t.lensPromptHint}
                              rows={3}
                            />
                            {!settings.lens?.questionPrompt?.trim() && lensDefaults?.question && (
                              <DefaultPrompt label={t.defaultTemplate} content={lensDefaults.question} />
                            )}
                          </div>
                        </div>
                      </details>
                    </SettingsGroup>

                    <SettingsGroup title={t.lensQuickActions}>
                      <div className="space-y-3">
                        {(settings.lens?.quickActions || []).map((qa, idx) => (
                          <div key={qa.id} className="rounded-lg border border-black/[0.06] dark:border-white/[0.08] p-3 space-y-2">
                            <div className="flex items-center justify-between gap-2">
                              <div className="flex-1">
                                <Label>{t.lensQuickActionLabel}</Label>
                                <Input
                                  value={qa.label}
                                  onChange={(v) => {
                                    const next = [...(settings.lens?.quickActions || [])]
                                    next[idx] = { ...next[idx], label: v }
                                    updateLensQuickActions(next)
                                  }}
                                  placeholder={t.lensQuickActionLabel}
                                />
                              </div>
                              <button
                                type="button"
                                onClick={() => {
                                  const next = (settings.lens?.quickActions || []).filter((_, i) => i !== idx)
                                  updateLensQuickActions(next)
                                }}
                                className="kv-icon-btn danger mt-5"
                                data-tauri-drag-region="false"
                                title={t.lensQuickActionDelete}
                              >
                                <Trash2 size={12} />
                              </button>
                            </div>
                            <div>
                              <Label>{t.lensQuickActionPrompt}</Label>
                              <TextArea
                                value={qa.prompt}
                                onChange={(v) => {
                                  const next = [...(settings.lens?.quickActions || [])]
                                  next[idx] = { ...next[idx], prompt: v }
                                  updateLensQuickActions(next)
                                }}
                                placeholder={t.lensQuickActionPrompt}
                                rows={2}
                              />
                            </div>
                            <SettingRow label={t.selectModelPair} description={lang === 'zh' ? '留空时使用 Lens 默认模型。' : 'Leave empty to use the default Lens model.'}>
                              <ModelPairSelect
                                providerId={qa.providerId || ''}
                                model={qa.model || ''}
                                providers={settings.providers}
                                platform={platform}
                                inheritLabel={t.lensLanguageInherit}
                                onChange={(providerId, model) => {
                                  const next = [...(settings.lens?.quickActions || [])]
                                  next[idx] = { ...next[idx], providerId, model }
                                  updateLensQuickActions(next)
                                }}
                              />
                            </SettingRow>
                          </div>
                        ))}
                        <button
                          type="button"
                          onClick={() => {
                            const current = settings.lens?.quickActions || []
                            const newAction: LensQuickAction = {
                              id: `qa-${Date.now()}`,
                              label: '',
                              prompt: '',
                              providerId: '',
                              model: '',
                            }
                            updateLensQuickActions([...current, newAction])
                          }}
                          className="kv-btn sm"
                          data-tauri-drag-region="false"
                        >
                          <Plus size={11} />
                          {t.lensQuickActionAdd}
                        </button>
                      </div>
                    </SettingsGroup>
                  </>
                )}
              </>
            )}

            {/* ===== 模型管理标签页 ===== */}
            {activeTab === 'providers' && (
              <div className="kv-providers">
                <div className="kv-provider-list">
                  {settings.providers.map((provider) => {
                    const isOnDevice = provider.baseUrl === 'applefoundation://local'
                    const configured = isOnDevice || provider.apiKeys.some((key) => key.trim())
                    return (
                      <button
                        key={provider.id}
                        type="button"
                        onClick={() => setSelectedProviderId(provider.id)}
                        className={`kv-provider-item ${selectedProvider?.id === provider.id ? 'active' : ''}`}
                        data-tauri-drag-region="false"
                      >
                        <span className={`kv-provider-dot ${configured ? 'on' : 'warn'}`} />
                        <span className="kv-provider-name">{provider.name || t.providerName}</span>
                      </button>
                    )
                  })}
                  <button
                    type="button"
                    onClick={addProvider}
                    className="kv-provider-add"
                    data-tauri-drag-region="false"
                  >
                    <Plus />
                    {t.addProvider}
                  </button>
                </div>

                <div className="kv-provider-detail">
                  {selectedProvider ? (() => {
                    const provider = selectedProvider
                    const isOnDevice = provider.baseUrl === 'applefoundation://local'
                    return (
                      <>
                        <SettingsGroup title={lang === 'zh' ? '供应商' : 'Provider'} className="!pt-0">
                          <div className="py-2 flex items-end gap-2.5">
                            <div className="min-w-0 flex-1">
                              <div className="kv-row-label mb-1.5">{t.providerName}</div>
                              <Input
                                value={provider.name}
                                onChange={(v) => updateProvider(provider.id, { name: v })}
                                placeholder="Provider name"
                              />
                            </div>
                            <span className={`kv-tag mb-0.5 ${isOnDevice || provider.apiKeys.some((key) => key.trim()) ? 'ok' : 'warn'}`}>
                              {isOnDevice ? (lang === 'zh' ? '本地' : 'Local') : provider.apiKeys.some((key) => key.trim()) ? t.connectionOk : t.permissionMissing}
                            </span>
                            <button
                              type="button"
                              onClick={() => setConfirmDeleteProviderId(provider.id)}
                              className="kv-icon-btn danger mb-1"
                              data-tauri-drag-region="false"
                              title={t.deleteProvider}
                              aria-label={t.deleteProvider}
                            >
                              <Trash2 size={12} />
                            </button>
                          </div>
                        </SettingsGroup>

                        <SettingsGroup title={lang === 'zh' ? '配置' : 'Configuration'}>
                          {!isOnDevice && (
                            <>
                              <FieldBlock label={t.baseUrl} description={lang === 'zh' ? 'OpenAI 兼容接口地址。' : 'OpenAI-compatible endpoint.'}>
                                <Input
                                  value={provider.baseUrl}
                                  onChange={(v) => updateProvider(provider.id, { baseUrl: v })}
                                  placeholder="https://api.openai.com/v1"
                                  mono
                                />
                              </FieldBlock>

                              <FieldBlock label={t.apiKey} description={t.apiKeysHint}>
                                <div className="space-y-1.5">
                                  {(provider.apiKeys.length > 0 ? provider.apiKeys : ['']).map((key, idx) => {
                                    const total = Math.max(provider.apiKeys.length, 1)
                                    return (
                                      <div key={`${provider.id}-${total}-${idx}`} className="flex items-center gap-1.5">
                                        <Input
                                          type="password"
                                          value={key}
                                          mono
                                          onChange={(v) => {
                                            const base = provider.apiKeys.length > 0 ? [...provider.apiKeys] : ['']
                                            base[idx] = v
                                            updateProvider(provider.id, { apiKeys: base })
                                          }}
                                          placeholder={idx === 0 ? `sk-... (${t.apiKeyPrimary})` : `sk-... (${t.apiKeyBackup})`}
                                        />
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
                                <div className="kv-row-control">
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
                            </>
                          )}

                          <FieldBlock label={t.registeredModels} description={lang === 'zh' ? '这些模型会出现在各功能的模型选择器中。' : 'These models appear in feature model selectors.'}>
                            <div className="flex items-center gap-1.5">
                              <Input
                                className="h-7 !text-[11px]"
                                placeholder={t.manualAddModel}
                                mono
                                value={manualInputs[provider.id] || ''}
                                onChange={(v) => setManualInputs(prev => ({ ...prev, [provider.id]: v }))}
                                onKeyDown={(e: React.KeyboardEvent<HTMLInputElement>) => {
                                  if (e.key !== 'Enter') return
                                  if (e.nativeEvent.isComposing || e.keyCode === 229) return
                                  addEnabledModel(provider.id, manualInputs[provider.id] || '')
                                  setManualInputs(prev => ({ ...prev, [provider.id]: '' }))
                                }}
                              />
                              <button
                                type="button"
                                onClick={() => {
                                  addEnabledModel(provider.id, manualInputs[provider.id] || '')
                                  setManualInputs(prev => ({ ...prev, [provider.id]: '' }))
                                }}
                                className="kv-btn sm"
                                data-tauri-drag-region="false"
                              >
                                {t.addModel}
                              </button>
                            </div>
                            <div className="kv-chips mt-2 min-h-[24px]">
                              {provider.enabledModels.length === 0 && (
                                <span className="kv-row-desc italic">
                                  {lang === 'zh' ? '暂无模型，从下方可用模型挑选或手动添加。' : 'No models yet. Pick from available models or add one manually.'}
                                </span>
                              )}
                              {provider.enabledModels.map(model => (
                                <span key={model} className="kv-chip">
                                  {model}
                                  <button
                                    type="button"
                                    onClick={() => removeEnabledModel(provider.id, model)}
                                    className="kv-chip-x"
                                    data-tauri-drag-region="false"
                                    aria-label={t.removeModel}
                                  >
                                    <X size={10} />
                                  </button>
                                </span>
                              ))}
                            </div>
                          </FieldBlock>

                          {!isOnDevice && (
                            <FieldBlock label={t.availableModels} description={lang === 'zh' ? '从供应商接口拉取模型列表。' : 'Fetch models from the provider endpoint.'}>
                              <div className="flex justify-end mb-2">
                                <button
                                  type="button"
                                  onClick={() => fetchModels(provider.id)}
                                  disabled={fetchingProviderId === provider.id}
                                  className="kv-btn sm"
                                  data-tauri-drag-region="false"
                                >
                                  <RefreshCw size={10} className={fetchingProviderId === provider.id ? 'animate-spin' : ''} />
                                  {fetchingProviderId === provider.id ? t.fetching : t.fetchModels}
                                </button>
                              </div>
                              <div className="flex flex-wrap gap-1 max-h-28 overflow-y-auto pr-1 custom-scrollbar">
                                {provider.availableModels.length > 0 ? (
                                  provider.availableModels.map((model) => (
                                    <button
                                      key={model}
                                      type="button"
                                      onClick={() => addEnabledModel(provider.id, model)}
                                      disabled={provider.enabledModels.includes(model)}
                                      className="kv-chip disabled:opacity-45"
                                      data-tauri-drag-region="false"
                                    >
                                      {model}
                                    </button>
                                  ))
                                ) : (
                                  <span className="kv-row-desc italic">{lang === 'zh' ? '尚未获取模型。' : 'No models fetched yet.'}</span>
                                )}
                              </div>
                            </FieldBlock>
                          )}
                        </SettingsGroup>

                        <SettingsGroup title={lang === 'zh' ? '快速预设' : 'Presets'}>
                          <div className="flex flex-wrap gap-1.5 py-2">
                            {PROVIDER_PRESETS
                              .filter(preset => !preset.onDevice || isMac)
                              .map(preset => (
                                <button
                                  key={preset.name}
                                  type="button"
                                  onClick={() => addProviderFromPreset(preset)}
                                  className="kv-btn sm"
                                  data-tauri-drag-region="false"
                                >
                                  <Plus size={11} strokeWidth={2.25} />
                                  {preset.name}
                                  {preset.onDevice && (
                                    <span className="kv-tag ok">{lang === 'zh' ? '本地' : 'Local'}</span>
                                  )}
                                </button>
                              ))}
                          </div>
                        </SettingsGroup>
                      </>
                    )
                  })() : (
                    <div className="kv-panel">
                      <div className="kv-panel-title">{lang === 'zh' ? '暂无提供商' : 'No providers'}</div>
                      <div className="kv-panel-body">{lang === 'zh' ? '添加一个模型供应商开始配置。' : 'Add a model provider to start configuring.'}</div>
                    </div>
                  )}
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

          <div className="kv-savebar">
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
      </div>

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
    </div>
  )
}
