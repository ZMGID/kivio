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
import { useWindowStore } from '../../utils/windowStore'
import {
  automationDraftStore,
  discardAutomationDraft,
  flushAutomationDraft,
  deleteAutomationDraft,
  forgetAutomationDraft,
  showAutomation,
  stageAutomationDraft,
} from './automationDraftStore'

let automationNavigation = 0

function claimAutomationNavigation() {
  automationNavigation += 1
  return automationNavigation
}

function automationNavigationCurrent(request: number) {
  return request === automationNavigation
}

// 一次读取只能落回它出发时的草稿。编辑会换成新对象，保存成功会推进 savedEpoch；
// 这两件事都会让草稿重新变干净，不能再只看当前的 dirty / saving。
type DraftReadBaseline = {
  session: number
  draft: Automation | null
  savedEpoch: number
}

function captureDraftReadBaseline(): DraftReadBaseline {
  const snap = automationDraftStore.getSnapshot()
  return { session: snap.session, draft: snap.draft, savedEpoch: snap.savedEpoch }
}

function draftReadMoved(baseline: DraftReadBaseline) {
  const latest = automationDraftStore.getSnapshot()
  return latest.session !== baseline.session
    || latest.draft !== baseline.draft
    || latest.savedEpoch !== baseline.savedEpoch
}

export function AutomationCenter({ items, loading, listError, onReload, onCreateByChat, renderList }: {
  items: AutomationMeta[]
  loading: boolean
  listError: string
  onReload: () => Promise<void>
  onCreateByChat: () => void
  renderList: (body: ReactNode, actions: { onCreate: () => void; onImport: () => void }) => ReactNode
}) {
  const t = useT()
  const english = useLang() === 'en'
  const [draftState] = useWindowStore(automationDraftStore)
  const [localError, setLocalError] = useState('')
  const [canvasEpoch, setCanvasEpoch] = useState(0)
  const [remoteHint, setRemoteHint] = useState('')
  const savedEpochSeen = useRef(draftState.savedEpoch)
  const editing = draftState.draft

  const loadList = useCallback(async () => {
    try { await onReload() }
    catch (err) { setLocalError(err instanceof Error ? err.message : String(err)) }
  }, [onReload])

  useEffect(() => {
    if (draftState.savedEpoch === savedEpochSeen.current) return
    savedEpochSeen.current = draftState.savedEpoch
    if (draftState.status === 'saved') void loadList()
  }, [draftState.savedEpoch, draftState.status, loadList])

  // 离开本页不是取消：未保存的草稿留在窗口里，保存完成后也不会写进另一份自动化。
  useEffect(() => () => {
    void flushAutomationDraft().catch(() => {})
  }, [])

  useEffect(() => {
    if (!isTauriRuntime()) return
    let cancelled = false
    let unlisten: (() => void) | undefined
    void api.onAutomationChanged((event) => {
      if (cancelled) return
      const current = automationDraftStore.getSnapshot()
      if (!current.draft || event.id !== current.draft.id) return
      if (event.kind === 'deleted') {
        forgetAutomationDraft(event.id)
        setRemoteHint('')
        setHash('#chat/automations')
        return
      }
      if (current.status === 'saving') return
      if (event.updatedAt && event.updatedAt === current.lastSelfUpdatedAt) return
      if (current.dirty) {
        setRemoteHint(t.chatAutomationRemoteUpdate)
        return
      }
      const baseline = captureDraftReadBaseline()
      void automationApi.get(current.draft.id).then((fresh) => {
        if (cancelled) return
        const latest = automationDraftStore.getSnapshot()
        if (latest.draft?.id !== fresh.id) return
        if (latest.dirty || latest.status === 'saving') {
          setRemoteHint(t.chatAutomationRemoteUpdate)
          return
        }
        if (draftReadMoved(baseline)) return
        if (
          fresh.updatedAt === latest.lastSelfUpdatedAt
          || fresh.updatedAt === latest.draft.updatedAt
        ) {
          return
        }
        showAutomation(fresh)
        setRemoteHint('')
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
  }, [t])

  const openId = useCallback(async (id: string) => {
    const request = claimAutomationNavigation()
    const snap = automationDraftStore.getSnapshot()
    if (snap.draft?.id === id && (snap.dirty || snap.status === 'saving')) return
    const session = snap.session
    try {
      await flushAutomationDraft()
      if (!automationNavigationCurrent(request)) return
      const current = automationDraftStore.getSnapshot()
      if (current.session !== session && current.draft) return
      const baseline = captureDraftReadBaseline()
      const automation = await automationApi.get(id)
      if (!automationNavigationCurrent(request)) return
      const latest = automationDraftStore.getSnapshot()
      if (latest.session !== session && latest.draft) return
      if (latest.draft?.id === id) {
        if (latest.dirty || latest.status === 'saving') {
          if (automation.updatedAt !== latest.draft.updatedAt) setRemoteHint(t.chatAutomationRemoteUpdate)
          return
        }
        if (draftReadMoved(baseline)) return
        if (automation.updatedAt === latest.draft.updatedAt) {
          setHash(automationHash(id))
          return
        }
      }
      const refreshMounted = latest.draft?.id === automation.id
      showAutomation(automation)
      setLocalError('')
      setRemoteHint('')
      // 同一份文档换内容时 key 必须变，否则节点、连线和视口仍是挂载时的那一份，改名会把旧图写回去。
      setCanvasEpoch((epoch) => (refreshMounted ? epoch + 1 : 0))
      setHash(automationHash(id))
    } catch (err) {
      if (!automationNavigationCurrent(request)) return
      const current = automationDraftStore.getSnapshot()
      if (current.session !== session && current.draft && current.draft.id !== id) return
      if (current.draft) setHash(automationHash(current.draft.id))
      if (!current.error) setLocalError(err instanceof Error ? err.message : String(err))
    }
  }, [t])

  const backToList = useCallback(async () => {
    const request = claimAutomationNavigation()
    const session = automationDraftStore.getSnapshot().session
    try {
      await flushAutomationDraft()
      if (!automationNavigationCurrent(request)) return
      if (automationDraftStore.getSnapshot().session !== session) return
      discardAutomationDraft()
      setRemoteHint('')
      setHash('#chat/automations')
      void loadList()
    } catch {
      if (!automationNavigationCurrent(request)) return
      if (automationDraftStore.getSnapshot().session !== session) return
      const current = automationDraftStore.getSnapshot().draft
      if (current) setHash(automationHash(current.id))
    }
  }, [loadList])

  useEffect(() => {
    const syncFromHash = () => {
      const id = getRouteAutomationId()
      if (id) void openId(id)
      else if (automationDraftStore.getSnapshot().draft) void backToList()
    }
    window.addEventListener('hashchange', syncFromHash)
    const retained = automationDraftStore.getSnapshot().draft
    if (!getRouteAutomationId() && retained) setHash(automationHash(retained.id))
    else syncFromHash()
    return () => {
      claimAutomationNavigation()
      window.removeEventListener('hashchange', syncFromHash)
    }
  }, [openId, backToList])

  const persist = useCallback((next: Automation) => {
    stageAutomationDraft(next)
  }, [])

  const create = useCallback(async () => {
    const request = claimAutomationNavigation()
    setLocalError('')
    const blank = createBlankAutomation()
    blank.name = t.chatAutomationUntitled
    try {
      if (automationDraftStore.getSnapshot().draft) await flushAutomationDraft()
      if (!automationNavigationCurrent(request)) return
      const saved = isTauriRuntime() ? await automationApi.save(blank) : blank
      if (!automationNavigationCurrent(request)) return
      showAutomation(saved)
      setRemoteHint('')
      setCanvasEpoch(0)
      setHash(automationHash(saved.id))
      void loadList()
    } catch (err) {
      if (!automationNavigationCurrent(request)) return
      setLocalError(err instanceof Error ? err.message : String(err))
    }
  }, [loadList, t])

  const importFromFile = useCallback(async () => {
    const request = claimAutomationNavigation()
    setLocalError('')
    if (!isTauriRuntime()) return
    try {
      const picked = await openDialog({
        multiple: false,
        filters: [{ name: 'JSON', extensions: ['json'] }],
      })
      if (typeof picked !== 'string') return
      if (automationDraftStore.getSnapshot().draft) await flushAutomationDraft()
      if (!automationNavigationCurrent(request)) return
      const imported = await automationApi.importFromFile(picked)
      if (!automationNavigationCurrent(request)) return
      showAutomation(imported)
      setRemoteHint('')
      setCanvasEpoch(0)
      setHash(automationHash(imported.id))
      void loadList()
    } catch (err) {
      if (!automationNavigationCurrent(request)) return
      const message = err instanceof Error ? err.message : String(err)
      setLocalError(`${t.chatAutomationImportFailed}${message}`)
    }
  }, [loadList, t])

  if (editing) {
    const visibleError = draftState.error || localError
    return (
      <div className="flex h-full min-h-0 flex-1 flex-col">
        {isTauriRuntime() && <div className="flex shrink-0 items-center gap-3 px-6 py-2 text-[12px]" role="status" aria-live="polite">
          <span>{draftState.status === 'saved' ? (english ? 'Saved' : '已保存')
            : draftState.status === 'saving' ? (english ? 'Saving…' : '正在保存…')
            : draftState.status === 'pending' ? (english ? 'Unsaved changes' : '有未保存的修改')
            : (english ? 'Save failed · draft retained' : '保存失败 · 草稿已保留')}</span>
          {draftState.status === 'error' && <Button size="sm" onClick={() => void flushAutomationDraft().catch(() => {})}>{english ? 'Retry save' : '重试保存'}</Button>}
        </div>}
        {visibleError ? <p role="alert" className="shrink-0 px-6 py-2 text-[13px] text-red-600 dark:text-red-400">{visibleError}</p> : null}
        {remoteHint ? (
          <p className="shrink-0 px-6 py-2 text-[13px] text-amber-700 dark:text-amber-400">{remoteHint}</p>
        ) : null}
        <AutomationEditor
          key={`${editing.id}:${canvasEpoch}`}
          automation={editing}
          onChange={persist}
          onBack={backToList}
          onFlushSave={flushAutomationDraft}
        />
      </div>
    )
  }

  return renderList(
    <AutomationList
      items={items}
      loading={loading}
      error={localError || listError}
      onCreate={() => void create()}
      onCreateByChat={onCreateByChat}
      onOpen={(id) => void openId(id)}
      onToggle={(id, enabled) => {
        void automationApi.setEnabled(id, enabled).then(loadList).catch((err) => {
          setLocalError(err instanceof Error ? err.message : String(err))
        })
      }}
      onDelete={async (id) => {
        if (!(await confirmDialog({ message: t.chatAutomationDeleteConfirm, confirmLabel: t.dialogDelete, danger: true }))) return
        try {
          await deleteAutomationDraft(id, () => automationApi.remove(id))
          await loadList()
        } catch (err) {
          setLocalError(err instanceof Error ? err.message : String(err))
        }
      }}
    />,
    { onCreate: () => void create(), onImport: () => void importFromFile() },
  )
}
