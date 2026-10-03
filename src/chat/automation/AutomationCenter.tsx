import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { api, isTauriRuntime } from '../../api/tauri'
import { useT, useLang } from '../../components/i18n'
import { automationHash, getRouteAutomationId, setHash } from '../chatRoutes'
import { automationApi } from './api'
import { AutomationEditor } from './AutomationEditor'
import { AutomationList } from './AutomationList'
import { createBlankAutomation } from './graph'
import type { Automation, AutomationMeta } from '../../api/automationContracts'
import { Button } from '../../components/Button'
import { confirmDialog } from '../../components/dialogQueue'

function clearTimeoutRef(ref: { current: ReturnType<typeof setTimeout> | null }) {
  if (ref.current == null) return
  window.clearTimeout(ref.current)
  ref.current = null
}

export function AutomationCenter({ items, loading, listError, onReload, renderList }: {
  items: AutomationMeta[]
  loading: boolean
  listError: string
  onReload: () => Promise<void>
  renderList: (body: ReactNode, actions: { onCreate: () => void; onImport: () => void }) => ReactNode
}) {
  const t = useT()
  const english = useLang() === 'en'
  const [saveState, setSaveState] = useState<'saved' | 'pending' | 'saving' | 'error'>('saved')
  const [error, setError] = useState('')
  const [editing, setEditing] = useState<Automation | null>(null)
  const [canvasEpoch, setCanvasEpoch] = useState(0)
  const [remoteHint, setRemoteHint] = useState('')
  const saveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const editingRef = useRef<Automation | null>(null)
  const lastSelfUpdatedAtRef = useRef('')
  const selfSaveInFlightRef = useRef(0)
  const dirtyRef = useRef(false)
  const saveQueueRef = useRef<Promise<void>>(Promise.resolve())
  const navigationRef = useRef(0)
  editingRef.current = editing

  // Autosave, execution, export and navigation all await the same ordered writer.
  const flushSave = useCallback((): Promise<void> => {
    clearTimeoutRef(saveTimerRef)
    const operation = saveQueueRef.current.catch(() => {}).then(async () => {
      while (dirtyRef.current && editingRef.current && isTauriRuntime()) {
        const draft = editingRef.current
        setSaveState('saving')
        selfSaveInFlightRef.current += 1
        try {
          const saved = await automationApi.save(draft)
          lastSelfUpdatedAtRef.current = saved.updatedAt
          if (editingRef.current === draft) {
            dirtyRef.current = false
            editingRef.current = saved
            setEditing(saved)
            setSaveState('saved')
            setError('')
          }
        } catch (err) {
          setSaveState('error')
          setError(err instanceof Error ? err.message : String(err))
          throw err
        } finally {
          selfSaveInFlightRef.current -= 1
        }
      }
    })
    saveQueueRef.current = operation
    return operation
  }, [])

  useEffect(() => () => {
    clearTimeoutRef(saveTimerRef)
    void flushSave().catch(() => {})
  }, [flushSave])

  const loadList = useCallback(async () => {
    try { await onReload() }
    catch (err) { setError(err instanceof Error ? err.message : String(err)) }
  }, [onReload])

  useEffect(() => {
    if (!isTauriRuntime()) return
    let cancelled = false
    let unlisten: (() => void) | undefined
    void api.onAutomationChanged((event) => {
      if (cancelled) return
      const current = editingRef.current
      if (!current || event.id !== current.id) return
      if (event.kind === 'deleted') {
        clearTimeoutRef(saveTimerRef)
        dirtyRef.current = false
        editingRef.current = null
        setRemoteHint('')
        setEditing(null)
        setHash('#chat/automations')
        return
      }
      if (selfSaveInFlightRef.current > 0) return
      if (event.updatedAt && event.updatedAt === lastSelfUpdatedAtRef.current) return
      if (dirtyRef.current) {
        setRemoteHint(t.chatAutomationRemoteUpdate)
        return
      }
      void automationApi.get(current.id).then((fresh) => {
        if (cancelled) return
        if (editingRef.current?.id !== fresh.id) return
        if (dirtyRef.current || selfSaveInFlightRef.current > 0) {
          setRemoteHint(t.chatAutomationRemoteUpdate)
          return
        }
        if (
          fresh.updatedAt === lastSelfUpdatedAtRef.current
          || fresh.updatedAt === editingRef.current.updatedAt
        ) {
          return
        }
        lastSelfUpdatedAtRef.current = fresh.updatedAt
        setRemoteHint('')
        setEditing(fresh)
        setCanvasEpoch((n) => n + 1)
      }).catch(() => {})
    }).then((fn) => {
      if (cancelled) fn()
      else unlisten = fn
    }).catch(() => {})
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [loadList, t])

  const openId = useCallback(async (id: string) => {
    if (editingRef.current?.id === id) return
    const request = ++navigationRef.current
    try {
      await flushSave()
      const automation = await automationApi.get(id)
      if (request !== navigationRef.current) return
      lastSelfUpdatedAtRef.current = automation.updatedAt
      editingRef.current = automation
      dirtyRef.current = false
      setError('')
      setSaveState('saved')
      setRemoteHint('')
      setCanvasEpoch(0)
      setEditing(automation)
      setHash(automationHash(id))
    } catch (err) {
      if (request !== navigationRef.current) return
      setError(err instanceof Error ? err.message : String(err))
      if (editingRef.current) setHash(automationHash(editingRef.current.id))
    }
  }, [flushSave])

  const backToList = useCallback(async () => {
    const request = ++navigationRef.current
    try {
      await flushSave()
      if (request !== navigationRef.current) return
      editingRef.current = null
      setRemoteHint('')
      setEditing(null)
      setHash('#chat/automations')
      void loadList()
    } catch {
      if (editingRef.current) setHash(automationHash(editingRef.current.id))
    }
  }, [flushSave, loadList])

  useEffect(() => {
    const syncFromHash = () => {
      const id = getRouteAutomationId()
      if (id) void openId(id)
      else if (editingRef.current) void backToList()
    }
    syncFromHash()
    window.addEventListener('hashchange', syncFromHash)
    return () => {
      navigationRef.current += 1
      window.removeEventListener('hashchange', syncFromHash)
    }
  }, [openId, backToList])

  const persist = useCallback((next: Automation) => {
    editingRef.current = next
    setEditing(next)
    if (!isTauriRuntime()) return
    dirtyRef.current = true
    setSaveState('pending')
    clearTimeoutRef(saveTimerRef)
    saveTimerRef.current = window.setTimeout(() => {
      saveTimerRef.current = null
      void flushSave().then(loadList).catch(() => {})
    }, 400)
  }, [flushSave, loadList])

  const create = useCallback(async () => {
    setError('')
    setSaveState('saved')
    const blank = createBlankAutomation()
    blank.name = t.chatAutomationUntitled
    try {
      const saved = isTauriRuntime() ? await automationApi.save(blank) : blank
      lastSelfUpdatedAtRef.current = saved.updatedAt
      setRemoteHint('')
      setCanvasEpoch(0)
      editingRef.current = saved
      setEditing(saved)
      setHash(automationHash(saved.id))
      void loadList()
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    }
  }, [loadList, t])

  const importFromFile = useCallback(async () => {
    setError('')
    setSaveState('saved')
    if (!isTauriRuntime()) return
    try {
      const picked = await openDialog({
        multiple: false,
        filters: [{ name: 'JSON', extensions: ['json'] }],
      })
      if (typeof picked !== 'string') return
      const imported = await automationApi.importFromFile(picked)
      lastSelfUpdatedAtRef.current = imported.updatedAt
      setRemoteHint('')
      setCanvasEpoch(0)
      setEditing(imported)
      setHash(automationHash(imported.id))
      void loadList()
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err)
      setError(`${t.chatAutomationImportFailed}${message}`)
    }
  }, [loadList, t])

  if (editing) {
    return (
      <div className="flex h-full min-h-0 flex-1 flex-col">
        {isTauriRuntime() && <div className="flex shrink-0 items-center gap-3 px-6 py-2 text-[12px]" role="status" aria-live="polite">
          <span>{saveState === 'saved' ? (english ? 'Saved' : '已保存')
            : saveState === 'saving' ? (english ? 'Saving…' : '正在保存…')
            : saveState === 'pending' ? (english ? 'Unsaved changes' : '有未保存的修改')
            : (english ? 'Save failed · draft retained' : '保存失败 · 草稿已保留')}</span>
          {saveState === 'error' && <Button size="sm" onClick={() => void flushSave().catch(() => {})}>{english ? 'Retry save' : '重试保存'}</Button>}
        </div>}
        {error ? <p role="alert" className="shrink-0 px-6 py-2 text-[13px] text-red-600 dark:text-red-400">{error}</p> : null}
        {remoteHint ? (
          <p className="shrink-0 px-6 py-2 text-[13px] text-amber-700 dark:text-amber-400">{remoteHint}</p>
        ) : null}
        <AutomationEditor
          key={`${editing.id}:${canvasEpoch}`}
          automation={editing}
          onChange={persist}
          onBack={backToList}
          onFlushSave={flushSave}
        />
      </div>
    )
  }

  return renderList(
    <AutomationList
      items={items}
      loading={loading}
      error={error || listError}
      onCreate={() => void create()}
      onOpen={(id) => void openId(id)}
      onToggle={(id, enabled) => {
        void automationApi.setEnabled(id, enabled).then(loadList).catch((err) => {
          setError(err instanceof Error ? err.message : String(err))
        })
      }}
      onDelete={async (id) => {
        if (!(await confirmDialog({ message: t.chatAutomationDeleteConfirm, confirmLabel: t.dialogDelete, danger: true }))) return
        void automationApi.remove(id).then(loadList).catch((err) => {
          setError(err instanceof Error ? err.message : String(err))
        })
      }}
    />,
    { onCreate: () => void create(), onImport: () => void importFromFile() },
  )
}
