import { useState, useEffect, useCallback, useRef } from 'react'
import {
  X, Save, Plus, Trash2, RefreshCw,
  Settings as SettingsIcon, Languages, Camera,
  Cloud, Info, Palette, Keyboard, SlidersHorizontal, Globe,
  Cpu, FileText, ShieldCheck, Sparkles
} from 'lucide-react'
import { api, type Settings as SettingsType, type ModelProvider, type DefaultPromptTemplates, type PermissionStatus } from './api/tauri'
import { i18n } from './settings/i18n'
import { buildHotkey } from './settings/utils'
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
  const [activeTab, setActiveTab] = useState<'general' | 'translate' | 'screenshot' | 'cowork' | 'providers' | 'about'>('general')
  const [saveError, setSaveError] = useState('')
  const [saveSuccess, setSaveSuccess] = useState(false)
  const [closeConfirmOpen, setCloseConfirmOpen] = useState(false)
  const [recordingTarget, setRecordingTarget] = useState<null | 'main' | 'screenshotTranslation' | 'cowork'>(null)
  const [defaultPrompts, setDefaultPrompts] = useState<DefaultPromptTemplates | null>(null)
  const [retryAttemptsInput, setRetryAttemptsInput] = useState('')
  const [permissionStatus, setPermissionStatus] = useState<PermissionStatus | null>(null)
  const [permissionsLoading, setPermissionsLoading] = useState(false)
  const [testingProviderId, setTestingProviderId] = useState<string | null>(null)
  const [providerTestFeedback, setProviderTestFeedback] = useState<Record<string, { ok: boolean; message: string }>>({})
  const saveSuccessTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const lang = settings?.settingsLanguage || 'zh'
  const t = i18n[lang]
  // 判断是否有未保存的更改
  const hasUnsavedChanges = settings ? JSON.stringify(settings) !== initialSettingsSnapshot : false

  // 初始化：加载设置、版本号、默认提示词
  useEffect(() => {
    let active = true
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
        // 使用默认设置以避免永远 loading
        const defaultSettings: SettingsData = {
          hotkey: 'CommandOrControl+Alt+T',
          theme: 'system',
          targetLang: 'auto',
          source: 'openai',
          autoPaste: true,
          launchAtStartup: false,
          translatorProviderId: 'default-translator',
          translatorModel: 'gpt-4o',
          translatorPrompt: '',
          providers: [
            { id: 'default-translator', name: 'OpenAI (Translator)', apiKey: '', baseUrl: 'https://api.openai.com/v1', availableModels: [], enabledModels: ['gpt-4o'] },
            { id: 'default-ocr', name: 'OpenAI (OCR)', apiKey: '', baseUrl: 'https://api.openai.com/v1', availableModels: [], enabledModels: ['gpt-4o'] }
          ],
          retryEnabled: true,
          retryAttempts: 3,
          screenshotTranslation: {
            enabled: true,
            hotkey: 'CommandOrControl+Shift+A',
            providerId: 'default-ocr',
            model: 'gpt-4o',
            directTranslate: false,
            thinkingEnabled: false,
            streamEnabled: true,
            prompt: ''
          },
          cowork: {
            enabled: true,
            hotkey: 'CommandOrControl+Shift+G',
            providerId: '',
            model: '',
            defaultLanguage: '',
            streamEnabled: true,
            thinkingEnabled: true,
            systemPrompt: '',
            questionPrompt: '',
            messageOrder: 'asc'
          },
          settingsLanguage: 'zh'
        }
        setSettings(defaultSettings)
        setInitialSettingsSnapshot(JSON.stringify(defaultSettings))
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
  }, [])

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
          apiKey: provider.apiKey,
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
      apiKey: '',
      baseUrl: 'https://api.openai.com/v1',
      availableModels: [],
      enabledModels: []
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
    // cowork providerId 为空表示 fallback 到 translator，删除时若已设置自身 provider 才需要级联
    const coworkHadOwnProvider = !!settings.cowork?.providerId
    const coworkProvider = coworkHadOwnProvider
      ? resolveProvider(nextProviders, settings.cowork?.providerId || '')
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
      ...(coworkHadOwnProvider ? {
        cowork: {
          ...settings.cowork,
          providerId: coworkProvider ? coworkProvider.id : '',
          model: resolveModel(coworkProvider, settings.cowork?.model || '')
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

      if (prev.cowork?.providerId === providerId) {
        next.cowork = {
          ...prev.cowork,
          model: resolveAfterRemoval(prev.cowork.model || ''),
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
          apiKey: currentProvider.apiKey,
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
   * 更新 Cowork 配置
   */
  const updateCowork = useCallback((updates: Partial<SettingsData['cowork']>) => {
    setSettings((prev) => {
      if (!prev) return prev
      const current = prev.cowork || {
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
      return { ...prev, cowork: { ...current, ...updates } }
    })
  }, [])

  /**
   * 切换快捷键录制状态
   */
  const toggleRecording = (target: 'main' | 'screenshotTranslation' | 'cowork') => {
    setRecordingTarget((current) => (current === target ? null : target))
  }

  // 当前语言对应的默认 cowork 提示词
  const coworkDefaults = defaultPrompts?.coworkPrompts?.[settings?.cowork?.defaultLanguage === 'en' ? 'en' : 'zh']

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
      } else if (recordingTarget === 'cowork') {
        updateCowork({ hotkey })
      }
      setRecordingTarget(null)
    }
    window.addEventListener('keydown', handler, true)
    return () => window.removeEventListener('keydown', handler, true)
  }, [recordingTarget, updateCowork, updateScreenshotTranslation, updateSettings])

  if (loading || !settings) {
    return (
      <div className="flex items-center justify-center h-full bg-neutral-200 dark:bg-black">
        <div className="w-6 h-6 border-2 border-neutral-300 dark:border-neutral-700 border-t-neutral-800 dark:border-t-neutral-200 rounded-full animate-spin" />
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
            { id: 'cowork' as const, label: t.coworkTabLabel, icon: Sparkles },
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
                        label={t.screenshotTranslateMode}
                        description={t.screenshotTranslateModeHint}
                      >
                        <Toggle
                          checked={settings.screenshotTranslation?.directTranslate ?? false}
                          onChange={(v) => updateScreenshotTranslation({ directTranslate: v })}
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

        {/* ===== Cowork 标签页 ===== */}
        {activeTab === 'cowork' && (
          <div className="space-y-8 animate-in fade-in slide-in-from-bottom-2 duration-300">
            <section>
              <SectionTitle icon={Sparkles}>{t.coworkSection}</SectionTitle>
              <div className="bg-white dark:bg-[#1C1C1E] rounded-xl shadow-[0_1px_3px_rgba(0,0,0,0.04)] overflow-hidden">
                <div className="divide-y divide-[#f0f0f0] dark:divide-white/5">
                  <SettingRow label={t.enabled}>
                    <Toggle
                      checked={settings.cowork?.enabled !== false}
                      onChange={(v) => updateCowork({ enabled: v })}
                    />
                  </SettingRow>

                  {settings.cowork?.enabled !== false && (
                    <>
                      <div className="px-4 py-3 space-y-1.5">
                        <span className="text-[12px] font-medium text-neutral-700 dark:text-neutral-200">{t.hotkey}</span>
                        <HotkeyInput
                          value={settings.cowork?.hotkey || 'CommandOrControl+Shift+G'}
                          placeholder="CommandOrControl+Shift+G"
                          recording={recordingTarget === 'cowork'}
                          onToggleRecording={() => toggleRecording('cowork')}
                          recordLabel={t.hotkeyRecord}
                          recordingLabel={t.hotkeyRecording}
                          recordingPlaceholder={t.hotkeyRecordingPlaceholder}
                        />
                      </div>
                      <SettingRow label={t.coworkResponseLanguage}>
                        <Select
                          className="w-44"
                          value={settings.cowork?.defaultLanguage || ''}
                          onChange={(v) => updateCowork({ defaultLanguage: v })}
                          options={[
                            { value: '', label: t.coworkLanguageInherit },
                            { value: 'zh', label: '中文' },
                            { value: 'en', label: 'English' },
                          ]}
                        />
                      </SettingRow>
                      <SettingRow label={t.coworkStreamEnabled}>
                        <Toggle
                          checked={settings.cowork?.streamEnabled !== false}
                          onChange={(v) => updateCowork({ streamEnabled: v })}
                        />
                      </SettingRow>
                      <SettingRow label={t.coworkThinkingEnabled} description={t.coworkThinkingHint}>
                        <Toggle
                          checked={settings.cowork?.thinkingEnabled !== false}
                          onChange={(v) => updateCowork({ thinkingEnabled: v })}
                        />
                      </SettingRow>
                      <SettingRow label={t.coworkMessageOrder}>
                        <Select
                          className="w-52"
                          value={settings.cowork?.messageOrder ?? 'asc'}
                          onChange={(v) => updateCowork({ messageOrder: v as 'asc' | 'desc' })}
                          options={[
                            { value: 'asc', label: t.coworkMessageOrderAsc },
                            { value: 'desc', label: t.coworkMessageOrderDesc },
                          ]}
                        />
                      </SettingRow>
                      <SettingRow label={t.selectModelPair}>
                        <Select
                          className="w-52"
                          value={`${settings.cowork?.providerId || ''}:${settings.cowork?.model || ''}`}
                          onChange={(v) => {
                            const [providerId, model] = v.split(':')
                            updateCowork({ providerId, model })
                          }}
                          options={[
                            { value: ':', label: t.coworkLanguageInherit },
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
                            <Label>{t.coworkSystemPrompt}</Label>
                            <TextArea
                              value={settings.cowork?.systemPrompt || ''}
                              onChange={(v) => updateCowork({ systemPrompt: v })}
                              placeholder={t.coworkPromptHint}
                              rows={2}
                            />
                            {!settings.cowork?.systemPrompt?.trim() && coworkDefaults?.system && (
                              <DefaultPrompt label={t.defaultTemplate} content={coworkDefaults.system} />
                            )}
                          </div>
                          <div>
                            <Label>{t.coworkQuestionPrompt}</Label>
                            <TextArea
                              value={settings.cowork?.questionPrompt || ''}
                              onChange={(v) => updateCowork({ questionPrompt: v })}
                              placeholder={t.coworkPromptHint}
                              rows={3}
                            />
                            {!settings.cowork?.questionPrompt?.trim() && coworkDefaults?.question && (
                              <DefaultPrompt label={t.defaultTemplate} content={coworkDefaults.question} />
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
            {settings.providers.map((provider) => (
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

                    {/* Base URL + API Key */}
                    <div className="px-4 py-3">
                      <div className="grid grid-cols-2 gap-4">
                        <div>
                          <Label>{t.baseUrl}</Label>
                          <div className="mt-1.5">
                            <Input
                              value={provider.baseUrl}
                              onChange={(v) => updateProvider(provider.id, { baseUrl: v })}
                              placeholder="https://api.openai.com/v1"
                            />
                          </div>
                        </div>
                        <div>
                          <Label>{t.apiKey}</Label>
                          <div className="mt-1.5">
                            <Input
                              type="password"
                              value={provider.apiKey}
                              onChange={(v) => updateProvider(provider.id, { apiKey: v })}
                              placeholder="sk-..."
                            />
                          </div>
                        </div>
                      </div>
                    </div>

                    {/* 连接测试 */}
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
                              if (e.key === 'Enter') {
                                addEnabledModel(provider.id, manualInputs[provider.id] || '')
                                setManualInputs(prev => ({ ...prev, [provider.id]: '' }))
                              }
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

                    {/* 可用模型 */}
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
                  </div>
                </div>
              </section>
            ))}

            {/* 添加新提供商按钮 */}
            <button
              onClick={addProvider}
              className="w-full py-4 border-2 border-dashed border-black/5 dark:border-white/5 rounded-[10px] text-neutral-400 hover:text-neutral-600 dark:hover:text-neutral-300 hover:border-black/10 dark:hover:border-white/10 hover:bg-black/5 dark:hover:bg-white/5 transition-all flex flex-col items-center gap-2"
            >
              <Plus size={20} strokeWidth={2} />
              <span className="text-[13px] font-medium">{t.addProvider}</span>
            </button>
          </div>
        )}

        {/* ===== 关于标签页 ===== */}
        {activeTab === 'about' && (
          <div className="space-y-8 animate-in fade-in slide-in-from-bottom-2 duration-300">
            <section>
              <div className="flex flex-col items-center justify-center py-10">
                <div className="w-16 h-16 rounded-2xl bg-neutral-900 dark:bg-white flex items-center justify-center mb-4 shadow-sm">
                  <span className="text-white dark:text-neutral-900 text-[20px] font-bold">K</span>
                </div>
                <h2 className="text-[16px] font-semibold text-neutral-900 dark:text-white mb-1">KeyLingo</h2>
                <p className="text-[13px] text-neutral-500 dark:text-neutral-400 mb-6">{lang === 'zh' ? '智能翻译与 AI 视觉工具' : 'Smart Translation & AI Vision Tool'}</p>
                <div className="bg-white dark:bg-[#1C1C1E] rounded-xl shadow-[0_1px_3px_rgba(0,0,0,0.04)] overflow-hidden w-full max-w-sm">
                  <div className="flex items-center justify-between px-4 py-3 border-b border-black/5 dark:border-white/5">
                    <span className="text-[13px] text-neutral-900 dark:text-neutral-100">{lang === 'zh' ? '版本' : 'Version'}</span>
                    <span className="text-[13px] text-neutral-500 dark:text-neutral-400">v{appVersion}</span>
                  </div>
                  <div className="flex items-center justify-between px-4 py-3">
                    <span className="text-[13px] text-neutral-900 dark:text-neutral-100">{lang === 'zh' ? '开发者' : 'Developer'}</span>
                    <span className="text-[13px] text-neutral-500 dark:text-neutral-400">ZM</span>
                  </div>
                </div>
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
