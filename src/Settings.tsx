import { useState, useEffect, useCallback, useRef } from 'react'
import {
  X, Save, Plus, Trash2, RefreshCw,
  Settings as SettingsIcon, Languages, Camera,
  Cloud, Info, Palette, Keyboard, SlidersHorizontal, Globe,
  Cpu, FileText, ShieldCheck, Aperture, ExternalLink, Download
} from 'lucide-react'
import ReactMarkdown from 'react-markdown'
import { api, type Settings as SettingsType, type ModelProvider, type DefaultPromptTemplates, type PermissionStatus, type UpdateInfo } from './api/tauri'
import { i18n } from './settings/i18n'
import { buildHotkey } from './settings/utils'
import { PROVIDER_PRESETS, type ProviderPreset } from './settings/providerPresets'
import {
  Toggle, Select, Input, TextArea, Label,
  SettingRow, PermissionItem, HotkeyInput, DefaultPrompt,
  SectionTitle,
} from './settings/components'

type SettingsData = SettingsType

interface SettingsProps {
  onClose: () => void
  onSettingsChange: () => void
}

/**
 * 设置面板主组件
 * 提供基础设置、翻译设置、截图设置、模型管理四大标签页
 */
export default function Settings({ onClose, onSettingsChange }: SettingsProps) {
  const [settings, setSettings] = useState<SettingsData | null>(null)
  const [initialSettingsSnapshot, setInitialSettingsSnapshot] = useState('')
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [appVersion, setAppVersion] = useState('')
  const [activeTab, setActiveTab] = useState<'general' | 'translate' | 'screenshot' | 'lens' | 'providers' | 'about'>('general')
  const [saveError, setSaveError] = useState('')
  const [saveSuccess, setSaveSuccess] = useState(false)
  const [closeConfirmOpen, setCloseConfirmOpen] = useState(false)
  const [recordingTarget, setRecordingTarget] = useState<null | 'main' | 'screenshotTranslation' | 'lens'>(null)
  const [defaultPrompts, setDefaultPrompts] = useState<DefaultPromptTemplates | null>(null)
  const [retryAttemptsInput, setRetryAttemptsInput] = useState('')
  const [permissionStatus, setPermissionStatus] = useState<PermissionStatus | null>(null)
  const [permissionsLoading, setPermissionsLoading] = useState(false)
  const [testingProviderId, setTestingProviderId] = useState<string | null>(null)
  const [providerTestFeedback, setProviderTestFeedback] = useState<Record<string, { ok: boolean; message: string }>>({})
  // 更新检查状态：'idle' / 'checking' / 'up-to-date' / 'available'
  const [updateStatus, setUpdateStatus] = useState<'idle' | 'checking' | 'up-to-date' | 'available'>('idle')
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null)
  // 下载/安装两段式状态机:idle → downloading(进度条) → downloaded(显示安装按钮) → 用户点击 → 应用退出
  // failed 时显示错误 + 重试 + 跳 GitHub 兜底
  const [downloadState, setDownloadState] = useState<'idle' | 'downloading' | 'downloaded' | 'failed'>('idle')
  const [downloadPercent, setDownloadPercent] = useState(0)
  const [downloadedPath, setDownloadedPath] = useState('')
  const [downloadError, setDownloadError] = useState('')
  // Apple Intelligence sidecar 可用性：mount 时查一次,unavailable 就把 onDevice 预设 chip 隐藏
  const [appleIntelligenceAvailable, setAppleIntelligenceAvailable] = useState(false)
  // 加载失败时的错误信息；非空则渲染错误 UI 而不是用合成默认值进入正常视图
  // （否则用户可能没察觉就 Save 把磁盘真实数据覆盖掉）
  const [loadError, setLoadError] = useState('')
  const [reloadKey, setReloadKey] = useState(0)
  const saveSuccessTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const lang = settings?.settingsLanguage || 'zh'
  const t = i18n[lang]
  // 判断是否有未保存的更改
  const hasUnsavedChanges = settings ? JSON.stringify(settings) !== initialSettingsSnapshot : false

  // 初始化：加载设置、版本号、默认提示词
  // 重试通过递增 reloadKey 触发本 effect 重跑
  useEffect(() => {
    let active = true
    setLoading(true)
    setLoadError('')
    api.getSettings()
      .then((data: SettingsData) => {
        if (!active) return
        setSettings(data)
        setInitialSettingsSnapshot(JSON.stringify(data))
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

  // 查 Apple Intelligence 可用性(macOS 26 + Apple Silicon + 已开启 → true)，决定预设 chip 是否露出
  useEffect(() => {
    let cancelled = false
    api.appleIntelligenceAvailable()
      .then(v => { if (!cancelled) setAppleIntelligenceAvailable(v) })
      .catch(() => {})
    return () => { cancelled = true }
  }, [])

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

  useEffect(() => {
    setProviderTestFeedback({})
  }, [lang])

  const retryAttempts = settings?.retryAttempts

  useEffect(() => {
    if (retryAttempts === undefined) return
    setRetryAttemptsInput(String(retryAttempts ?? 3))
  }, [retryAttempts])

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
      await api.saveSettings(settings)
      setInitialSettingsSnapshot(JSON.stringify(settings))
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
      const prefix = lang === 'zh' ? '保存失败：' : 'Save failed: '
      setSaveError(`${prefix}${message.replace(/\n/g, ' / ')}`)
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
  const handleSaveAndClose = async () => {
    const saved = await handleSave()
    if (saved) {
      setCloseConfirmOpen(false)
      onClose()
    }
  }

  // Esc 键关闭（带未保存提示）
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (recordingTarget) return
      if (e.key === 'Escape') {
        handleCloseRequest()
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [handleCloseRequest, recordingTarget])

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
      availableModels: [...preset.defaultModels],
      enabledModels: [...preset.defaultModels],
    }
    setSettings({
      ...settings,
      providers: [...settings.providers, newProvider]
    })
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
        providerId: 'default-ocr',
        model: 'gpt-4o',
        directTranslate: false,
        thinkingEnabled: false,
        streamEnabled: true,
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
        messageOrder: 'asc' as const
      }
      return { ...prev, lens: { ...current, ...updates } }
    })
  }, [])

  /**
   * 切换快捷键录制状态
   */
  const toggleRecording = (target: 'main' | 'screenshotTranslation' | 'lens') => {
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

  return (
    <div className="flex bg-[#f8f9fa] dark:bg-black text-neutral-900 dark:text-neutral-100 font-sans rounded-xl border border-black/5 dark:border-white/10 shadow-none overflow-hidden h-full w-full">
      {/* 左侧侧边栏 */}
      <div className="w-[180px] flex flex-col border-r border-black/5 dark:border-white/5 bg-white dark:bg-[#1C1C1E] shrink-0">
        {/* 标题 */}
        <div className="px-5 py-4" data-tauri-drag-region>
          <h2 className="font-semibold text-[14px] tracking-tight text-neutral-800 dark:text-neutral-100">{t.settings}</h2>
        </div>

        {/* 导航项 */}
        <nav className="flex-1 px-3 space-y-0.5">
          {[
            { id: 'general' as const, label: t.tabGeneral, icon: SettingsIcon },
            { id: 'translate' as const, label: t.tabTranslate, icon: Languages },
            { id: 'screenshot' as const, label: t.tabScreenshot, icon: Camera },
            { id: 'lens' as const, label: t.lensTabLabel, icon: Aperture },
            { id: 'providers' as const, label: t.tabModels, icon: Cloud },
            { id: 'about' as const, label: lang === 'zh' ? '关于' : 'About', icon: Info },
          ].map((item) => {
            const Icon = item.icon
            const active = activeTab === item.id
            return (
              <button
                key={item.id}
                onClick={() => setActiveTab(item.id)}
                className={`relative w-full flex items-center gap-2.5 px-3 h-12 rounded-lg text-[13px] font-medium transition-all duration-150 ${active
                  ? 'bg-[#f0f4ff] dark:bg-blue-900/20 text-[#2563eb] dark:text-blue-400'
                  : 'text-neutral-500 dark:text-neutral-400 hover:text-neutral-700 dark:hover:text-neutral-200 hover:bg-neutral-50 dark:hover:bg-neutral-800/50'
                  }`}
                data-tauri-drag-region="false"
              >
                {active && (
                  <div className="absolute left-0 top-1/2 -translate-y-1/2 w-[3px] h-5 rounded-r-full bg-[#2563eb] dark:bg-blue-400" />
                )}
                <Icon size={18} strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" className={active ? 'text-[#2563eb] dark:text-blue-400' : ''} />
                {item.label}
              </button>
            )
          })}
        </nav>

      </div>

      {/* 右侧内容区域 */}
      <div className="flex-1 flex flex-col min-w-0">
        {/* 顶部关闭按钮 */}
        <div className="flex justify-end px-4 pt-3" data-tauri-drag-region>
          <button
            onClick={handleCloseRequest}
            className="p-1.5 hover:bg-black/5 dark:hover:bg-white/10 rounded-lg text-neutral-400 hover:text-neutral-600 dark:hover:text-neutral-300 transition-all"
            data-tauri-drag-region="false"
          >
            <X size={16} strokeWidth={2} />
          </button>
        </div>
        {/* 内容滚动区 */}
        <div className="flex-1 overflow-auto px-5 py-2 space-y-5 custom-scrollbar">
        {/* ===== 基础设置标签页 ===== */}
        {activeTab === 'general' && (
          <div className="space-y-8 animate-in fade-in slide-in-from-bottom-2 duration-300">
            {/* 外观 */}
            <section>
              <SectionTitle icon={Palette}>{lang === 'zh' ? '外观' : 'Appearance'}</SectionTitle>
              <div className="bg-white dark:bg-[#1C1C1E] rounded-xl shadow-[0_1px_3px_rgba(0,0,0,0.04)] overflow-hidden divide-y divide-[#f0f0f0] dark:divide-white/5">
                <SettingRow label={t.theme}>
                  <Select
                    className="w-36"
                    value={settings.theme || 'system'}
                    onChange={(v) => updateSettings({ theme: v as SettingsData['theme'] })}
                    options={[
                      { value: 'system', label: t.themeSystem },
                      { value: 'light', label: t.themeLight },
                      { value: 'dark', label: t.themeDark },
                    ]}
                  />
                </SettingRow>
                <SettingRow label={t.language}>
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
              </div>
            </section>

            {/* 快捷键 */}
            <section>
              <SectionTitle icon={Keyboard}>{t.hotkey}</SectionTitle>
              <div className="bg-white dark:bg-[#1C1C1E] rounded-xl shadow-[0_1px_3px_rgba(0,0,0,0.04)] overflow-hidden px-4 py-3">
                <HotkeyInput
                  value={settings.hotkey}
                  placeholder={t.hotkeyPlaceholder}
                  recording={recordingTarget === 'main'}
                  onToggleRecording={() => toggleRecording('main')}
                  recordLabel={t.hotkeyRecord}
                  recordingLabel={t.hotkeyRecording}
                  recordingPlaceholder={t.hotkeyRecordingPlaceholder}
                />
              </div>
            </section>

            {/* 行为 */}
            <section>
              <SectionTitle icon={SlidersHorizontal}>{lang === 'zh' ? '行为' : 'Behavior'}</SectionTitle>
              <div className="bg-white dark:bg-[#1C1C1E] rounded-xl shadow-[0_1px_3px_rgba(0,0,0,0.04)] overflow-hidden divide-y divide-[#f0f0f0] dark:divide-white/5">
                <SettingRow label={t.retryEnabled} description={t.retryAttemptsHint}>
                  <Toggle
                    checked={settings.retryEnabled ?? true}
                    onChange={(v) => updateSettings({ retryEnabled: v })}
                  />
                </SettingRow>
                {settings.retryEnabled !== false && (
                  <div className="px-4 py-2.5 animate-in fade-in slide-in-from-top-1 duration-150">
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
                  </div>
                )}
                <SettingRow label={t.autoPaste}>
                  <Toggle
                    checked={settings.autoPaste ?? true}
                    onChange={(v) => updateSettings({ autoPaste: v })}
                  />
                </SettingRow>
                <SettingRow label={t.launchAtStartup}>
                  <Toggle
                    checked={settings.launchAtStartup ?? false}
                    onChange={(v) => updateSettings({ launchAtStartup: v })}
                  />
                </SettingRow>
              </div>
            </section>

            {/* 权限状态（仅 macOS 显示） */}
            {permissionStatus?.platform === 'macos' && (
              <section>
                <SectionTitle icon={ShieldCheck}>{t.permissions}</SectionTitle>
                <div className="bg-white dark:bg-[#1C1C1E] rounded-xl shadow-[0_1px_3px_rgba(0,0,0,0.04)] overflow-hidden divide-y divide-[#f0f0f0] dark:divide-white/5">
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
                  <div className="flex justify-end px-4 py-2.5">
                    <button
                      type="button"
                      onClick={refreshPermissions}
                      disabled={permissionsLoading}
                      className={`text-[11px] font-medium flex items-center gap-1 px-2 py-1 rounded-md transition-all ${permissionsLoading
                        ? 'text-neutral-400 cursor-not-allowed'
                        : 'text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-200 hover:bg-black/5 dark:hover:bg-white/5'
                        }`}
                      data-tauri-drag-region="false"
                    >
                      <RefreshCw size={10} className={permissionsLoading ? 'animate-spin' : ''} />
                      {t.refreshPermissions}
                    </button>
                  </div>
                </div>
              </section>
            )}
          </div>
        )}

        {/* ===== 翻译设置标签页 ===== */}
        {activeTab === 'translate' && (
          <div className="space-y-8 animate-in fade-in slide-in-from-bottom-2 duration-300">
            {/* 目标语言 */}
            <section>
              <SectionTitle icon={Globe}>{t.targetLang}</SectionTitle>
              <div className="bg-white dark:bg-[#1C1C1E] rounded-xl shadow-[0_1px_3px_rgba(0,0,0,0.04)] overflow-hidden">
                <SettingRow label={t.targetLang}>
                  <Select
                    className="w-40"
                    value={settings.targetLang || 'auto'}
                    onChange={(v) => updateSettings({ targetLang: v })}
                    options={[
                      { value: 'auto', label: t.langAuto },
                      { value: 'en', label: t.langEn },
                      { value: 'zh', label: t.langZh },
                      { value: 'ja', label: t.langJa },
                      { value: 'ko', label: t.langKo },
                      { value: 'fr', label: t.langFr },
                      { value: 'de', label: t.langDe },
                    ]}
                  />
                </SettingRow>
              </div>
            </section>

            {/* 翻译引擎 */}
            <section>
              <SectionTitle icon={Cpu}>{t.engine}</SectionTitle>
              <div className="bg-white dark:bg-[#1C1C1E] rounded-xl shadow-[0_1px_3px_rgba(0,0,0,0.04)] overflow-hidden">
                <SettingRow label={t.selectModelPair}>
                  <Select
                    className="w-52"
                    value={`${settings.translatorProviderId}:${settings.translatorModel}`}
                    onChange={(v) => {
                      const [providerId, model] = v.split(':')
                      updateSettings({ translatorProviderId: providerId, translatorModel: model })
                    }}
                    options={settings.providers.flatMap(p =>
                      p.enabledModels.map(m => ({
                        value: `${p.id}:${m}`,
                        label: `${p.name} - ${m}`
                      }))
                    )}
                  />
                </SettingRow>
              </div>
            </section>

            {/* 提示词 */}
            <section>
              <SectionTitle icon={FileText}>{t.translatorPrompt}</SectionTitle>
              <div className="bg-white dark:bg-[#1C1C1E] rounded-xl shadow-[0_1px_3px_rgba(0,0,0,0.04)] overflow-hidden px-4 py-3">
                <TextArea
                  value={settings.translatorPrompt || ''}
                  onChange={(v) => updateSettings({ translatorPrompt: v })}
                  placeholder={t.translatorPromptHint}
                  rows={3}
                />
                {!settings.translatorPrompt?.trim() && defaultPrompts?.translationTemplate && (
                  <DefaultPrompt label={t.defaultTemplate} content={defaultPrompts.translationTemplate} />
                )}
              </div>
            </section>
          </div>
        )}

        {/* ===== 截图设置标签页 ===== */}
        {activeTab === 'screenshot' && (
          <div className="space-y-8 animate-in fade-in slide-in-from-bottom-2 duration-300">
            {/* 截图翻译设置 */}
            <section>
              <SectionTitle icon={Camera}>{t.screenshotTranslate}</SectionTitle>
              <div className="bg-white dark:bg-[#1C1C1E] rounded-xl shadow-[0_1px_3px_rgba(0,0,0,0.04)] overflow-hidden">
                <div className="divide-y divide-[#f0f0f0] dark:divide-white/5">
                  <SettingRow label={t.enabled}>
                    <Toggle
                      checked={settings.screenshotTranslation?.enabled ?? true}
                      onChange={(v) => updateScreenshotTranslation({ enabled: v })}
                    />
                  </SettingRow>

                  {settings.screenshotTranslation?.enabled !== false && (
                    <>
                      <div className="px-4 py-3 space-y-1.5">
                        <span className="text-[12px] font-medium text-neutral-700 dark:text-neutral-200">{t.hotkey}</span>
                        <HotkeyInput
                          value={settings.screenshotTranslation?.hotkey || 'CommandOrControl+Shift+A'}
                          placeholder="CommandOrControl+Shift+A"
                          recording={recordingTarget === 'screenshotTranslation'}
                          onToggleRecording={() => toggleRecording('screenshotTranslation')}
                          recordLabel={t.hotkeyRecord}
                          recordingLabel={t.hotkeyRecording}
                          recordingPlaceholder={t.hotkeyRecordingPlaceholder}
                        />
                      </div>
                      <SettingRow label={t.selectModelPair}>
                        <Select
                          className="w-52"
                          value={`${settings.screenshotTranslation.providerId}:${settings.screenshotTranslation.model}`}
                          onChange={(v) => {
                            const [providerId, model] = v.split(':')
                            updateScreenshotTranslation({ providerId, model })
                          }}
                          options={settings.providers.flatMap(p =>
                            p.enabledModels.map(m => ({
                              value: `${p.id}:${m}`,
                              label: `${p.name} - ${m}`
                            }))
                          )}
                        />
                      </SettingRow>
                      <SettingRow
                        label={t.screenshotShowOriginal}
                        description={t.screenshotShowOriginalHint}
                      >
                        <Toggle
                          checked={!(settings.screenshotTranslation?.directTranslate ?? false)}
                          onChange={(v) => updateScreenshotTranslation({ directTranslate: !v })}
                        />
                      </SettingRow>
                      <SettingRow
                        label={t.screenshotTranslationThinking}
                        description={t.screenshotTranslationThinkingHint}
                      >
                        <Toggle
                          checked={settings.screenshotTranslation?.thinkingEnabled ?? false}
                          onChange={(v) => updateScreenshotTranslation({ thinkingEnabled: v })}
                        />
                      </SettingRow>
                      <SettingRow
                        label={t.screenshotTranslationStream}
                        description={t.screenshotTranslationStreamHint}
                      >
                        <Toggle
                          checked={settings.screenshotTranslation?.streamEnabled !== false}
                          onChange={(v) => updateScreenshotTranslation({ streamEnabled: v })}
                        />
                      </SettingRow>
                      <SettingRow
                        label={t.useSystemOcr}
                        description={t.useSystemOcrHint}
                      >
                        <Toggle
                          checked={settings.screenshotTranslation?.useSystemOcr ?? false}
                          onChange={(v) => updateScreenshotTranslation({ useSystemOcr: v })}
                        />
                      </SettingRow>
                      <SettingRow label={t.lensKeepFullscreen} description={t.lensKeepFullscreenHint}>
                        <Toggle
                          checked={settings.screenshotTranslation?.keepFullscreenAfterCapture !== false}
                          onChange={(v) => updateScreenshotTranslation({ keepFullscreenAfterCapture: v })}
                        />
                      </SettingRow>
                      <details className="group">
                        <summary className="flex items-center gap-2 cursor-pointer text-[12px] font-medium text-neutral-500 hover:text-neutral-800 dark:hover:text-neutral-200 transition-colors list-none px-4 py-3">
                          <div className="p-0.5 rounded text-neutral-400 group-open:rotate-90 transition-transform">
                            <svg width="8" height="8" viewBox="0 0 8 8" fill="none" xmlns="http://www.w3.org/2000/svg">
                              <path d="M2.5 1.5L5.5 4L2.5 6.5" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
                            </svg>
                          </div>
                          {t.customPrompts}
                        </summary>
                        <div className="px-4 pb-4 space-y-2">
                          <Label>{t.screenshotTranslationPrompt}</Label>
                          <TextArea
                            value={settings.screenshotTranslation?.prompt || ''}
                            onChange={(v) => updateScreenshotTranslation({ prompt: v })}
                            placeholder={t.screenshotTranslationPromptHint}
                            rows={3}
                          />
                          {!settings.screenshotTranslation?.prompt?.trim() && (defaultPrompts?.screenshotTranslationTemplate || defaultPrompts?.translationTemplate) && (
                            <DefaultPrompt
                              label={t.defaultTemplate}
                              content={defaultPrompts?.screenshotTranslationTemplate || defaultPrompts?.translationTemplate || ''}
                            />
                          )}
                        </div>
                      </details>
                    </>
                  )}
                </div>
              </div>
            </section>

          </div>
        )}

        {/* ===== Lens 标签页 ===== */}
        {activeTab === 'lens' && (
          <div className="space-y-8 animate-in fade-in slide-in-from-bottom-2 duration-300">
            <section>
              <SectionTitle icon={Aperture}>{t.lensSection}</SectionTitle>
              <div className="bg-white dark:bg-[#1C1C1E] rounded-xl shadow-[0_1px_3px_rgba(0,0,0,0.04)] overflow-hidden">
                <div className="divide-y divide-[#f0f0f0] dark:divide-white/5">
                  <SettingRow label={t.enabled}>
                    <Toggle
                      checked={settings.lens?.enabled !== false}
                      onChange={(v) => updateLens({ enabled: v })}
                    />
                  </SettingRow>

                  {settings.lens?.enabled !== false && (
                    <>
                      <div className="px-4 py-3 space-y-1.5">
                        <span className="text-[12px] font-medium text-neutral-700 dark:text-neutral-200">{t.hotkey}</span>
                        <HotkeyInput
                          value={settings.lens?.hotkey || 'CommandOrControl+Shift+G'}
                          placeholder="CommandOrControl+Shift+G"
                          recording={recordingTarget === 'lens'}
                          onToggleRecording={() => toggleRecording('lens')}
                          recordLabel={t.hotkeyRecord}
                          recordingLabel={t.hotkeyRecording}
                          recordingPlaceholder={t.hotkeyRecordingPlaceholder}
                        />
                      </div>
                      <SettingRow label={t.lensResponseLanguage}>
                        <Select
                          className="w-44"
                          value={settings.lens?.defaultLanguage || ''}
                          onChange={(v) => updateLens({ defaultLanguage: v })}
                          options={[
                            { value: '', label: t.lensLanguageInherit },
                            { value: 'zh', label: '中文' },
                            { value: 'en', label: 'English' },
                          ]}
                        />
                      </SettingRow>
                      <SettingRow label={t.lensStreamEnabled}>
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
                      <SettingRow label={t.lensMessageOrder}>
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
                      <SettingRow label={t.lensKeepFullscreen} description={t.lensKeepFullscreenHint}>
                        <Toggle
                          checked={settings.lens?.keepFullscreenAfterCapture !== false}
                          onChange={(v) => updateLens({ keepFullscreenAfterCapture: v })}
                        />
                      </SettingRow>
                      <SettingRow label={t.selectModelPair}>
                        <Select
                          className="w-52"
                          value={`${settings.lens?.providerId || ''}:${settings.lens?.model || ''}`}
                          onChange={(v) => {
                            const [providerId, model] = v.split(':')
                            updateLens({ providerId, model })
                          }}
                          options={[
                            { value: ':', label: t.lensLanguageInherit },
                            ...settings.providers.flatMap(p =>
                              p.enabledModels.map(m => ({
                                value: `${p.id}:${m}`,
                                label: `${p.name} - ${m}`
                              }))
                            )
                          ]}
                        />
                      </SettingRow>
                      <details className="group">
                        <summary className="flex items-center gap-2 cursor-pointer text-[12px] font-medium text-neutral-500 hover:text-neutral-800 dark:hover:text-neutral-200 transition-colors list-none px-4 py-3">
                          <div className="p-0.5 rounded text-neutral-400 group-open:rotate-90 transition-transform">
                            <svg width="8" height="8" viewBox="0 0 8 8" fill="none" xmlns="http://www.w3.org/2000/svg">
                              <path d="M2.5 1.5L5.5 4L2.5 6.5" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
                            </svg>
                          </div>
                          {t.customPrompts}
                        </summary>
                        <div className="px-4 pb-4 space-y-4">
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
                    </>
                  )}
                </div>
              </div>
            </section>
          </div>
        )}

        {/* ===== 模型管理标签页 ===== */}
        {activeTab === 'providers' && (
          <div className="space-y-8 animate-in fade-in slide-in-from-bottom-2 duration-300">
            {settings.providers.map((provider) => {
              // 端上 provider(Apple Intelligence)：不需 baseURL/API Key/连接测试/可用模型 fetch，
              // 这些字段对用户毫无意义,渲染时全部隐藏。
              const isOnDevice = provider.baseUrl === 'applefoundation://local'
              return (
              <section key={provider.id} className="relative group">
                <div className="absolute right-3 top-3 z-10 opacity-0 group-hover:opacity-100 transition-opacity">
                  <button
                    onClick={() => deleteProvider(provider.id)}
                    className="p-1.5 text-neutral-400 hover:text-red-500 hover:bg-red-50 dark:hover:bg-red-950/30 rounded-lg transition-all"
                  >
                    <Trash2 size={14} />
                  </button>
                </div>

                <div className="bg-white dark:bg-[#1C1C1E] rounded-xl shadow-[0_1px_3px_rgba(0,0,0,0.04)] overflow-hidden">
                  <div className="divide-y divide-[#f0f0f0] dark:divide-white/5">
                    {/* 名称 */}
                    <div className="px-4 py-3">
                      <Label>{t.providerName}</Label>
                      <div className="mt-1.5">
                        <Input
                          value={provider.name}
                          onChange={(v) => updateProvider(provider.id, { name: v })}
                          placeholder="e.g. OpenAI / DeepSeek"
                        />
                      </div>
                    </div>

                    {/* Base URL — 端上 provider(Apple Intelligence)用哨兵 baseURL,无展示价值,隐藏 */}
                    {!isOnDevice && (
                    <div className="px-4 py-3">
                      <Label>{t.baseUrl}</Label>
                      <div className="mt-1.5">
                        <Input
                          value={provider.baseUrl}
                          onChange={(v) => updateProvider(provider.id, { baseUrl: v })}
                          placeholder="https://api.openai.com/v1"
                        />
                      </div>
                    </div>
                    )}

                    {/* API Keys — 端上 provider 不需 key,隐藏 */}
                    {!isOnDevice && (
                    <div className="px-4 py-3">
                      <div className="flex items-center justify-between">
                        <Label>{t.apiKey}</Label>
                        <span className="text-[10px] text-neutral-400 dark:text-neutral-500">
                          {t.apiKeysHint}
                        </span>
                      </div>
                      <div className="mt-1.5 space-y-1.5">
                        {(provider.apiKeys.length > 0 ? provider.apiKeys : ['']).map((key, idx) => {
                          const total = Math.max(provider.apiKeys.length, 1)
                          // key 含 total（apiKeys.length）：add/remove 时整列强制 remount，
                          // 避免删除 idx 0 后 React 把旧 row 0 的 DOM（焦点 / 光标 / 浏览器自动填充状态）复用给新 idx 0
                          return (
                            <div key={`${provider.id}-${total}-${idx}`} className="flex items-center gap-1.5">
                              <div className="flex-1">
                                <Input
                                  type="password"
                                  value={key}
                                  onChange={(v) => {
                                    const base = provider.apiKeys.length > 0 ? [...provider.apiKeys] : ['']
                                    base[idx] = v
                                    updateProvider(provider.id, { apiKeys: base })
                                  }}
                                  placeholder={idx === 0 ? `sk-... (${t.apiKeyPrimary})` : `sk-... (${t.apiKeyBackup})`}
                                />
                              </div>
                              {total > 1 && (
                                <button
                                  type="button"
                                  onClick={() => {
                                    const next = provider.apiKeys.filter((_, i) => i !== idx)
                                    updateProvider(provider.id, { apiKeys: next })
                                  }}
                                  className="text-neutral-400 hover:text-red-500 transition-colors p-1"
                                  title={t.removeKey}
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
                        className="mt-2 text-[11px] text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-200 px-2 py-1 rounded bg-black/5 dark:bg-white/5 transition-all flex items-center gap-1"
                      >
                        <Plus size={11} />
                        {t.addKey}
                      </button>
                    </div>
                    )}

                    {/* 连接测试 — 端上 provider 不走 HTTP,无连接可测,隐藏 */}
                    {!isOnDevice && (
                    <div className="flex items-center justify-between gap-3 px-4 py-3">
                      <button
                        type="button"
                        onClick={() => handleTestConnection(provider.id)}
                        disabled={testingProviderId === provider.id}
                        className={`text-[11px] font-medium flex items-center gap-1 px-2.5 py-1 rounded-md transition-all border ${testingProviderId === provider.id
                          ? 'text-neutral-400 border-black/5 dark:border-white/5 cursor-not-allowed'
                          : 'text-neutral-500 border-black/10 dark:border-white/10 hover:text-neutral-900 dark:hover:text-neutral-200 hover:bg-black/5 dark:hover:bg-white/5'
                          }`}
                        data-tauri-drag-region="false"
                      >
                        <RefreshCw size={10} className={testingProviderId === provider.id ? 'animate-spin' : ''} />
                        {testingProviderId === provider.id ? t.testingConnection : t.testConnection}
                      </button>
                      {providerTestFeedback[provider.id] && (
                        <span className={`text-[11px] truncate ${providerTestFeedback[provider.id].ok
                          ? 'text-emerald-600 dark:text-emerald-400'
                          : 'text-rose-600 dark:text-rose-400'
                          }`} title={providerTestFeedback[provider.id].message}>
                          {providerTestFeedback[provider.id].message}
                        </span>
                      )}
                    </div>
                    )}

                    {/* 已启用模型 */}
                    <div className="px-4 py-3 space-y-3">
                      <div className="flex justify-between items-center">
                        <span className="text-[12px] font-medium text-neutral-700 dark:text-neutral-200">{t.registeredModels}</span>
                        <div className="flex items-center gap-1">
                          <Input
                            className="h-7 w-32 !text-[11px] !py-0"
                            placeholder={t.manualAddModel}
                            value={manualInputs[provider.id] || ''}
                            onChange={(v) => setManualInputs(prev => ({ ...prev, [provider.id]: v }))}
                            onKeyDown={(e: React.KeyboardEvent<HTMLInputElement>) => {
                              if (e.key !== 'Enter') return
                              // IME (拼音 / 假名等) 选词期间的 Enter 用于确认候选词，不应触发添加
                              if (e.nativeEvent.isComposing || e.keyCode === 229) return
                              addEnabledModel(provider.id, manualInputs[provider.id] || '')
                              setManualInputs(prev => ({ ...prev, [provider.id]: '' }))
                            }}
                          />
                          <button
                            onClick={() => {
                              addEnabledModel(provider.id, manualInputs[provider.id] || '')
                              setManualInputs(prev => ({ ...prev, [provider.id]: '' }))
                            }}
                            className="text-[10px] text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-200 px-2 py-1 rounded bg-black/5 dark:bg-white/5 transition-all text-nowrap"
                          >
                            {t.addModel}
                          </button>
                        </div>
                      </div>
                      <div className="flex flex-wrap gap-2 min-h-[24px]">
                        {provider.enabledModels.map(model => (
                          <span key={model} className="flex items-center gap-1.5 px-2 py-0.5 bg-neutral-100 dark:bg-neutral-800 rounded-md text-[11px] text-neutral-700 dark:text-neutral-300 font-mono border border-black/5 dark:border-white/5">
                            {model}
                            <button
                              onClick={() => removeEnabledModel(provider.id, model)}
                              className="text-neutral-400 hover:text-red-500 transition-colors"
                            >
                              <X size={10} />
                            </button>
                          </span>
                        ))}
                      </div>
                    </div>

                    {/* 可用模型 — 端上 provider 没有 /models 端点,fetch 无意义,隐藏 */}
                    {!isOnDevice && (
                    <div className="px-4 py-3 space-y-2">
                      <div className="flex justify-between items-center">
                        <span className="text-[12px] font-medium text-neutral-700 dark:text-neutral-200">{t.availableModels}</span>
                        <button
                          onClick={() => fetchModels(provider.id)}
                          disabled={fetchingProviderId === provider.id}
                          className={`text-[11px] font-medium flex items-center gap-1 px-2 py-0.5 rounded-md transition-all ${fetchingProviderId === provider.id
                            ? 'text-neutral-400 cursor-not-allowed'
                            : 'text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-200 hover:bg-black/5 dark:hover:bg-white/5'
                            }`}
                        >
                          <RefreshCw size={10} className={fetchingProviderId === provider.id ? 'animate-spin' : ''} />
                          {fetchingProviderId === provider.id ? t.fetching : t.fetchModels}
                        </button>
                      </div>
                      <div className="flex flex-wrap gap-1.5 max-h-32 overflow-y-auto pr-1 custom-scrollbar">
                        {provider.availableModels.length > 0 ? (
                          provider.availableModels.map(m => (
                            <button
                              key={m}
                              onClick={() => addEnabledModel(provider.id, m)}
                              disabled={provider.enabledModels.includes(m)}
                              className={`px-2 py-0.5 rounded text-[10px] font-mono border transition-all ${provider.enabledModels.includes(m)
                                ? 'bg-neutral-50 dark:bg-neutral-900/50 text-neutral-400 border-transparent cursor-default'
                                : 'bg-black/5 dark:bg-white/5 text-neutral-500 border-black/5 dark:border-white/5 hover:border-neutral-300 dark:hover:border-neutral-600'
                                }`}
                            >
                              {m}
                            </button>
                          ))
                        ) : (
                          <span className="text-[11px] text-neutral-400 italic">No models fetched yet</span>
                        )}
                      </div>
                    </div>
                    )}
                  </div>
                </div>
              </section>
              )
            })}

            {/* 快速预设 chip + 自定义按钮 */}
            <div className="space-y-2">
              <div className="flex flex-wrap gap-2">
                {PROVIDER_PRESETS
                  .filter(preset => !preset.onDevice || appleIntelligenceAvailable)
                  .map(preset => (
                  <button
                    key={preset.name}
                    type="button"
                    onClick={() => addProviderFromPreset(preset)}
                    className="flex items-center gap-1.5 text-[12px] font-medium px-3 py-1.5 rounded-md bg-white dark:bg-[#1C1C1E] text-neutral-700 dark:text-neutral-200 border border-black/10 dark:border-white/10 hover:bg-black/5 dark:hover:bg-white/5 hover:border-black/20 dark:hover:border-white/20 transition-all"
                  >
                    <Plus size={12} strokeWidth={2.25} />
                    {preset.name}
                    {preset.onDevice && (
                      <span className="text-[10px] font-medium text-emerald-600 dark:text-emerald-400 ml-0.5">· 本地</span>
                    )}
                  </button>
                ))}
              </div>
              <button
                onClick={addProvider}
                className="w-full py-3 border-2 border-dashed border-black/5 dark:border-white/5 rounded-[10px] text-neutral-400 hover:text-neutral-600 dark:hover:text-neutral-300 hover:border-black/10 dark:hover:border-white/10 hover:bg-black/5 dark:hover:bg-white/5 transition-all flex items-center justify-center gap-2"
              >
                <Plus size={16} strokeWidth={2} />
                <span className="text-[12px] font-medium">{t.addProvider}</span>
              </button>
            </div>
          </div>
        )}

        {/* ===== 关于标签页 ===== */}
        {activeTab === 'about' && (
          <div className="space-y-6 animate-in fade-in slide-in-from-bottom-2 duration-300">
            <section>
              <div className="flex flex-col items-center justify-center py-8">
                <div className="w-16 h-16 rounded-2xl bg-neutral-900 dark:bg-white flex items-center justify-center mb-4 shadow-sm">
                  <span className="text-white dark:text-neutral-900 text-[20px] font-bold">K</span>
                </div>
                <h2 className="text-[16px] font-semibold text-neutral-900 dark:text-white mb-1">KeyLingo</h2>
                <p className="text-[13px] text-neutral-500 dark:text-neutral-400 mb-6">{lang === 'zh' ? '智能翻译与 AI 视觉工具' : 'Smart Translation & AI Vision Tool'}</p>
                <div className="bg-white dark:bg-[#1C1C1E] rounded-xl shadow-[0_1px_3px_rgba(0,0,0,0.04)] overflow-hidden w-full max-w-sm">
                  <div className="flex items-center justify-between px-4 py-3 border-b border-black/5 dark:border-white/5">
                    <span className="text-[13px] text-neutral-900 dark:text-neutral-100">{t.currentVersion}</span>
                    <span className="text-[13px] text-neutral-500 dark:text-neutral-400 font-mono">v{appVersion}</span>
                  </div>
                  <div className="flex items-center justify-between px-4 py-3">
                    <span className="text-[13px] text-neutral-900 dark:text-neutral-100">{lang === 'zh' ? '开发者' : 'Developer'}</span>
                    <span className="text-[13px] text-neutral-500 dark:text-neutral-400">ZM</span>
                  </div>
                </div>
              </div>
            </section>

            {/* 自动更新检查（仅检查 + 跳转 GH 下载，不做自动安装） */}
            <section>
              <SectionTitle icon={Download}>{t.checkUpdate}</SectionTitle>
              <div className="bg-white dark:bg-[#1C1C1E] rounded-xl shadow-[0_1px_3px_rgba(0,0,0,0.04)] overflow-hidden">
                <SettingRow
                  label={t.autoCheckUpdate}
                  description={t.autoCheckUpdateHint}
                >
                  <Toggle
                    checked={settings?.autoCheckUpdate ?? true}
                    onChange={(v) => updateSettings({ autoCheckUpdate: v })}
                  />
                </SettingRow>

                {/* 检查按钮 + 状态 */}
                <div className="flex items-center justify-between gap-3 px-4 py-3 border-t border-black/5 dark:border-white/5">
                  <div className="flex items-center gap-2 min-w-0">
                    <span className="text-[13px] text-neutral-900 dark:text-neutral-100">{t.checkUpdate}</span>
                    {updateStatus === 'up-to-date' && (
                      <span className="text-[11px] text-emerald-600 dark:text-emerald-400">{t.upToDate}</span>
                    )}
                  </div>
                  <button
                    type="button"
                    onClick={handleCheckUpdate}
                    disabled={updateStatus === 'checking'}
                    className={`flex items-center gap-1 text-[11px] font-medium px-2.5 py-1 rounded-md border transition-all ${
                      updateStatus === 'checking'
                        ? 'text-neutral-400 border-black/5 dark:border-white/5 cursor-not-allowed'
                        : 'text-neutral-600 dark:text-neutral-300 border-black/10 dark:border-white/10 hover:bg-black/5 dark:hover:bg-white/5'
                    }`}
                    data-tauri-drag-region="false"
                  >
                    <RefreshCw size={11} className={updateStatus === 'checking' ? 'animate-spin' : ''} />
                    {updateStatus === 'checking' ? t.checkingUpdate : t.checkUpdate}
                  </button>
                </div>

                {/* 发现新版本 panel */}
                {updateStatus === 'available' && updateInfo && (
                  <div className="px-4 py-4 border-t border-black/5 dark:border-white/5 bg-emerald-50/50 dark:bg-emerald-500/5">
                    <div className="flex items-baseline gap-2 mb-2">
                      <span className="text-[13px] font-semibold text-emerald-700 dark:text-emerald-300">{t.updateAvailable}</span>
                      <span className="text-[12px] font-mono text-emerald-600 dark:text-emerald-400">v{updateInfo.version}</span>
                    </div>
                    {updateInfo.body && (
                      <div className="prose prose-sm dark:prose-invert max-w-none text-[12px] leading-relaxed text-neutral-700 dark:text-neutral-300 max-h-40 overflow-y-auto mb-3 px-2 py-1.5 bg-white/60 dark:bg-black/20 rounded-md">
                        <ReactMarkdown>{updateInfo.body}</ReactMarkdown>
                      </div>
                    )}

                    {/* 下载进度条:downloading 时显示 */}
                    {downloadState === 'downloading' && (
                      <div className="mb-3">
                        <div className="flex items-center justify-between text-[11px] text-neutral-600 dark:text-neutral-400 mb-1">
                          <span>{t.downloading}</span>
                          <span className="font-mono tabular-nums">{downloadPercent}%</span>
                        </div>
                        <div className="h-1.5 rounded-full bg-emerald-100 dark:bg-emerald-900/40 overflow-hidden">
                          <div
                            className="h-full bg-emerald-500 dark:bg-emerald-400 transition-[width] duration-150 ease-out"
                            style={{ width: `${downloadPercent}%` }}
                          />
                        </div>
                      </div>
                    )}

                    {/* 错误显示 */}
                    {downloadState === 'failed' && downloadError && (
                      <div className="mb-3 px-2 py-1.5 rounded-md bg-red-50 dark:bg-red-500/10 text-[11px] text-red-600 dark:text-red-400 break-words">
                        {t.downloadFailed}: {downloadError}
                      </div>
                    )}

                    <div className="flex gap-2 flex-wrap">
                      {downloadState === 'idle' && (
                        <>
                          <button
                            type="button"
                            onClick={handleDownloadAndInstall}
                            className="flex items-center gap-1.5 text-[12px] font-medium px-3 py-1.5 rounded-md bg-emerald-600 text-white hover:bg-emerald-700 transition-all"
                            data-tauri-drag-region="false"
                          >
                            <Download size={12} />
                            {t.downloadAndInstall}
                          </button>
                          <button
                            type="button"
                            onClick={handleOpenReleasePage}
                            className="flex items-center gap-1.5 text-[12px] font-medium px-3 py-1.5 rounded-md text-neutral-600 dark:text-neutral-400 border border-black/10 dark:border-white/10 hover:bg-black/5 dark:hover:bg-white/5 transition-all"
                            data-tauri-drag-region="false"
                          >
                            <ExternalLink size={12} />
                            {t.downloadFromGithub}
                          </button>
                        </>
                      )}
                      {downloadState === 'downloading' && (
                        <button
                          type="button"
                          disabled
                          className="flex items-center gap-1.5 text-[12px] font-medium px-3 py-1.5 rounded-md bg-emerald-600/60 text-white cursor-not-allowed"
                        >
                          <RefreshCw size={12} className="animate-spin" />
                          {t.downloading}
                        </button>
                      )}
                      {downloadState === 'downloaded' && (
                        <button
                          type="button"
                          onClick={handleInstall}
                          className="flex items-center gap-1.5 text-[12px] font-medium px-3 py-1.5 rounded-md bg-emerald-600 text-white hover:bg-emerald-700 transition-all"
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
                            className="flex items-center gap-1.5 text-[12px] font-medium px-3 py-1.5 rounded-md bg-emerald-600 text-white hover:bg-emerald-700 transition-all"
                            data-tauri-drag-region="false"
                          >
                            <RefreshCw size={12} />
                            {t.retryDownload}
                          </button>
                          <button
                            type="button"
                            onClick={handleOpenReleasePage}
                            className="flex items-center gap-1.5 text-[12px] font-medium px-3 py-1.5 rounded-md text-neutral-600 dark:text-neutral-400 border border-black/10 dark:border-white/10 hover:bg-black/5 dark:hover:bg-white/5 transition-all"
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
                        className="text-[12px] font-medium px-3 py-1.5 rounded-md text-neutral-600 dark:text-neutral-400 hover:bg-black/5 dark:hover:bg-white/5 transition-all"
                        data-tauri-drag-region="false"
                      >
                        {t.updateLater}
                      </button>
                    </div>
                  </div>
                )}
              </div>
            </section>
          </div>
        )}
      </div>

      {/* 底部操作栏 */}
      <div className="flex justify-between items-center px-5 py-3 border-t border-black/5 dark:border-white/5 bg-white dark:bg-[#1C1C1E] shrink-0">
        <div className="flex items-center gap-3 min-w-0">
          <span className="text-[10px] font-medium text-neutral-400 dark:text-neutral-500 tracking-wide">v{appVersion}</span>
          {saveError && (
            <span
              className="text-[11px] text-red-500 dark:text-red-400 truncate max-w-[240px]"
              title={saveError}
            >
              {saveError}
            </span>
          )}
          {saveSuccess && !saveError && (
            <span className="text-[11px] text-emerald-600 dark:text-emerald-400">
              {t.saved}
            </span>
          )}
        </div>
        <div className="flex gap-3">
          <button
            onClick={handleCloseRequest}
            className="px-4 py-2 text-[13px] font-medium text-neutral-600 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-200 hover:bg-black/5 dark:hover:bg-white/5 rounded-lg transition-all duration-200"
            data-tauri-drag-region="false"
          >
            {t.cancel}
          </button>
          <button
            onClick={handleSave}
            disabled={saving}
            className="flex items-center gap-2 px-5 py-2 bg-neutral-900 dark:bg-white text-white dark:text-neutral-900 rounded-lg text-[13px] font-medium shadow-sm hover:shadow hover:bg-neutral-800 dark:hover:bg-neutral-100 disabled:opacity-60 disabled:cursor-not-allowed transition-all duration-200 active:scale-95"
            data-tauri-drag-region="false"
          >
            <Save size={14} strokeWidth={2} />
            {saving ? t.saving : t.save}
          </button>
        </div>
      </div>

      {/* 未保存更改确认弹窗 */}
      {closeConfirmOpen && (
        <div className="absolute inset-0 z-50 bg-black/30 backdrop-blur-[1px] flex items-center justify-center p-4" data-tauri-drag-region="false">
          <div className="w-full max-w-[320px] rounded-xl border border-black/10 dark:border-white/10 bg-white dark:bg-neutral-900 shadow-lg p-4 space-y-3">
            <h3 className="text-[14px] font-semibold text-neutral-900 dark:text-neutral-100">{t.unsavedChanges}</h3>
            <p className="text-[12px] text-neutral-600 dark:text-neutral-300 leading-relaxed">{t.unsavedChangesDesc}</p>
            <div className="flex justify-end gap-2 pt-1">
              <button
                type="button"
                onClick={() => setCloseConfirmOpen(false)}
                className="px-3 py-1.5 text-[12px] rounded-md text-neutral-600 dark:text-neutral-300 hover:bg-black/5 dark:hover:bg-white/5 transition-colors"
              >
                {t.continueEditing}
              </button>
              <button
                type="button"
                onClick={handleDiscardAndClose}
                className="px-3 py-1.5 text-[12px] rounded-md text-neutral-700 dark:text-neutral-200 border border-black/10 dark:border-white/10 hover:bg-black/5 dark:hover:bg-white/5 transition-colors"
              >
                {t.discardAndClose}
              </button>
              <button
                type="button"
                onClick={handleSaveAndClose}
                disabled={saving}
                className="px-3 py-1.5 text-[12px] rounded-md bg-neutral-900 dark:bg-white text-white dark:text-neutral-900 disabled:opacity-60 disabled:cursor-not-allowed"
              >
                {saving ? t.saving : t.saveAndClose}
              </button>
            </div>
          </div>
        </div>
      )}
      </div>
    </div>
  )
}
