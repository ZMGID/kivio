import { useState, useEffect, type CSSProperties, type ReactNode } from 'react'
import { X, Save, Globe, Keyboard, Camera, Sparkles } from 'lucide-react'

type AppRegionStyle = CSSProperties & { WebkitAppRegion?: 'drag' | 'no-drag' }
const dragStyle: AppRegionStyle = { WebkitAppRegion: 'drag' }
const noDragStyle: AppRegionStyle = { WebkitAppRegion: 'no-drag' }

type SettingsData = {
  hotkey: string
  theme: 'system' | 'light' | 'dark'
  targetLang: string
  source: 'bing' | 'openai' | 'custom'
  openai: {
    apiKey: string
    baseURL: string
    model: string
  }
  screenshotTranslation: {
    enabled: boolean
    hotkey: string
    ocrSource: 'system' | 'glm' | 'openai'
    glmApiKey: string
    openai?: {
      apiKey: string
      baseURL: string
      model: string
    }
  }
  screenshotExplain: {
    enabled: boolean
    hotkey: string
    model: {
      provider: 'glm' | 'openai'
      apiKey: string
      baseURL: string
      modelName: string
    }
    defaultLanguage: 'zh' | 'en'
    customPrompts?: {
      systemPrompt?: string
      summaryPrompt?: string
      questionPrompt?: string
    }
  }
  settingsLanguage?: 'zh' | 'en'
}

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
    theme: '主题',
    themeSystem: '跟随系统',
    themeLight: '浅色',
    themeDark: '深色',
    language: '界面语言',
    hotkey: '翻译快捷键',
    hotkeyPlaceholder: '例如: Command+Option+T',
    targetLang: '目标语言',
    langAuto: '自动 (中↔英)',
    langEn: '英语',
    langZh: '中文',
    langJa: '日语',
    langKo: '韩语',
    langFr: '法语',
    langDe: '德语',
    engine: '翻译引擎',
    engineBing: 'Bing 翻译 (免费)',
    engineAI: 'AI 翻译 (DeepSeek/智谱等)',
    baseUrl: '接口地址',
    apiKey: 'API 密钥',
    modelName: '模型名称',
    screenshotTranslate: '截图翻译',
    screenshotExplain: '截图解释',
    enabled: '启用',
    ocrSource: 'OCR 识别源',
    ocrSystem: '系统 OCR (离线免费)',
    ocrGlm: 'GLM-4V (在线高精度)',
    ocrOpenai: 'OpenAI (自定义)',
    getApiKey: '获取 API Key',
    responseLanguage: '回复语言',
    visionModel: '视觉模型',
    visionGlm: 'GLM-4V (推荐)',
    visionOpenai: 'OpenAI / 自定义',
    customPrompts: '自定义提示词',
    customPromptsHint: '留空使用默认值',
    systemPrompt: '系统提示词',
    summaryPrompt: '总结提示词',
    version: '版本',
  },
  en: {
    settings: 'Settings',
    save: 'Save',
    cancel: 'Cancel',
    tabGeneral: 'General',
    tabTranslate: 'Translate',
    tabScreenshot: 'Screenshot',
    theme: 'Theme',
    themeSystem: 'System',
    themeLight: 'Light',
    themeDark: 'Dark',
    language: 'Language',
    hotkey: 'Hotkey',
    hotkeyPlaceholder: 'e.g. Command+Option+T',
    targetLang: 'Target Language',
    langAuto: 'Auto (ZH↔EN)',
    langEn: 'English',
    langZh: 'Chinese',
    langJa: 'Japanese',
    langKo: 'Korean',
    langFr: 'French',
    langDe: 'German',
    engine: 'Translation Engine',
    engineBing: 'Bing Translate (Free)',
    engineAI: 'AI (DeepSeek/Zhipu/OpenAI)',
    baseUrl: 'Base URL',
    apiKey: 'API Key',
    modelName: 'Model Name',
    screenshotTranslate: 'Screenshot Translation',
    screenshotExplain: 'Screenshot Explain',
    enabled: 'Enabled',
    ocrSource: 'OCR Source',
    ocrSystem: 'System OCR (Offline)',
    ocrGlm: 'GLM-4V (Online)',
    ocrOpenai: 'OpenAI (Custom)',
    getApiKey: 'Get API Key',
    responseLanguage: 'Response Language',
    visionModel: 'Vision Model',
    visionGlm: 'GLM-4V (Recommended)',
    visionOpenai: 'OpenAI / Custom',
    customPrompts: 'Custom Prompts',
    customPromptsHint: 'Leave empty for defaults',
    systemPrompt: 'System Prompt',
    summaryPrompt: 'Summary Prompt',
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
      style={noDragStyle}
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
        style={noDragStyle}
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

function Input({ value, onChange, type = 'text', placeholder = '', className = '' }: {
  value: string
  onChange: (v: string) => void
  type?: string
  placeholder?: string
  className?: string
}) {
  return (
    <input
      type={type}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={placeholder}
      className={`w-full px-3 py-1.5 rounded-lg border border-black/5 dark:border-white/5 bg-neutral-100 dark:bg-neutral-800 text-[13px] font-mono text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 focus:outline-none focus:ring-1 focus:ring-neutral-400 dark:focus:ring-neutral-500 transition-all ${className}`}
      style={noDragStyle}
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
      style={noDragStyle}
    />
  )
}

function Label({ children }: { children: ReactNode }) {
  return <label className="block text-[11px] font-medium text-neutral-500 dark:text-neutral-400 mb-1.5 uppercase tracking-wide">{children}</label>
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
      style={noDragStyle}
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
  const [activeTab, setActiveTab] = useState<'general' | 'translate' | 'screenshot'>('general')

  const lang = settings?.settingsLanguage || 'zh'
  const t = i18n[lang]

  useEffect(() => {
    if (window.api) {
      window.api.getSettings().then((data: SettingsData) => {
        setSettings(data)
        setLoading(false)
      })
      window.api.getAppVersion().then((ver: string) => {
        setAppVersion(ver)
      })
    }
  }, [])

  const handleSave = async () => {
    if (!settings || !window.api) return
    await window.api.saveSettings(settings)
    onSettingsChange()
    onClose()
  }

  const updateSettings = (updates: Partial<SettingsData>) => {
    if (!settings) return
    setSettings({ ...settings, ...updates })
  }

  const updateOpenAI = (updates: Partial<SettingsData['openai']>) => {
    if (!settings) return
    setSettings({ ...settings, openai: { ...settings.openai, ...updates } })
  }

  const updateScreenshotTranslation = (updates: Partial<SettingsData['screenshotTranslation']>) => {
    if (!settings) return
    const current = settings.screenshotTranslation || { enabled: true, hotkey: 'Command+Shift+A', ocrSource: 'system', glmApiKey: '' }
    setSettings({ ...settings, screenshotTranslation: { ...current, ...updates } })
  }

  const updateScreenshotTranslationOpenAI = (updates: Partial<NonNullable<SettingsData['screenshotTranslation']['openai']>>) => {
    if (!settings) return
    const current = settings.screenshotTranslation || { enabled: true, hotkey: 'Command+Shift+A', ocrSource: 'system', glmApiKey: '' }
    const currentOpenAI = current.openai || { apiKey: '', baseURL: 'https://api.openai.com/v1', model: 'gpt-4o' }

    setSettings({
      ...settings,
      screenshotTranslation: {
        ...current,
        openai: { ...currentOpenAI, ...updates }
      }
    })
  }

  const updateScreenshotExplain = (updates: Partial<SettingsData['screenshotExplain']>) => {
    if (!settings) return
    const current = settings.screenshotExplain || {
      enabled: true,
      hotkey: 'Command+Shift+E',
      model: { provider: 'glm', apiKey: '', baseURL: 'https://open.bigmodel.cn/api/paas/v4', modelName: 'glm-4v-flash' },
      defaultLanguage: 'zh'
    }
    setSettings({ ...settings, screenshotExplain: { ...current, ...updates } })
  }

  const updateExplainModel = (updates: Partial<SettingsData['screenshotExplain']['model']>) => {
    if (!settings) return
    const current = settings.screenshotExplain || {
      enabled: true,
      hotkey: 'Command+Shift+E',
      model: { provider: 'glm', apiKey: '', baseURL: 'https://open.bigmodel.cn/api/paas/v4', modelName: 'glm-4v-flash' },
      defaultLanguage: 'zh'
    }
    setSettings({
      ...settings,
      screenshotExplain: {
        ...current,
        model: { ...current.model, ...updates }
      }
    })
  }

  const updateCustomPrompts = (updates: Partial<NonNullable<SettingsData['screenshotExplain']['customPrompts']>>) => {
    if (!settings) return
    const current = settings.screenshotExplain || {
      enabled: true,
      hotkey: 'Command+Shift+E',
      model: { provider: 'glm', apiKey: '', baseURL: 'https://open.bigmodel.cn/api/paas/v4', modelName: 'glm-4v-flash' },
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
    <div className="flex flex-col h-full bg-neutral-50/50 dark:bg-neutral-900/95 text-neutral-900 dark:text-neutral-100 select-none font-sans">
      {/* 标题栏 */}
      <div className="flex justify-between items-center px-5 py-4 border-b border-black/5 dark:border-white/5 bg-white/50 dark:bg-neutral-900/50 backdrop-blur-xl" style={dragStyle}>
        <h2 className="font-semibold text-[15px] tracking-tight">{t.settings}</h2>
        <button
          onClick={onClose}
          className="p-1.5 hover:bg-black/5 dark:hover:bg-white/10 rounded-lg text-neutral-400 hover:text-neutral-600 dark:hover:text-neutral-300 transition-all duration-200"
          style={noDragStyle}
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
              <Label>{t.engine}</Label>
              <Select
                value={settings.source}
                onChange={(v) => updateSettings({ source: v as SettingsData['source'] })}
                options={[
                  { value: 'bing', label: t.engineBing },
                  { value: 'openai', label: t.engineAI },
                ]}
              />

              {settings.source === 'openai' && (
                <div className="mt-4 pt-4 border-t border-black/5 dark:border-white/5 space-y-4">
                  <div>
                    <Label>{t.baseUrl}</Label>
                    <Input
                      value={settings.openai.baseURL}
                      onChange={(v) => updateOpenAI({ baseURL: v })}
                      placeholder="https://api.deepseek.com/v1"
                    />
                  </div>
                  <div>
                    <Label>{t.apiKey}</Label>
                    <Input
                      type="password"
                      value={settings.openai.apiKey}
                      onChange={(v) => updateOpenAI({ apiKey: v })}
                      placeholder="sk-..."
                    />
                  </div>
                  <div>
                    <Label>{t.modelName}</Label>
                    <Input
                      value={settings.openai.model}
                      onChange={(v) => updateOpenAI({ model: v })}
                      placeholder="deepseek-chat"
                    />
                  </div>
                </div>
              )}
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
                      value={settings.screenshotTranslation?.hotkey || 'Command+Shift+A'}
                      onChange={(v) => updateScreenshotTranslation({ hotkey: v })}
                      placeholder="Command+Shift+A"
                    />
                  </div>
                  <div>
                    <Label>{t.ocrSource}</Label>
                    <Select
                      value={settings.screenshotTranslation?.ocrSource || 'system'}
                      onChange={(v) => updateScreenshotTranslation({ ocrSource: v as 'system' | 'glm' | 'openai' })}
                      options={[
                        { value: 'system', label: t.ocrSystem },
                        { value: 'glm', label: t.ocrGlm },
                        { value: 'openai', label: t.ocrOpenai },
                      ]}
                    />
                  </div>
                  {settings.screenshotTranslation?.ocrSource === 'glm' && (
                    <div className="animate-in fade-in slide-in-from-top-1 duration-200">
                      <Label>GLM API Key</Label>
                      <Input
                        type="password"
                        value={settings.screenshotTranslation?.glmApiKey || ''}
                        onChange={(v) => updateScreenshotTranslation({ glmApiKey: v })}
                        placeholder="..."
                      />
                      <p className="text-[10px] text-neutral-400 mt-2 flex items-center gap-1">
                        <span className="w-1 h-1 rounded-full bg-neutral-400" />
                        <a
                          href="#"
                          onClick={() => window.api?.openExternal('https://bigmodel.cn/console/apikey')}
                          className="hover:text-neutral-600 dark:hover:text-neutral-300 underline underline-offset-2 transition-colors"
                        >
                          {t.getApiKey}
                        </a>
                      </p>
                    </div>
                  )}
                  {settings.screenshotTranslation?.ocrSource === 'openai' && (
                    <div className="pl-4 border-l-2 border-black/5 dark:border-white/5 space-y-4 animate-in fade-in slide-in-from-left-2 duration-200">
                      <div>
                        <Label>{t.baseUrl}</Label>
                        <Input
                          value={settings.screenshotTranslation?.openai?.baseURL || ''}
                          onChange={(v) => updateScreenshotTranslationOpenAI({ baseURL: v })}
                          placeholder="https://api.openai.com/v1"
                        />
                      </div>
                      <div>
                        <Label>{t.modelName}</Label>
                        <Input
                          value={settings.screenshotTranslation?.openai?.model || ''}
                          onChange={(v) => updateScreenshotTranslationOpenAI({ model: v })}
                          placeholder="gpt-4o"
                        />
                      </div>
                      <div>
                        <Label>{t.apiKey}</Label>
                        <Input
                          type="password"
                          value={settings.screenshotTranslation?.openai?.apiKey || ''}
                          onChange={(v) => updateScreenshotTranslationOpenAI({ apiKey: v })}
                          placeholder="sk-..."
                        />
                      </div>
                    </div>
                  )}
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
                      value={settings.screenshotExplain?.hotkey || 'Command+Shift+E'}
                      onChange={(v) => updateScreenshotExplain({ hotkey: v })}
                      placeholder="Command+Shift+E"
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
                    <Label>{t.visionModel}</Label>
                    <Select
                      value={settings.screenshotExplain?.model?.provider || 'glm'}
                      onChange={(v) => {
                        if (v === 'glm') {
                          updateExplainModel({
                            provider: 'glm',
                            baseURL: 'https://open.bigmodel.cn/api/paas/v4',
                            modelName: 'glm-4v-flash'
                          })
                        } else {
                          updateExplainModel({
                            provider: 'openai',
                            baseURL: settings.screenshotExplain?.model?.baseURL || 'https://api.openai.com/v1',
                            modelName: settings.screenshotExplain?.model?.modelName || 'gpt-4-vision-preview'
                          })
                        }
                      }}
                      options={[
                        { value: 'glm', label: t.visionGlm },
                        { value: 'openai', label: t.visionOpenai },
                      ]}
                    />
                  </div>

                  {settings.screenshotExplain?.model?.provider === 'openai' && (
                    <div className="pl-4 border-l-2 border-black/5 dark:border-white/5 space-y-4 animate-in fade-in slide-in-from-left-2 duration-200">
                      <div>
                        <Label>{t.baseUrl}</Label>
                        <Input
                          value={settings.screenshotExplain?.model?.baseURL || ''}
                          onChange={(v) => updateExplainModel({ baseURL: v })}
                          placeholder="https://api.openai.com/v1"
                        />
                      </div>
                      <div>
                        <Label>{t.modelName}</Label>
                        <Input
                          value={settings.screenshotExplain?.model?.modelName || ''}
                          onChange={(v) => updateExplainModel({ modelName: v })}
                          placeholder="gpt-4-vision-preview"
                        />
                      </div>
                      <div>
                        <Label>{t.apiKey}</Label>
                        <Input
                          type="password"
                          value={settings.screenshotExplain?.model?.apiKey || ''}
                          onChange={(v) => updateExplainModel({ apiKey: v })}
                          placeholder="sk-..."
                        />
                      </div>
                    </div>
                  )}

                  {(settings.screenshotExplain?.model?.provider === 'glm' || !settings.screenshotExplain?.model?.provider) && (
                    <div className="animate-in fade-in slide-in-from-top-1 duration-200">
                      <Label>GLM API Key</Label>
                      <Input
                        type="password"
                        value={settings.screenshotExplain?.model?.apiKey || ''}
                        onChange={(v) => updateExplainModel({ apiKey: v })}
                        placeholder="..."
                      />
                      <p className="text-[10px] text-neutral-400 mt-2 flex items-center gap-1">
                        <span className="w-1 h-1 rounded-full bg-neutral-400" />
                        <a
                          href="#"
                          onClick={() => window.api?.openExternal('https://bigmodel.cn/console/apikey')}
                          className="hover:text-neutral-600 dark:hover:text-neutral-300 underline underline-offset-2 transition-colors"
                        >
                          {t.getApiKey}
                        </a>
                      </p>
                    </div>
                  )}

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
      </div>

      {/* 底部操作栏 */}
      <div className="flex justify-between items-center px-5 py-4 border-t border-black/5 dark:border-white/5 bg-white/50 dark:bg-neutral-900/50 backdrop-blur-xl">
        <span className="text-[10px] font-medium text-neutral-400 dark:text-neutral-500 tracking-wide">v{appVersion}</span>
        <div className="flex gap-3">
          <button
            onClick={onClose}
            className="px-4 py-2 text-[13px] font-medium text-neutral-600 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-200 hover:bg-black/5 dark:hover:bg-white/5 rounded-lg transition-all duration-200"
            style={noDragStyle}
          >
            {t.cancel}
          </button>
          <button
            onClick={handleSave}
            className="flex items-center gap-2 px-5 py-2 bg-neutral-900 dark:bg-white text-white dark:text-neutral-900 rounded-lg text-[13px] font-medium shadow-sm hover:shadow hover:bg-neutral-800 dark:hover:bg-neutral-100 transition-all duration-200 active:scale-95"
            style={noDragStyle}
          >
            <Save size={14} strokeWidth={2} />
            {t.save}
          </button>
        </div>
      </div>
    </div>
  )
}
