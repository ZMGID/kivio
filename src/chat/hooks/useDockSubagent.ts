import { useCallback, useEffect, useState } from 'react'
import { onDockSubagentRequest, type DockSubagent } from '../dock/dockPreview'

export function useDockSubagent(conversationId: string | null, onOpen?: () => void) {
  const [selection, setSelection] = useState<DockSubagent | null>(null)
  useEffect(() => { setSelection(null) }, [conversationId])
  useEffect(() => onDockSubagentRequest(target => {
    if (target.conversationId !== conversationId) return
    setSelection(target)
    onOpen?.()
  }), [conversationId, onOpen])
  const close = useCallback(() => setSelection(null), [])
  return { selection: selection?.conversationId === conversationId ? selection : null, close }
}
