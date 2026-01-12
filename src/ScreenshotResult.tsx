import { useState, useEffect, type CSSProperties } from 'react'
import { X, Copy, Check, Cpu, Scan } from 'lucide-react'
import './index.css'

type AppRegionStyle = CSSProperties & { WebkitAppRegion?: 'drag' | 'no-drag' }
const dragStyle: AppRegionStyle = { WebkitAppRegion: 'drag' }
const noDragStyle: AppRegionStyle = { WebkitAppRegion: 'no-drag' }

export default function ScreenshotResult() {
  const [status, setStatus] = useState<'processing' | 'ready' | 'error'>('processing')
  const [original, setOriginal] = useState('')
  const [translated, setTranslated] = useState('')
  const [error, setError] = useState('')
  const [copiedOriginal, setCopiedOriginal] = useState(false)
  const [copiedTranslated, setCopiedTranslated] = useState(false)
  const [ocrSource, setOcrSource] = useState<string>('')
  const [translateSource, setTranslateSource] = useState<string>('')

  useEffect(() => {
    if (!window.api) return

    window.api.getSettings().then((settings) => {
      const ocr = settings.screenshotTranslation?.ocrSource || 'system'
      setOcrSource(ocr === 'system' ? '系统 OCR' : (ocr === 'openai' ? 'OpenAI' : 'GLM-4V'))

      if (settings.source === 'bing') {
        setTranslateSource('Bing')
      } else if (settings.source === 'openai') {
        setTranslateSource(settings.openai?.model || 'AI')
      } else {
        setTranslateSource('自定义')
      }
    })

    const cleanup1 = window.api.onScreenshotProcessing(() => {
      setStatus('processing')
    })

    const cleanup2 = window.api.onScreenshotResult((data) => {
      setOriginal(data.original)
      setTranslated(data.translated)
      setStatus('ready')
    })

    const cleanup3 = window.api.onScreenshotError((errorMsg) => {
      setError(errorMsg)
      setStatus('error')
    })

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        handleClose()
      }
    }
    window.addEventListener('keydown', handleKeyDown)

    return () => {
      cleanup1?.()
      cleanup2?.()
      cleanup3?.()
      window.removeEventListener('keydown', handleKeyDown)
    }
  }, [])

  const handleClose = () => {
    if (window.api) {
      window.api.closeScreenshotWindow()
    }
  }

  const handleCopy = (text: string, type: 'original' | 'translated') => {
    if (!navigator.clipboard) return
    navigator.clipboard.writeText(text).then(() => {
      if (type === 'original') {
        setCopiedOriginal(true)
        setTimeout(() => setCopiedOriginal(false), 2000)
      } else {
        setCopiedTranslated(true)
        setTimeout(() => setCopiedTranslated(false), 2000)
      }
    })
  }

  return (
    <div
      className="h-screen w-screen flex flex-col bg-white/95 dark:bg-neutral-900/95 backdrop-blur-2xl text-neutral-900 dark:text-neutral-100 select-none overflow-hidden"
      style={dragStyle}
    >
      {/* 标题栏 */}
      <div className="flex justify-between items-center px-4 py-3 border-b border-neutral-200/60 dark:border-neutral-700/60">
        <h2 className="text-sm font-medium text-neutral-600 dark:text-neutral-300">截图翻译</h2>
        <button
          onClick={handleClose}
          className="p-1 text-neutral-400 hover:text-neutral-600 dark:text-neutral-500 dark:hover:text-neutral-300 rounded-md hover:bg-black/5 dark:hover:bg-white/10 transition-colors"
          style={noDragStyle}
        >
          <X size={16} strokeWidth={1.5} />
        </button>
      </div>

      {/* 内容区 */}
      <div className="flex-1 overflow-auto p-4" style={noDragStyle}>
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
                  >
                    {copiedOriginal ? <Check size={10} /> : <Copy size={10} />}
                    {copiedOriginal ? '已复制' : '复制'}
                  </button>
                </div>
                <div className="p-3 bg-neutral-50 dark:bg-neutral-800/50 rounded-lg text-sm leading-relaxed whitespace-pre-wrap select-text">
                  {original}
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
                >
                  {copiedTranslated ? <Check size={10} /> : <Copy size={10} />}
                  {copiedTranslated ? '已复制' : '复制'}
                </button>
              </div>
              <div className="p-3 bg-neutral-100/80 dark:bg-neutral-800 rounded-lg text-sm leading-relaxed whitespace-pre-wrap select-text">
                {translated}
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
