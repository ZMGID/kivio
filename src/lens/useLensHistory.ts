import { useCallback, useEffect, useRef, useState } from 'react'
import { api } from '../api/tauri'
import { HISTORY_MAX, loadHistoryFromStorage, saveHistoryToStorage } from './history'
import type { HistoryItem } from './types'

export function useLensHistory() {
  const [items, setItems] = useState<HistoryItem[]>(loadHistoryFromStorage)
  const previousIds = useRef(new Set(items.map((item) => item.id)))

  const upsert = useCallback((item: HistoryItem) => {
    setItems((current) => [
      item,
      ...current.filter((candidate) => candidate.id !== item.id),
    ].slice(0, HISTORY_MAX))
  }, [])

  useEffect(() => {
    saveHistoryToStorage(items)
    const currentIds = new Set(items.map((item) => item.id))
    for (const id of previousIds.current) {
      if (!currentIds.has(id)) {
        void api.lensDeleteHistoryImage(id).catch((error) => {
          console.error('[lens-history] delete failed:', error)
        })
      }
    }
    previousIds.current = currentIds
  }, [items])

  return { items, upsert }
}
