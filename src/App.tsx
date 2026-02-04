import { useState, useEffect, useRef } from 'react'
import { Settings as SettingsIcon, Cpu } from 'lucide-react'
import Settings from './Settings'
import ScreenshotResult from './ScreenshotResult'
import ScreenshotExplain from './ScreenshotExplain'
import { api } from './api/tauri'
import './index.css'

function Translator({ translateSource, onOpenSettings }: { translateSource: string; onOpenSettings: () => void }) {
  const [input, setInput] = useState('')
  const [result, setResult] = useState('')
  const [loading, setLoading] = useState(false)
  const resultRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLInputElement>(null)
  const translateSeq = useRef(0)

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

  useEffect(() => {
    const handler = async (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        console.log('[Translator] ESC pressed, hiding window')
        try {
          await api.closeWindow()
          console.log('[Translator] Window hidden')
        } catch (err) {
          console.error('[Translator] Failed to hide window:', err)
        }
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [])

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

  const handleKeyDown = async (e: React.KeyboardEvent) => {
    if (e.key === 'Enter') {
      const textToCommit = result || input
      await api.commitTranslation(textToCommit)
      // verify: backend handles hiding window
      setInput('')
      setResult('')
    } else if (e.key === 'Escape') {
      console.log('[Translator] ESC in input, hiding window')
      await api.closeWindow()
    }
  }

  return (
    <div
      className="window-container flex flex-col bg-white/80 dark:bg-neutral-900/80 backdrop-blur-2xl rounded-xl shadow-[0_8px_32px_rgba(0,0,0,0.12)] select-none overflow-hidden relative group"
    >
      {/* 设置按钮 */}
      <button
        onClick={onOpenSettings}
        className="absolute top-2.5 right-2.5 z-10 p-1 text-neutral-400 hover:text-neutral-600 dark:text-neutral-500 dark:hover:text-neutral-300 rounded-md hover:bg-black/5 dark:hover:bg-white/10 opacity-0 group-hover:opacity-100 transition-all duration-150"
      >
        <SettingsIcon size={13} strokeWidth={1.5} />
      </button>

      {/* 主内容区 */}
      <div
        className="relative z-0 flex-1 flex flex-col justify-center px-3.5 py-2.5"
        onMouseDown={(e) => {
          if (e.target === e.currentTarget) {
            api.startDragging()
          }
        }}
      >
        {/* 翻译结果 */}
        {(result || loading) && (
          <div
            ref={resultRef}
            className="mb-2 px-3 py-2 bg-neutral-100/80 dark:bg-neutral-800/60 rounded-lg max-h-14 overflow-y-auto"
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
  const getMode = () => {
    const urlParams = new URLSearchParams(window.location.search)
    const hash = window.location.hash.replace('#', '')
    return urlParams.get('mode') || hash.split('?')[0] || ''
  }

  const [mode, setMode] = useState(getMode)
  const [themeMode, setThemeMode] = useState<'system' | 'light' | 'dark'>('system')
  const [translateSource, setTranslateSource] = useState<string>('')

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
    const handler = () => setMode(getMode())
    window.addEventListener('hashchange', handler)
    return () => window.removeEventListener('hashchange', handler)
  }, [])

  useEffect(() => {
    let cleanup: (() => void) | undefined
    api.onOpenSettings(() => {
      window.location.hash = '#settings'
      setMode('settings')
    }).then((unlisten) => {
      cleanup = unlisten
    })
    return () => {
      cleanup?.()
    }
  }, [])

  useEffect(() => {
    const resize = async () => {
      if (mode === 'settings') {
        console.log('[App] Resizing to settings size: 420x520')
        await api.resizeWindow(420, 520)
        // 延迟后再次确认大小，防止 Tauri 窗口状态不同步
        setTimeout(() => api.resizeWindow(420, 520), 100)
      } else if (mode === '' || mode === 'translator') {
        console.log('[App] Resizing to translator size: 360x120')
        await api.resizeWindow(360, 120)
      }
    }
    resize()
  }, [mode])

  const openSettings = async () => {
    window.location.hash = '#settings'
    setMode('settings')
    // 确保窗口大小正确
    await api.resizeWindow(420, 520)
  }

  const closeSettings = async () => {
    console.log('[App] closeSettings called')
    window.location.hash = ''
    setMode('')
    try {
      await api.resizeWindow(360, 120)
      await api.hideWindow()
      console.log('[App] Window hidden successfully')
    } catch (err) {
      console.error('[App] Error hiding window:', err)
    }
  }

  if (mode === 'screenshot') return <ScreenshotResult />
  if (mode === 'explain') return <ScreenshotExplain />
  if (mode === 'settings') {
    return (
      <div className="h-screen w-screen overflow-hidden">
        <Settings onClose={closeSettings} onSettingsChange={applyTheme} />
      </div>
    )
  }
  return <Translator translateSource={translateSource} onOpenSettings={openSettings} />
}

export default App
