import { useState, useEffect, useRef, useCallback } from 'react'
import { Copy, Check, Cpu, Scan, Code, Eye, X } from 'lucide-react'
import { api } from './api/tauri'
import ReactMarkdown from 'react-markdown'
import remarkMath from 'remark-math'
import rehypeKatex from 'rehype-katex'
import 'katex/dist/katex.min.css'
import './index.css'
import { i18n, type Lang } from './settings/i18n'
import { copyToClipboard } from './utils/clipboard'

/**
 * 截图翻译结果展示组件
 * 悬浮窗风格：无系统装饰、顶部隐形 drag bar、右上角浮动工具按钮
 */
export default function ScreenshotResult() {
  const [status, setStatus] = useState<'processing' | 'ready' | 'error'>('processing')
  const [original, setOriginal] = useState('')
  const [translated, setTranslated] = useState('')
  const [showRaw, setShowRaw] = useState(false)
  const [error, setError] = useState('')
  const [copiedOriginal, setCopiedOriginal] = useState(false)
  const [copiedTranslated, setCopiedTranslated] = useState(false)
  const [ocrSource, setOcrSource] = useState<string>('')
  const [translateSource, setTranslateSource] = useState<string>('')
  const [lang, setLang] = useState<Lang>('zh')
  const isMountedRef = useRef(true)
  const originalCopyTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const translatedCopyTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const t = i18n[lang]
  const showOriginal = Boolean(original)
  const translationPending = status === 'ready' && Boolean(original) && !translated.trim()
  const hasContent = status === 'ready' && (original.trim() || translated.trim())

  /** 加载模型信息 + 界面语言 */
  const loadModelInfo = useCallback(async () => {
    try {
      const settings = await api.getSettings()
      if (!isMountedRef.current) return
      const ocrModel = settings.screenshotTranslation?.model || 'OpenAI OCR'
      setOcrSource(ocrModel)
      const directTranslate = settings.screenshotTranslation?.directTranslate ?? false
      const transModel = directTranslate
        ? (settings.screenshotTranslation?.model || 'OpenAI OCR')
        : (settings.translatorModel || 'AI')
      setTranslateSource(transModel)
      setLang((settings.settingsLanguage as Lang) || 'zh')
    } catch (err) {
      if (!isMountedRef.current) return
      console.error('Failed to load settings', err)
    }
  }, [])

  useEffect(() => {
    isMountedRef.current = true
    void loadModelInfo()

    let cleanup1: (() => void) | undefined
    let cleanup2: (() => void) | undefined
    let cleanup3: (() => void) | undefined
    let active = true

    api.onScreenshotProcessing(() => {
      if (!isMountedRef.current) return
      setStatus('processing')
      setError('')
      void loadModelInfo()
    }).then((unlisten) => {
      if (!active) { unlisten(); return }
      cleanup1 = unlisten
    }).catch((err) => { console.error('Failed to listen screenshot-processing', err) })

    api.onScreenshotResult((data) => {
      if (!isMountedRef.current) return
      setOriginal(data.original)
      setTranslated(data.translated)
      setStatus('ready')
    }).then((unlisten) => {
      if (!active) { unlisten(); return }
      cleanup2 = unlisten
    }).catch((err) => { console.error('Failed to listen screenshot-result', err) })

    api.onScreenshotError((errorMsg) => {
      if (!isMountedRef.current) return
      setError(errorMsg)
      setStatus('error')
    }).then((unlisten) => {
      if (!active) { unlisten(); return }
      cleanup3 = unlisten
    }).catch((err) => { console.error('Failed to listen screenshot-error', err) })

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') handleClose()
    }
    window.addEventListener('keydown', handleKeyDown)

    return () => {
      active = false
      isMountedRef.current = false
      cleanup1?.()
      cleanup2?.()
      cleanup3?.()
      if (originalCopyTimeoutRef.current) clearTimeout(originalCopyTimeoutRef.current)
      if (translatedCopyTimeoutRef.current) clearTimeout(translatedCopyTimeoutRef.current)
      window.removeEventListener('keydown', handleKeyDown)
    }
  }, [loadModelInfo])

  const handleClose = () => { api.closeScreenshotWindow() }

  /** 复制：复用 utils/clipboard 的 copyToClipboard */
  const handleCopy = async (text: string, type: 'original' | 'translated') => {
    const ok = await copyToClipboard(text)
    if (!ok || !isMountedRef.current) return
    if (type === 'original') {
      setCopiedOriginal(true)
      if (originalCopyTimeoutRef.current) clearTimeout(originalCopyTimeoutRef.current)
      originalCopyTimeoutRef.current = setTimeout(() => {
        if (isMountedRef.current) setCopiedOriginal(false)
      }, 2000)
    } else {
      setCopiedTranslated(true)
      if (translatedCopyTimeoutRef.current) clearTimeout(translatedCopyTimeoutRef.current)
      translatedCopyTimeoutRef.current = setTimeout(() => {
        if (isMountedRef.current) setCopiedTranslated(false)
      }, 2000)
    }
  }

  return (
    <div className="h-screen w-screen flex flex-col bg-white/95 dark:bg-neutral-900/95 backdrop-blur-2xl text-neutral-900 dark:text-neutral-100 select-none overflow-hidden rounded-2xl relative">
      {/* 顶部隐形 drag bar */}
      <div
        className="absolute top-0 left-0 right-0 h-7 z-10"
        data-tauri-drag-region
      />

      {/* 浮动工具按钮（右上角） */}
      <div className="absolute top-1.5 right-2 z-20 flex items-center gap-0.5">
        {hasContent && (
          <button
            onClick={() => setShowRaw(!showRaw)}
            className={`p-1 rounded-md transition-all duration-200 flex items-center gap-0.5 ${showRaw
              ? 'bg-neutral-100 dark:bg-neutral-800 text-neutral-900 dark:text-white'
              : 'text-neutral-400 hover:text-neutral-600 dark:text-neutral-500 dark:hover:text-neutral-300 hover:bg-black/5 dark:hover:bg-white/10'
              }`}
            title={showRaw ? t.shotShowPreview : t.shotShowSource}
          >
            {showRaw ? <Eye size={12} strokeWidth={2} /> : <Code size={12} strokeWidth={2} />}
          </button>
        )}
        <button
          onClick={handleClose}
          className="p-1 text-neutral-400 hover:text-neutral-600 dark:text-neutral-500 dark:hover:text-neutral-300 rounded-md hover:bg-black/5 dark:hover:bg-white/10 transition-all duration-200"
          title={t.shotClose}
        >
          <X size={13} strokeWidth={2} />
        </button>
      </div>

      {/* 内容区 */}
      <div className="flex-1 overflow-auto px-3 pt-7 pb-2 custom-scrollbar">
        {/* 处理中 */}
        {status === 'processing' && (
          <div className="flex flex-col items-center justify-center h-full gap-2.5">
            <div className="w-7 h-7 border-2 border-neutral-300 dark:border-neutral-600 border-t-neutral-600 dark:border-t-neutral-300 rounded-full animate-spin" />
            <p className="text-xs text-neutral-500 dark:text-neutral-400">{t.shotProcessing}</p>
          </div>
        )}

        {/* 错误 */}
        {status === 'error' && (
          <div className="flex flex-col items-center justify-center h-full gap-2">
            <p className="text-sm text-red-500">{t.shotErrorTitle}</p>
            <p className="text-xs text-neutral-500 dark:text-neutral-400 max-w-xs text-center">{error}</p>
            <button
              onClick={handleClose}
              className="mt-1 px-3 py-1 text-xs text-neutral-600 dark:text-neutral-300 bg-neutral-100 dark:bg-neutral-800 hover:bg-neutral-200 dark:hover:bg-neutral-700 rounded-md transition-colors"
            >
              {t.shotClose}
            </button>
          </div>
        )}

        {/* 结果 */}
        {status === 'ready' && (
          <div className="space-y-3">
            {/* 原文 */}
            {showOriginal && (
              <div className="group">
                <div className="flex items-center justify-between mb-1">
                  <div className="flex items-center gap-1.5 text-[11px] text-neutral-500 dark:text-neutral-400">
                    <span>{t.shotOriginal}</span>
                    {ocrSource && (
                      <span className="flex items-center gap-1 px-1.5 py-0.5 text-[10px] bg-neutral-100 dark:bg-neutral-800 rounded max-w-[140px] truncate">
                        <Scan size={9} strokeWidth={1.5} className="shrink-0" />
                        <span className="truncate">{ocrSource}</span>
                      </span>
                    )}
                  </div>
                  <button
                    onClick={() => handleCopy(original, 'original')}
                    className="flex items-center gap-1 px-1.5 py-0.5 text-[10px] text-neutral-400 hover:text-neutral-700 dark:text-neutral-500 dark:hover:text-neutral-200 rounded hover:bg-black/5 dark:hover:bg-white/10 opacity-0 group-hover:opacity-100 transition-opacity"
                  >
                    {copiedOriginal ? <Check size={10} /> : <Copy size={10} />}
                    {copiedOriginal ? t.shotCopied : t.shotCopy}
                  </button>
                </div>
                <div className="p-2.5 bg-neutral-50 dark:bg-neutral-800/50 rounded-lg text-[13px] leading-relaxed select-text prose prose-sm dark:prose-invert max-w-none prose-p:my-1">
                  {showRaw ? (
                    <pre className="whitespace-pre-wrap font-mono text-[11.5px] bg-transparent p-0 m-0 border-none shadow-none text-neutral-600 dark:text-neutral-400">
                      {original}
                    </pre>
                  ) : (
                    <ReactMarkdown remarkPlugins={[remarkMath]} rehypePlugins={[rehypeKatex]}>
                      {original}
                    </ReactMarkdown>
                  )}
                </div>
              </div>
            )}

            {/* 译文 */}
            <div className="group">
              <div className="flex items-center justify-between mb-1">
                <div className="flex items-center gap-1.5 text-[11px] text-neutral-500 dark:text-neutral-400">
                  <span>{t.shotTranslated}</span>
                  {translateSource && (
                    <span className="flex items-center gap-1 px-1.5 py-0.5 text-[10px] bg-neutral-100 dark:bg-neutral-800 rounded max-w-[140px] truncate">
                      <Cpu size={9} strokeWidth={1.5} className="shrink-0" />
                      <span className="truncate">{translateSource}</span>
                    </span>
                  )}
                </div>
                <button
                  onClick={() => handleCopy(translated, 'translated')}
                  disabled={translationPending}
                  className="flex items-center gap-1 px-2 py-0.5 text-[10px] text-white bg-neutral-800 dark:bg-neutral-200 dark:text-neutral-800 hover:bg-neutral-700 dark:hover:bg-neutral-300 rounded-md transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
                >
                  {copiedTranslated ? <Check size={10} /> : <Copy size={10} />}
                  {copiedTranslated ? t.shotCopied : t.shotCopy}
                </button>
              </div>
              <div className="p-2.5 bg-neutral-100/80 dark:bg-neutral-800 rounded-lg text-[13px] leading-relaxed select-text prose prose-sm dark:prose-invert max-w-none prose-p:my-1">
                {translationPending ? (
                  <p className="text-[12px] text-neutral-500 dark:text-neutral-400">{t.shotTranslating}</p>
                ) : showRaw ? (
                  <pre className="whitespace-pre-wrap font-mono text-[11.5px] bg-transparent p-0 m-0 border-none shadow-none text-neutral-600 dark:text-neutral-400">
                    {translated}
                  </pre>
                ) : (
                  <ReactMarkdown remarkPlugins={[remarkMath]} rehypePlugins={[rehypeKatex]}>
                    {translated}
                  </ReactMarkdown>
                )}
              </div>
            </div>
          </div>
        )}
      </div>

      {/* 底部提示 */}
      {status === 'ready' && (
        <div className="px-3 py-1.5 border-t border-neutral-200/60 dark:border-neutral-700/60 shrink-0">
          <p className="text-center text-[10px] text-neutral-400 dark:text-neutral-500">
            {t.shotHintClose}
          </p>
        </div>
      )}
    </div>
  )
}
