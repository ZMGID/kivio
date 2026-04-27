import { useState, useEffect, useRef, useCallback } from 'react'
import { Send, X, Loader2, Clock, Code, Eye, Cpu, Copy, Check, RefreshCw, Square } from 'lucide-react'
import { api, type ExplainStreamPayload, type ExplainMessage } from './api/tauri'
import ReactMarkdown from 'react-markdown'
import remarkMath from 'remark-math'
import rehypeKatex from 'rehype-katex'
import 'katex/dist/katex.min.css'
import { i18n, type Lang } from './settings/i18n'
import { copyToClipboard } from './utils/clipboard'

// 对话消息类型
type Message = ExplainMessage

// 窗口尺寸：紧凑(无对话) ↔ 展开(有对话)
const COMPACT_WIDTH = 440
const COMPACT_HEIGHT = 180
const EXPANDED_WIDTH = 500
const EXPANDED_HEIGHT = 600

// 流式插槽：标记当前流写入哪条消息
type StreamSlot = { imageId: string; kind: 'summary' | 'answer'; index: number }

/**
 * 截图解释组件 —— 紧凑/展开双模式
 * - 紧凑：缩略图 + 初始总结（或空状态）+ 输入框
 * - 展开：缩略图 + 完整对话流 + 输入框
 * 支持复制 / 重新生成 / 停止生成。
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
  const [streamingActive, setStreamingActive] = useState(false)
  const [isExpanded, setIsExpanded] = useState(false)
  const [modelName, setModelName] = useState('')
  const [autoSummaryEnabled, setAutoSummaryEnabled] = useState(true)
  const [lang, setLang] = useState<Lang>('zh')
  const [copiedIndex, setCopiedIndex] = useState<number | null>(null)

  const messagesEndRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLTextAreaElement>(null)
  const imageIdRef = useRef('')
  const streamingRef = useRef<StreamSlot | null>(null)
  const streamEnabledRef = useRef(false)
  const autoSummaryRef = useRef(true)
  const resizeTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const copyTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const t = i18n[lang]
  const formatError = (err: unknown) => (err instanceof Error ? err.message : String(err))

  /** 防抖窗口尺寸调整 */
  const resizeWindow = useCallback((width: number, height: number) => {
    if (resizeTimeoutRef.current) clearTimeout(resizeTimeoutRef.current)
    resizeTimeoutRef.current = setTimeout(() => {
      void api.resizeWindow(width, height)
    }, 50)
  }, [])

  /** 加载流式开关 / 模型名 / 自动总结 / 界面语言 */
  const loadModelInfo = useCallback(async () => {
    try {
      const settings = await api.getSettings()
      const stream = settings.screenshotExplain.streamEnabled ?? false
      streamEnabledRef.current = stream
      const model = settings.screenshotExplain?.model || settings.translatorModel || ''
      setModelName(model)
      const auto = settings.screenshotExplain.autoSummaryEnabled ?? true
      setAutoSummaryEnabled(auto)
      autoSummaryRef.current = auto
      setLang((settings.settingsLanguage as Lang) || 'zh')
      return true
    } catch (err) {
      console.error('Failed to load model info:', err)
      streamEnabledRef.current = false
      setModelName('')
      autoSummaryRef.current = true
      return false
    }
  }, [])

  const ensureSettings = useCallback(async () => {
    await loadModelInfo()
  }, [loadModelInfo])

  /** 加载截图缩略图 */
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
   * 把"非流式最终结果"或"流式末态错误信息"写回消息列表的指定索引。
   * 流式模式下，已有 delta 拼好的 content；本函数负责合并/覆盖。
   */
  const writeAssistant = useCallback(
    (index: number, content: string, opts: { isError: boolean; stream: boolean }) => {
      setMessages(prev => {
        const next = [...prev]
        const existing = next[index]
        if (!existing) {
          // 非流式：消息可能还没占位（getInitialSummary 非流式分支），直接 push
          next[index] = { role: 'assistant', content }
          return next
        }
        if (opts.stream) {
          // 流式成功：保留已拼接的 content（content 已等于 finalContent）；
          // 流式失败：在已有 content 后追加错误信息
          const merged = opts.isError && existing.content
            ? `${existing.content}\n\n${content}`
            : content
          next[index] = { ...existing, content: merged }
        } else {
          next[index] = { ...existing, content }
        }
        return next
      })
    },
    [],
  )

  /** 获取初始 AI 总结 */
  const getInitialSummary = useCallback(async (id: string) => {
    setInitializing(true)
    setLoading(true)
    await ensureSettings()
    const stream = streamEnabledRef.current
    // 不论流式与否，都先占位一条 assistant，写入逻辑统一走 writeAssistant(0, ...)
    setMessages([{ role: 'assistant', content: '' }])
    if (stream) {
      streamingRef.current = { imageId: id, kind: 'summary', index: 0 }
      setStreamingActive(true)
    }
    try {
      const result = await api.explainGetInitialSummary(id)
      if (imageIdRef.current !== id) return
      const content = result.success ? (result.summary ?? '') : `错误: ${result.error}`
      writeAssistant(0, content, { isError: !result.success, stream })
    } catch (err) {
      if (imageIdRef.current !== id) return
      writeAssistant(0, `错误: ${formatError(err)}`, { isError: true, stream })
    } finally {
      if (streamingRef.current?.imageId === id && streamingRef.current?.kind === 'summary') {
        streamingRef.current = null
      }
      if (imageIdRef.current === id) {
        setLoading(false)
        setInitializing(false)
        setStreamingActive(false)
        inputRef.current?.focus()
      }
    }
  }, [ensureSettings, writeAssistant])

  /** 加载历史记录 */
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
      setStreamingActive(false)
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
      // 紧凑窗口
      resizeWindow(COMPACT_WIDTH, COMPACT_HEIGHT)
      // 根据 autoSummaryEnabled 决定是否自动调模型
      if (autoSummaryRef.current) {
        getInitialSummary(decodedId)
      } else {
        setMessages([])
        setInitializing(false)
        setLoading(false)
        // 让聚焦在小动画后执行，避免初始输入框抖动
        setTimeout(() => inputRef.current?.focus(), 60)
      }
    }

    const parseHash = async () => {
      const hash = window.location.hash
      const params = new URLSearchParams(hash.split('?')[1] || '')
      const id = params.get('imageId')
      if (id) await applyImageId(decodeURIComponent(id))
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
      // done 事件：结束流式 UI 状态，由 finally 块清 streamingRef
      if (payload.done) {
        setStreamingActive(false)
        return
      }
      if (payload.delta) {
        setMessages(prev => {
          if (current.index >= prev.length) return prev
          const next = [...prev]
          const existing = next[current.index]
          if (!existing) return prev
          next[current.index] = { ...existing, content: `${existing.content}${payload.delta}` }
          return next
        })
      }
    }).then((dispose) => {
      unlisten = dispose
    }).catch((err) => {
      console.error('Failed to listen explain stream:', err)
    })
    return () => { unlisten?.() }
  }, [])

  // 消息列表自动滚动到底部
  useEffect(() => { messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' }) }, [messages])

  // 用户消息出现 → 切换到展开模式
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

  // 卸载时清理 copy timeout
  useEffect(() => () => {
    if (copyTimeoutRef.current) clearTimeout(copyTimeoutRef.current)
  }, [])

  /** 发送用户问题 */
  const handleSend = async () => {
    if (!input.trim() || loading || historyMode || !imageId) return
    const userMessage: Message = { role: 'user', content: input }
    const requestImageId = imageId
    const baseIndex = messages.length
    const assistantIndex = baseIndex + 1
    await ensureSettings()
    const stream = streamEnabledRef.current
    setMessages(prev => [...prev, userMessage, { role: 'assistant', content: '' }])
    if (stream) {
      streamingRef.current = { imageId: requestImageId, kind: 'answer', index: assistantIndex }
      setStreamingActive(true)
    }
    setInput('')
    setLoading(true)
    try {
      const result = await api.explainAskQuestion(requestImageId, [...messages, userMessage])
      if (imageIdRef.current !== requestImageId) return
      const content = result.success ? (result.response ?? '') : `错误: ${result.error}`
      writeAssistant(assistantIndex, content, { isError: !result.success, stream })
    } catch (err) {
      if (imageIdRef.current !== requestImageId) return
      writeAssistant(assistantIndex, `错误: ${formatError(err)}`, { isError: true, stream })
    } finally {
      if (streamingRef.current?.imageId === requestImageId && streamingRef.current?.kind === 'answer') {
        streamingRef.current = null
      }
      if (imageIdRef.current === requestImageId) {
        setLoading(false)
        setStreamingActive(false)
      }
    }
  }

  /** 重新生成最后一条助手消息 */
  const handleRegenerate = async () => {
    if (loading || historyMode || !imageId) return
    // 找最后一条 assistant
    const lastIdx = messages.map(m => m.role).lastIndexOf('assistant')
    if (lastIdx < 0) return
    const before = messages.slice(0, lastIdx)
    const prevRole = before[before.length - 1]?.role
    // 决定是初始总结（没有任何用户消息）还是问答（前面是 user）
    const isSummary = !before.some(m => m.role === 'user')
    if (!isSummary && prevRole !== 'user') return

    // 若有正在跑的流，先取消
    if (streamingRef.current) {
      try { await api.explainCancelStream() } catch (e) { console.error(e) }
    }
    const requestImageId = imageId
    const stream = streamEnabledRef.current
    setMessages([...before, { role: 'assistant', content: '' }])
    streamingRef.current = stream
      ? { imageId: requestImageId, kind: isSummary ? 'summary' : 'answer', index: lastIdx }
      : null
    if (stream) setStreamingActive(true)
    setLoading(true)
    try {
      const result = isSummary
        ? await api.explainGetInitialSummary(requestImageId)
        : await api.explainAskQuestion(requestImageId, before)
      if (imageIdRef.current !== requestImageId) return
      const content = isSummary
        ? (result as { success: boolean; summary?: string; error?: string }).success
          ? ((result as { summary?: string }).summary ?? '')
          : `错误: ${(result as { error?: string }).error}`
        : (result as { success: boolean; response?: string; error?: string }).success
          ? ((result as { response?: string }).response ?? '')
          : `错误: ${(result as { error?: string }).error}`
      const ok = (result as { success: boolean }).success
      writeAssistant(lastIdx, content, { isError: !ok, stream })
    } catch (err) {
      if (imageIdRef.current !== requestImageId) return
      writeAssistant(lastIdx, `错误: ${formatError(err)}`, { isError: true, stream })
    } finally {
      if (streamingRef.current?.imageId === requestImageId) {
        streamingRef.current = null
      }
      if (imageIdRef.current === requestImageId) {
        setLoading(false)
        setStreamingActive(false)
      }
    }
  }

  /** 停止流式生成 */
  const handleStop = async () => {
    try {
      await api.explainCancelStream()
    } catch (err) {
      console.error('Failed to cancel stream:', err)
    }
    streamingRef.current = null
    setStreamingActive(false)
    setLoading(false)
  }

  /** 复制消息内容 */
  const handleCopy = async (text: string, index: number) => {
    const ok = await copyToClipboard(text)
    if (!ok) return
    setCopiedIndex(index)
    if (copyTimeoutRef.current) clearTimeout(copyTimeoutRef.current)
    copyTimeoutRef.current = setTimeout(() => setCopiedIndex(null), 2000)
  }

  // 键盘事件
  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); handleSend() }
    else if (e.key === 'Escape') {
      // 流式中先停，再关
      if (streamingActive) {
        e.preventDefault()
        void handleStop()
      } else {
        handleClose()
      }
    }
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
        setStreamingActive(false)
        setIsExpanded(true)
        resizeWindow(EXPANDED_WIDTH, EXPANDED_HEIGHT)
      }
    } catch (err) {
      console.error('Failed to load history record:', err)
    }
  }

  const handleClose = async () => {
    if (streamingActive) {
      try { await api.explainCancelStream() } catch (e) { console.error(e) }
    }
    if (messages.length > 0) await saveToHistory()
    try {
      await api.explainCloseCurrent()
    } catch (err) {
      console.error('Failed to clean up explain image:', err)
    }
    streamingRef.current = null
    api.closeExplainWindow()
  }

  const hasConversation = messages.some(m => m.role === 'user')
  // 紧凑模式下，是否已有助手回复（用于决定是否显示"源码/预览"切换按钮）
  const hasAssistantContent = messages.some(m => m.role === 'assistant' && m.content.trim().length > 0)
  // 最后一条助手消息的 index（用于"重新生成"只在最末助手消息上显示）
  const lastAssistantIdx = messages.map(m => m.role).lastIndexOf('assistant')

  return (
    <div className="h-screen w-screen flex flex-col bg-white dark:bg-neutral-900 overflow-hidden font-sans text-neutral-900 dark:text-neutral-100 rounded-2xl transition-all duration-300 ease-out relative">
      {/* 顶部隐形拖动条（28px 高，覆盖整宽，让用户能从顶部边缘拖动窗口） */}
      <div
        className="absolute top-0 left-0 right-0 h-7 z-10"
        data-tauri-drag-region
      />

      {/* 浮动工具按钮（无独立标题栏，让位于内容） */}
      <div
        className="absolute top-1.5 right-2 z-20 flex items-center gap-0.5"
      >
        {modelName && (
          <span className="flex items-center gap-1 px-1.5 py-0.5 text-[10px] font-medium text-neutral-500 dark:text-neutral-400 bg-neutral-100/80 dark:bg-neutral-800/80 backdrop-blur rounded-md mr-1 max-w-[120px] truncate">
            <Cpu size={10} strokeWidth={2} className="shrink-0" />
            <span className="truncate">{modelName}</span>
          </span>
        )}
        {hasAssistantContent && (
          <button
            onClick={() => setShowRaw(!showRaw)}
            className={`p-1 rounded-md transition-all duration-200 flex items-center gap-0.5 ${showRaw
              ? 'bg-neutral-100 dark:bg-neutral-800 text-neutral-900 dark:text-white'
              : 'text-neutral-400 hover:text-neutral-600 dark:text-neutral-500 dark:hover:text-neutral-300 hover:bg-black/5 dark:hover:bg-white/10'
              }`}
            title={showRaw ? t.explainShowPreview : t.explainShowSource}
          >
            {showRaw ? <Eye size={12} strokeWidth={2} /> : <Code size={12} strokeWidth={2} />}
          </button>
        )}
        <button
          onClick={() => setShowHistory(!showHistory)}
          className={`p-1 rounded-md transition-all duration-200 ${showHistory
            ? 'bg-neutral-100 dark:bg-neutral-800 text-neutral-900 dark:text-white'
            : 'text-neutral-400 hover:text-neutral-600 dark:text-neutral-500 dark:hover:text-neutral-300 hover:bg-black/5 dark:hover:bg-white/10'
            }`}
          title={t.explainHistory}
        >
          <Clock size={13} strokeWidth={2} />
        </button>
        <button
          onClick={handleClose}
          className="p-1 text-neutral-400 hover:text-neutral-600 dark:text-neutral-500 dark:hover:text-neutral-300 rounded-md hover:bg-black/5 dark:hover:bg-white/10 transition-all duration-200"
          title={t.explainHintClose}
        >
          <X size={13} strokeWidth={2} />
        </button>
      </div>

      {/* 历史记录面板（浮在工具按钮下方） */}
      {showHistory && (
        <div className="border-b border-black/5 dark:border-white/5 bg-neutral-50/95 dark:bg-neutral-900/95 backdrop-blur max-h-48 overflow-y-auto custom-scrollbar z-10 shrink-0 mt-8">
          <div className="p-2.5 space-y-1.5">
            <p className="text-[11px] font-medium text-neutral-500 dark:text-neutral-400 px-1">{t.explainHistory}</p>
            {history.length === 0 ? (
              <p className="text-xs text-neutral-400 text-center py-4">{t.explainHistoryEmpty}</p>
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
                        {(record.messages[0]?.content?.slice(0, 80) ?? '')}...
                      </p>
                      <span className="text-[10px] text-neutral-300 dark:text-neutral-600 whitespace-nowrap pt-0.5">
                        {new Date(record.timestamp).toLocaleDateString(lang === 'en' ? 'en-US' : 'zh-CN', { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' })}
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
      <div className="flex-1 flex flex-col overflow-hidden transition-all duration-300 ease-out">
        {/* 紧凑模式：图片 + 总结/空提示 */}
        {!hasConversation && (
          <div className="flex gap-3 px-3 pt-7 pb-2 flex-1 overflow-hidden">
            {imagePreview && (
              <div className="w-16 h-12 rounded-lg overflow-hidden shrink-0 border border-black/5 dark:border-white/5 bg-neutral-100 dark:bg-neutral-800">
                <img src={imagePreview} alt="Screenshot" className="w-full h-full object-cover" />
              </div>
            )}
            <div className="flex-1 min-w-0 overflow-y-auto custom-scrollbar relative group">
              {initializing ? (
                <div className="flex items-center gap-2 py-1">
                  <div className="w-3.5 h-3.5 border-2 border-neutral-300 dark:border-neutral-600 border-t-transparent rounded-full animate-spin" />
                  <span className="text-xs text-neutral-500 dark:text-neutral-400">{t.explainObserving}</span>
                </div>
              ) : messages[0]?.content ? (
                <>
                  <div className="text-[12.5px] leading-[1.55] text-neutral-800 dark:text-neutral-200 pr-6">
                    {showRaw ? (
                      <pre className="whitespace-pre-wrap font-mono text-[11.5px] bg-transparent p-0 m-0 border-none shadow-none text-inherit">
                        {messages[0].content}
                      </pre>
                    ) : (
                      <div className="prose prose-sm dark:prose-invert max-w-none prose-p:my-1 prose-p:leading-[1.55]">
                        <ReactMarkdown remarkPlugins={[remarkMath]} rehypePlugins={[rehypeKatex]}>
                          {messages[0].content}
                        </ReactMarkdown>
                      </div>
                    )}
                  </div>
                  {/* 紧凑模式操作条：复制 + 重新生成（不在加载中且非历史只读） */}
                  {!loading && !historyMode && (
                    <div className="absolute top-0 right-0 flex gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
                      <button
                        onClick={() => handleCopy(messages[0].content, 0)}
                        className="p-1 text-neutral-400 hover:text-neutral-700 dark:text-neutral-500 dark:hover:text-neutral-200 rounded hover:bg-black/5 dark:hover:bg-white/10"
                        title={copiedIndex === 0 ? t.explainCopied : t.explainCopy}
                      >
                        {copiedIndex === 0 ? <Check size={12} strokeWidth={2} /> : <Copy size={12} strokeWidth={2} />}
                      </button>
                      <button
                        onClick={handleRegenerate}
                        className="p-1 text-neutral-400 hover:text-neutral-700 dark:text-neutral-500 dark:hover:text-neutral-200 rounded hover:bg-black/5 dark:hover:bg-white/10"
                        title={t.explainRegenerate}
                      >
                        <RefreshCw size={12} strokeWidth={2} />
                      </button>
                    </div>
                  )}
                  {/* 流式中显示停止按钮 */}
                  {streamingActive && (
                    <div className="absolute top-0 right-0">
                      <button
                        onClick={handleStop}
                        className="p-1 text-neutral-500 hover:text-red-500 rounded hover:bg-black/5 dark:hover:bg-white/10"
                        title={t.explainStop}
                      >
                        <Square size={12} strokeWidth={2.5} fill="currentColor" />
                      </button>
                    </div>
                  )}
                </>
              ) : (
                <div className="flex items-center h-full">
                  <span className="text-xs text-neutral-400 dark:text-neutral-500">
                    {autoSummaryEnabled ? t.explainObserving : t.explainPlaceholderEmpty}
                  </span>
                </div>
              )}
            </div>
          </div>
        )}

        {/* 展开模式：缩略图 + 对话流 */}
        {hasConversation && (
          <>
            {imagePreview && (
              <div className="px-4 pt-7 pb-1 shrink-0">
                <div className="w-14 h-10 rounded-lg overflow-hidden border border-black/5 dark:border-white/5 bg-neutral-100 dark:bg-neutral-800">
                  <img src={imagePreview} alt="Screenshot" className="w-full h-full object-cover" />
                </div>
              </div>
            )}
            <div className={`flex-1 overflow-y-auto px-4 ${imagePreview ? 'pt-3' : 'pt-7'} pb-3 space-y-4 custom-scrollbar`}>
              {messages.map((msg, idx) => (
                <div key={idx} className={`flex ${msg.role === 'user' ? 'justify-end' : 'justify-start'}`}>
                  {msg.role === 'assistant' ? (
                    <div className="max-w-[92%] group relative">
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
                      {/* 助手消息底部操作条 */}
                      {msg.content.trim() && !historyMode && (
                        <div className="flex items-center gap-0.5 mt-1.5 opacity-0 group-hover:opacity-100 transition-opacity">
                          <button
                            onClick={() => handleCopy(msg.content, idx)}
                            className="flex items-center gap-1 px-1.5 py-0.5 text-[10px] text-neutral-400 hover:text-neutral-700 dark:text-neutral-500 dark:hover:text-neutral-200 rounded hover:bg-black/5 dark:hover:bg-white/10"
                            title={copiedIndex === idx ? t.explainCopied : t.explainCopy}
                          >
                            {copiedIndex === idx ? <Check size={11} strokeWidth={2} /> : <Copy size={11} strokeWidth={2} />}
                            <span>{copiedIndex === idx ? t.explainCopied : t.explainCopy}</span>
                          </button>
                          {idx === lastAssistantIdx && !loading && (
                            <button
                              onClick={handleRegenerate}
                              className="flex items-center gap-1 px-1.5 py-0.5 text-[10px] text-neutral-400 hover:text-neutral-700 dark:text-neutral-500 dark:hover:text-neutral-200 rounded hover:bg-black/5 dark:hover:bg-white/10"
                              title={t.explainRegenerate}
                            >
                              <RefreshCw size={11} strokeWidth={2} />
                              <span>{t.explainRegenerate}</span>
                            </button>
                          )}
                        </div>
                      )}
                    </div>
                  ) : (
                    <div className="max-w-[85%] px-3.5 py-2 bg-neutral-100 dark:bg-neutral-800/80 text-neutral-900 dark:text-neutral-100 rounded-2xl rounded-tr-sm text-[13.5px] leading-6">
                      {msg.content}
                    </div>
                  )}
                </div>
              ))}
              {/* 加载中 / 流式打字指示 + 停止按钮 */}
              {loading && !initializing && (
                <div className="flex justify-start items-center gap-2">
                  <div className="px-3.5 py-2.5 bg-neutral-100 dark:bg-neutral-800/80 rounded-2xl rounded-tl-sm flex items-center gap-1">
                    {streamingActive && messages[messages.length - 1]?.content ? (
                      <>
                        <span className="w-1.5 h-1.5 rounded-full bg-neutral-400 animate-pulse" />
                        <span className="w-1.5 h-1.5 rounded-full bg-neutral-400 animate-pulse [animation-delay:0.2s]" />
                        <span className="w-1.5 h-1.5 rounded-full bg-neutral-400 animate-pulse [animation-delay:0.4s]" />
                      </>
                    ) : (
                      <Loader2 className="animate-spin text-neutral-400" size={16} strokeWidth={2} />
                    )}
                  </div>
                  {streamingActive && (
                    <button
                      onClick={handleStop}
                      className="flex items-center gap-1 px-2 py-1 text-[11px] text-neutral-500 hover:text-red-500 dark:text-neutral-400 rounded-md hover:bg-black/5 dark:hover:bg-white/10 transition-colors"
                      title={t.explainStop}
                    >
                      <Square size={11} strokeWidth={2.5} fill="currentColor" />
                      <span>{t.explainStop}</span>
                    </button>
                  )}
                </div>
              )}
              <div ref={messagesEndRef} />
            </div>
          </>
        )}
      </div>

      {/* 输入区域 */}
      <div className="px-3 py-2 bg-white dark:bg-neutral-900 border-t border-black/5 dark:border-white/5 shrink-0">
        <div className="flex gap-2 items-end">
          <textarea
            ref={inputRef}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={
              historyMode
                ? t.explainPlaceholderReadonly
                : (!autoSummaryEnabled && !hasAssistantContent && !hasConversation
                    ? t.explainPlaceholderEmpty
                    : t.explainPlaceholder)
            }
            disabled={loading || initializing || historyMode || !imageId}
            className="flex-1 px-3 py-2 bg-neutral-100 dark:bg-neutral-800/50 border border-transparent focus:bg-white dark:focus:bg-neutral-800 focus:border-neutral-200 dark:focus:border-neutral-700 rounded-xl resize-none focus:outline-none text-[13px] text-neutral-900 dark:text-white placeholder-neutral-400 dark:placeholder-neutral-500 transition-all duration-200"
            rows={1}
            style={{ minHeight: '36px', maxHeight: '120px' }}
            data-tauri-drag-region="false"
          />
          <button
            onClick={handleSend}
            disabled={!input.trim() || loading || initializing || historyMode || !imageId}
            className="p-2 bg-neutral-900 dark:bg-white text-white dark:text-neutral-900 rounded-xl hover:bg-neutral-700 dark:hover:bg-neutral-200 disabled:opacity-30 disabled:cursor-not-allowed transition-all duration-200 active:scale-95 shrink-0"
            data-tauri-drag-region="false"
          >
            <Send size={15} strokeWidth={2} />
          </button>
        </div>
        <div className="flex justify-between items-center mt-1 px-1">
          <p className="text-[10px] text-neutral-400 dark:text-neutral-500">{t.explainHintSend}</p>
          <p className="text-[10px] text-neutral-400 dark:text-neutral-500">{t.explainHintClose}</p>
        </div>
      </div>
    </div>
  )
}
