import { useCallback, useEffect, useRef, useState } from 'react'
import { api } from '../api/tauri'
import { HISTORY_MAX, HISTORY_THUMB_SIZE, loadHistoryFromStorage, makeThumbnail, saveHistoryToStorage } from './history'
import type { HistoryItem } from './types'

export function useLensHistory() {
  const [items, setItems] = useState<HistoryItem[]>(loadHistoryFromStorage)
  const previousIds = useRef(new Set(items.map((item) => item.id)))
  const recordingVersions = useRef(new Map<string, number>())
  const recordingSequence = useRef(0)
  const imageCommits = useRef(new Map<string, Promise<unknown>>())
  const lifetime = useRef(0)

  const upsert = useCallback((item: HistoryItem) => {
    setItems((current) => [
      item,
      ...current.filter((candidate) => candidate.id !== item.id),
    ].slice(0, HISTORY_MAX))
  }, [])

  /** A completed turn is a durable snapshot, independent of the currently displayed opening.
   * A later invoke may enrich the same turn after `done`; only its newest snapshot wins. */
  const recordCompleted = useCallback(async (item: HistoryItem, imageId: string) => {
    const version = ++recordingSequence.current
    recordingVersions.current.set(item.id, version)
    const generation = lifetime.current
    try {
      let commit = imageCommits.current.get(imageId)
      if (imageId && !commit) {
        commit = api.lensCommitImageToHistory(imageId)
        imageCommits.current.set(imageId, commit)
        void commit.finally(() => {
          if (imageCommits.current.get(imageId) === commit) imageCommits.current.delete(imageId)
        }).catch(() => undefined)
      }
      // Start reading the preview while the visible opening still owns its Blob URL.
      // Publication waits for both the thumbnail and durable image persistence.
      const [, thumbnail] = await Promise.all([
        commit,
        item.imagePreview ? makeThumbnail(item.imagePreview, HISTORY_THUMB_SIZE) : '',
      ])
      if (generation !== lifetime.current || recordingVersions.current.get(item.id) !== version) return
      upsert({ ...item, imagePreview: thumbnail })
    } catch (error) {
      console.error('[lens-history] commit failed:', error)
    } finally {
      if (recordingVersions.current.get(item.id) === version) recordingVersions.current.delete(item.id)
    }
  }, [upsert])

  useEffect(() => () => { lifetime.current += 1 }, [])

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

  return { items, upsert, recordCompleted }
}
