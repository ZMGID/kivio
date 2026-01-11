import { useState, useEffect, useRef, type CSSProperties } from 'react'
import { Send, X, Loader2, Image, Clock, ChevronDown, ChevronRight, Cpu } from 'lucide-react'

interface Message {
  role: 'user' | 'assistant'
  content: string
}

type AppRegionStyle = CSSProperties & { WebkitAppRegion?: 'drag' | 'no-drag' }
const dragStyle: AppRegionStyle = { WebkitAppRegion: 'drag' }
const noDragStyle: AppRegionStyle = { WebkitAppRegion: 'no-drag' }

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
  const [modelName, setModelName] = useState('')
  const messagesEndRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLTextAreaElement>(null)

  useEffect(() => {
    const hash = window.location.hash
    const params = new URLSearchParams(hash.split('?')[1] || '')
    const id = params.get('imageId')
    if (id) {
      const decoded = decodeURIComponent(id)
      setImageId(decoded)
      loadImage(decoded)
      getInitialSummary(decoded)
    }
    loadHistory()
    loadModelInfo()
  }, [])

  const loadModelInfo = async () => {
    if (!window.api) return
    const settings = await window.api.getSettings()
    const model = settings.screenshotExplain?.model
    if (model) setModelName(model.modelName || (model.provider === 'glm' ? 'GLM-4V' : 'GPT-4V'))
  }

  const loadImage = async (id: string) => {
    if (!window.api) return
    const result = await window.api.explainReadImage(id)
    if (result.success) setImagePreview(result.data ?? '')
  }

  const getInitialSummary = async (id: string) => {
    setInitializing(true)
    setLoading(true)
    if (window.api) {
      const result = await window.api.explainGetInitialSummary(id)
      setMessages([{ role: 'assistant', content: result.success ? (result.summary ?? '') : `错误: ${result.error}` }])
    }
    setLoading(false)
    setInitializing(false)
    inputRef.current?.focus()
  }

  useEffect(() => { messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' }) }, [messages])

  const handleSend = async () => {
    if (!input.trim() || loading) return
    const userMessage: Message = { role: 'user', content: input }
    setMessages(prev => [...prev, userMessage])
    setInput('')
    setLoading(true)
    if (window.api) {
      const result = await window.api.explainAskQuestion(imageId, [...messages, userMessage])
      setMessages(prev => [...prev, { role: 'assistant', content: result.success ? (result.response ?? '') : `错误: ${result.error}` }])
    }
    setLoading(false)
  }

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); handleSend() }
    else if (e.key === 'Escape') handleClose()
  }

  const loadHistory = async () => {
    if (!window.api) return
    const result = await window.api.explainGetHistory()
    if (result.success) setHistory(result.history || [])
  }

  const saveToHistory = async () => {
    if (window.api && imageId && messages.length > 0) {
      await window.api.explainSaveHistory(messages)
      await loadHistory()
    }
  }

  const loadHistoryRecord = async (historyId: string) => {
    if (!window.api) return
    const result = await window.api.explainLoadHistory(historyId)
    if (result.success && result.record) {
      setMessages(result.record.messages)
      setShowHistory(false)
      setImagePreview('')
    }
  }

  const handleClose = async () => {
    if (messages.length > 0) await saveToHistory()
    window.api?.closeExplainWindow()
  }

  return (
    <div className="h-screen w-screen flex flex-col bg-white/95 dark:bg-neutral-900/95 backdrop-blur-2xl overflow-hidden font-sans text-neutral-900 dark:text-neutral-100">
      {/* 顶部栏 */}
      <div className="flex items-center justify-between px-4 py-3.5 border-b border-black/5 dark:border-white/5 pl-20 select-none" style={dragStyle}>
        <div className="flex items-center gap-2.5">
          <h1 className="text-[13px] font-semibold text-neutral-700 dark:text-neutral-200 tracking-wide">截图解释</h1>
          {modelName && (
            <span className="flex items-center gap-1 px-2 py-0.5 text-[10px] font-medium text-neutral-500 dark:text-neutral-400 bg-neutral-100 dark:bg-neutral-800 rounded-full border border-neutral-200/50 dark:border-neutral-700/50">
              <Cpu size={10} strokeWidth={2} />{modelName}
            </span>
          )}
        </div>
        <div className="flex items-center gap-1" style={noDragStyle}>
          <button
            onClick={() => setShowHistory(!showHistory)}
            className={`p-1.5 rounded-lg transition-all duration-200 ${
              showHistory
                ? 'bg-neutral-100 dark:bg-neutral-800 text-neutral-900 dark:text-white'
                : 'text-neutral-400 hover:text-neutral-600 dark:text-neutral-500 dark:hover:text-neutral-300 hover:bg-black/5 dark:hover:bg-white/10'
            }`}
            title="历史记录"
          >
            <Clock size={16} strokeWidth={2} />
          </button>
          <button
            onClick={handleClose}
            className="p-1.5 text-neutral-400 hover:text-neutral-600 dark:text-neutral-500 dark:hover:text-neutral-300 rounded-lg hover:bg-black/5 dark:hover:bg-white/10 transition-all duration-200"
          >
            <X size={16} strokeWidth={2} />
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
                        {record.messages[0]?.content.slice(0, 60)}...
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

      {/* 图片预览栏 */}
      {imagePreview && (
        <div className="border-b border-black/5 dark:border-white/5 bg-white/50 dark:bg-neutral-900/50 backdrop-blur-sm">
          <button
            onClick={() => setShowImage(!showImage)}
            className="w-full px-4 py-2 flex items-center gap-2 text-xs font-medium text-neutral-500 dark:text-neutral-400 hover:text-neutral-700 dark:hover:text-neutral-200 transition-colors"
            style={noDragStyle}
          >
            {showImage ? <ChevronDown size={14} strokeWidth={2} /> : <ChevronRight size={14} strokeWidth={2} />}
            <Image size={14} strokeWidth={2} />
            <span>{showImage ? '隐藏截图' : '显示截图'}</span>
          </button>
          {showImage && (
            <div className="px-4 pb-3 animate-in slide-in-from-top-2 duration-200">
              <div className="rounded-xl overflow-hidden border border-black/5 dark:border-white/5 shadow-sm bg-neutral-100/50 dark:bg-neutral-800/50">
                <img src={imagePreview} alt="Screenshot" className="max-w-full max-h-48 mx-auto object-contain" />
              </div>
            </div>
          )}
        </div>
      )}

      {/* 消息列表 */}
      <div className="flex-1 overflow-y-auto p-4 space-y-5" style={noDragStyle}>
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
              className={`max-w-[85%] px-4 py-2.5 text-[13.5px] leading-relaxed shadow-sm ${
                msg.role === 'user'
                  ? 'bg-neutral-900 dark:bg-white text-white dark:text-neutral-900 rounded-2xl rounded-tr-sm'
                  : 'bg-white dark:bg-neutral-800 border border-black/5 dark:border-white/5 text-neutral-700 dark:text-neutral-200 rounded-2xl rounded-tl-sm'
              }`}
            >
              <p className="whitespace-pre-wrap">{msg.content}</p>
            </div>
          </div>
        ))}

        {loading && !initializing && (
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
            placeholder="输入问题..."
            disabled={loading || initializing}
            className="flex-1 px-4 py-3 bg-neutral-100 dark:bg-neutral-800/50 border-transparent focus:bg-white dark:focus:bg-neutral-800 border focus:border-neutral-200 dark:focus:border-neutral-700 rounded-xl resize-none focus:outline-none text-[13.5px] text-neutral-900 dark:text-white placeholder-neutral-400 dark:placeholder-neutral-500 transition-all duration-200 shadow-inner"
            rows={1}
            style={{ minHeight: '44px', maxHeight: '120px' }}
          />
          <button
            onClick={handleSend}
            disabled={!input.trim() || loading || initializing}
            className="p-3 bg-neutral-900 dark:bg-white text-white dark:text-neutral-900 rounded-xl hover:bg-neutral-700 dark:hover:bg-neutral-200 disabled:opacity-30 disabled:cursor-not-allowed transition-all duration-200 shadow-sm active:scale-95"
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
