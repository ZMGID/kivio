import { lazy, Suspense, useState, useEffect, useLayoutEffect, useRef, useCallback } from 'react'
import { Settings as SettingsIcon, Cpu } from 'lucide-react'
import { listen } from '@tauri-apps/api/event'
import { api } from './api/tauri'
import { i18n, normalizeLang, type Lang } from './settings/i18n'
import { useWindowInteractionFocus } from './utils/windowFocus'
import { ChatWindowHost } from './chat/ChatWindowHost'
import {
  getRememberedChatRoute,
  hashPath,
  isChatWindowPlacementVisible,
  isChatPath,
  isChatSettingsPath,
  rememberChatGeometry,
  rememberCurrentChatRoute,
  restoreChatWindowGeometry,
  snapshotChatWindowGeometry,
} from './chat/persistence'
import { ChatErrorBoundary } from './chat/ChatErrorBoundary'
import { normalizeThemeColorId } from './themeColors'
import './index.css'

const Lens = lazy(() => import('./Lens'))
const Chat = lazy(() => import('./chat/Chat'))

function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
}

/**
 * 翻譯器主元件
 * 磨砂玻璃風格懸浮窗：頂部 drag bar、輸入與結果分層級、底部提示與模型晶片。
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
  const [resultInput, setResultInput] = useState('')
  const [loading, setLoading] = useState(false)
  const resultRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLInputElement>(null)
  const translateSeq = useRef(0)
  const requestWindowFocus = useWindowInteractionFocus()
  const t = i18n[lang]

  // 輸入防抖翻譯：600ms 延遲後傳送翻譯請求
  useEffect(() => {
    const seq = ++translateSeq.current
    setResult('')
    setResultInput('')
    const trimmed = input.trim()
    if (!trimmed) {
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
        setResultInput(input)
      } catch (e) {
        if (seq !== translateSeq.current) return
        console.error(e)
        setResult(typeof e === 'string' ? e : (e as Error).message || 'Error')
        setResultInput(input)
      } finally {
        if (seq === translateSeq.current) setLoading(false)
      }
    }, 600)
    return () => clearTimeout(timer)
  }, [input])

  useEffect(() => {
    const handler = async (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        try {
          await api.closeWindow()
        } catch (err) {
          console.error('[Translator] Failed to close window:', err)
        }
      }
      if ((e.metaKey || e.ctrlKey) && e.key === ',') {
        e.preventDefault()
        try {
          await api.openSettingsWindow()
          await api.closeTranslatorWindow()
        } catch (err) {
          console.error('[Translator] Failed to open settings:', err)
        }
      }
    }
    window.addEventListener('keydown', handler, true)
    return () => window.removeEventListener('keydown', handler, true)
  }, [])

  // 結果區域自動滾動到底部
  useEffect(() => {
    if (resultRef.current) {
      resultRef.current.scrollTop = resultRef.current.scrollHeight
    }
  }, [result])

  // 輸入框自動滾動到右側（顯示最新輸入）
  useEffect(() => {
    if (inputRef.current) {
      inputRef.current.scrollLeft = inputRef.current.scrollWidth
    }
  }, [input])

  // Enter 鍵提交翻譯結果
  // IME 合成中（中/日/韓輸入法選詞按回車）不要觸發：isComposing 是組合事件官方標誌，
  // keyCode === 229 是瀏覽器在 IME 攔截 keydown 時的兜底訊號，兩個條件並查更穩。
  const handleKeyDown = async (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key !== 'Enter') return
    if (e.nativeEvent.isComposing || e.keyCode === 229) return
    if (loading || !result || resultInput !== input) return
    const textToCommit = result
    await api.commitTranslation(textToCommit)
    setInput('')
    setResult('')
    setResultInput('')
  }

  return (
    <div
      className="window-container"
      onPointerEnter={requestWindowFocus}
      onPointerMove={requestWindowFocus}
      onPointerDownCapture={requestWindowFocus}
    >
      {/* 卡片：填滿外殼 padding 內區域；圓角 + 陰影都在這層 */}
      <div className="window-frosted h-full w-full flex flex-col select-none overflow-hidden relative group">
        {/* 頂部隱形 drag bar */}
        <div
          className="absolute top-0 left-0 right-0 h-6 z-10"
          data-tauri-drag-region
        />

        {/* 設定按鈕（懸浮右上角） */}
        <button
          onClick={onOpenSettings}
          className="absolute top-1.5 right-2 z-20 p-1 text-neutral-400 hover:text-neutral-700 dark:text-neutral-500 dark:hover:text-neutral-200 rounded-md hover:bg-black/5 dark:hover:bg-white/10 opacity-60 hover:opacity-100 transition-all duration-150"
          title={t.translatorSettings}
        >
          <SettingsIcon size={13} strokeWidth={1.75} />
        </button>

        {/* 主內容區 */}
        <div className="relative z-0 flex-1 flex flex-col justify-center px-3.5 pt-3 pb-2.5">
        {/* 翻譯結果展示（微漸變背景 + 柔光內描邊） */}
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

        {/* 輸入框（更精緻的圓角 + focus 漸變） */}
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
 * 應用根元件
 * 根據 URL hash 切換不同檢視模式（翻譯器、設定、lens）
 */
function App() {
  // 從 URL hash 和查詢引數解析當前模式
  const getMode = () => {
    const urlParams = new URLSearchParams(window.location.search)
    const hash = window.location.hash.replace('#', '')
    const path = urlParams.get('mode') || hash.split('?')[0] || ''

    // 支援 #chat 或 #chat/conversation-id
    if (isChatPath(path)) {
      return 'chat'
    }

    return path
  }

  const [mode, setMode] = useState(getMode)
  const [themeMode, setThemeMode] = useState<'system' | 'light' | 'dark'>('system')
  const [translateSource, setTranslateSource] = useState<string>('')
  const [lang, setLang] = useState<Lang>('zh-TW')

  useEffect(() => {
    const path = hashPath()
    if (path === 'chat') {
      const rememberedRoute = getRememberedChatRoute()
      if (rememberedRoute && rememberedRoute !== window.location.hash) {
        window.location.hash = rememberedRoute
        setMode('chat')
      }
      return
    }
    if (isChatPath(path)) {
      rememberCurrentChatRoute()
    }
  }, [])

  // 應用主題設定
  const applyTheme = async () => {
    const settings = await api.getSettings()
    const nextMode = (settings.theme || 'system') as 'system' | 'light' | 'dark'
    setThemeMode(nextMode)
    const themeColor = normalizeThemeColorId(settings.themeColor)
    const isDark = nextMode === 'dark'
      || (nextMode === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches)
    if (isDark) {
      document.documentElement.classList.add('dark')
    } else {
      document.documentElement.classList.remove('dark')
    }
    // 同步 chat 視窗原生背景（Windows 不透明視窗），避免伸縮時露白底閃爍。其他視窗/平臺 no-op。
    void api.setChatWindowBackground(isDark)
    document.documentElement.dataset.themeColor = themeColor
    setTranslateSource(settings.translatorModel || 'AI')
    setLang(normalizeLang(settings.settingsLanguage))
    // 首次應用主題後（下一幀）再開啟主題色過渡，避免初始 light↔dark 閃爍；
    // 之後使用者切換主題/系統主題變化時才平滑過渡。classList.add 冪等。
    requestAnimationFrame(() => {
      document.documentElement.classList.add('theme-transitions-ready')
    })
  }

  // 初始化主題並監聽系統主題變化
  useEffect(() => {
    applyTheme()
    const mq = window.matchMedia('(prefers-color-scheme: dark)')
    const changeHandler = () => {
      if (themeMode === 'system') applyTheme()
    }
    mq.addEventListener('change', changeHandler)
    return () => mq.removeEventListener('change', changeHandler)
  }, [themeMode])

  // 監聽 hash 變化切換模式
  useEffect(() => {
    const handler = () => {
      const path = hashPath()
      const nextMode = getMode()
      if (isChatPath(path)) {
        rememberCurrentChatRoute()
      }
      setMode(nextMode)
    }
    window.addEventListener('hashchange', handler)
    return () => window.removeEventListener('hashchange', handler)
  }, [])

  useEffect(() => {
    if (!isTauriRuntime()) return
    let cancelled = false
    let cleanup: (() => void) | undefined

    listen('chat-open-request', () => {
      const path = hashPath()
      // 全域 listen 會收到 emit_to("chat") 的事件（Tauri v2 的 Any 目標語義），所以 lens/translate/
      // settings/translator 窗也會收到這條廣播。只有 chat 窗該回應，否則其它視窗會把自己導航成 chat。
      if (!isChatPath(path)) return
      if (path !== 'chat' && !isChatSettingsPath(path)) return
      const rememberedRoute = getRememberedChatRoute()
      if (rememberedRoute && rememberedRoute !== window.location.hash) {
        window.location.hash = rememberedRoute
        setMode('chat')
      }
    }).then((unlisten) => {
      if (cancelled) {
        unlisten()
      } else {
        cleanup = unlisten
      }
    }).catch((err) => {
      console.error('[App] Failed to listen for chat open requests:', err)
    })

    return () => {
      cancelled = true
      cleanup?.()
    }
  }, [])

  const persistChatWindowGeometry = useCallback(async () => {
    if (!isTauriRuntime()) return
    try {
      const win = (await import('@tauri-apps/api/window')).getCurrentWindow()
      const geometry = await snapshotChatWindowGeometry(win)
      if (geometry) rememberChatGeometry(geometry)
    } catch (err) {
      console.error('[App] Failed to remember chat window geometry:', err)
    }
  }, [])

  const revealChatWindow = useCallback(async () => {
    if (!isTauriRuntime()) return
    try {
      const win = (await import('@tauri-apps/api/window')).getCurrentWindow()
      const [visible, minimized] = await Promise.all([win.isVisible(), win.isMinimized()])
      const placementVisible = visible && !minimized
        ? await isChatWindowPlacementVisible(win)
        : false
      if (!visible || minimized || !placementVisible) {
        if (minimized) {
          await win.unminimize()
        }
        await restoreChatWindowGeometry(win)
        await api.showWindow()
        await api.focusWindow()
      }
      await persistChatWindowGeometry()
    } catch (err) {
      console.error('[App] Failed to reveal chat window:', err)
    }
  }, [persistChatWindowGeometry])

  useLayoutEffect(() => {
    if (mode !== 'chat') return
    if (!isTauriRuntime()) return
    void revealChatWindow()
  }, [mode, revealChatWindow])

  useEffect(() => {
    if (mode !== 'chat') return
    if (!isTauriRuntime()) return
    let cancelled = false
    let unlistenResize: (() => void) | undefined
    let unlistenMove: (() => void) | undefined
    let readyToRemember = false
    let geomTimer: ReturnType<typeof setTimeout> | undefined

    const setup = async () => {
      try {
        const win = (await import('@tauri-apps/api/window')).getCurrentWindow()
        await new Promise(resolve => window.setTimeout(resolve, 0))
        if (!cancelled) readyToRemember = true

        // resize/move 在拖動中高頻觸發；幾何持久化（多次 IPC 讀尺寸 + 寫 store）debounce 到停止後做一次，
        // 否則每幀都發 IPC 會和視窗伸縮/拖動的渲染搶資源，造成明顯示卡頓（Windows/WebView2 尤甚）。
        const persistIfReady = () => {
          if (!readyToRemember || cancelled) return
          if (geomTimer !== undefined) clearTimeout(geomTimer)
          geomTimer = setTimeout(() => {
            if (!cancelled) void persistChatWindowGeometry()
          }, 250)
        }

        const resizeHandler = await win.onResized(() => {
          persistIfReady()
        })
        const moveHandler = await win.onMoved(() => {
          persistIfReady()
        })
        if (cancelled) {
          resizeHandler()
          moveHandler()
        } else {
          unlistenResize = resizeHandler
          unlistenMove = moveHandler
        }
      } catch (err) {
        console.error('[App] Failed to track chat window geometry:', err)
      }
    }

    void setup()
    return () => {
      cancelled = true
      if (geomTimer !== undefined) clearTimeout(geomTimer)
      unlistenResize?.()
      unlistenMove?.()
    }
  }, [mode, persistChatWindowGeometry])

  // 根據當前模式調整視窗大小
  useEffect(() => {
    const resize = async () => {
      if (mode === '' || mode === 'translator') {
        await api.resizeWindow(392, 152)
      }
    }
    resize()
  }, [mode])

  // 開啟設定頁
  const openSettings = async () => {
    try {
      await api.openSettingsWindow()
      await api.closeTranslatorWindow()
    } catch (err) {
      console.error('[App] Error opening settings window:', err)
    }
  }

  // 根據當前模式渲染對應檢視
  if (mode === 'lens') {
    return (
      <Suspense fallback={null}>
        <Lens />
      </Suspense>
    )
  }
  if (mode === 'chat') {
    return (
      <ChatWindowHost>
        <Suspense
          fallback={
            <div className="flex h-full w-full items-center justify-center bg-transparent">
              <div className="h-6 w-6 animate-spin rounded-full border-2 border-neutral-300 border-t-neutral-800 dark:border-neutral-700 dark:border-t-neutral-200" />
            </div>
          }
        >
          <ChatErrorBoundary>
            <Chat onSettingsChange={applyTheme} />
          </ChatErrorBoundary>
        </Suspense>
      </ChatWindowHost>
    )
  }
  return <Translator translateSource={translateSource} lang={lang} onOpenSettings={openSettings} />
}

export default App
