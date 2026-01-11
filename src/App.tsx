import { useState, useEffect, useRef, type CSSProperties } from 'react'
import { Settings as SettingsIcon, Cpu } from 'lucide-react'
import Settings from './Settings'
import ScreenshotResult from './ScreenshotResult'
import ScreenshotExplain from './ScreenshotExplain'
import './index.css'

type AppRegionStyle = CSSProperties & { WebkitAppRegion?: 'drag' | 'no-drag' }
const dragStyle: AppRegionStyle = { WebkitAppRegion: 'drag' }
const noDragStyle: AppRegionStyle = { WebkitAppRegion: 'no-drag' }

function Translator() {
  const [input, setInput] = useState('')
  const [result, setResult] = useState('')
  const [loading, setLoading] = useState(false)
  const [showSettings, setShowSettings] = useState(false)
  const [themeMode, setThemeMode] = useState<'system' | 'light' | 'dark'>('system')
  const [translateSource, setTranslateSource] = useState<string>('')
  const resultRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLInputElement>(null)

  const applyTheme = async () => {
    if (!window.api) return
    const settings = await window.api.getSettings()
    const mode = settings.theme || 'system'
    setThemeMode(mode)
    const isDark = mode === 'dark' || (mode === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches)
    if (isDark) {
      document.documentElement.classList.add('dark')
    } else {
      document.documentElement.classList.remove('dark')
    }

    if (settings.source === 'bing') {
      setTranslateSource('Bing')
    } else if (settings.source === 'openai') {
      setTranslateSource(settings.openai?.model || 'AI')
    } else {
      setTranslateSource('')
    }
  }

  useEffect(() => {
    applyTheme()
    const mq = window.matchMedia('(prefers-color-scheme: dark)')
    const changeHandler = () => {
      if (themeMode === 'system') applyTheme()
    }
    mq.addEventListener('change', changeHandler)
    return () => mq.removeEventListener('change', changeHandler)
  }, [themeMode])

  useEffect(() => {
    if (window.api) {
      if (showSettings) {
        window.api.resizeWindow(420, 520)
      } else {
        window.api.resizeWindow(340, 110)
      }
    }
  }, [showSettings])

  useEffect(() => {
    if (window.api) {
      const removeListener = window.api.onOpenSettings(() => {
        setShowSettings(true)
      })
      return () => {
        removeListener?.()
      }
    }
  }, [])

  useEffect(() => {
    if (showSettings) return
    const timer = setTimeout(async () => {
      if (input.trim()) {
        setLoading(true)
        try {
          if (!window.api) return
          const translated = await window.api.translateText(input)
          setResult(translated)
        } catch {
          setResult('Error')
        } finally {
          setLoading(false)
        }
      } else {
        setResult('')
      }
    }, 600)
    return () => clearTimeout(timer)
  }, [input, showSettings])

  useEffect(() => {
    if (resultRef.current) {
      resultRef.current.scrollTop = resultRef.current.scrollHeight
    }
  }, [result])

  useEffect(() => {
    if (inputRef.current) {
      inputRef.current.scrollLeft = inputRef.current.scrollWidth
    }
  }, [input])

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (showSettings) return
    if (e.key === 'Enter') {
      const textToCommit = result || input
      if (window.api) {
        window.api.commitTranslation(textToCommit)
        setInput('')
        setResult('')
      }
    } else if (e.key === 'Escape') {
      if (window.api) {
        window.api.closeWindow()
      }
    }
  }

  if (showSettings) {
    const handleCloseSettings = () => {
      setShowSettings(false)
      if (window.api) {
        window.api.hideWindow()
      }
    }
    return (
      <div className="h-screen w-screen bg-white/95 dark:bg-neutral-900/95 backdrop-blur-2xl rounded-xl border border-black/10 dark:border-white/10 shadow-2xl overflow-hidden">
        <Settings onClose={handleCloseSettings} onSettingsChange={applyTheme} />
      </div>
    )
  }

  return (
    <div
      className="h-screen w-screen flex flex-col bg-white/80 dark:bg-neutral-900/80 backdrop-blur-2xl rounded-xl border border-black/10 dark:border-white/10 shadow-2xl select-none overflow-hidden relative group"
      style={dragStyle}
    >
      {/* 设置按钮 */}
      <button
        onClick={() => setShowSettings(true)}
        className="absolute top-2.5 right-2.5 p-1 text-neutral-400 hover:text-neutral-600 dark:text-neutral-500 dark:hover:text-neutral-300 rounded-md hover:bg-black/5 dark:hover:bg-white/10 opacity-0 group-hover:opacity-100 transition-all duration-150"
        style={noDragStyle}
      >
        <SettingsIcon size={13} strokeWidth={1.5} />
      </button>

      {/* 主内容区 */}
      <div className="flex-1 flex flex-col justify-center px-3.5 py-2.5">
        {/* 翻译结果 */}
        {(result || loading) && (
          <div
            ref={resultRef}
            className="mb-2 px-3 py-2 bg-neutral-100/80 dark:bg-neutral-800/60 rounded-lg max-h-14 overflow-y-auto"
            style={noDragStyle}
          >
            {loading ? (
              <div className="flex items-center gap-2 text-neutral-400 text-sm">
                <div className="w-3.5 h-3.5 border-[1.5px] border-neutral-400 border-t-transparent rounded-full animate-spin" />
                <span className="text-xs">翻译中</span>
              </div>
            ) : (
              <p className="text-neutral-800 dark:text-neutral-100 text-[15px] font-normal select-text leading-relaxed">
                {result}
              </p>
            )}
          </div>
        )}

        {/* 输入框 */}
        <input
          ref={inputRef}
          autoFocus
          className="w-full px-3 py-2 bg-neutral-100/60 dark:bg-neutral-800/40 rounded-lg text-[15px] text-neutral-900 dark:text-white placeholder-neutral-400 dark:placeholder-neutral-500 focus:outline-none focus:bg-neutral-100 dark:focus:bg-neutral-800/60 transition-colors"
          style={noDragStyle}
          placeholder="输入文本..."
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
        />

        {/* 底部提示 */}
        <div className="mt-2 flex justify-between items-center text-[10px] text-neutral-400 dark:text-neutral-500">
          <div className="flex items-center gap-2">
            <span>↵ 确认</span>
            <span>esc 关闭</span>
          </div>
          {translateSource && (
            <span className="flex items-center gap-1 opacity-60">
              <Cpu size={9} strokeWidth={1.5} />
              {translateSource}
            </span>
          )}
        </div>
      </div>
    </div>
  )
}

function App() {
  const urlParams = new URLSearchParams(window.location.search)
  const hash = window.location.hash.replace('#', '')
  const mode = urlParams.get('mode') || hash.split('?')[0]

  if (mode === 'screenshot') return <ScreenshotResult />
  if (mode === 'explain') return <ScreenshotExplain />
  return <Translator />
}

export default App
