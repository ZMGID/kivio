import { useCallback, useEffect, useState } from 'react'
import {
  externalCliSettingsApi,
  type ExternalCliAccount,
} from '../../api/externalCliSettings'

/** 读账号要拉起一次 CLI，同一次会话内每个 CLI 只读一遍，除非用户手动刷新。 */
const cache = new Map<string, ExternalCliAccount>()

export function useCliAccount(agentId: string | null) {
  const [account, setAccount] = useState<ExternalCliAccount | null>(
    agentId ? cache.get(agentId) ?? null : null,
  )
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const load = useCallback(
    async (force: boolean, isCurrent: () => boolean = () => true) => {
      if (!agentId) return
      // 命中缓存时初始 state 已经是它了（详情按 CLI id 重新挂载）。
      if (!force && cache.has(agentId)) return
      setLoading(true)
      setError(null)
      try {
        const next = await externalCliSettingsApi.getExternalCliAccount(agentId)
        cache.set(agentId, next)
        if (isCurrent()) setAccount(next)
      } catch (err) {
        if (isCurrent()) setError(err instanceof Error ? err.message : String(err))
      } finally {
        if (isCurrent()) setLoading(false)
      }
    },
    [agentId],
  )

  useEffect(() => {
    let current = true
    void load(false, () => current)
    return () => {
      current = false
    }
  }, [load])

  return { account, loading, error, refresh: () => load(true) }
}
