import { useState, useEffect, type ReactNode } from 'react'
import { X, Save, Globe, Keyboard, Camera, Sparkles, Cpu, Plus, Trash2, RefreshCw } from 'lucide-react'
import { api, type Settings as SettingsType, type ModelProvider } from './api/tauri'

type SettingsData = SettingsType

interface SettingsProps {
  onClose: () => void
  onSettingsChange: () => void
}

// 多语言文本
const i18n = {
  zh: {
    settings: '设置',
    save: '保存',
    cancel: '取消',
    tabGeneral: '基础',
    tabTranslate: '翻译',
    tabScreenshot: '截图',
    tabModels: '模型',
    theme: '主题',
    themeSystem: '跟随系统',
    themeLight: '浅色',
    themeDark: '深色',
    language: '界面语言',
    hotkey: '翻译快捷键',
    hotkeyPlaceholder: '例如: CommandOrControl+Alt+T',
    targetLang: '目标语言',
    langAuto: '自动 (中↔英)',
    langEn: '英语',
    langZh: '中文',
    langJa: '日语',
    langKo: '韩语',
    langFr: '法语',
    langDe: '德语',
    engine: '翻译引擎',
    engineAI: 'AI 翻译 (OpenAI 兼容)',
    baseUrl: '接口地址',
    apiKey: 'API 密钥',
    modelName: '模型名称',
    providerName: '供应商名称',
    addProvider: '添加驱动',
    editProvider: '编辑',
    deleteProvider: '删除',
    selectProvider: '选择驱动',
    fetchModels: '获取模型列表',
    fetching: '正在获取...',
    autoPaste: '自动上屏',
    screenshotTranslate: '截图翻译',
    screenshotExplain: '截图解释',
    enabled: '启用',
    responseLanguage: '回复语言',
    visionModel: '视觉模型',
    visionOpenai: 'OpenAI / 自定义',
    customPrompts: '自定义提示词',
    customPromptsHint: '留空使用默认值',
    systemPrompt: '系统提示词',
    summaryPrompt: '总结提示词',
    availableModels: '可用模型',
    registeredModels: '已启用模型',
    addModel: '添加',
    removeModel: '移除',
    manualAddModel: '手动添加',
    selectModelPair: '选择模型组合',
    version: '版本',
  },
  en: {
    settings: 'Settings',
    save: 'Save',
    cancel: 'Cancel',
    tabGeneral: 'General',
    tabTranslate: 'Translate',
    tabScreenshot: 'Screenshot',
    tabModels: 'Models',
    theme: 'Theme',
    themeSystem: 'System',
    themeLight: 'Light',
    themeDark: 'Dark',
    language: 'Language',
    hotkey: 'Hotkey',
    hotkeyPlaceholder: 'e.g. CommandOrControl+Alt+T',
    targetLang: 'Target Language',
    langAuto: 'Auto (ZH↔EN)',
    langEn: 'English',
    langZh: 'Chinese',
    langJa: 'Japanese',
    langKo: 'Korean',
    langFr: 'French',
    langDe: 'German',
    engine: 'Translation Engine',
    engineAI: 'AI (OpenAI Compatible)',
    baseUrl: 'Base URL',
    apiKey: 'API Key',
    modelName: 'Model Name',
    providerName: 'Provider Name',
    addProvider: 'Add Provider',
    editProvider: 'Edit',
    deleteProvider: 'Delete',
    selectProvider: 'Select Provider',
    fetchModels: 'Fetch Models',
    fetching: 'Fetching...',
    autoPaste: 'Auto Paste',
    screenshotTranslate: 'Screenshot Translation',
    screenshotExplain: 'Screenshot Explain',
    enabled: 'Enabled',
    responseLanguage: 'Response Language',
    visionModel: 'Vision Model',
    visionOpenai: 'OpenAI / Custom',
    customPrompts: 'Custom Prompts',
    customPromptsHint: 'Leave empty for defaults',
    systemPrompt: 'System Prompt',
    summaryPrompt: 'Summary Prompt',
    availableModels: 'Available Models',
    registeredModels: 'Enabled Models',
    addModel: 'Add',
    removeModel: 'Remove',
    manualAddModel: 'Manual Add',
    selectModelPair: 'Select Model Pair',
    version: 'Version',
  }
}

// 通用组件
function Toggle({ checked, onChange }: { checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      type="button"
      onClick={() => onChange(!checked)}
      className={`relative w-[34px] h-5 rounded-full transition-all duration-200 ease-in-out ${checked ? 'bg-neutral-900 dark:bg-white' : 'bg-neutral-200 dark:bg-neutral-700'}`}
      data-tauri-drag-region="false"
    >
      <span className={`absolute top-[2px] left-[2px] w-4 h-4 bg-white dark:bg-neutral-900 rounded-full shadow-sm transition-transform duration-200 ${checked ? 'translate-x-[14px]' : ''}`} />
    </button>
  )
}

function Select({ value, onChange, options, className = '' }: {
  value: string
  onChange: (v: string) => void
  options: { value: string; label: string }[]
  className?: string
}) {
  return (
    <div className="relative">
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className={`w-full appearance-none px-3 py-1.5 pr-8 rounded-lg border border-black/5 dark:border-white/5 bg-neutral-100 dark:bg-neutral-800 text-[13px] text-neutral-900 dark:text-neutral-100 focus:outline-none focus:ring-1 focus:ring-neutral-400 dark:focus:ring-neutral-500 transition-all ${className}`}
        data-tauri-drag-region="false"
      >
        {options.map(opt => <option key={opt.value} value={opt.value}>{opt.label}</option>)}
      </select>
      <div className="absolute right-2.5 top-1/2 -translate-y-1/2 pointer-events-none text-neutral-400">
        <svg width="10" height="6" viewBox="0 0 10 6" fill="none" xmlns="http://www.w3.org/2000/svg">
          <path d="M1 1L5 5L9 1" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </div>
    </div>
  )
}

function Input({ value, onChange, type = 'text', placeholder = '', className = '', list, ...props }: {
  value: string
  onChange: (v: string) => void
  type?: string
  placeholder?: string
  className?: string
  list?: string
} & Omit<React.InputHTMLAttributes<HTMLInputElement>, 'value' | 'onChange'>) {
  return (
    <input
      type={type}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={placeholder}
      list={list}
      className={`w-full px-3 py-1.5 rounded-lg border border-black/5 dark:border-white/5 bg-neutral-100 dark:bg-neutral-800 text-[13px] font-mono text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 focus:outline-none focus:ring-1 focus:ring-neutral-400 dark:focus:ring-neutral-500 transition-all ${className}`}
      data-tauri-drag-region="false"
      {...props}
    />
  )
}

function TextArea({ value, onChange, placeholder = '', rows = 2 }: {
  value: string
  onChange: (v: string) => void
  placeholder?: string
  rows?: number
}) {
  return (
    <textarea
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={placeholder}
      rows={rows}
      className="w-full px-3 py-2 rounded-lg border border-black/5 dark:border-white/5 bg-neutral-100 dark:bg-neutral-800 text-[13px] font-mono text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 focus:outline-none focus:ring-1 focus:ring-neutral-400 dark:focus:ring-neutral-500 transition-all resize-none"
      data-tauri-drag-region="false"
    />
  )
}

function Label({ children, className = '' }: { children: ReactNode; className?: string }) {
  return <label className={`block text-[11px] font-medium text-neutral-500 dark:text-neutral-400 mb-1.5 uppercase tracking-wide ${className}`}>{children}</label>
}

function Card({ children, className = '' }: { children: ReactNode; className?: string }) {
  return (
    <div className={`p-4 rounded-xl bg-white dark:bg-neutral-800/50 border border-black/5 dark:border-white/5 shadow-sm ${className}`}>
      {children}
    </div>
  )
}

function SectionTitle({ icon, children }: { icon: ReactNode; children: ReactNode }) {
  return (
    <div className="flex items-center gap-2 mb-4">
      <div className="p-1 rounded-md bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-300">
        {icon}
      </div>
      <span className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">{children}</span>
    </div>
  )
}

// 标签页按钮
function TabButton({ active, onClick, icon, label }: {
  active: boolean
  onClick: () => void
  icon: ReactNode
  label: string
}) {
  return (
    <button
      onClick={onClick}
      className={`flex items-center gap-2 px-3 py-1.5 rounded-lg text-[13px] font-medium transition-all duration-200 ${active
        ? 'bg-white dark:bg-neutral-800 text-neutral-900 dark:text-white shadow-sm ring-1 ring-black/5 dark:ring-white/10'
        : 'text-neutral-500 dark:text-neutral-400 hover:bg-black/5 dark:hover:bg-white/5 hover:text-neutral-700 dark:hover:text-neutral-200'
        }`}
      data-tauri-drag-region="false"
    >
      {icon}
      {label}
    </button>
  )
}

export default function Settings({ onClose, onSettingsChange }: SettingsProps) {
  const [settings, setSettings] = useState<SettingsData | null>(null)
  const [loading, setLoading] = useState(true)
  const [appVersion, setAppVersion] = useState('')
  const [activeTab, setActiveTab] = useState<'general' | 'translate' | 'screenshot' | 'models'>('general')

  const lang = settings?.settingsLanguage || 'zh'
  const t = i18n[lang]

  useEffect(() => {
    api.getSettings()
      .then((data: SettingsData) => {
        setSettings(data)
        setLoading(false)
      })
      .catch((err) => {
        console.error('Failed to load settings:', err)
        // 使用默认设置以避免永远 loading
        setSettings({
          hotkey: 'CommandOrControl+Alt+T',
          theme: 'system',
          targetLang: 'auto',
          source: 'openai',
          autoPaste: true,
          translatorProviderId: 'default-translator',
          translatorModel: 'gpt-4o',
          providers: [
            { id: 'default-translator', name: 'OpenAI (Translator)', apiKey: '', baseUrl: 'https://api.openai.com/v1', availableModels: [], enabledModels: ['gpt-4o'] },
            { id: 'default-ocr', name: 'OpenAI (OCR)', apiKey: '', baseUrl: 'https://api.openai.com/v1', availableModels: [], enabledModels: ['gpt-4o'] },
            { id: 'default-explain', name: 'OpenAI (Explain)', apiKey: '', baseUrl: 'https://api.openai.com/v1', availableModels: [], enabledModels: ['gpt-4o'] }
          ],
          screenshotTranslation: {
            enabled: true,
            hotkey: 'CommandOrControl+Shift+A',
            providerId: 'default-ocr',
            model: 'gpt-4o'
          },
          screenshotExplain: {
            enabled: true,
            hotkey: 'CommandOrControl+Shift+E',
            providerId: 'default-explain',
            model: 'gpt-4o',
            defaultLanguage: 'zh'
          },
          explainHistory: [],
          settingsLanguage: 'zh'
        })
        setLoading(false)
      })
    api.getAppVersion()
      .then((ver: string) => setAppVersion(ver))
      .catch(() => setAppVersion('unknown'))
    // resizeWindow 已在 App.tsx 中处理，此处不再重复调用
  }, [])

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        console.log('[Settings] ESC pressed, calling onClose')
        onClose()
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [onClose])

  const handleSave = async () => {
    if (!settings) return
    await api.saveSettings(settings)
    onSettingsChange()
    onClose()
  }

  const updateSettings = (updates: Partial<SettingsData>) => {
    if (!settings) return
    setSettings({ ...settings, ...updates })
  }

  const updateProvider = (id: string, updates: Partial<ModelProvider>) => {
    if (!settings) return
    setSettings({
      ...settings,
      providers: settings.providers.map(p => p.id === id ? { ...p, ...updates } : p)
    })
  }

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

  const resolveProvider = (providers: ModelProvider[], providerId: string) => {
    return providers.find(p => p.id === providerId) ?? providers[0]
  }

  const resolveModel = (provider: ModelProvider | undefined, currentModel: string) => {
    if (!provider) return currentModel
    if (provider.enabledModels.includes(currentModel)) return currentModel
    return provider.enabledModels[0] || currentModel
  }

  const deleteProvider = (id: string) => {
    if (!settings) return
    const nextProviders = settings.providers.filter(p => p.id !== id)
    const translatorProvider = resolveProvider(nextProviders, settings.translatorProviderId)
    const screenshotProvider = resolveProvider(nextProviders, settings.screenshotTranslation?.providerId || '')
    const explainProvider = resolveProvider(nextProviders, settings.screenshotExplain?.providerId || '')

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
      screenshotExplain: {
        ...settings.screenshotExplain,
        providerId: explainProvider ? explainProvider.id : '',
        model: resolveModel(explainProvider, settings.screenshotExplain?.model || '')
      }
    })
  }

  const addEnabledModel = (providerId: string, model: string) => {
    if (!settings || !model.trim()) return
    const provider = settings.providers.find(p => p.id === providerId)
    if (!provider || provider.enabledModels.includes(model)) return
    updateProvider(providerId, {
      enabledModels: [...provider.enabledModels, model.trim()]
    })
  }

  const removeEnabledModel = (providerId: string, model: string) => {
    if (!settings) return
    const provider = settings.providers.find(p => p.id === providerId)
    if (!provider) return
    updateProvider(providerId, {
      enabledModels: provider.enabledModels.filter(m => m !== model)
    })
  }

  const [fetchingProviderId, setFetchingProviderId] = useState<string | null>(null)
  const [manualInputs, setManualInputs] = useState<Record<string, string>>({})

  const fetchModels = async (providerId: string) => {
    if (!settings || fetchingProviderId) return
    setFetchingProviderId(providerId)
    try {
      const models = await api.fetchModels(providerId)
      const provider = settings.providers.find(p => p.id === providerId)
      if (provider) {
        updateProvider(providerId, { availableModels: models })
      }
    } catch (err) {
      console.error('Failed to fetch models:', err)
    } finally {
      setFetchingProviderId(null)
    }
  }

  const updateScreenshotTranslation = (updates: Partial<SettingsData['screenshotTranslation']>) => {
    if (!settings) return
    const current = settings.screenshotTranslation || {
      enabled: true,
      hotkey: 'CommandOrControl+Shift+A',
      providerId: 'default-ocr'
    }
    setSettings({ ...settings, screenshotTranslation: { ...current, ...updates } })
  }

  const updateScreenshotExplain = (updates: Partial<SettingsData['screenshotExplain']>) => {
    if (!settings) return
    const current = settings.screenshotExplain || {
      enabled: true,
      hotkey: 'CommandOrControl+Shift+E',
      providerId: 'default-explain',
      defaultLanguage: 'zh'
    }
    setSettings({ ...settings, screenshotExplain: { ...current, ...updates } })
  }

  const updateCustomPrompts = (updates: Partial<NonNullable<SettingsData['screenshotExplain']['customPrompts']>>) => {
    if (!settings) return
    const current = settings.screenshotExplain || {
      enabled: true,
      hotkey: 'CommandOrControl+Shift+E',
      providerId: 'default-explain',
      defaultLanguage: 'zh'
    }
    setSettings({
      ...settings,
      screenshotExplain: {
        ...current,
        customPrompts: { ...current.customPrompts, ...updates }
      }
    })
  }

  if (loading || !settings) {
    return (
      <div className="flex items-center justify-center h-full bg-neutral-50 dark:bg-neutral-900">
        <div className="w-6 h-6 border-2 border-neutral-300 dark:border-neutral-700 border-t-neutral-800 dark:border-t-neutral-200 rounded-full animate-spin" />
      </div>
    )
  }

  return (
    <div className="window-container flex flex-col bg-white/95 dark:bg-neutral-900/95 backdrop-blur-2xl text-neutral-900 dark:text-neutral-100 font-sans rounded-xl shadow-[0_8px_32px_rgba(0,0,0,0.12)] overflow-hidden">
      {/* 标题栏 */}
      <div
        className="flex justify-between items-center px-5 py-4 border-b border-black/5 dark:border-white/5 bg-white/50 dark:bg-neutral-900/50 backdrop-blur-xl rounded-t-xl"
        data-tauri-drag-region
      >
        <h2 className="font-semibold text-[15px] tracking-tight">{t.settings}</h2>
        <button
          onClick={onClose}
          className="p-1.5 hover:bg-black/5 dark:hover:bg-white/10 rounded-lg text-neutral-400 hover:text-neutral-600 dark:hover:text-neutral-300 transition-all duration-200"
          data-tauri-drag-region="false"
        >
          <X size={18} strokeWidth={2} />
        </button>
      </div>

      {/* 标签页导航 */}
      <div className="px-2 py-2 border-b border-black/5 dark:border-white/5 bg-neutral-100/50 dark:bg-neutral-900/30">
        <div className="flex p-1 bg-neutral-200/50 dark:bg-neutral-800/50 rounded-xl">
          <TabButton
            active={activeTab === 'general'}
            onClick={() => setActiveTab('general')}
            icon={<Globe size={14} strokeWidth={2} />}
            label={t.tabGeneral}
          />
          <TabButton
            active={activeTab === 'translate'}
            onClick={() => setActiveTab('translate')}
            icon={<Keyboard size={14} strokeWidth={2} />}
            label={t.tabTranslate}
          />
          <TabButton
            active={activeTab === 'screenshot'}
            onClick={() => setActiveTab('screenshot')}
            icon={<Camera size={14} strokeWidth={2} />}
            label={t.tabScreenshot}
          />
          <TabButton
            active={activeTab === 'models'}
            onClick={() => setActiveTab('models')}
            icon={<Cpu size={14} strokeWidth={2} />}
            label={t.tabModels}
          />
        </div>
      </div>

      {/* 内容区域 */}
      <div className="flex-1 overflow-auto p-5 space-y-5 scrollbar-hide">
        {/* 基础设置 */}
        {activeTab === 'general' && (
          <div className="space-y-4 animate-in fade-in slide-in-from-bottom-2 duration-300">
            <Card>
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <Label>{t.theme}</Label>
                  <Select
                    value={settings.theme || 'system'}
                    onChange={(v) => updateSettings({ theme: v as SettingsData['theme'] })}
                    options={[
                      { value: 'system', label: t.themeSystem },
                      { value: 'light', label: t.themeLight },
                      { value: 'dark', label: t.themeDark },
                    ]}
                  />
                </div>
                <div>
                  <Label>{t.language}</Label>
                  <Select
                    value={settings.settingsLanguage || 'zh'}
                    onChange={(v) => updateSettings({ settingsLanguage: v as 'zh' | 'en' })}
                    options={[
                      { value: 'zh', label: '中文' },
                      { value: 'en', label: 'English' },
                    ]}
                  />
                </div>
              </div>
            </Card>

            <Card>
              <Label>{t.hotkey}</Label>
              <Input
                value={settings.hotkey}
                onChange={(v) => updateSettings({ hotkey: v })}
                placeholder={t.hotkeyPlaceholder}
              />
            </Card>

            <Card>
              <div className="flex items-center justify-between">
                <Label>{t.autoPaste}</Label>
                <Toggle
                  checked={settings.autoPaste ?? true}
                  onChange={(v) => updateSettings({ autoPaste: v })}
                />
              </div>
            </Card>
          </div>
        )}

        {/* 翻译设置 */}
        {activeTab === 'translate' && (
          <div className="space-y-4 animate-in fade-in slide-in-from-bottom-2 duration-300">
            <Card>
              <Label>{t.targetLang}</Label>
              <Select
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
            </Card>

            <Card>
              <SectionTitle icon={<Globe size={14} strokeWidth={2} />}>
                {t.engine}
              </SectionTitle>
              <div className="space-y-4">
                <div>
                  <Label>{t.selectModelPair}</Label>
                  <Select
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
                </div>
              </div>
            </Card>
          </div>
        )}

        {/* 截图设置 */}
        {activeTab === 'screenshot' && (
          <div className="space-y-4 animate-in fade-in slide-in-from-bottom-2 duration-300">
            {/* 截图翻译 */}
            <Card>
              <div className="flex items-center justify-between mb-4">
                <SectionTitle icon={<Camera size={14} className="text-purple-500" strokeWidth={2} />}>
                  {t.screenshotTranslate}
                </SectionTitle>
                <Toggle
                  checked={settings.screenshotTranslation?.enabled ?? true}
                  onChange={(v) => updateScreenshotTranslation({ enabled: v })}
                />
              </div>

              {settings.screenshotTranslation?.enabled !== false && (
                <div className="space-y-4 animate-in fade-in slide-in-from-top-2 duration-200">
                  <div>
                    <Label>{t.hotkey}</Label>
                    <Input
                      value={settings.screenshotTranslation?.hotkey || 'CommandOrControl+Shift+A'}
                      onChange={(v) => updateScreenshotTranslation({ hotkey: v })}
                      placeholder="CommandOrControl+Shift+A"
                    />
                  </div>
                  <div>
                    <Label>{t.selectModelPair}</Label>
                    <Select
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
                  </div>
                </div>
              )}
            </Card>

            {/* 截图解释 */}
            <Card>
              <div className="flex items-center justify-between mb-4">
                <SectionTitle icon={<Sparkles size={14} className="text-amber-500" strokeWidth={2} />}>
                  {t.screenshotExplain}
                </SectionTitle>
                <Toggle
                  checked={settings.screenshotExplain?.enabled !== false}
                  onChange={(v) => updateScreenshotExplain({ enabled: v })}
                />
              </div>

              {settings.screenshotExplain?.enabled !== false && (
                <div className="space-y-4 animate-in fade-in slide-in-from-top-2 duration-200">
                  <div>
                    <Label>{t.hotkey}</Label>
                    <Input
                      value={settings.screenshotExplain?.hotkey || 'CommandOrControl+Shift+E'}
                      onChange={(v) => updateScreenshotExplain({ hotkey: v })}
                      placeholder="CommandOrControl+Shift+E"
                    />
                  </div>
                  <div>
                    <Label>{t.responseLanguage}</Label>
                    <Select
                      value={settings.screenshotExplain?.defaultLanguage || 'zh'}
                      onChange={(v) => updateScreenshotExplain({ defaultLanguage: v as 'zh' | 'en' })}
                      options={[
                        { value: 'zh', label: '中文' },
                        { value: 'en', label: 'English' },
                      ]}
                    />
                  </div>
                  <div>
                    <Label>{t.selectModelPair}</Label>
                    <Select
                      value={`${settings.screenshotExplain.providerId}:${settings.screenshotExplain.model}`}
                      onChange={(v) => {
                        const [providerId, model] = v.split(':')
                        updateScreenshotExplain({ providerId, model })
                      }}
                      options={settings.providers.flatMap(p =>
                        p.enabledModels.map(m => ({
                          value: `${p.id}:${m}`,
                          label: `${p.name} - ${m}`
                        }))
                      )}
                    />
                  </div>

                  {/* 自定义提示词 */}
                  <details className="group pt-2 border-t border-black/5 dark:border-white/5">
                    <summary className="flex items-center gap-2 cursor-pointer text-[11px] font-medium text-neutral-500 hover:text-neutral-800 dark:hover:text-neutral-200 transition-colors list-none">
                      <div className="p-0.5 rounded bg-neutral-200 dark:bg-neutral-700 text-neutral-500 dark:text-neutral-400 group-open:rotate-90 transition-transform">
                        <svg width="8" height="8" viewBox="0 0 8 8" fill="none" xmlns="http://www.w3.org/2000/svg">
                          <path d="M2.5 1.5L5.5 4L2.5 6.5" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
                        </svg>
                      </div>
                      {t.customPrompts}
                    </summary>
                    <div className="mt-4 space-y-4 pl-1 animate-in slide-in-from-top-2 duration-200">
                      <div>
                        <Label>{t.systemPrompt}</Label>
                        <TextArea
                          value={settings.screenshotExplain?.customPrompts?.systemPrompt || ''}
                          onChange={(v) => updateCustomPrompts({ systemPrompt: v })}
                          placeholder={t.customPromptsHint}
                          rows={2}
                        />
                      </div>
                      <div>
                        <Label>{t.summaryPrompt}</Label>
                        <TextArea
                          value={settings.screenshotExplain?.customPrompts?.summaryPrompt || ''}
                          onChange={(v) => updateCustomPrompts({ summaryPrompt: v })}
                          placeholder={t.customPromptsHint}
                          rows={3}
                        />
                      </div>
                    </div>
                  </details>
                </div>
              )}
            </Card>
          </div>
        )}

        {/* 模型管理 */}
        {activeTab === 'models' && (
          <div className="space-y-4 animate-in fade-in slide-in-from-bottom-2 duration-300">
            {settings.providers.map((provider) => (
              <Card key={provider.id} className="relative group">
                <div className="absolute right-4 top-4 flex gap-2 opacity-0 group-hover:opacity-100 transition-opacity">
                  <button
                    onClick={() => deleteProvider(provider.id)}
                    className="p-1.5 text-neutral-400 hover:text-red-500 hover:bg-red-50 dark:hover:bg-red-950/30 rounded-lg transition-all"
                  >
                    <Trash2 size={14} />
                  </button>
                </div>

                <div className="space-y-4">
                  <div>
                    <Label>{t.providerName}</Label>
                    <Input
                      value={provider.name}
                      onChange={(v) => updateProvider(provider.id, { name: v })}
                      placeholder="e.g. OpenAI / DeepSeek"
                    />
                  </div>
                  <div className="grid grid-cols-2 gap-4">
                    <div>
                      <Label>{t.baseUrl}</Label>
                      <Input
                        value={provider.baseUrl}
                        onChange={(v) => updateProvider(provider.id, { baseUrl: v })}
                        placeholder="https://api.openai.com/v1"
                      />
                    </div>
                    <div>
                      <Label>{t.apiKey}</Label>
                      <Input
                        type="password"
                        value={provider.apiKey}
                        onChange={(v) => updateProvider(provider.id, { apiKey: v })}
                        placeholder="sk-..."
                      />
                    </div>
                  </div>

                  {/* 已启用模型管理 */}
                  <div className="space-y-3 pt-3 border-t border-black/5 dark:border-white/5">
                    <div className="flex justify-between items-center">
                      <Label className="mb-0">{t.registeredModels}</Label>
                      <div className="flex gap-2">
                        <div className="relative flex items-center gap-1">
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
                    </div>
                    <div className="flex flex-wrap gap-2 min-h-[24px]">
                      {provider.enabledModels.map(model => (
                        <span key={model} className="flex items-center gap-1.5 px-2 py-0.5 bg-neutral-100 dark:bg-neutral-800 rounded-md text-[11px] text-neutral-700 dark:text-neutral-300 font-mono border border-black/5 dark:border-white/5 group/tag">
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

                  {/* 获取可用模型 */}
                  <div className="space-y-2">
                    <div className="flex justify-between items-center">
                      <Label className="mb-0">{t.availableModels}</Label>
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
                    <div className="flex flex-wrap gap-1.5 max-h-32 overflow-y-auto pr-1 scrollbar-thin">
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
              </Card>
            ))}

            <button
              onClick={addProvider}
              className="w-full py-4 border-2 border-dashed border-black/5 dark:border-white/5 rounded-xl text-neutral-400 hover:text-neutral-600 dark:hover:text-neutral-300 hover:border-black/10 dark:hover:border-white/10 hover:bg-black/5 dark:hover:bg-white/5 transition-all flex flex-col items-center gap-2"
            >
              <Plus size={20} strokeWidth={2} />
              <span className="text-[13px] font-medium">{t.addProvider}</span>
            </button>
          </div>
        )}
      </div>

      {/* 底部操作栏 */}
      <div className="flex justify-between items-center px-5 py-4 border-t border-black/5 dark:border-white/5 bg-white/50 dark:bg-neutral-900/50 backdrop-blur-xl rounded-b-xl">
        <span className="text-[10px] font-medium text-neutral-400 dark:text-neutral-500 tracking-wide">v{appVersion}</span>
        <div className="flex gap-3">
          <button
            onClick={onClose}
            className="px-4 py-2 text-[13px] font-medium text-neutral-600 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-200 hover:bg-black/5 dark:hover:bg-white/5 rounded-lg transition-all duration-200"
            data-tauri-drag-region="false"
          >
            {t.cancel}
          </button>
          <button
            onClick={handleSave}
            className="flex items-center gap-2 px-5 py-2 bg-neutral-900 dark:bg-white text-white dark:text-neutral-900 rounded-lg text-[13px] font-medium shadow-sm hover:shadow hover:bg-neutral-800 dark:hover:bg-neutral-100 transition-all duration-200 active:scale-95"
            data-tauri-drag-region="false"
          >
            <Save size={14} strokeWidth={2} />
            {t.save}
          </button>
        </div>
      </div>
    </div>
  )
}
