import { useCallback, useState } from 'react'

/** Hold the composer until its window has applied the selection for the next message. */
export function useAgentSelection(conversationId: string | null) {
  const [requests, setRequests] = useState<{ id: string }[]>([])
  const save = useCallback(async (id: string, change: () => Promise<void>) => {
    const request = { id }
    setRequests(previous => [...previous, request])
    try {
      await change()
    } finally {
      setRequests(previous => previous.filter(item => item !== request))
    }
  }, [])
  return { pending: requests.some(request => request.id === conversationId), save }
}
