import { useCallback, useEffect, useRef, useState } from 'react'
import { api, type ChatMemoryLayerContent, type ChatMemoryState } from '../api/tauri'
import { MEMORY_L1_MAX_BYTES, utf8ByteLength, type MemoryLayerKey } from './memoryLayers'
import type { Lang } from '../components/i18n'

export interface SettingsMemoryPort {
  get(): Promise<ChatMemoryState>
  save(layer: MemoryLayerKey, content: string): Promise<ChatMemoryLayerContent>
  openFolder(): Promise<{ success: boolean; path?: string | null; error?: string | null }>
}

const backendMemoryPort: SettingsMemoryPort = {
  get: api.chatMemoryGet,
  save: api.chatMemorySave,
  openFolder: api.chatMemoryOpenFolder,
}

type View = {
  drafts: Record<MemoryLayerKey, string>
  snapshots: Record<MemoryLayerKey, string>
  dir: string
  loading: boolean
  savingLayer: MemoryLayerKey | null
  error: string
  success: string
}

const initialView = (): View => ({
  drafts: { l1: '', l2: '' }, snapshots: { l1: '', l2: '' },
  dir: '', loading: false, savingLayer: null, error: '', success: '',
})

/** Owns the two memory-file drafts, refresh/save races and open-folder feedback. */
export function useSettingsMemoryEditor(
  port: SettingsMemoryPort = backendMemoryPort,
  lang: Lang = 'zh',
  active = false,
) {
  const [view, setView] = useState<View>(initialView)
  const viewRef = useRef(view)
  const refreshSequence = useRef(0)
  const saving = useRef(false)
  const live = useRef(true)
  const commit = useCallback((update: (current: View) => View) => {
    if (!live.current) return
    const next = update(viewRef.current)
    viewRef.current = next
    setView(next)
  }, [])

  const edit = useCallback((layer: MemoryLayerKey, value: string) => {
    commit((current) => ({
      ...current, drafts: { ...current.drafts, [layer]: value }, success: '', error: '',
    }))
  }, [commit])

  const refresh = useCallback(async () => {
    const sequence = ++refreshSequence.current
    commit((current) => ({ ...current, loading: true, error: '' }))
    try {
      const result = await port.get()
      if (sequence !== refreshSequence.current) return
      const snapshots = { l1: result.l1.content, l2: result.l2.content }
      commit((current) => ({
        ...current,
        snapshots,
        drafts: {
          l1: current.drafts.l1 === current.snapshots.l1 ? snapshots.l1 : current.drafts.l1,
          l2: current.drafts.l2 === current.snapshots.l2 ? snapshots.l2 : current.drafts.l2,
        },
        dir: result.dir,
        success: '',
      }))
    } catch (error) {
      if (sequence === refreshSequence.current) {
        commit((current) => ({ ...current, error: error instanceof Error ? error.message : String(error) }))
      }
    } finally {
      if (sequence === refreshSequence.current) commit((current) => ({ ...current, loading: false }))
    }
  }, [commit, port])

  const save = useCallback(async (layer: MemoryLayerKey) => {
    if (saving.current) return
    const submitted = viewRef.current.drafts[layer]
    if (layer === 'l1' && utf8ByteLength(submitted) > MEMORY_L1_MAX_BYTES) {
      commit((current) => ({
        ...current,
        error: lang === 'zh'
          ? `L1 超过 ${MEMORY_L1_MAX_BYTES} 字节，请先精简或归档到 L2。`
          : `L1 exceeds ${MEMORY_L1_MAX_BYTES} bytes. Shorten it or archive details into L2.`,
      }))
      return
    }
    saving.current = true
    // A read started before this save must not restore a stale baseline later.
    refreshSequence.current += 1
    commit((current) => ({ ...current, savingLayer: layer, loading: false, error: '', success: '' }))
    try {
      const saved = await port.save(layer, submitted)
      commit((current) => ({
        ...current,
        drafts: current.drafts[layer] === submitted
          ? { ...current.drafts, [layer]: saved.content }
          : current.drafts,
        snapshots: { ...current.snapshots, [layer]: saved.content },
        success: lang === 'zh' ? `${layer.toUpperCase()} 已保存` : `${layer.toUpperCase()} saved`,
      }))
    } catch (error) {
      commit((current) => ({ ...current, error: error instanceof Error ? error.message : String(error) }))
    } finally {
      saving.current = false
      commit((current) => ({ ...current, savingLayer: null }))
    }
  }, [commit, lang, port])

  const openFolder = useCallback(async () => {
    commit((current) => ({ ...current, error: '' }))
    try {
      const result = await port.openFolder()
      if (!result.success) {
        commit((current) => ({
          ...current,
          error: result.error || (lang === 'zh' ? '打开记忆文件夹失败' : 'Failed to open memory folder'),
        }))
      } else if (result.path) {
        commit((current) => ({ ...current, dir: result.path! }))
      }
    } catch (error) {
      commit((current) => ({ ...current, error: error instanceof Error ? error.message : String(error) }))
    }
  }, [commit, lang, port])

  useEffect(() => {
    if (active) void refresh()
  }, [active, refresh])

  useEffect(() => {
    live.current = true
    return () => {
      live.current = false
      refreshSequence.current += 1
    }
  }, [])

  return { view, edit, refresh, save, openFolder }
}
