import { useEffect, useRef, useState } from 'react'
import { ArrowRight } from 'lucide-react'
import { api } from '../api/tauri'
import type { Lang } from '../components/i18n'
import type { SubagentIrcMessage } from '../generated/chatProtocol'
import { AgentAvatar } from './AgentAvatar'
import { useTauriEvent } from './hooks/useTauriEvent'

type Route = Pick<SubagentIrcMessage, 'id' | 'fromAgent' | 'toAgent'>

const MAX_PENDING_ROUTES = 12
const MIN_ROUTE_MS = 300
const SINGLE_ROUTE_MS = 1500

export function IrcStatus({ conversationId, lang }: { conversationId: string; lang: Lang }) {
  const [routes, setRoutes] = useState<Route[]>([])
  const shown = useRef<{ route: Route; startedAt: number } | null>(null)
  const route = routes[0]
  const routeCount = routes.length
  useTauriEvent(api.onChatSubagentIrc, ({ id, conversationId: root, fromAgent, toAgent, observation }) => {
    if (root !== conversationId || !observation) return
    setRoutes(queue => {
      if (queue.some(item => item.id === id)) return queue
      const next = { id, fromAgent, toAgent }
      const current = queue[0]
      if (current && queue.length > MAX_PENDING_ROUTES) return [current, ...queue.slice(2), next]
      return [...queue, next]
    })
  }, [conversationId])
  useEffect(() => {
    if (!route) {
      shown.current = null
      return
    }
    const now = performance.now()
    if (shown.current?.route !== route) shown.current = { route, startedAt: now }
    const duration = Math.max(MIN_ROUTE_MS, SINGLE_ROUTE_MS / routeCount)
    const remaining = Math.max(0, duration - (now - shown.current.startedAt))
    const timer = window.setTimeout(() => setRoutes(queue => queue.slice(1)), remaining)
    return () => window.clearTimeout(timer)
  }, [route, routeCount])
  if (!route) return null
  const label = `${lang === 'zh' ? '代理通信' : 'Agent communication'} · ${route.fromAgent} → ${route.toAgent}`
  return <span className="inline-flex shrink-0 items-center gap-1.5 text-[var(--text-muted)]"
    role="status" aria-label={label} title={label}>
    <AgentAvatar agentId={route.fromAgent} />
    <ArrowRight size={12} aria-hidden="true" />
    <AgentAvatar agentId={route.toAgent} />
  </span>
}
