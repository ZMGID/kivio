import { useCallback, useEffect, useRef, useState } from 'react'
import { Sidebar } from './Sidebar'
import { MessageList } from './MessageList'
import { InputBar } from './InputBar'
import { ModelSelector } from './ModelSelector'
import { chatApi } from './api'
import { api } from '../api/tauri'
import type { Conversation } from './types'

interface ChatProps {
  onOpenSettings: () => void
}

export default function Chat({ onOpenSettings }: ChatProps) {
  const [currentConversation, setCurrentConversation] = useState<Conversation | null>(null)
  const sidebarCollapsed = false
  const [streaming, setStreaming] = useState(false)
  const [streamingContent, setStreamingContent] = useState('')
  const [streamingReasoning, setStreamingReasoning] = useState('')
  const [streamError, setStreamError] = useState('')
  const [sidebarRefreshKey, setSidebarRefreshKey] = useState(0)
  const currentConversationIdRef = useRef<string | null>(null)

  const getRouteConversationId = useCallback(() => {
    const hash = window.location.hash.replace('#', '').split('?')[0]
    if (!hash.startsWith('chat/')) return null
    return decodeURIComponent(hash.slice('chat/'.length))
  }, [])

  const syncRoute = useCallback((conversationId: string | null) => {
    const nextHash = conversationId ? `#chat/${encodeURIComponent(conversationId)}` : '#chat'
    if (window.location.hash !== nextHash) {
      window.location.hash = nextHash
    }
  }, [])

  const refreshSidebar = useCallback(() => {
    setSidebarRefreshKey((key) => key + 1)
  }, [])

  // 重新加载对话
  const reloadConversation = useCallback(async (conversationId: string) => {
    try {
      const conv = await chatApi.getConversation(conversationId)
      setCurrentConversation(conv)
    } catch (err) {
      console.error('Failed to reload conversation:', err)
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '对话加载失败')
    }
  }, [])

  // 监听流式响应事件
  useEffect(() => {
    let unlisten: (() => void) | undefined

    const setupListener = async () => {
      unlisten = await api.onChatStream((payload) => {
        if (payload.reasoningDelta) {
          setStreamingReasoning((prev) => prev + payload.reasoningDelta)
        }
        if (payload.delta) {
          setStreamingContent((prev) => prev + payload.delta)
        }
        if (payload.done) {
          setStreaming(false)
          setStreamingContent('')
          setStreamingReasoning('')
          if (payload.reason === 'error') {
            setStreamError('回复生成失败，请稍后重试。')
          }
          const conversationId = currentConversationIdRef.current
          if (conversationId && payload.reason !== 'cancelled') {
            void reloadConversation(conversationId)
            refreshSidebar()
          }
        }
      })
    }

    setupListener()
    return () => {
      unlisten?.()
    }
  }, [refreshSidebar, reloadConversation])

  useEffect(() => {
    currentConversationIdRef.current = currentConversation?.id ?? null
  }, [currentConversation?.id])

  useEffect(() => {
    const loadFromRoute = () => {
      const conversationId = getRouteConversationId()
      if (!conversationId) {
        setCurrentConversation(null)
        return
      }
      void reloadConversation(conversationId)
    }
    loadFromRoute()
    window.addEventListener('hashchange', loadFromRoute)
    return () => window.removeEventListener('hashchange', loadFromRoute)
  }, [getRouteConversationId, reloadConversation])

  // 选择对话
  const handleSelectConversation = async (conversationId: string) => {
    try {
      const conv = await chatApi.getConversation(conversationId)
      setCurrentConversation(conv)
      syncRoute(conversationId)
      setStreamError('')
    } catch (err) {
      console.error('Failed to load conversation:', err)
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '对话加载失败')
    }
  }

  // 新建对话
  const handleNewConversation = async () => {
    try {
      const conv = await chatApi.createConversation()
      setCurrentConversation(conv)
      syncRoute(conv.id)
      refreshSidebar()
      setStreamError('')
    } catch (err) {
      console.error('Failed to create conversation:', err)
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '创建对话失败')
    }
  }

  // 发送消息
  const handleSendMessage = async (content: string) => {
    if (!currentConversation || streaming) return

    setStreaming(true)
    setStreamingContent('')
    setStreamingReasoning('')
    setStreamError('')

    try {
      const updatedConv = await chatApi.sendMessage(currentConversation.id, content)
      setCurrentConversation(updatedConv)
      refreshSidebar()
    } catch (err) {
      console.error('Failed to send message:', err)
      setStreaming(false)
      setStreamingContent('')
      setStreamingReasoning('')
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '发送失败')
    }
  }

  // 切换模型
  const handleModelChange = async (providerId: string, model: string) => {
    if (!currentConversation) return

    try {
      const updatedConv = await chatApi.updateConversation(currentConversation.id, {
        providerId,
        model,
      })
      setCurrentConversation(updatedConv)
      refreshSidebar()
    } catch (err) {
      console.error('Failed to change model:', err)
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '模型切换失败')
    }
  }

  // 触发截图
  const handleTriggerScreenshot = async () => {
    try {
      await api.lensRequest()
    } catch (err) {
      console.error('Failed to trigger screenshot:', err)
    }
  }

  return (
    <div className="flex h-screen bg-white dark:bg-neutral-900">
      {/* 左侧边栏 */}
      <Sidebar
        currentConversationId={currentConversation?.id}
        onSelectConversation={handleSelectConversation}
        onNewConversation={handleNewConversation}
        onOpenSettings={onOpenSettings}
        collapsed={sidebarCollapsed}
        refreshKey={sidebarRefreshKey}
      />

      {/* 主内容区 */}
      <div className="flex-1 flex flex-col">
        {/* 顶部栏 */}
        {currentConversation && (
          <div className="h-14 border-b border-neutral-200 dark:border-neutral-800 flex items-center justify-between px-4">
            {/* 对话标题 */}
            <div className="text-sm font-medium text-neutral-700 dark:text-neutral-300">
              {currentConversation.title}
            </div>

            {/* 模型选择器 */}
            <ModelSelector
              currentProviderId={currentConversation.provider_id}
              currentModel={currentConversation.model}
              onModelChange={handleModelChange}
            />
          </div>
        )}

        {/* 消息列表 */}
        <MessageList
          messages={currentConversation?.messages || []}
          streaming={streaming}
          streamingContent={streamingContent}
          streamingReasoning={streamingReasoning}
          error={streamError}
        />

        {/* 输入栏 */}
        {currentConversation && (
          <InputBar
            onSend={handleSendMessage}
            disabled={streaming}
            onTriggerScreenshot={handleTriggerScreenshot}
          />
        )}

        {/* 空状态（无对话选中） */}
        {!currentConversation && (
          <div className="flex-1 flex items-center justify-center">
            <div className="text-center">
              <h2 className="text-3xl font-medium text-neutral-900 dark:text-neutral-100 mb-4">
                今天我能为您做些什么？
              </h2>
              <button
                onClick={handleNewConversation}
                className="px-6 py-3 bg-blue-500 hover:bg-blue-600 text-white rounded-xl font-medium transition-colors"
              >
                开始新对话
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
