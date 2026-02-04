import { useState, useEffect, useRef } from 'react'
import { X, Copy, Check, Cpu, Scan, Code, Eye } from 'lucide-react'
import { api } from './api/tauri'
import ReactMarkdown from 'react-markdown'
import remarkMath from 'remark-math'
import rehypeKatex from 'rehype-katex'
import 'katex/dist/katex.min.css'
import './index.css'

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
  const isMountedRef = useRef(true)
  const originalCopyTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const translatedCopyTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(() => {
    isMountedRef.current = true
    const loadSettings = async () => {
      try {
        const settings = await api.getSettings()
        if (!isMountedRef.current) return
        const ocrModel =
          settings.screenshotTranslation?.model || 'OpenAI OCR'
        setOcrSource(ocrModel)
        const transModel = settings.translatorModel || 'AI'
        setTranslateSource(transModel)
      } catch (err) {
        if (!isMountedRef.current) return
        console.error('Failed to load settings', err)
      }
    }
    loadSettings()

    let cleanup1: (() => void) | undefined
    let cleanup2: (() => void) | undefined
    let cleanup3: (() => void) | undefined
    let active = true

    api.onScreenshotProcessing(() => {
      if (!isMountedRef.current) return
      setStatus('processing')
    }).then((unlisten) => {
      if (!active) {
        unlisten()
        return
      }
      cleanup1 = unlisten
    }).catch((err) => {
      console.error('Failed to listen screenshot-processing', err)
    })

    api.onScreenshotResult((data) => {
      if (!isMountedRef.current) return
      setOriginal(data.original)
      setTranslated(data.translated)
      setStatus('ready')
    }).then((unlisten) => {
      if (!active) {
        unlisten()
        return
      }
      cleanup2 = unlisten
    }).catch((err) => {
      console.error('Failed to listen screenshot-result', err)
    })

    api.onScreenshotError((errorMsg) => {
      if (!isMountedRef.current) return
      setError(errorMsg)
      setStatus('error')
    }).then((unlisten) => {
      if (!active) {
        unlisten()
        return
      }
      cleanup3 = unlisten
    }).catch((err) => {
      console.error('Failed to listen screenshot-error', err)
    })

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        handleClose()
      }
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
  }, [])

  const handleClose = () => {
    api.closeScreenshotWindow()
  }

  const tryLegacyCopy = (text: string) => {
    const textarea = document.createElement('textarea')
    textarea.value = text
    textarea.setAttribute('readonly', '')
    textarea.style.position = 'absolute'
    textarea.style.left = '-9999px'
    document.body.appendChild(textarea)
    textarea.select()
    const result = document.execCommand('copy')
    document.body.removeChild(textarea)
    return result
  }

  const handleCopy = (text: string, type: 'original' | 'translated') => {
    const setCopiedFlag = () => {
      if (!isMountedRef.current) return
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

    if (!navigator.clipboard) {
      const ok = tryLegacyCopy(text)
      if (!ok) console.warn('Clipboard API unavailable and fallback copy failed.')
      else setCopiedFlag()
      return
    }

    navigator.clipboard.writeText(text)
      .then(() => setCopiedFlag())
      .catch((err) => {
        const ok = tryLegacyCopy(text)
        if (ok) {
          setCopiedFlag()
        } else {
          console.warn('Failed to copy text to clipboard.', err)
        }
      })
  }

  return (
    <div className="h-screen w-screen flex flex-col bg-white/95 dark:bg-neutral-900/95 backdrop-blur-2xl text-neutral-900 dark:text-neutral-100 select-none overflow-hidden">
      {/* 标题栏 */}
      <div
        className="flex justify-between items-center px-4 py-3 border-b border-neutral-200/60 dark:border-neutral-700/60"
        data-tauri-drag-region
      >
        <h2 className="text-sm font-medium text-neutral-600 dark:text-neutral-300">截图翻译</h2>
        <div className="flex items-center gap-2">
          {status === 'ready' && (
            <button
              onClick={() => setShowRaw(!showRaw)}
              className={`p-1 text-neutral-400 hover:text-neutral-600 dark:text-neutral-500 dark:hover:text-neutral-300 rounded-md hover:bg-black/5 dark:hover:bg-white/10 transition-colors flex items-center gap-1 px-2`}
              title={showRaw ? "显示预览" : "显示源码"}
              data-tauri-drag-region="false"
            >
              {showRaw ? <Eye size={14} /> : <Code size={14} />}
              <span className="text-[11px] font-medium">{showRaw ? '预览' : '源码'}</span>
            </button>
          )}
          <button
            onClick={handleClose}
            className="p-1 text-neutral-400 hover:text-neutral-600 dark:text-neutral-500 dark:hover:text-neutral-300 rounded-md hover:bg-black/5 dark:hover:bg-white/10 transition-colors"
            data-tauri-drag-region="false"
          >
            <X size={16} strokeWidth={1.5} />
          </button>
        </div>
      </div>

      {/* 内容区 */}
      <div className="flex-1 overflow-auto p-4" data-tauri-drag-region="false">
        {/* 处理中状态 */}
        {status === 'processing' && (
          <div className="flex flex-col items-center justify-center h-full gap-3">
            <div className="w-8 h-8 border-2 border-neutral-300 dark:border-neutral-600 border-t-neutral-600 dark:border-t-neutral-300 rounded-full animate-spin" />
            <p className="text-sm text-neutral-500 dark:text-neutral-400">正在识别并翻译...</p>
          </div>
        )}

        {/* 错误状态 */}
        {status === 'error' && (
          <div className="flex flex-col items-center justify-center h-full gap-3">
            <p className="text-sm text-red-500">识别失败</p>
            <p className="text-xs text-neutral-500 dark:text-neutral-400 max-w-xs text-center">{error}</p>
            <button
              onClick={handleClose}
              className="mt-2 px-3 py-1.5 text-xs text-neutral-600 dark:text-neutral-300 bg-neutral-100 dark:bg-neutral-800 hover:bg-neutral-200 dark:hover:bg-neutral-700 rounded-md transition-colors"
              data-tauri-drag-region="false"
            >
              关闭
            </button>
          </div>
        )}

        {/* 结果展示 */}
        {status === 'ready' && (
          <div className="space-y-4">
            {/* 原文 */}
            {original && (
              <div className="group">
                <div className="flex items-center justify-between mb-1.5">
                  <div className="flex items-center gap-2 text-xs text-neutral-500 dark:text-neutral-400">
                    <span>原文</span>
                    {ocrSource && (
                      <span className="flex items-center gap-1 px-1.5 py-0.5 text-[10px] bg-neutral-100 dark:bg-neutral-800 rounded">
                        <Scan size={9} strokeWidth={1.5} />
                        {ocrSource}
                      </span>
                    )}
                  </div>
                  <button
                    onClick={() => handleCopy(original, 'original')}
                    className="flex items-center gap-1 px-1.5 py-0.5 text-[10px] text-neutral-400 hover:text-neutral-600 dark:text-neutral-500 dark:hover:text-neutral-300 opacity-0 group-hover:opacity-100 transition-opacity"
                    data-tauri-drag-region="false"
                  >
                    {copiedOriginal ? <Check size={10} /> : <Copy size={10} />}
                    {copiedOriginal ? '已复制' : '复制'}
                  </button>
                </div>
                <div className="p-3 bg-neutral-50 dark:bg-neutral-800/50 rounded-lg text-sm leading-relaxed select-text prose dark:prose-invert max-w-none">
                  {showRaw ? (
                    <pre className="whitespace-pre-wrap font-mono text-[12px] bg-transparent p-0 m-0 border-none shadow-none text-neutral-600 dark:text-neutral-400">
                      {original}
                    </pre>
                  ) : (
                    <ReactMarkdown
                      remarkPlugins={[remarkMath]}
                      rehypePlugins={[rehypeKatex]}
                    >
                      {original}
                    </ReactMarkdown>
                  )}
                </div>
              </div>
            )}

            {/* 翻译结果 */}
            <div className="group">
              <div className="flex items-center justify-between mb-1.5">
                <div className="flex items-center gap-2 text-xs text-neutral-500 dark:text-neutral-400">
                  <span>译文</span>
                  {translateSource && (
                    <span className="flex items-center gap-1 px-1.5 py-0.5 text-[10px] bg-neutral-100 dark:bg-neutral-800 rounded">
                      <Cpu size={9} strokeWidth={1.5} />
                      {translateSource}
                    </span>
                  )}
                </div>
                <button
                  onClick={() => handleCopy(translated, 'translated')}
                  className="flex items-center gap-1 px-2 py-1 text-[10px] text-white bg-neutral-800 dark:bg-neutral-200 dark:text-neutral-800 hover:bg-neutral-700 dark:hover:bg-neutral-300 rounded-md transition-colors"
                  data-tauri-drag-region="false"
                >
                  {copiedTranslated ? <Check size={10} /> : <Copy size={10} />}
                  {copiedTranslated ? '已复制' : '复制'}
                </button>
              </div>
              <div className="p-3 bg-neutral-100/80 dark:bg-neutral-800 rounded-lg text-sm leading-relaxed select-text prose dark:prose-invert max-w-none">
                {showRaw ? (
                  <pre className="whitespace-pre-wrap font-mono text-[12px] bg-transparent p-0 m-0 border-none shadow-none text-neutral-600 dark:text-neutral-400">
                    {translated}
                  </pre>
                ) : (
                  <ReactMarkdown
                    remarkPlugins={[remarkMath]}
                    rehypePlugins={[rehypeKatex]}
                  >
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
        <div className="px-4 py-2 border-t border-neutral-200/60 dark:border-neutral-700/60">
          <p className="text-center text-[10px] text-neutral-400 dark:text-neutral-500">
            esc 关闭
          </p>
        </div>
      )}
    </div>
  )
}
