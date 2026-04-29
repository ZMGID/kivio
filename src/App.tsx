import { useState, useEffect, useRef } from 'react'
import { Settings as SettingsIcon, Cpu } from 'lucide-react'
import Settings from './Settings'
import Lens from './Lens'
import { api } from './api/tauri'
import { i18n, type Lang } from './settings/i18n'
import './index.css'

/**
 * 翻译器主组件
 * 磨砂玻璃风格悬浮窗：顶部 drag bar、输入与结果分层级、底部提示与模型芯片。
 */
function Translator({
  translateSource,
  lang,
  onOpenSettings,
}: {
  translateSource: string
  lang: Lang
  onOpenSettings: () => void
}) {
  const [input, setInput] = useState('')
  const [result, setResult] = useState('')
  const [loading, setLoading] = useState(false)
  const resultRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLInputElement>(null)
  const translateSeq = useRef(0)
  const t = i18n[lang]

  // 输入防抖翻译：600ms 延迟后发送翻译请求
  useEffect(() => {
    const seq = ++translateSeq.current
    const trimmed = input.trim()
    if (!trimmed) {
      setResult('')
      setLoading(false)
      return
    }

    const timer = setTimeout(async () => {
      if (seq !== translateSeq.current) return
      setLoading(true)
      try {
        const translated = await api.translateText(input)
        if (seq !== translateSeq.current) return
        setResult(translated)
      } catch (e) {
        if (seq !== translateSeq.current) return
        console.error(e)
        setResult(typeof e === 'string' ? e : (e as Error).message || 'Error')
      } finally {
        if (seq === translateSeq.current) setLoading(false)
      }
    }, 600)
    return () => clearTimeout(timer)
  }, [input])

  // Esc 键隐藏窗口
  useEffect(() => {
    const handler = async (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        try {
          await api.closeWindow()
        } catch (err) {
          console.error('[Translator] Failed to hide window:', err)
        }
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [])

  // 结果区域自动滚动到底部
  useEffect(() => {
    if (resultRef.current) {
      resultRef.current.scrollTop = resultRef.current.scrollHeight
    }
  }, [result])

  // 输入框自动滚动到右侧（显示最新输入）
  useEffect(() => {
    if (inputRef.current) {
      inputRef.current.scrollLeft = inputRef.current.scrollWidth
    }
  }, [input])

  // Enter 键提交翻译结果
  const handleKeyDown = async (e: React.KeyboardEvent) => {
    if (e.key === 'Enter') {
      const textToCommit = result || input
      await api.commitTranslation(textToCommit)
      setInput('')
      setResult('')
    }
  }

  return (
    <div className="window-container">
      {/* 卡片：填满外壳 padding 内区域；圆角 + 阴影都在这层 */}
      <div className="window-frosted h-full w-full flex flex-col select-none overflow-hidden relative group">
        {/* 顶部隐形 drag bar */}
        <div
          className="absolute top-0 left-0 right-0 h-6 z-10"
          data-tauri-drag-region
        />

        {/* 设置按钮（悬浮右上角） */}
        <button
          onClick={onOpenSettings}
          className="absolute top-1.5 right-2 z-20 p-1 text-neutral-400 hover:text-neutral-700 dark:text-neutral-500 dark:hover:text-neutral-200 rounded-md hover:bg-black/5 dark:hover:bg-white/10 opacity-60 hover:opacity-100 transition-all duration-150"
          title={t.translatorSettings}
        >
          <SettingsIcon size={13} strokeWidth={1.75} />
        </button>

        {/* 主内容区 */}
        <div className="relative z-0 flex-1 flex flex-col justify-center px-3.5 pt-3 pb-2.5">
        {/* 翻译结果展示（微渐变背景 + 柔光内描边） */}
        {(result || loading) && (
          <div
            ref={resultRef}
            className="mb-2 px-3 py-2 rounded-xl max-h-14 overflow-y-auto custom-scrollbar bg-gradient-to-br from-neutral-100/90 to-neutral-50/80 dark:from-neutral-800/70 dark:to-neutral-800/40 ring-1 ring-black/[0.04] dark:ring-white/[0.06] shadow-sm"
          >
            {loading ? (
              <div className="flex items-center gap-2 text-neutral-500 dark:text-neutral-400">
                <span className="flex gap-0.5">
                  <span className="w-1 h-1 rounded-full bg-neutral-400 dark:bg-neutral-500 animate-pulse" />
                  <span className="w-1 h-1 rounded-full bg-neutral-400 dark:bg-neutral-500 animate-pulse [animation-delay:0.2s]" />
                  <span className="w-1 h-1 rounded-full bg-neutral-400 dark:bg-neutral-500 animate-pulse [animation-delay:0.4s]" />
                </span>
                <span className="text-[11px]">{t.translatorTranslating}</span>
              </div>
            ) : (
              <p className="text-neutral-800 dark:text-neutral-100 text-[14.5px] font-normal select-text leading-[1.5]">
                {result}
              </p>
            )}
          </div>
        )}

        {/* 输入框（更精致的圆角 + focus 渐变） */}
        <input
          ref={inputRef}
          autoFocus
          className="w-full px-3.5 py-2 bg-white/70 dark:bg-neutral-800/40 ring-1 ring-black/[0.05] dark:ring-white/[0.06] rounded-xl text-[14.5px] text-neutral-900 dark:text-white placeholder-neutral-400 dark:placeholder-neutral-500 focus:outline-none focus:ring-black/[0.12] dark:focus:ring-white/[0.18] focus:bg-white dark:focus:bg-neutral-800/70 transition-all"
          placeholder={t.translatorPlaceholder}
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
        />

        {/* 底部提示 */}
        <div className="mt-1.5 flex justify-between items-center text-[10px] text-neutral-400 dark:text-neutral-500">
          <div className="flex items-center gap-2">
            <span>{t.translatorHintEnter}</span>
            <span>{t.translatorHintEsc}</span>
          </div>
          {translateSource && (
            <span className="flex items-center gap-1 opacity-70 max-w-[140px] truncate">
              <Cpu size={9} strokeWidth={1.5} className="shrink-0" />
              <span className="truncate">{translateSource}</span>
            </span>
          )}
        </div>
        </div>
      </div>
    </div>
  )
}

/**
 * 应用根组件
 * 根据 URL hash 切换不同视图模式（翻译器、设置、lens）
 */
function App() {
  // 从 URL hash 和查询参数解析当前模式
  const getMode = () => {
    const urlParams = new URLSearchParams(window.location.search)
    const hash = window.location.hash.replace('#', '')
    return urlParams.get('mode') || hash.split('?')[0] || ''
  }

  const [mode, setMode] = useState(getMode)
  const [themeMode, setThemeMode] = useState<'system' | 'light' | 'dark'>('system')
  const [translateSource, setTranslateSource] = useState<string>('')
  const [lang, setLang] = useState<Lang>('zh')

  // 应用主题设置
  const applyTheme = async () => {
    const settings = await api.getSettings()
    const nextMode = (settings.theme || 'system') as 'system' | 'light' | 'dark'
    setThemeMode(nextMode)
    const isDark = nextMode === 'dark' || (nextMode === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches)
    if (isDark) {
      document.documentElement.classList.add('dark')
    } else {
      document.documentElement.classList.remove('dark')
    }
    setTranslateSource(settings.translatorModel || 'AI')
    setLang((settings.settingsLanguage as Lang) || 'zh')
  }

  // 初始化主题并监听系统主题变化
  useEffect(() => {
    applyTheme()
    const mq = window.matchMedia('(prefers-color-scheme: dark)')
    const changeHandler = () => {
      if (themeMode === 'system') applyTheme()
    }
    mq.addEventListener('change', changeHandler)
    return () => mq.removeEventListener('change', changeHandler)
  }, [themeMode])

  // 监听 hash 变化切换模式
  useEffect(() => {
    const handler = () => setMode(getMode())
    window.addEventListener('hashchange', handler)
    return () => window.removeEventListener('hashchange', handler)
  }, [])

  // 监听后端触发的打开设置事件
  // 仅 main webview（hash 为空 / translator / settings）响应；
  // lens webview 即便误收广播也不切换视图，避免多设置界面。
  useEffect(() => {
    let cleanup: (() => void) | undefined
    api.onOpenSettings(() => {
      const currentHash = window.location.hash.replace('#', '').split('?')[0]
      if (currentHash !== '' && currentHash !== 'translator' && currentHash !== 'settings') return
      window.location.hash = '#settings'
      setMode('settings')
    }).then((unlisten) => {
      cleanup = unlisten
    })
    return () => {
      cleanup?.()
    }
  }, [])

  // 根据当前模式调整窗口大小
  useEffect(() => {
    const resize = async () => {
      if (mode === 'settings') {
        await api.resizeWindow(640, 520)
      } else if (mode === '' || mode === 'translator') {
        await api.resizeWindow(392, 152)
      }
    }
    resize()
  }, [mode])

  // 打开设置页
  const openSettings = async () => {
    window.location.hash = '#settings'
    setMode('settings')
    // 确保窗口大小正确，设置页不置顶
    await api.resizeWindow(640, 520)
    await api.setAlwaysOnTop(false)
  }

  // 关闭设置页，返回翻译器
  const closeSettings = async () => {
    try {
      await api.hideWindow()
    } catch (err) {
      console.error('[App] Error hiding window:', err)
    }
    window.location.hash = ''
    setMode('')
    await api.resizeWindow(392, 152)
  }

  // 根据模式渲染对应视图
  if (mode === 'lens') return <Lens />
  if (mode === 'settings') {
    return (
      <div className="h-screen w-screen overflow-hidden">
        <Settings onClose={closeSettings} onSettingsChange={applyTheme} />
      </div>
    )
  }
  return <Translator translateSource={translateSource} lang={lang} onOpenSettings={openSettings} />
}

export default App
