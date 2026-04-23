import { useState, useEffect, useRef, useCallback } from 'react'
import { Send, X, Loader2, Clock, Code, Eye, Cpu } from 'lucide-react'
import { api, type ExplainStreamPayload } from './api/tauri'
import ReactMarkdown from 'react-markdown'
import remarkMath from 'remark-math'
import rehypeKatex from 'rehype-katex'
import 'katex/dist/katex.min.css'

// 对话消息类型
interface Message {
  role: 'user' | 'assistant'
  content: string
}

// 窗口尺寸常量
const COMPACT_WIDTH = 480
const COMPACT_HEIGHT = 280
const EXPANDED_WIDTH = 520
const EXPANDED_HEIGHT = 680

/**
 * 截图解释组件 —— 紧凑/展开双模式
 * 初始状态：小窗口，左侧缩略图 + 右侧总结 + 底部输入框
 * 提问后：窗口自动展开，显示完整对话流
 */
export default function ScreenshotExplain() {
  const [imageId, setImageId] = useState('')
  const [imagePreview, setImagePreview] = useState('')
  const [messages, setMessages] = useState<Message[]>([])
  const [input, setInput] = useState('')
  const [loading, setLoading] = useState(false)
  const [initializing, setInitializing] = useState(true)
  const [showHistory, setShowHistory] = useState(false)
  const [history, setHistory] = useState<Array<{ id: string; timestamp: number; messages: Message[] }>>([])
  const [historyMode, setHistoryMode] = useState(false)
  const [showRaw, setShowRaw] = useState(false)
  const [streamEnabled, setStreamEnabled] = useState(false)
  const [isExpanded, setIsExpanded] = useState(false)
  const [modelName, setModelName] = useState('')

  const messagesEndRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLTextAreaElement>(null)
  const imageIdRef = useRef('')
  const streamingRef = useRef<null | { imageId: string; kind: 'summary' | 'answer'; index: number }>(null)
  const streamEnabledRef = useRef(false)
  const resizeTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const formatError = (err: unknown) => (err instanceof Error ? err.message : String(err))

  /**
   * 调整窗口尺寸（带防抖）
   */
  const resizeWindow = useCallback((width: number, height: number) => {
    if (resizeTimeoutRef.current) {
      clearTimeout(resizeTimeoutRef.current)
    }
    resizeTimeoutRef.current = setTimeout(() => {
      void api.resizeWindow(width, height)
    }, 50)
  }, [])

  /**
   * 加载模型和流式配置
   */
  const loadModelInfo = useCallback(async () => {
    try {
      const settings = await api.getSettings()
      const stream = settings.screenshotExplain.streamEnabled ?? false
      setStreamEnabled(stream)
      streamEnabledRef.current = stream
      const model = settings.screenshotExplain?.model || settings.translatorModel || ''
      setModelName(model)
      return true
    } catch (err) {
      console.error('Failed to load model info:', err)
      setStreamEnabled(false)
      streamEnabledRef.current = false
      setModelName('')
      return false
    }
  }, [])

  const ensureSettings = useCallback(async () => {
    await loadModelInfo()
  }, [loadModelInfo])

  /**
   * 加载截图图片
   */
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

  /**
   * 获取初始 AI 总结
   */
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

  /**
   * 加载历史记录
   */
  const loadHistory = useCallback(async () => {
    try {
      const result = await api.explainGetHistory()
      if (result.success) setHistory(result.history || [])
    } catch (err) {
      console.error('Failed to load history:', err)
    }
  }, [])

  // 初始化：监听 URL hash 变化加载图片
  useEffect(() => {
    const applyImageId = async (decodedId: string) => {
      if (!decodedId || decodedId === imageIdRef.current) return
      streamingRef.current = null
      imageIdRef.current = decodedId
      setHistoryMode(false)
      setShowHistory(false)
      setImageId(decodedId)
      setImagePreview('')
      setMessages([])
      setIsExpanded(false)
      setInitializing(true)
      await loadModelInfo()
      loadImage(decodedId)
      getInitialSummary(decodedId)
      // 初始紧凑窗口
      resizeWindow(COMPACT_WIDTH, COMPACT_HEIGHT)
    }

    const parseHash = async () => {
      const hash = window.location.hash
      const params = new URLSearchParams(hash.split('?')[1] || '')
      const id = params.get('imageId')
      if (id) {
        await applyImageId(decodeURIComponent(id))
      }
    }

    const onHashChange = () => { void parseHash() }
    window.addEventListener('hashchange', onHashChange)
    const init = async () => {
      await loadModelInfo()
      await parseHash()
      await loadHistory()
      // 内容准备就绪后显示窗口，避免闪白
      await api.showWindow()
    }
    void init()
    return () => window.removeEventListener('hashchange', onHashChange)
  }, [getInitialSummary, loadHistory, loadImage, loadModelInfo, resizeWindow])

  // 监听流式输出事件
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

  // 消息列表自动滚动到底部
  useEffect(() => { messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' }) }, [messages])

  // 检测是否需要展开窗口
  useEffect(() => {
    const hasUserMessages = messages.some(m => m.role === 'user')
    if (hasUserMessages && !isExpanded) {
      setIsExpanded(true)
      resizeWindow(EXPANDED_WIDTH, EXPANDED_HEIGHT)
    }
  }, [messages, isExpanded, resizeWindow])

  // textarea 自动增高
  useEffect(() => {
    const textarea = inputRef.current
    if (textarea) {
      textarea.style.height = 'auto'
      textarea.style.height = `${Math.min(textarea.scrollHeight, 120)}px`
    }
  }, [input])

  /**
   * 发送用户问题
   */
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

  // 键盘事件：Enter 发送，Shift+Enter 换行，Esc 关闭
  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); handleSend() }
    else if (e.key === 'Escape') handleClose()
  }

  // 保存当前对话到历史记录
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

  // 加载历史记录中的某条对话
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
        setIsExpanded(true)
        resizeWindow(EXPANDED_WIDTH, EXPANDED_HEIGHT)
      }
    } catch (err) {
      console.error('Failed to load history record:', err)
    }
  }

  // 关闭窗口并清理资源
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

  // 当前是否有对话内容（用于判断是否显示对话区域）
  const hasConversation = messages.some(m => m.role === 'user')

  return (
    <div className="h-screen w-screen flex flex-col bg-white dark:bg-neutral-900 overflow-hidden font-sans text-neutral-900 dark:text-neutral-100 rounded-2xl">
      {/* 顶部工具栏 */}
      <div
        className="flex items-center justify-between px-3 py-2 border-b border-black/5 dark:border-white/5 shrink-0 select-none"
        data-tauri-drag-region
      >
        <div className="flex items-center gap-2" data-tauri-drag-region="false">
          <span className="text-[12px] font-semibold text-neutral-700 dark:text-neutral-200 tracking-tight">截图解释</span>
        </div>
        <div className="flex items-center gap-0.5" data-tauri-drag-region="false">
          <span className="flex items-center gap-1 px-1.5 py-0.5 text-[10px] font-medium text-neutral-500 dark:text-neutral-400 bg-neutral-100 dark:bg-neutral-800 rounded-md mr-1">
            <Cpu size={10} strokeWidth={2} />{modelName}
          </span>
          <button
            onClick={() => setShowHistory(!showHistory)}
            className={`p-1.5 rounded-md transition-all duration-200 ${showHistory
              ? 'bg-neutral-100 dark:bg-neutral-800 text-neutral-900 dark:text-white'
              : 'text-neutral-400 hover:text-neutral-600 dark:text-neutral-500 dark:hover:text-neutral-300 hover:bg-black/5 dark:hover:bg-white/10'
              }`}
            title="历史记录"
            data-tauri-drag-region="false"
          >
            <Clock size={14} strokeWidth={2} />
          </button>
          <button
            onClick={() => setShowRaw(!showRaw)}
            className={`p-1.5 rounded-md transition-all duration-200 flex items-center gap-1 ${showRaw
              ? 'bg-neutral-100 dark:bg-neutral-800 text-neutral-900 dark:text-white'
              : 'text-neutral-400 hover:text-neutral-600 dark:text-neutral-500 dark:hover:text-neutral-300 hover:bg-black/5 dark:hover:bg-white/10'
              }`}
            title={showRaw ? '显示预览' : '显示源码'}
            data-tauri-drag-region="false"
          >
            {showRaw ? <Eye size={13} strokeWidth={2} /> : <Code size={13} strokeWidth={2} />}
            <span className="text-[10px] font-medium">{showRaw ? '预览' : '源码'}</span>
          </button>
          <button
            onClick={handleClose}
            className="p-1.5 text-neutral-400 hover:text-neutral-600 dark:text-neutral-500 dark:hover:text-neutral-300 rounded-md hover:bg-black/5 dark:hover:bg-white/10 transition-all duration-200"
            title="关闭"
            data-tauri-drag-region="false"
          >
            <X size={14} strokeWidth={2} />
          </button>
        </div>
      </div>

      {/* 历史记录面板 */}
      {showHistory && (
        <div className="border-b border-black/5 dark:border-white/5 bg-neutral-50/90 dark:bg-neutral-900/90 max-h-48 overflow-y-auto custom-scrollbar z-10 shrink-0">
          <div className="p-2.5 space-y-1.5">
            <p className="text-[11px] font-medium text-neutral-500 dark:text-neutral-400 px-1">历史记录</p>
            {history.length === 0 ? (
              <p className="text-xs text-neutral-400 text-center py-4">暂无历史记录</p>
            ) : (
              <div className="space-y-1">
                {history.map((record) => (
                  <button
                    key={record.id}
                    onClick={() => loadHistoryRecord(record.id)}
                    className="w-full text-left p-2.5 rounded-lg bg-white dark:bg-neutral-800 hover:bg-neutral-50 dark:hover:bg-neutral-700/50 border border-black/5 dark:border-white/5 transition-all group"
                  >
                    <div className="flex items-start justify-between gap-2">
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

      {/* 主体内容区域 */}
      <div className="flex-1 flex flex-col overflow-hidden">
        {/* 紧凑模式：图片 + 总结并排 */}
        {!hasConversation && (
          <div className="flex gap-3 p-4 shrink-0">
            {/* 图片缩略图 */}
            {imagePreview && (
              <div className="w-20 h-14 rounded-lg overflow-hidden shrink-0 border border-black/5 dark:border-white/5 bg-neutral-100 dark:bg-neutral-800">
                <img src={imagePreview} alt="Screenshot" className="w-full h-full object-cover" />
              </div>
            )}
            {/* 总结内容 */}
            <div className="flex-1 min-w-0 overflow-y-auto custom-scrollbar max-h-[140px]">
              {initializing ? (
                <div className="flex items-center gap-2 py-2">
                  <div className="w-4 h-4 border-2 border-neutral-300 dark:border-neutral-600 border-t-transparent rounded-full animate-spin" />
                  <span className="text-xs text-neutral-500 dark:text-neutral-400">AI 正在观察图片...</span>
                </div>
              ) : (
                <div className="text-[13px] leading-6 text-neutral-800 dark:text-neutral-200">
                  {messages[0]?.content || ''}
                </div>
              )}
            </div>
          </div>
        )}

        {/* 展开模式：图片小缩略图 + 对话区域 */}
        {hasConversation && (
          <>
            {/* 顶部小缩略图 */}
            {imagePreview && (
              <div className="px-4 pt-3 pb-1 shrink-0">
                <div className="w-16 h-11 rounded-lg overflow-hidden border border-black/5 dark:border-white/5 bg-neutral-100 dark:bg-neutral-800">
                  <img src={imagePreview} alt="Screenshot" className="w-full h-full object-cover" />
                </div>
              </div>
            )}
            {/* 对话区域 */}
            <div className="flex-1 overflow-y-auto px-4 py-3 space-y-4 custom-scrollbar">
              {messages.map((msg, idx) => (
                <div key={idx} className={`flex ${msg.role === 'user' ? 'justify-end' : 'justify-start'}`}>
                  {msg.role === 'assistant' ? (
                    <div className="max-w-[90%]">
                      <div className="prose dark:prose-invert max-w-none text-[13.5px] leading-7 text-neutral-800 dark:text-neutral-200">
                        {showRaw ? (
                          <pre className="whitespace-pre-wrap font-mono text-[12px] bg-transparent p-0 m-0 border-none shadow-none text-inherit">
                            {msg.content}
                          </pre>
                        ) : (
                          <ReactMarkdown remarkPlugins={[remarkMath]} rehypePlugins={[rehypeKatex]}>
                            {msg.content}
                          </ReactMarkdown>
                        )}
                      </div>
                    </div>
                  ) : (
                    <div className="max-w-[85%] px-3.5 py-2 bg-neutral-100 dark:bg-neutral-800/80 text-neutral-900 dark:text-neutral-100 rounded-2xl rounded-tr-sm text-[13.5px] leading-6">
                      {msg.content}
                    </div>
                  )}
                </div>
              ))}
              {/* 加载中 / 流式打字指示 */}
              {loading && !initializing && (
                <div className="flex justify-start">
                  <div className="px-3.5 py-2.5 bg-neutral-100 dark:bg-neutral-800/80 rounded-2xl rounded-tl-sm flex items-center gap-1">
                    {/* 流式模式且已有内容时显示脉冲点，否则显示旋转图标 */}
                    {streamEnabled && streamingRef.current && messages[messages.length - 1]?.content ? (
                      <>
                        <span className="w-1.5 h-1.5 rounded-full bg-neutral-400 animate-pulse" />
                        <span className="w-1.5 h-1.5 rounded-full bg-neutral-400 animate-pulse [animation-delay:0.2s]" />
                        <span className="w-1.5 h-1.5 rounded-full bg-neutral-400 animate-pulse [animation-delay:0.4s]" />
                      </>
                    ) : (
                      <Loader2 className="animate-spin text-neutral-400" size={16} strokeWidth={2} />
                    )}
                  </div>
                </div>
              )}
              <div ref={messagesEndRef} />
            </div>
          </>
        )}
      </div>

      {/* 输入区域 */}
      <div className="p-3 bg-white dark:bg-neutral-900 border-t border-black/5 dark:border-white/5 shrink-0">
        <div className="flex gap-2 items-end">
          <textarea
            ref={inputRef}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={historyMode ? '历史记录为只读' : '输入问题...'}
            disabled={loading || initializing || historyMode || !imageId}
            className="flex-1 px-3.5 py-2.5 bg-neutral-100 dark:bg-neutral-800/50 border border-transparent focus:bg-white dark:focus:bg-neutral-800 focus:border-neutral-200 dark:focus:border-neutral-700 rounded-xl resize-none focus:outline-none text-[13.5px] text-neutral-900 dark:text-white placeholder-neutral-400 dark:placeholder-neutral-500 transition-all duration-200"
            rows={1}
            style={{ minHeight: '40px', maxHeight: '120px' }}
            data-tauri-drag-region="false"
          />
          <button
            onClick={handleSend}
            disabled={!input.trim() || loading || initializing || historyMode || !imageId}
            className="p-2.5 bg-neutral-900 dark:bg-white text-white dark:text-neutral-900 rounded-xl hover:bg-neutral-700 dark:hover:bg-neutral-200 disabled:opacity-30 disabled:cursor-not-allowed transition-all duration-200 active:scale-95 shrink-0"
            data-tauri-drag-region="false"
          >
            <Send size={16} strokeWidth={2} />
          </button>
        </div>
        <div className="flex justify-between items-center mt-1.5 px-1">
          <p className="text-[10px] text-neutral-400 dark:text-neutral-500">↵ 发送 · ⇧↵ 换行</p>
          <p className="text-[10px] text-neutral-400 dark:text-neutral-500">esc 关闭</p>
        </div>
      </div>
    </div>
  )
}
