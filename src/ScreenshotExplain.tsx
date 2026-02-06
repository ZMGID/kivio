import { useState, useEffect, useRef, useCallback } from 'react'
import { Send, X, Loader2, Image, Clock, ChevronDown, ChevronRight, Cpu, Code, Eye } from 'lucide-react'
import { api, type ExplainStreamPayload } from './api/tauri'
import ReactMarkdown from 'react-markdown'
import remarkMath from 'remark-math'
import rehypeKatex from 'rehype-katex'
import 'katex/dist/katex.min.css'

interface Message {
  role: 'user' | 'assistant'
  content: string
}

export default function ScreenshotExplain() {
  const [imageId, setImageId] = useState('')
  const [imagePreview, setImagePreview] = useState('')
  const [messages, setMessages] = useState<Message[]>([])
  const [input, setInput] = useState('')
  const [loading, setLoading] = useState(false)
  const [initializing, setInitializing] = useState(true)
  const [showImage, setShowImage] = useState(true)
  const [showHistory, setShowHistory] = useState(false)
  const [history, setHistory] = useState<Array<{ id: string; timestamp: number; messages: Message[] }>>([])
  const [historyMode, setHistoryMode] = useState(false)
  const [modelName, setModelName] = useState('')
  const [showRaw, setShowRaw] = useState(false)
  const [streamEnabled, setStreamEnabled] = useState(false)
  const messagesEndRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLTextAreaElement>(null)
  const imageIdRef = useRef('')
  const streamingRef = useRef<null | { imageId: string; kind: 'summary' | 'answer'; index: number }>(null)
  const settingsLoadedRef = useRef(false)
  const streamEnabledRef = useRef(false)

  const formatError = (err: unknown) => (err instanceof Error ? err.message : String(err))

  const loadModelInfo = useCallback(async () => {
    try {
      const settings = await api.getSettings()
      const stream = settings.screenshotExplain.streamEnabled ?? false
      setStreamEnabled(stream)
      streamEnabledRef.current = stream
      const providerId = settings.screenshotExplain.providerId
      const model = settings.screenshotExplain.model
      const provider = settings.providers.find(p => p.id === providerId)
      // 优先显示具体模型名，如果没找到则显示 Provider 默认
      if (model) {
        setModelName(model)
      } else if (provider) {
        setModelName(provider.enabledModels[0] || provider.name)
      } else {
        setModelName('AI')
      }
      settingsLoadedRef.current = true
      return true
    } catch (err) {
      console.error('Failed to load model info:', err)
      if (!settingsLoadedRef.current) {
        setModelName('AI')
        setStreamEnabled(false)
        streamEnabledRef.current = false
      }
      return false
    }
  }, [])

  const ensureSettings = useCallback(async () => {
    if (settingsLoadedRef.current) return
    await loadModelInfo()
  }, [loadModelInfo])

  const loadImage = useCallback(async (id: string) => {
    try {
      const result = await api.explainReadImage(id)
      if (imageIdRef.current !== id) return
      if (result.success) setImagePreview(result.data ?? '')
    } catch (err) {
      if (imageIdRef.current !== id) return
      console.error('Failed to load image:', err)
    }
  }, [])

  const getInitialSummary = useCallback(async (id: string) => {
    setInitializing(true)
    setLoading(true)
    await ensureSettings()
    const stream = streamEnabledRef.current
    if (stream) {
      streamingRef.current = { imageId: id, kind: 'summary', index: 0 }
      setMessages([{ role: 'assistant', content: '' }])
    }
    try {
      const result = await api.explainGetInitialSummary(id)
      if (imageIdRef.current !== id) return
      const content = result.success ? (result.summary ?? '') : `错误: ${result.error}`
      if (stream) {
        setMessages(prev => {
          const next = [...prev]
          const existing = next[0] ?? { role: 'assistant', content: '' }
          const merged = result.success ? content : (existing.content ? `${existing.content}\n\n${content}` : content)
          next[0] = { ...existing, content: merged }
          return next
        })
      } else {
        setMessages([{ role: 'assistant', content }])
      }
    } catch (err) {
      if (imageIdRef.current !== id) return
      const content = `错误: ${formatError(err)}`
      if (stream) {
        setMessages(prev => {
          const next = [...prev]
          const existing = next[0] ?? { role: 'assistant', content: '' }
          const merged = existing.content ? `${existing.content}\n\n${content}` : content
          next[0] = { ...existing, content: merged }
          return next
        })
      } else {
        setMessages([{ role: 'assistant', content }])
      }
    } finally {
      if (streamingRef.current?.imageId === id && streamingRef.current?.kind === 'summary') {
        streamingRef.current = null
      }
      if (imageIdRef.current === id) {
        setLoading(false)
        setInitializing(false)
        inputRef.current?.focus()
      }
    }
  }, [ensureSettings])

  const loadHistory = useCallback(async () => {
    try {
      const result = await api.explainGetHistory()
      if (result.success) setHistory(result.history || [])
    } catch (err) {
      console.error('Failed to load history:', err)
    }
  }, [])

  useEffect(() => {
    const applyImageId = async (decodedId: string) => {
      if (!decodedId || decodedId === imageIdRef.current) return
      streamingRef.current = null
      imageIdRef.current = decodedId
      setHistoryMode(false)
      setShowHistory(false)
      setShowImage(true)
      setImageId(decodedId)
      setImagePreview('')
      setMessages([])
      await ensureSettings()
      loadImage(decodedId)
      getInitialSummary(decodedId)
    }

    const parseHash = () => {
      const hash = window.location.hash
      const params = new URLSearchParams(hash.split('?')[1] || '')
      const id = params.get('imageId')
      if (id) {
        void applyImageId(decodeURIComponent(id))
      }
    }

    window.addEventListener('hashchange', parseHash)
    const init = async () => {
      await loadModelInfo()
      parseHash()
      loadHistory()
    }
    void init()
    return () => window.removeEventListener('hashchange', parseHash)
  }, [ensureSettings, getInitialSummary, loadHistory, loadImage, loadModelInfo])

  useEffect(() => {
    let unlisten: (() => void) | undefined
    api.onExplainStream((payload: ExplainStreamPayload) => {
      const current = streamingRef.current
      if (!current) return
      if (payload.imageId !== current.imageId || payload.kind !== current.kind) return
      setMessages(prev => {
        if (current.index >= prev.length) return prev
        const next = [...prev]
        const existing = next[current.index]
        if (!existing) return prev
        next[current.index] = { ...existing, content: `${existing.content}${payload.delta}` }
        return next
      })
    }).then((dispose) => {
      unlisten = dispose
    }).catch((err) => {
      console.error('Failed to listen explain stream:', err)
    })
    return () => {
      unlisten?.()
    }
  }, [])

  useEffect(() => { messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' }) }, [messages])

  const handleSend = async () => {
    if (!input.trim() || loading || historyMode || !imageId) return
    const userMessage: Message = { role: 'user', content: input }
    const requestImageId = imageId
    const baseIndex = messages.length
    await ensureSettings()
    const stream = streamEnabledRef.current
    if (stream) {
      streamingRef.current = { imageId: requestImageId, kind: 'answer', index: baseIndex + 1 }
      setMessages(prev => [...prev, userMessage, { role: 'assistant', content: '' }])
    } else {
      setMessages(prev => [...prev, userMessage])
    }
    setInput('')
    setLoading(true)
    try {
      const result = await api.explainAskQuestion(requestImageId, [...messages, userMessage])
      if (imageIdRef.current !== requestImageId) return
      const content = result.success ? (result.response ?? '') : `错误: ${result.error}`
      if (stream) {
        setMessages(prev => {
          const next = [...prev]
          const existing = next[baseIndex + 1]
          if (!existing) return prev
          const merged = result.success ? content : (existing.content ? `${existing.content}\n\n${content}` : content)
          next[baseIndex + 1] = { ...existing, content: merged }
          return next
        })
      } else {
        setMessages(prev => [...prev, { role: 'assistant', content }])
      }
    } catch (err) {
      if (imageIdRef.current !== requestImageId) return
      const content = `错误: ${formatError(err)}`
      if (stream) {
        setMessages(prev => {
          const next = [...prev]
          const existing = next[baseIndex + 1]
          if (!existing) return prev
          const merged = existing.content ? `${existing.content}\n\n${content}` : content
          next[baseIndex + 1] = { ...existing, content: merged }
          return next
        })
      } else {
        setMessages(prev => [...prev, { role: 'assistant', content }])
      }
    } finally {
      if (streamingRef.current?.imageId === requestImageId && streamingRef.current?.kind === 'answer') {
        streamingRef.current = null
      }
      if (imageIdRef.current === requestImageId) {
        setLoading(false)
      }
    }
  }

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); handleSend() }
    else if (e.key === 'Escape') handleClose()
  }

  const saveToHistory = async () => {
    if (imageId && messages.length > 0) {
      try {
        await api.explainSaveHistory(messages)
        await loadHistory()
      } catch (err) {
        console.error('Failed to save history:', err)
      }
    }
  }

  const loadHistoryRecord = async (historyId: string) => {
    try {
      await api.explainCloseCurrent()
      const result = await api.explainLoadHistory(historyId)
      if (result.success && result.record) {
        setMessages(result.record.messages)
        setShowHistory(false)
        setHistoryMode(true)
        setImageId('')
        imageIdRef.current = ''
        setImagePreview('')
        streamingRef.current = null
      }
    } catch (err) {
      console.error('Failed to load history record:', err)
    }
  }

  const handleClose = async () => {
    if (messages.length > 0) await saveToHistory()
    try {
      await api.explainCloseCurrent()
    } catch (err) {
      console.error('Failed to clean up explain image:', err)
    }
    streamingRef.current = null
    api.closeExplainWindow()
  }

  return (
    <div className="h-screen w-screen flex flex-col bg-white/95 dark:bg-neutral-900/95 backdrop-blur-2xl overflow-hidden font-sans text-neutral-900 dark:text-neutral-100">
      {/* 顶部工具栏 */}
      <div
        className="flex items-center justify-between px-4 py-2 border-b border-black/5 dark:border-white/5 select-none"
        data-tauri-drag-region
      >
        <div className="flex items-center gap-2">
          {imagePreview && (
            <button
              onClick={() => setShowImage(!showImage)}
              className="flex items-center gap-1.5 text-[11px] font-medium text-neutral-500 dark:text-neutral-400 hover:text-neutral-700 dark:hover:text-neutral-200 transition-colors"
              data-tauri-drag-region="false"
            >
              {showImage ? <ChevronDown size={14} strokeWidth={2} /> : <ChevronRight size={14} strokeWidth={2} />}
              <Image size={13} strokeWidth={2} />
              <span>{showImage ? '隐藏截图' : '显示截图'}</span>
            </button>
          )}
          {modelName && (
            <span className="flex items-center gap-1 px-2 py-0.5 text-[10px] font-medium text-neutral-500 dark:text-neutral-400 bg-neutral-100 dark:bg-neutral-800 rounded-full border border-neutral-200/50 dark:border-neutral-700/50">
              <Cpu size={10} strokeWidth={2} />{modelName}
            </span>
          )}
        </div>
        <div className="flex items-center gap-1" data-tauri-drag-region="false">
          <button
            onClick={() => setShowHistory(!showHistory)}
            className={`p-1.5 rounded-lg transition-all duration-200 ${showHistory
              ? 'bg-neutral-100 dark:bg-neutral-800 text-neutral-900 dark:text-white'
              : 'text-neutral-400 hover:text-neutral-600 dark:text-neutral-500 dark:hover:text-neutral-300 hover:bg-black/5 dark:hover:bg-white/10'
              }`}
            title="历史记录"
            data-tauri-drag-region="false"
          >
            <Clock size={15} strokeWidth={2} />
          </button>
          <button
            onClick={() => setShowRaw(!showRaw)}
            className={`p-1.5 rounded-lg transition-all duration-200 flex items-center gap-1.5 ${showRaw
              ? 'bg-neutral-100 dark:bg-neutral-800 text-neutral-900 dark:text-white'
              : 'text-neutral-400 hover:text-neutral-600 dark:text-neutral-500 dark:hover:text-neutral-300 hover:bg-black/5 dark:hover:bg-white/10'
              }`}
            title={showRaw ? '显示预览' : '显示源码'}
            data-tauri-drag-region="false"
          >
            {showRaw ? <Eye size={14} strokeWidth={2} /> : <Code size={14} strokeWidth={2} />}
            <span className="text-[11px] font-medium">{showRaw ? '预览' : '源码'}</span>
          </button>
          <button
            onClick={handleClose}
            className="p-1.5 text-neutral-400 hover:text-neutral-600 dark:text-neutral-500 dark:hover:text-neutral-300 rounded-lg hover:bg-black/5 dark:hover:bg-white/10 transition-all duration-200"
            data-tauri-drag-region="false"
          >
            <X size={15} strokeWidth={2} />
          </button>
        </div>
      </div>

      {/* 历史记录面板 */}
      {showHistory && (
        <div className="border-b border-black/5 dark:border-white/5 bg-neutral-50/90 dark:bg-neutral-900/90 backdrop-blur-md max-h-56 overflow-y-auto z-10">
          <div className="p-3 space-y-2">
            <div className="flex items-center justify-between px-1">
              <p className="text-[11px] font-medium text-neutral-500 dark:text-neutral-400">历史记录</p>
            </div>
            {history.length === 0 ? (
              <p className="text-xs text-neutral-400 text-center py-6">暂无历史记录</p>
            ) : (
              <div className="space-y-1">
                {history.map((record) => (
                  <button
                    key={record.id}
                    onClick={() => loadHistoryRecord(record.id)}
                    className="w-full text-left p-3 rounded-xl bg-white dark:bg-neutral-800 hover:bg-neutral-50 dark:hover:bg-neutral-700/50 border border-black/5 dark:border-white/5 transition-all group"
                  >
                    <div className="flex items-start justify-between gap-3">
                      <p className="text-xs text-neutral-600 dark:text-neutral-300 line-clamp-2 flex-1 group-hover:text-neutral-900 dark:group-hover:text-white transition-colors">
                        {(record.messages[0]?.content?.slice(0, 60) ?? '')}...
                      </p>
                      <span className="text-[10px] text-neutral-300 dark:text-neutral-600 whitespace-nowrap pt-0.5">
                        {new Date(record.timestamp).toLocaleDateString('zh-CN', { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' })}
                      </span>
                    </div>
                  </button>
                ))}
              </div>
            )}
          </div>
        </div>
      )}

      {/* 图片预览 */}
      {imagePreview && showImage && (
        <div className="border-b border-black/5 dark:border-white/5 bg-white/50 dark:bg-neutral-900/50 backdrop-blur-sm">
          <div className="px-4 py-3 animate-in slide-in-from-top-2 duration-200">
            <div className="rounded-xl overflow-hidden border border-black/5 dark:border-white/5 shadow-sm bg-neutral-100/50 dark:bg-neutral-800/50">
              <img src={imagePreview} alt="Screenshot" className="max-w-full max-h-48 mx-auto object-contain" />
            </div>
          </div>
        </div>
      )}

      {/* 消息列表 */}
      <div className="flex-1 overflow-y-auto p-4 space-y-5" data-tauri-drag-region="false">
        {initializing && (
          <div className="flex flex-col items-center justify-center py-12 gap-4 animate-in fade-in duration-500">
            <div className="relative">
              <div className="w-8 h-8 border-2 border-neutral-200 dark:border-neutral-700 rounded-full" />
              <div className="absolute top-0 left-0 w-8 h-8 border-2 border-neutral-800 dark:border-white border-t-transparent border-l-transparent border-r-transparent rounded-full animate-spin" />
            </div>
            <p className="text-xs font-medium text-neutral-500 dark:text-neutral-400">正在分析图片内容...</p>
          </div>
        )}

        {messages.map((msg, idx) => (
          <div key={idx} className={`flex ${msg.role === 'user' ? 'justify-end' : 'justify-start'} animate-in slide-in-from-bottom-2 fade-in duration-300`}>
            <div
              className={`max-w-[85%] px-4 py-2.5 text-[13.5px] leading-relaxed shadow-sm ${msg.role === 'user'
                ? 'bg-white dark:bg-white text-neutral-900 border border-black/5 dark:border-white/10 rounded-2xl rounded-tr-sm'
                : 'bg-white dark:bg-neutral-800 border border-black/5 dark:border-white/5 text-neutral-700 dark:text-neutral-200 rounded-2xl rounded-tl-sm'
                }`}
            >
              <div className="prose dark:prose-invert max-w-none text-[13.5px] leading-relaxed">
                {showRaw ? (
                  <pre className="whitespace-pre-wrap font-mono text-[12px] bg-transparent p-0 m-0 border-none shadow-none text-inherit">
                    {msg.content}
                  </pre>
                ) : (
                  <ReactMarkdown
                    remarkPlugins={[remarkMath]}
                    rehypePlugins={[rehypeKatex]}
                  >
                    {msg.content}
                  </ReactMarkdown>
                )}
              </div>
            </div>
          </div>
        ))}

        {loading && !initializing && !(streamEnabled && streamingRef.current) && (
          <div className="flex justify-start animate-in fade-in duration-200">
            <div className="bg-white dark:bg-neutral-800 px-4 py-2.5 rounded-2xl rounded-tl-sm border border-black/5 dark:border-white/5 shadow-sm">
              <Loader2 className="animate-spin text-neutral-400" size={16} strokeWidth={2} />
            </div>
          </div>
        )}
        <div ref={messagesEndRef} />
      </div>

      {/* 输入区域 */}
      <div className="p-4 bg-white/80 dark:bg-neutral-900/80 backdrop-blur-md border-t border-black/5 dark:border-white/5">
        <div className="flex gap-2.5 items-end">
          <textarea
            ref={inputRef}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={historyMode ? '历史记录为只读' : '输入问题...'}
            disabled={loading || initializing || historyMode || !imageId}
            className="flex-1 px-4 py-3 bg-neutral-100 dark:bg-neutral-800/50 border-transparent focus:bg-white dark:focus:bg-neutral-800 border focus:border-neutral-200 dark:focus:border-neutral-700 rounded-xl resize-none focus:outline-none text-[13.5px] text-neutral-900 dark:text-white placeholder-neutral-400 dark:placeholder-neutral-500 transition-all duration-200 shadow-inner"
            rows={1}
            style={{ minHeight: '44px', maxHeight: '120px' }}
            data-tauri-drag-region="false"
          />
          <button
            onClick={handleSend}
            disabled={!input.trim() || loading || initializing || historyMode || !imageId}
            className="p-3 bg-neutral-900 dark:bg-white text-white dark:text-neutral-900 rounded-xl hover:bg-neutral-700 dark:hover:bg-neutral-200 disabled:opacity-30 disabled:cursor-not-allowed transition-all duration-200 shadow-sm active:scale-95"
            data-tauri-drag-region="false"
          >
            <Send size={18} strokeWidth={2} />
          </button>
        </div>
        <div className="flex justify-between items-center mt-2 px-1">
          <p className="text-[10px] text-neutral-400 dark:text-neutral-500">↵ 发送 · ⇧↵ 换行</p>
          <p className="text-[10px] text-neutral-400 dark:text-neutral-500">esc 关闭</p>
        </div>
      </div>
    </div>
  )
}
