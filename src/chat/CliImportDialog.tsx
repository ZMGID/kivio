import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { ArrowUpRight, Check, Loader2, RefreshCw } from 'lucide-react'
import { Button } from '../components/Button'
import { useT, type I18n } from '../components/i18n'
import { chatApi } from './api'
import type { ChatProject, ImportableCliSession } from './types'
import { useCloseAnimation } from './useCloseAnimation'
import { useWindowStore } from '../utils/windowStore'
import {
  cliImportItemKey,
  cliImportStore,
  finishCliImportSuccess,
  markCliImportNotified,
  markCliImportRefreshed,
  startCliImport,
} from './cliImportStore'

/// 各 CLI 的展示名。后端返回的是 `RuntimeAgentDef.id`。
const AGENT_LABELS: Record<string, string> = {
  claude: 'Claude Code',
  codex: 'Codex',
  grok: 'Grok',
  kimi: 'Kimi Code',
  opencode: 'OpenCode',
  cursor: 'Cursor',
}

/// kimi 的 `session/load` 能绑定但不重放历史，本地 wire 也没有稳定可读的 assistant 正文，
/// 所以导进来消息区是空的。这条必须在勾选前就说清楚，不能等导完让用户以为丢数据了。
const NO_HISTORY_AGENTS = new Set(['kimi'])

interface CliImportDialogProps {
  project: ChatProject
  onClose: () => void
  /** 导入成功后回调，交给外层刷新列表并跳转。 */
  onImported: (conversationIds: string[]) => void
  /** 点击一条「Kivio 里已经有了」的会话时，跳到那条已存在的对话。 */
  onOpenConversation: (conversationId: string) => void
}

function formatWhen(ms: number | null | undefined, t: I18n): string {
  if (!ms) return ''
  const diff = Date.now() - ms
  const day = 86_400_000
  if (diff < 3_600_000) return t.chatSkillCliJustNow
  if (diff < day) return t.chatSkillCliHoursAgo.replace('{n}', String(Math.floor(diff / 3_600_000)))
  if (diff < 30 * day) return t.chatSkillCliDaysAgo.replace('{n}', String(Math.floor(diff / day)))
  return new Date(ms).toLocaleDateString()
}

function initialCliImportSelection(projectId: string): Set<string> {
  const operation = cliImportStore.getSnapshot().byProject[projectId]
  if (!operation || (operation.phase !== 'running' && operation.phase !== 'error')) return new Set()
  const items = operation.phase === 'error' && operation.failures.length > 0
    ? operation.failures
    : operation.items
  return new Set(items.map((item) => cliImportItemKey(item)))
}

export function CliImportDialog({
  project,
  onClose,
  onImported,
  onOpenConversation,
}: CliImportDialogProps) {
  const [sessions, setSessions] = useState<ImportableCliSession[]>([])
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState('')
  const loadGeneration = useRef(0)
  const [imports] = useWindowStore(cliImportStore)
  const operation = imports.byProject[project.id]
  const importing = operation?.phase === 'running'
  const reconciling = operation?.phase === 'error'
    && operation.refreshedGeneration !== operation.settledGeneration
  const [selected, setSelected] = useState<Set<string>>(() => initialCliImportSelection(project.id))
  const handleClose = useCallback(() => {
    const current = cliImportStore.getSnapshot().byProject[project.id]
    if (current && current.phase === 'success' && current.failures.length === 0) {
      finishCliImportSuccess(project.id, current.settledGeneration)
    }
    onClose()
  }, [onClose, project.id])
  const { closing, startClose, onAnimationEnd } = useCloseAnimation(handleClose)
  const t = useT()
  const failureText = operation && operation.failures.length > 0
    ? t.chatSkillCliImportFailures
      .replace('{n}', String(operation.failures.length))
      .replace('{detail}', operation.failures.map((failure) => failure.error).slice(0, 2).join('；'))
    : ''
  const error = operation?.error || failureText || loadError

  const load = useCallback(async () => {
    const request = ++loadGeneration.current
    setLoading(true)
    setLoadError('')
    try {
      const next = await chatApi.listImportableCliSessions(project.id)
      if (request === loadGeneration.current) setSessions(next)
    } catch (err) {
      if (request !== loadGeneration.current) return
      setLoadError(err instanceof Error ? err.message : String(err))
      setSessions([])
    } finally {
      if (request === loadGeneration.current) setLoading(false)
    }
  }, [project.id])

  useEffect(() => {
    void load()
    return () => { loadGeneration.current += 1 }
  }, [load])

  const loadRef = useRef(load)
  loadRef.current = load
  const importedRef = useRef(onImported)
  importedRef.current = onImported
  const closeRef = useRef(startClose)
  closeRef.current = startClose
  const settledGeneration = operation?.settledGeneration ?? 0
  const operationPhase = operation?.phase ?? 'idle'
  useEffect(() => {
    const current = cliImportStore.getSnapshot().byProject[project.id]
    if (!current || current.phase === 'running' || current.settledGeneration === 0) return
    const generation = current.settledGeneration
    if (current.notifiedGeneration !== generation) {
      markCliImportNotified(project.id, generation)
      const claimed = cliImportStore.getSnapshot().byProject[project.id]
      if (!claimed || claimed.notifiedGeneration !== generation || claimed.phase === 'running') return
      if (claimed.importedIds.length) importedRef.current(claimed.importedIds)
    }
    const pending = cliImportStore.getSnapshot().byProject[project.id]
    if (!pending || pending.phase === 'running' || pending.settledGeneration !== generation) return
    if (pending.phase === 'success' && pending.failures.length === 0) {
      setSelected(new Set())
      closeRef.current()
      return
    }
    if (pending.refreshedGeneration === generation) return
    const failedKeys = pending.failures.length > 0
      ? pending.failures.map((failure) => cliImportItemKey(failure))
      : pending.items.map((item) => cliImportItemKey(item))
    setSelected(new Set(failedKeys))
    let active = true
    void loadRef.current().then(() => {
      if (!active) return
      const latest = cliImportStore.getSnapshot().byProject[project.id]
      if (!latest || latest.phase === 'running' || latest.settledGeneration !== generation) return
      if (latest.refreshedGeneration === generation) return
      if (latest.phase === 'error') setSelected(new Set(failedKeys))
      markCliImportRefreshed(project.id, generation)
    })
    return () => { active = false }
  }, [operationPhase, project.id, settledGeneration])

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') startClose()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [startClose])

  const grouped = useMemo(() => {
    const map = new Map<string, ImportableCliSession[]>()
    for (const session of sessions) {
      const list = map.get(session.agentId) ?? []
      list.push(session)
      map.set(session.agentId, list)
    }
    return [...map.entries()].sort((a, b) => a[0].localeCompare(b[0]))
  }, [sessions])

  const keyOf = (s: ImportableCliSession) => cliImportItemKey(s)

  const toggle = (session: ImportableCliSession) => {
    // 已经有对话绑着这条原生会话就不能再导（绑定是 1:1 的，导第二次会让两边快照都残缺）。
    // 但点击不该没反应——跳到那条已存在的对话去。
    if (session.boundConversationId) {
      onOpenConversation(session.boundConversationId)
      startClose()
      return
    }
    if (session.alreadyImported) return
    setSelected((prev) => {
      const next = new Set(prev)
      const key = keyOf(session)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      return next
    })
  }

  const runImport = () => {
    if (!selected.size || importing || loading || reconciling) return
    const current = cliImportStore.getSnapshot().byProject[project.id]
    if (current?.phase === 'running'
      || (current?.phase === 'error' && current.refreshedGeneration !== current.settledGeneration)) return
    const items = [...selected].map((key) => {
      const session = sessions.find((candidate) => keyOf(candidate) === key)
      if (session) return { agentId: session.agentId, sessionId: session.sessionId }
      const [agentId, sessionId] = key.split('::')
      return { agentId, sessionId }
    })
    void startCliImport(project.id, items)
  }

  const rootPath = (project.root_path ?? project.rootPath ?? '').trim()

  return createPortal(
    <div
      className={`${closing ? 'chat-motion-fade-out' : 'chat-motion-fade'} fixed inset-0 z-[300] flex items-center justify-center bg-neutral-900/30 px-4 backdrop-blur-[1px]`}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) startClose()
      }}
    >
      <div
        className={`${closing ? 'chat-motion-modal-out' : 'chat-motion-modal-in'} flex max-h-[80vh] w-full max-w-[560px] flex-col rounded-[10px] border border-neutral-200 bg-neutral-50 shadow-xl`}
        role="dialog"
        aria-modal="true"
        aria-busy={importing}
        aria-label={t.chatImportFromCli}
        onAnimationEnd={onAnimationEnd}
      >
        <div className="flex items-start justify-between gap-3 border-b border-neutral-200 px-4 py-3">
          <div className="min-w-0">
            <h3 className="text-[14px] font-semibold text-neutral-900">
              {t.chatImportFromCli}
            </h3>
            <p className="mt-0.5 truncate text-[11px] text-neutral-500 dark:text-neutral-400">
              {t.chatSkillCliImportScopeHint.replace('{path}', rootPath)}
            </p>
          </div>
          <Button variant="ghost" size="sm" onClick={() => void load()} disabled={loading}>
            <RefreshCw strokeWidth={1.75} className={loading ? 'animate-spin' : undefined} />
            {t.externalAgentsRescan}
          </Button>
        </div>

        <div className="min-h-[160px] flex-1 overflow-y-auto px-4 py-3">
          {loading ? (
            <div className="flex items-center gap-2 py-8 text-[12px] text-neutral-500 dark:text-neutral-400">
              <Loader2 strokeWidth={1.75} className="animate-spin" />
              {t.chatSkillCliScanningHint}
            </div>
          ) : !sessions.length ? (
            <p className="py-8 text-center text-[12px] text-neutral-500 dark:text-neutral-400">
              {t.chatSkillCliNoSessions}
            </p>
          ) : (
            grouped.map(([agentId, list]) => (
              <div key={agentId} className="mb-4 last:mb-0">
                <div className="mb-1.5 flex items-baseline gap-2">
                  <span className="text-[12px] font-medium text-neutral-700">
                    {AGENT_LABELS[agentId] ?? agentId}
                  </span>
                  <span className="text-[11px] text-neutral-400">{t.chatSkillCliCount.replace('{n}', String(list.length))}</span>
                  {NO_HISTORY_AGENTS.has(agentId) && (
                    <span className="text-[11px] text-amber-600 dark:text-amber-500">
                      {t.chatSkillCliNoHistoryHint}
                    </span>
                  )}
                </div>
                <ul className="space-y-1">
                  {list.map((session) => {
                    const key = keyOf(session)
                    const checked = selected.has(key)
                    const bound = Boolean(session.boundConversationId)
                    // 「已导入」和「Kivio 里已经有了」是两回事：后者是 Kivio 自己跑出来的会话，
                    // 用户从没导过它，标成"已导入"是在撒谎。
                    const boundLabel = session.alreadyImported ? t.chatSkillCliImported : t.chatSkillCliBound
                    return (
                      <li key={key}>
                        <button
                          type="button"
                          onClick={() => toggle(session)}
                          title={bound ? t.chatSkillCliOpenBound : undefined}
                          className={`flex w-full items-start gap-2 rounded-[6px] px-2 py-1.5 text-left transition-colors ${
                            bound ? 'opacity-55' : ''
                          } hover:bg-neutral-100`}
                        >
                          <span
                            className={`mt-0.5 flex h-[14px] w-[14px] shrink-0 items-center justify-center rounded-[3px] ${
                              bound
                                ? 'text-neutral-400'
                                : checked
                                  ? 'rounded-[3px] border border-[var(--accent)] bg-[var(--accent)] text-[var(--text-onaccent)]'
                                  : 'border border-neutral-300'
                            }`}
                          >
                            {bound ? (
                              <ArrowUpRight strokeWidth={2} className="h-[12px] w-[12px]" />
                            ) : (
                              checked && <Check strokeWidth={3} className="h-[10px] w-[10px]" />
                            )}
                          </span>
                          <span className="min-w-0 flex-1">
                            <span className="block truncate text-[12px] text-neutral-800">
                              {session.title || t.chatSkillCliUntitled}
                            </span>
                            <span className="mt-0.5 block text-[11px] text-neutral-400">
                              {session.messageCount == null
                                ? t.chatSkillCliUnknownCount
                                : t.chatSkillCliCount.replace('{n}', String(session.messageCount))}
                              {formatWhen(session.updatedAt, t) && ` · ${formatWhen(session.updatedAt, t)}`}
                              {bound && ` · ${boundLabel}`}
                            </span>
                          </span>
                        </button>
                      </li>
                    )
                  })}
                </ul>
              </div>
            ))
          )}
        </div>

        {error && (
          <p className="border-t border-neutral-200 px-4 py-2 text-[11px] text-red-600 dark:text-red-400">
            {error}
          </p>
        )}

        <div className="flex items-center justify-end gap-2 border-t border-neutral-200 px-4 py-3">
          <Button variant="ghost" size="sm" onClick={startClose} disabled={importing}>
            {t.cancel}
          </Button>
          <Button size="sm" onClick={() => void runImport()} disabled={!selected.size || importing || loading || reconciling}>
            {importing && <Loader2 strokeWidth={1.75} className="animate-spin" />}
            {t.chatSkillCliImportAction.replace('{n}', selected.size ? String(selected.size) : '')}
          </Button>
        </div>
      </div>
    </div>,
    document.body,
  )
}
