import { useEffect, useState } from 'react'
import { dockApi } from '../dock/api'

/** 弹出窗口标题栏工作区面板使用的有效工作目录。 */
export function usePopoutWorkdir(conversationId: string, projectId: string | null, runtimeKey: string, worktreePath?: string | null) {
  const [workdir, setWorkdir] = useState('')
  useEffect(() => {
    if (worktreePath) return
    let cancelled = false
    dockApi.resolveCwd(conversationId, projectId).then((cwd) => {
      if (!cancelled) setWorkdir(cwd)
    }).catch(() => {
      if (!cancelled) setWorkdir('')
    })
    return () => {
      cancelled = true
    }
  }, [conversationId, projectId, runtimeKey, worktreePath])
  return worktreePath || workdir
}
