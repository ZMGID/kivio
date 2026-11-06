import { useEffect, useState } from 'react'
import { subagentsApi } from '../api/subagents'
import { api } from '../api/tauri'
import { notifyError } from '../components/appNoticeStore'
import type { Lang } from '../components/i18n'
import { chatApi } from './api'

export function useBackgroundTaskWait(conversationId: string | null, lang: Lang) {
  const [wait, setWait] = useState<{ conversationId: string; stop: () => Promise<void> }>()
  useEffect(() => {
    setWait(undefined)
    if (!conversationId) return
    const id = conversationId
    let disposed = false
    let revision = 0
    let stopping = false
    async function refresh() {
      const current = ++revision
      const pending = await subagentsApi.waiting(id).catch(() => false)
      if (!disposed && revision === current) setWait(pending ? { conversationId: id, stop } : undefined)
    }
    async function stop() {
      if (disposed || stopping) return
      stopping = true
      try {
        await chatApi.cancelStream(id, 'backgroundWait')
      } catch {
        if (!disposed) void notifyError(lang === 'zh' ? '无法停止等待任务结果' : 'Could not stop waiting for task results')
      } finally {
        if (!disposed) await refresh()
        stopping = false
      }
    }
    const subscription = api.onChatSubagentActivity(changed => {
      if (changed === id) void refresh()
    })
    const refreshAfterSubscribe = () => {
      if (!disposed) void refresh()
    }
    void subscription.then(refreshAfterSubscribe, refreshAfterSubscribe)
    return () => {
      disposed = true
      void subscription.then(unlisten => unlisten(), () => {})
    }
  }, [conversationId, lang])
  return wait?.conversationId === conversationId ? wait.stop : undefined
}
