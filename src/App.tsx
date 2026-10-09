import { lazy, Suspense, useState, useEffect, useLayoutEffect, useRef, useCallback } from 'react'
import { Select, TextArea } from './settings/public/controls'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { api, isTauriRuntime } from './api/tauri'
import { getSettingsCached, subscribeSettings, updateSettingsCached } from './api/settingsCache'
import { applyThemeSettings, disposeTheme } from './theme/theme'
import { i18n, type Lang } from './components/i18n'
import { useWindowInteractionFocus } from './api/windowFocus'
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
import { isChatPopoutPath } from './chat/popout/popoutRoutes'
import { ChatErrorBoundary } from './chat/ChatErrorBoundary'
import './styles/app.css'
import './styles/translator.css'
import { getTranslationLanguageOptions } from './settings/public/translationLanguages'

const Lens = lazy(() => import('./Lens'))
const Chat = lazy(() => import('./chat/Chat'))
const ChatPopout = lazy(() => import('./chat/popout/ChatPopout'))

/**
 * 翻译器主组件
 * 轻量翻译浮层：上方多行原文，下方译文；随正文增长并保留紧凑上限。
 */
function Translator({
  translateSource,
  lang,
  targetLang,
  onTargetLangChange,
}: {
  translateSource: string
  lang: Lang
  targetLang: string | null
  onTargetLangChange: (value: string) => Promise<void>
}) {
  const [input, setInput] = useState('')
  const [result, setResult] = useState('')
  const [resultInput, setResultInput] = useState('')
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const [savingLanguage, setSavingLanguage] = useState(false)
  const [languageError, setLanguageError] = useState('')
  const [resolvedTargetLang, setResolvedTargetLang] = useState('')
  const savingLanguageRef = useRef(false)
  const [submitting, setSubmitting] = useState(false)
  // 同一帧内重复回车也必须被拦住，不能只等待 React 更新状态。
  const submittingRef = useRef(false)
  const resultRef = useRef<HTMLDivElement>(null)
  const contentRef = useRef<HTMLDivElement>(null)
  const translateSeq = useRef(0)
  const previousTargetLang = useRef<string | null>(null)
  const requestWindowFocus = useWindowInteractionFocus()
  const t = i18n[lang]
  const languageOptions = getTranslationLanguageOptions(t)
  const targetName = languageOptions.find(option => option.value === (targetLang === 'auto' ? resolvedTargetLang : targetLang))?.label
  const targetLabel = targetLang === 'auto'
    ? (targetName ? `${lang === 'zh' ? '自动' : 'Auto'} → ${targetName}` : t.langAuto)
    : `${lang === 'zh' ? '译为' : 'To '}${targetName ?? ''}`

  // 自动模式的预览也由后端解析，避免界面和实际翻译维护两套语言规则。
  useEffect(() => {
    setResolvedTargetLang('')
    if (targetLang !== 'auto' || !input.trim() || savingLanguage) return
    let cancelled = false
    void api.resolveTranslationTargetLang(input).then(value => {
      if (!cancelled) setResolvedTargetLang(value)
    }).catch(err => console.error('[Translator] Failed to resolve target language:', err))
    return () => { cancelled = true }
  }, [input, targetLang, savingLanguage])

  const changeTargetLang = async (value: string) => {
    if (value === targetLang || savingLanguageRef.current || submittingRef.current) return
    savingLanguageRef.current = true
    translateSeq.current += 1
    setSavingLanguage(true)
    setLanguageError('')
    try {
      await onTargetLangChange(value)
    } catch (err) {
      setLanguageError(err instanceof Error ? err.message : String(err))
    } finally {
      savingLanguageRef.current = false
      setSavingLanguage(false)
      contentRef.current?.querySelector('textarea')?.focus()
    }
  }

  // 输入防抖 600ms；切换已保存的语言后立即重译。
  useEffect(() => {
    const seq = ++translateSeq.current
    setResult('')
    setResultInput('')
    setError('')
    setLoading(false)
    const trimmed = input.trim()
    if (targetLang === null || savingLanguage) return

    const languageChanged = previousTargetLang.current !== null && previousTargetLang.current !== targetLang
    previousTargetLang.current = targetLang
    if (!trimmed) return
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
        setError(e instanceof Error ? e.message : String(e))
      } finally {
        if (seq === translateSeq.current) setLoading(false)
      }
    }, languageChanged ? 0 : 600)
    return () => {
      clearTimeout(timer)
      translateSeq.current += 1
    }
  }, [input, targetLang, savingLanguage])

  // Esc 键关闭输入翻译窗口，释放不常用的 main WebView。
  useEffect(() => {
    const handler = async (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !e.defaultPrevented) {
        try {
          await api.closeTranslatorWindow()
        } catch (err) {
          console.error('[Translator] Failed to close window:', err)
        }
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [])

  // 窗口随正文增长，达到上限后由输入和译文区域分别滚动。
  useLayoutEffect(() => {
    const content = contentRef.current
    if (!content || !isTauriRuntime()) return
    let lastHeight = 0
    const resize = () => {
      const height = Math.min(360, Math.max(220, Math.ceil(content.getBoundingClientRect().height) + 34))
      if (height === lastHeight) return
      lastHeight = height
      void api.resizeWindow(460, height).catch(err => console.error('[Translator] Failed to resize:', err))
    }
    resize()
    const observer = new ResizeObserver(resize)
    observer.observe(content)
    return () => observer.disconnect()
  }, [])

  // Enter 键提交翻译结果
  // IME 合成中（中/日/韩输入法选词按回车）不要触发：isComposing 是组合事件官方标志，
  // keyCode === 229 是浏览器在 IME 拦截 keydown 时的兜底信号，两个条件并查更稳。
  const handleKeyDown = async (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key !== 'Enter' || e.shiftKey) return
    if (e.nativeEvent.isComposing || e.keyCode === 229) return
    e.preventDefault()
    if (submittingRef.current || savingLanguageRef.current || loading || !result || resultInput !== input) return
    submittingRef.current = true
    setSubmitting(true)
    setError('')
    try {
      await api.commitTranslation(result)
      setInput('')
      setResult('')
      setResultInput('')
    } catch (e) {
      console.error('[Translator] Failed to commit translation:', e)
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      submittingRef.current = false
      setSubmitting(false)
    }
  }

  return (
    <div
      className="window-container"
      onPointerEnter={requestWindowFocus}
      onPointerMove={requestWindowFocus}
      onPointerDownCapture={requestWindowFocus}
    >
      <div className="window-frosted translator-panel">
        <div ref={contentRef} className="translator-content">
          <header className="translator-header" data-tauri-drag-region>
            <span data-tauri-drag-region>{t.tabTranslate}</span>
            {translateSource && <span className="translator-model" title={translateSource} data-tauri-drag-region>{translateSource}</span>}
          </header>
          <div className="translator-source">
            <TextArea
              variant="plain"
              rows={2}
              maxRows={4}
              autoFocus
              autoCapitalize="off"
              autoCorrect="off"
              autoComplete="off"
              spellCheck={false}
              placeholder={t.translatorPlaceholder}
              value={input}
              readOnly={submitting}
              aria-label={t.translatorPlaceholder}
              aria-busy={loading || submitting}
              onChange={setInput}
              onKeyDown={handleKeyDown}
            />
          </div>
          <div className="translator-language-row">
            <div className="translator-divider" />
            <Select
              size="sm"
              ariaLabel={t.targetLang}
              value={targetLang ?? 'auto'}
              triggerLabel={targetLabel}
              options={languageOptions}
              disabled={targetLang === null || savingLanguage || submitting}
              onChange={value => { void changeTargetLang(value) }}
            />
          </div>
          <div ref={resultRef} className="translator-result custom-scrollbar" aria-live="polite" aria-busy={loading}>
            {loading ? (
              <p className="translator-status">{t.translatorTranslating}</p>
            ) : result ? (
              <p className="translator-translation">{result}</p>
            ) : !error ? (
              <p className="translator-status">{lang === 'zh' ? '译文会显示在这里' : 'Translation appears here'}</p>
            ) : null}
            {(error || languageError) && <p role="alert" className="translator-error">{languageError || error}</p>}
          </div>
          <footer className="translator-footer">
            <span>{t.translatorHintEnter} · {t.translatorHintEsc}</span>
            <span>{lang === 'zh' ? '⇧ ↵ 换行' : '⇧ ↵ New line'}</span>
          </footer>
        </div>
      </div>
    </div>
  )
}

/**
 * 应用根组件
 * 根据 URL hash 切换不同视图模式（翻译器、设置、lens）
 */
// 与 index.css 的 --font-sans 默认栈保持一致（跨 mac/Win 含 CJK 覆盖）。
const UI_FONT_FALLBACK_STACK =
  'system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC", "Microsoft YaHei", "Hiragino Sans GB", sans-serif'
const UI_MONO_FALLBACK_STACK =
  'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Monaco, Consolas, monospace'

function App() {
  // 从 URL hash 和查询参数解析当前模式
  const getMode = () => {
    const urlParams = new URLSearchParams(window.location.search)
    const hash = window.location.hash.replace('#', '')
    const path = urlParams.get('mode') || hash.split('?')[0] || ''

    // 弹出窗必须走独立瘦壳，不能落入主窗 Chat.tsx（会把侧栏/设置/中心页一起打进来）。
    if (isChatPopoutPath(path)) {
      return 'chat-popout'
    }
    // 支持 #chat 或 #chat/conversation-id
    if (isChatPath(path)) {
      return 'chat'
    }

    return path
  }

  const [mode, setMode] = useState(getMode)
  const [translucentSidebar, setTranslucentSidebar] = useState(false)
  const [translateSource, setTranslateSource] = useState<string>('')
  const [lang, setLang] = useState<Lang>('zh')
  const [targetLang, setTargetLang] = useState<string | null>(null)

  useLayoutEffect(() => {
    const root = document.documentElement
    if (root.dataset.themeBooting !== 'true') return
    // The themed Suspense surface now covers loading; restore native transparency.
    root.style.removeProperty('background-color')
    delete root.dataset.themeBooting
  }, [])

  useEffect(() => {
    const path = hashPath()
    if (path === 'chat') {
      // 窗口以 `#chat` 创建说明 Rust 侧没有已存路由（有的话会直接烤进 URL，见
      // windows.rs::ensure_chat_window）。getRememberedChatRoute() 会自动迁移
      // localStorage 遗留值（如果有的话），无需显式调用 adoptLegacyRememberedChatRoute。
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

  // 迟到的 getSettingsCached 不能盖住更新的设置。主题色由 theme 模块负责。
  const applyGeneration = useRef(0)
  const applyThemeRef = useRef<() => Promise<void>>(async () => {})

  // 字体、半透明和语言仍在这里应用。明暗色板与系统监听归 theme 模块。
  const applyTheme = async () => {
    const generation = ++applyGeneration.current
    const settings = await getSettingsCached()
    if (generation !== applyGeneration.current) return
    applyThemeSettings(settings)
    setTranslucentSidebar(settings.translucentSidebar)
    // UI 字号（整体缩放）+ 自定义字体：仅作用于聊天窗口，翻译窗/Lens 保持原始几何与布局。
    // 直接读 hash（稳定的 import）而非 mode state，避免让 applyTheme 变成不稳定依赖。
    const root = document.documentElement
    if (isChatPath(hashPath())) {
      const scale = Math.min(1.4, Math.max(0.8, settings.uiFontScale ?? 1))
      // 用原生 webview 缩放（等同浏览器 Cmd+加号），而非 CSS zoom —— CSS zoom 会打乱
      // 聊天消息列表 virtualizer 虚拟滚动的 scrollTop/scrollHeight 几何量，导致流式生成时跟随钉底失效。
      if (isTauriRuntime()) void getCurrentWebview().setZoom(scale).catch(() => {})
      const family = (settings.uiFontFamily ?? '').trim()
      // 默认字体栈与 index.css 的 --font-sans 保持一致；自定义字体拼到最前，缺失时回退系统字体。
      root.style.setProperty(
        '--font-sans',
        family
          ? `"${family}", ${UI_FONT_FALLBACK_STACK}`
          : UI_FONT_FALLBACK_STACK,
      )
      const mono = (settings.uiFontMono ?? '').trim()
      root.style.setProperty(
        '--font-mono',
        mono
          ? `"${mono}", ${UI_MONO_FALLBACK_STACK}`
          : UI_MONO_FALLBACK_STACK,
      )
    }
    setTargetLang(settings.targetLang || 'auto')
    setTranslateSource(settings.translatorModel || 'AI')
    setLang((settings.settingsLanguage as Lang) || 'zh')
    // 首次应用主题后（下一帧）再开启主题色过渡，避免初始 light↔dark 闪烁；
    // 之后用户切换主题/系统主题变化时才平滑过渡。classList.add 幂等。
    requestAnimationFrame(() => {
      if (generation !== applyGeneration.current) return
      document.documentElement.classList.add('theme-transitions-ready')
    })
  }
  applyThemeRef.current = applyTheme

  // 设置缓存更新（含其它窗口）重新应用。冷启动不会通知订阅者，所以这里先读一次。
  useEffect(() => {
    void applyThemeRef.current()
    const unsubscribe = subscribeSettings(() => {
      void applyThemeRef.current()
    })
    return () => {
      applyGeneration.current += 1
      unsubscribe()
      disposeTheme()
    }
  }, [])

  // 监听 hash 变化切换模式
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
      // 全局 listen 会收到 emit_to("chat") 的事件（Tauri v2 的 Any 目标语义），所以 lens/translate/
      // settings/translator 窗也会收到这条广播。只有 chat 窗该响应，否则其它窗口会把自己导航成 chat。
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
    if (isChatPopoutPath(hashPath())) return
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
        if (!isChatPopoutPath(hashPath())) {
          await restoreChatWindowGeometry(win)
        }
        await api.showWindow()
        await api.focusWindow()
      }
      await persistChatWindowGeometry()
    } catch (err) {
      console.error('[App] Failed to reveal chat window:', err)
    }
  }, [persistChatWindowGeometry])

  // 首次创建 chat 窗口时后端保持 hidden，把 show 交给前端；此处再把 show 从“App 挂载即弹出”
  // 推迟到“Chat 首屏内容就绪”（onContentReady → revealChatWindowNow），避免窗口弹出后还在转圈。
  const revealedRef = useRef(false)
  const revealTimerRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  const revealChatWindowNow = useCallback(() => {
    if (revealedRef.current) return
    revealedRef.current = true
    if (revealTimerRef.current !== undefined) {
      clearTimeout(revealTimerRef.current)
      revealTimerRef.current = undefined
    }
    void revealChatWindow()
  }, [revealChatWindow])

  useLayoutEffect(() => {
    if (mode !== 'chat' && mode !== 'chat-popout') return
    if (!isTauriRuntime()) return
    // 不变量：chat 是专用窗口，其 hash 恒为 #chat（含子路由），mode 一旦为 'chat' 便不再变。
    // 本兜底据此成立——若未来 chat 窗允许 mode 离开 'chat'，cleanup 会清掉未触发的兜底 timer
    // 而新分支早退，可能导致窗口永久 hidden；届时需改为窗口存活期内独立保证 reveal。
    // 已 reveal 过（防御性：正常不会二次进入）→ 直接校正一次几何/可见性。
    if (revealedRef.current) {
      void revealChatWindow()
      return
    }
    // 兜底：内容就绪信号 3s 内未到达（chunk 加载失败 / 组件抛错被 ErrorBoundary 接住 / 信号丢失）
    // 也强制 show，绝不让窗口永久 hidden。
    revealTimerRef.current = setTimeout(() => {
      revealChatWindowNow()
    }, 3000)
    return () => {
      if (revealTimerRef.current !== undefined) {
        clearTimeout(revealTimerRef.current)
        revealTimerRef.current = undefined
      }
    }
  }, [mode, revealChatWindow, revealChatWindowNow])

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

        // resize/move 在拖动中高频触发；几何持久化（多次 IPC 读尺寸 + 写 store）debounce 到停止后做一次，
        // 否则每帧都发 IPC 会和窗口伸缩/拖动的渲染抢资源，造成明显卡顿（Windows/WebView2 尤甚）。
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

  // 根据当前模式渲染对应视图
  if (mode === 'lens') {
    return (
      <Suspense fallback={null}>
        <Lens />
      </Suspense>
    )
  }
  const chatSuspenseFallback = (
    <div className="flex h-full w-full items-center justify-center bg-[var(--theme-surface)]">
      <div className="h-6 w-6 animate-spin rounded-full border-2 border-[var(--theme-surface-border)] border-t-[var(--text)]" />
    </div>
  )
  if (mode === 'chat-popout') {
    return (
      <ChatWindowHost translucentSidebar={translucentSidebar}>
        <Suspense fallback={chatSuspenseFallback}>
          <ChatErrorBoundary>
            <ChatPopout onContentReady={revealChatWindowNow} />
          </ChatErrorBoundary>
        </Suspense>
      </ChatWindowHost>
    )
  }
  if (mode === 'chat') {
    return (
      <ChatWindowHost translucentSidebar={translucentSidebar}>
        <Suspense fallback={chatSuspenseFallback}>
          <ChatErrorBoundary>
            <Chat onSettingsChange={applyTheme} onContentReady={revealChatWindowNow} />
          </ChatErrorBoundary>
        </Suspense>
      </ChatWindowHost>
    )
  }
  return <Translator
    translateSource={translateSource}
    lang={lang}
    targetLang={targetLang}
    onTargetLangChange={async value => {
      const saved = await updateSettingsCached(current => ({ ...current, targetLang: value }))
      setTargetLang(saved.targetLang)
    }}
  />
}

export default App
