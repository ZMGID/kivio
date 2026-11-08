// 标题栏工作区面板，参照 T3 Code 的 ThreadDetailsPanel：
// 段一 = 项目行 / Open in 拆分按钮；段二 = 分支 / Git 主操作拆分按钮 / Changes（仅项目对话）；
// 段三 = 历史分支 / 任务；段四 = 子代理。
// Git 数据来自 useGitBadge（只在面板可见时订阅），主操作规则见 workspaceGitAction。
// 位置：浮层贴触发按钮右下展开；内联常驻时贴对话区右缘，并通过 --chat-details-inset 让出内容宽度。
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { Activity, ArrowDown, ArrowUp, Check, CloudUpload, Code2, FileDiff, FolderOpen, GitBranch, GitCommitHorizontal, GitFork, Loader2, Monitor, Square, SquareMenu } from 'lucide-react'
import { Button, IconButton } from '../../components/Button'
import { notifyError, notifySuccess } from '../../components/appNoticeStore'
import { i18n, type Lang } from '../../components/i18n'
import { Input } from '../../settings/public/controls'
import type { GitBranchItem } from '../../api/dockContracts'
import { getRememberedEditor, rememberEditor } from '../persistence'
import { dockApi } from './api'
import { PANEL_ICON, PANEL_SCOPE_ATTR, PanelRow, PanelSection, PanelSkeletonRows, SplitRow } from './WorkspacePanelParts'
import { motionClass, usePresence } from './workspacePanelMotion'
import { requestDockSubagent } from './dockPreview'
import { AgentAvatar } from '../AgentAvatar'
import { AgentStatus } from '../AgentProgress'
import { useSubagentRoster } from '../useSubagentRoster'
import { subagentsApi } from '../../api/subagents'
import { useGitBadge } from './useGitBadge'
import { useBackgroundTasks } from '../useBackgroundTasks'
import { partitionTasks } from '../backgroundTasks'
import { ConversationBranchDialog } from '../ConversationBranches'
import { resolveGitMenu, resolveGitQuickAction, type GitAction, type GitHint } from './workspaceGitAction'

export type WorkspaceHistoryBranches = {
  disabled: boolean
  onSwitch: (branchId: string) => Promise<boolean>
}

type WorkspaceActionsMenuProps = {
  workdir: string
  /** 关联项目名；为空表示无项目对话，按领域规则不显示 Git。 */
  projectName: string | null
  conversationId: string | null
  dockOpen: boolean
  lang: Lang
  /** 弹出窗口没有右栏，始终以浮层打开。 */
  inlineAllowed?: boolean
  onOpenGitPanel?: () => void
  onOpenTasks?: () => void
  historyBranches?: WorkspaceHistoryBranches | null
}

const EDITOR_LABELS: Record<string, string> = { vscode: 'VS Code', cursor: 'Cursor', zed: 'Zed', windsurf: 'Windsurf' }
const ITEM = 'kv-menu-row w-full text-neutral-800 transition-colors hover:bg-neutral-100 disabled:opacity-50'
const PANEL_WIDTH = 300
const PANEL_GAP = 16
/** 面板离窗口/对话区右缘的最小距离。 */
const PANEL_EDGE = 8
/** 内联展开后对话列至少保留的宽度，否则改为浮层。 */
const MIN_CHAT_WIDTH = 600

/** 依据对话主区实际宽度（已扣除侧栏）判断能否内联常驻。 */
function useInlineRoom(enabled: boolean, conversationId: string | null) {
  const [room, setRoom] = useState(false)
  useEffect(() => {
    if (!enabled) {
      setRoom(false)
      return
    }
    const pane = document.querySelector<HTMLElement>('.chat-main-pane')
    const measure = () => {
      const width = pane?.getBoundingClientRect().width || window.innerWidth
      setRoom(width - PANEL_WIDTH - PANEL_GAP >= MIN_CHAT_WIDTH)
    }
    measure()
    const observer = pane && typeof ResizeObserver !== 'undefined' ? new ResizeObserver(measure) : null
    if (pane) observer?.observe(pane)
    window.addEventListener('resize', measure)
    return () => {
      observer?.disconnect()
      window.removeEventListener('resize', measure)
    }
  }, [enabled, conversationId])
  return room
}

export function WorkspaceActionsMenu({
  workdir,
  projectName,
  conversationId,
  dockOpen,
  lang,
  inlineAllowed = true,
  onOpenGitPanel,
  onOpenTasks,
  historyBranches,
}: WorkspaceActionsMenuProps) {
  const t = i18n[lang]
  const inline = useInlineRoom(inlineAllowed && !dockOpen, conversationId)
  const [choice, setChoice] = useState<boolean | null>(null)
  const open = choice ?? inline
  const presence = usePresence(open)
  // 行内点击后的「关闭」在内联模式下不生效，面板常驻。
  const setOpen = (next: boolean) => { if (next || !inline) setChoice(next) }
  const showGit = Boolean(projectName)
  const { state, diffStat, loading: gitLoading, applyMutationState, refresh } = useGitBadge(open && showGit ? workdir : '', true)
  const tasks = useBackgroundTasks(conversationId)
  const runningTasks = partitionTasks(tasks).running.length
  const [place, setPlace] = useState<{ top: number; right: number }>({ top: 0, right: 8 })
  const wrapRef = useRef<HTMLDivElement>(null)
  const panelRef = useRef<HTMLDivElement>(null)
  const [busy, setBusy] = useState<GitAction | 'branch' | 'init' | null>(null)
  const [editors, setEditors] = useState<string[] | null>(null)
  const [commitDialog, setCommitDialog] = useState<'commit' | 'commit_push' | null>(null)
  const [branchDialog, setBranchDialog] = useState(false)
  const [message, setMessage] = useState('')
  const [branches, setBranches] = useState<GitBranchItem[] | null>(null)
  const [remotes, setRemotes] = useState<string[] | null>(null)
  const { roster, refresh: refreshRoster } = useSubagentRoster(open ? conversationId : null)

  // 模式切换时回到「按空间自动展开」。
  useEffect(() => { setChoice(null) }, [inline])
  // 浮层：顶边贴按钮下沿、右边与按钮右缘对齐；内联：贴对话区右缘，与标题栏下沿留同样间距。
  useLayoutEffect(() => {
    if (!presence.present) return
    const measure = () => {
      const button = wrapRef.current?.getBoundingClientRect()
      if (!button) return
      const pane = inline ? document.querySelector<HTMLElement>('.chat-main-pane')?.getBoundingClientRect() : null
      const right = pane ? window.innerWidth - pane.right + PANEL_EDGE : window.innerWidth - button.right
      setPlace({ top: button.bottom + 6, right: Math.max(PANEL_EDGE, right) })
    }
    measure()
    window.addEventListener('resize', measure)
    return () => window.removeEventListener('resize', measure)
  }, [presence.present, inline])
  // 内联展开时把主区右侧让出来，对话列往左挤；浮层模式不让位。
  useEffect(() => {
    if (!open || !inline) return
    const root = document.documentElement
    root.style.setProperty('--chat-details-inset', `${PANEL_WIDTH + PANEL_GAP}px`)
    return () => { root.style.removeProperty('--chat-details-inset') }
  }, [open, inline])
  // 浮层模式不放遮罩：点面板（含其下拉、对话框）外或按 Esc 关闭，页面其余部分照常可操作。
  useEffect(() => {
    if (!open || inline) return
    const onDown = (event: PointerEvent) => {
      const target = event.target as Element | null
      if (target?.closest?.(`[${PANEL_SCOPE_ATTR}]`) || wrapRef.current?.contains(target)) return
      setChoice(false)
    }
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || document.querySelector('dialog[open]')) return
      setChoice(false)
      wrapRef.current?.querySelector<HTMLElement>('button')?.focus({ preventScroll: true })
    }
    document.addEventListener('pointerdown', onDown)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('pointerdown', onDown)
      document.removeEventListener('keydown', onKey)
    }
  }, [open, inline])

  // 打开浮层时把焦点移进面板，键盘可直接 Tab 操作；内联常驻不抢焦点。
  useEffect(() => {
    if (!open || inline || choice !== true) return
    panelRef.current?.querySelector<HTMLElement>('button:not(:disabled)')?.focus({ preventScroll: true })
  }, [open, inline, choice])

  useEffect(() => {
    if (!open) return
    let cancelled = false
    dockApi.availableEditors().then((ids) => { if (!cancelled) setEditors(ids) }).catch(() => { if (!cancelled) setEditors([]) })
    return () => { cancelled = true }
  }, [open])

  const isRepo = state?.status === 'ready'
  const head = isRepo ? state.head : ''
  // 当前分支变化（含本面板切换、外部 checkout）时重新读取分支与远程，避免列表停在旧数据。
  const loadBranches = useCallback(() => {
    let cancelled = false
    dockApi.gitBranches(workdir).then((result) => {
      if (cancelled) return
      setBranches(result.branches)
      setRemotes(result.remotes)
    }).catch(() => {
      if (cancelled) return
      setBranches([])
      setRemotes([])
    })
    return () => { cancelled = true }
  }, [workdir])
  useEffect(() => {
    if (!open || !isRepo) return
    return loadBranches()
  }, [open, isRepo, head, loadBranches])
  useEffect(() => {
    setBranches(null)
    setRemotes(null)
  }, [workdir])

  const name = projectName || workdir.split(/[\\/]/).filter(Boolean).pop() || workdir
  const editorList = editors ?? []
  // PATH 探测为空时仍显示 VS Code 行：真的找不到时由后端报错提示。
  const preferred = editorList.find((id) => id === getRememberedEditor()) ?? editorList[0] ?? 'vscode'
  const editorName = (id: string) => EDITOR_LABELS[id] ?? id
  const remotesLoaded = remotes !== null
  const hasRemote = (remotes?.length ?? 0) > 0
  const quick = resolveGitQuickAction(isRepo ? state : null, hasRemote, Boolean(busy) || !remotesLoaded)
  const menuState = resolveGitMenu(isRepo ? state : null, hasRemote, Boolean(busy) || !remotesLoaded)
  const gitPending = showGit && (!state || (isRepo && !remotesLoaded))
  const syncMeta = isRepo && (state.ahead > 0 || state.behind > 0) ? (
    <span className="inline-flex items-center gap-1.5">
      {state.ahead > 0 && <span className="inline-flex items-center"><ArrowUp className="size-3" />{state.ahead}</span>}
      {state.behind > 0 && <span className="inline-flex items-center"><ArrowDown className="size-3" />{state.behind}</span>}
    </span>
  ) : isRepo && quick.kind === 'hint' && quick.hint === 'upToDate' ? <Check className="size-3.5" /> : null
  const actionLabel: Record<GitAction, string> = {
    commit: t.workspaceCommit,
    commit_push: t.workspaceCommitPush,
    push: t.workspacePush,
  }
  const busyLabel: Record<GitAction, string> = {
    commit: t.workspaceCommitting,
    commit_push: t.workspaceCommittingPushing,
    push: t.workspacePushing,
  }
  const hintLabel: Record<GitHint, string> = {
    busy: '',
    unavailable: t.workspaceHintUnavailable,
    detached: t.workspaceHintDetached,
    noRemote: t.workspaceHintNoRemote,
    behind: t.workspaceHintBehind,
    upToDate: t.workspaceHintUpToDate,
  }
  /** 行内放得下的短说明，完整原因在悬停提示里。 */
  const hintShort: Record<GitHint, string> = {
    busy: '',
    unavailable: t.workspaceShortUnavailable,
    detached: t.workspaceShortDetached,
    noRemote: t.workspaceShortNoRemote,
    behind: t.workspaceShortBehind,
    upToDate: t.workspaceShortUpToDate,
  }

  const run = async (task: () => Promise<unknown>) => {
    try {
      await task()
    } catch (err) {
      notifyError(err instanceof Error ? err.message : String(err))
    }
  }

  const openInEditor = (editor: string) => {
    rememberEditor(editor)
    void run(async () => {
      await dockApi.openInEditor(workdir, editor)
      notifySuccess(t.workspaceOpenedIn.replace('{editor}', editorName(editor)))
    })
  }

  const switchBranch = (branch: GitBranchItem) => {
    if (branch.current || busy) return
    setBusy('branch')
    void run(async () => {
      const result = await dockApi.gitSwitchBranch(workdir, branch.name)
      applyMutationState(result.state)
      if (!result.ok) throw new Error(result.message)
      notifySuccess(t.workspaceSwitchedTo.replace('{branch}', branch.name))
    }).finally(() => {
      setBusy(null)
      loadBranches()
    })
  }

  const initRepo = async () => {
    if (busy) return
    setBusy('init')
    await run(async () => {
      const result = await dockApi.gitInit(workdir)
      applyMutationState(result.state)
      if (!result.ok) throw new Error(result.message)
      loadBranches()
    })
    setBusy(null)
  }

  const gitAction = async (action: GitAction) => {
    if (busy) return
    setBusy(action)
    await run(async () => {
      if (action !== 'push') {
        const staged = await dockApi.gitStageAll(workdir)
        if (!staged.ok) throw new Error(staged.message)
        const committed = await dockApi.gitCommit(workdir, message.trim())
        applyMutationState(committed.state)
        if (!committed.ok) throw new Error(committed.message)
        setMessage('')
      }
      if (action !== 'commit') {
        const pushed = await dockApi.gitPush(workdir)
        applyMutationState(pushed.state)
        if (!pushed.ok) throw new Error(pushed.message)
      }
      notifySuccess(action === 'commit' ? t.workspaceCommitted : action === 'push' ? t.workspacePushed : t.workspaceCommittedPushed)
      void refresh({ silent: true })
    })
    setBusy(null)
  }

  const startAction = (action: GitAction) => {
    if (action === 'push') void gitAction('push')
    else setCommitDialog(action)
  }

  const submitCommit = async () => {
    if (!commitDialog || !message.trim()) return
    const action = commitDialog
    setCommitDialog(null)
    await gitAction(action)
  }

  const tasksLabel = runningTasks > 0 ? t.chatBgJobsRunning.replace('{n}', String(runningTasks)) : t.dockTabTasks
  const conversationSection = conversationId ? (
    <PanelSection>
      {historyBranches && (
        <PanelRow
          icon={<GitFork className={PANEL_ICON} />}
          disabled={historyBranches.disabled}
          onClick={() => setBranchDialog(true)}
        >
          <span className="min-w-0 flex-1 truncate">{t.workspaceHistoryBranches}</span>
        </PanelRow>
      )}
      <PanelRow
        icon={<Activity className={`${PANEL_ICON} ${runningTasks > 0 ? '!text-emerald-500' : ''}`} />}
        onClick={onOpenTasks ? () => { setOpen(false); onOpenTasks() } : undefined}
      >
        <span className="min-w-0 flex-1 truncate">{t.dockTabTasks}</span>
        {runningTasks > 0 && (
          <span key={runningTasks} title={tasksLabel} aria-label={tasksLabel} className="chat-motion-pop shrink-0 rounded-full bg-emerald-500/10 px-1.5 text-[11px] font-medium tabular-nums text-emerald-600 dark:text-emerald-400">
            {runningTasks}
          </span>
        )}
      </PanelRow>
    </PanelSection>
  ) : null

  return (
    <div ref={wrapRef} className="relative shrink-0" data-tauri-drag-region="false">
      <IconButton
        label={runningTasks > 0 ? `${t.workspaceMenu} · ${tasksLabel}` : t.workspaceMenu}
        size="sm"
        variant="ghost"
        onClick={() => setChoice(!open)}
        aria-expanded={open}
        aria-haspopup="dialog"
        aria-pressed={open && !inline}
        className="relative"
      >
        <SquareMenu size={15} strokeWidth={1.75} />
        {runningTasks > 0 && !open && (
          <span aria-hidden className="absolute right-1 top-1 flex size-1.5">
            <span className="absolute inline-flex size-full animate-ping rounded-full bg-emerald-400 opacity-75 motion-reduce:hidden" />
            <span className="relative inline-flex size-1.5 rounded-full bg-emerald-500" />
          </span>
        )}
      </IconButton>

      {presence.present && createPortal(
        <div
          ref={panelRef}
          {...{ [PANEL_SCOPE_ATTR]: '' }}
          className={`kv-menu custom-scrollbar fixed z-40 overflow-y-auto overscroll-contain !rounded-2xl !p-0 ${motionClass(presence.closing)}`}
          style={{
            top: place.top,
            right: place.right,
            width: PANEL_WIDTH,
            maxHeight: `calc(100vh - ${place.top}px - 12px)`,
            ['--chat-popover-origin' as string]: 'top right',
          }}
          role={inline ? 'complementary' : 'dialog'}
          aria-label={t.workspaceMenu}
          aria-hidden={presence.closing || undefined}
          data-inline={inline || undefined}
          data-tauri-drag-region="false"
        >
          <PanelSection>
            <PanelRow
              icon={<Monitor className={PANEL_ICON} />}
              title={workdir}
              trailing={<FolderOpen className={PANEL_ICON} />}
              onClick={() => void run(() => dockApi.fsOpenPath(workdir, '', 'open'))}
            >
              <span className="min-w-0 flex-1 truncate">{name}</span>
            </PanelRow>
            <SplitRow
              icon={<Code2 className={PANEL_ICON} />}
              label={t.workspaceOpenIn.replace('{editor}', editorName(preferred))}
              onPrimary={() => openInEditor(preferred)}
              menuLabel={t.workspaceChooseEditor}
              menu={editorList.length > 1 ? (close) => editorList.map((id) => (
                <button key={id} type="button" role="menuitemradio" aria-checked={id === preferred} className={ITEM} onClick={() => { close(); openInEditor(id) }}>
                  <span className="min-w-0 flex-1 truncate">{editorName(id)}</span>
                  {id === preferred && <Check size={13} strokeWidth={2} className="shrink-0 text-neutral-500" />}
                </button>
              )) : null}
            />
          </PanelSection>

          {gitPending && (
            <PanelSection>
              <div role="status" aria-label={t.dockLoading}>
                <PanelSkeletonRows widths={[72, 120, 96]} />
              </div>
            </PanelSection>
          )}

          {showGit && state && !gitPending && (isRepo ? (
            <PanelSection>
              <SplitRow
                icon={busy === 'branch' ? <Loader2 className={`${PANEL_ICON} animate-spin`} /> : <GitBranch className={PANEL_ICON} />}
                label={state.head || 'HEAD'}
                meta={syncMeta}
                disabled={Boolean(busy)}
                menuLabel={t.dockGitBranchLabel}
                menu={(close) => (branches ?? []).length === 0 ? (
                  <div className="px-3 py-2 text-[11px] text-neutral-400">{branches === null ? t.dockLoading : t.dockGitBranchesEmpty}</div>
                ) : (branches ?? []).map((branch) => (
                  <button key={branch.name} type="button" role="menuitemradio" aria-checked={branch.current} disabled={branch.current || Boolean(busy)} className={`${ITEM} ${branch.current ? 'font-medium disabled:opacity-100' : ''}`} onClick={() => { close(); switchBranch(branch) }}>
                    <span className="min-w-0 flex-1 truncate">{branch.name}</span>
                    {branch.current && <Check size={13} strokeWidth={2} className="shrink-0 text-neutral-500" />}
                  </button>
                ))}
              />
              <SplitRow
                icon={busy && busy !== 'branch' && busy !== 'init' ? <Loader2 className={`${PANEL_ICON} animate-spin`} /> : quick.action === 'commit' ? <GitCommitHorizontal className={PANEL_ICON} /> : <CloudUpload className={PANEL_ICON} />}
                label={busy && busy !== 'branch' && busy !== 'init' ? busyLabel[busy] : actionLabel[quick.action]}
                meta={quick.kind === 'hint' && quick.hint !== 'busy' && !busy ? hintShort[quick.hint] : undefined}
                disabled={quick.kind === 'hint'}
                hint={quick.kind === 'hint' ? hintLabel[quick.hint] || undefined : undefined}
                onPrimary={() => { if (quick.kind === 'run') startAction(quick.action) }}
                menuLabel={t.workspaceGitOptions}
                menu={(close) => (
                  <>
                    <button type="button" role="menuitem" disabled={!menuState.commit} className={ITEM} onClick={() => { close(); startAction('commit') }}>
                      <GitCommitHorizontal size={14} strokeWidth={1.8} className="shrink-0" />
                      <span className="min-w-0 flex-1">{t.workspaceCommit}</span>
                    </button>
                    <button type="button" role="menuitem" disabled={!menuState.commit_push} className={ITEM} onClick={() => { close(); startAction('commit_push') }}>
                      <CloudUpload size={14} strokeWidth={1.8} className="shrink-0" />
                      <span className="min-w-0 flex-1">{t.workspaceCommitPush}</span>
                    </button>
                    <button type="button" role="menuitem" disabled={!menuState.push} className={ITEM} onClick={() => { close(); startAction('push') }}>
                      <CloudUpload size={14} strokeWidth={1.8} className="shrink-0" />
                      <span className="min-w-0 flex-1">{t.workspacePush}</span>
                    </button>
                    {remotesLoaded && !hasRemote && (
                      <div className="px-3 py-1.5 text-[11px] text-neutral-400">{t.workspaceHintNoRemote}</div>
                    )}
                  </>
                )}
              />
              <PanelRow icon={<FileDiff className={PANEL_ICON} />} onClick={onOpenGitPanel ? () => { setOpen(false); onOpenGitPanel() } : undefined}>
                <span className="flex min-w-0 flex-1 items-baseline gap-1.5">
                  <span className="shrink-0">{t.workspaceChanges}</span>
                  <span className="min-w-0 truncate font-normal text-neutral-400">
                    {state.entries.length > 0 ? t.workspaceFilesChanged.replace('{n}', String(state.entries.length)) : t.workspaceClean}
                  </span>
                  {diffStat ? (
                    <span key={`${diffStat.additions}:${diffStat.deletions}`} className="chat-motion-pop ml-auto flex shrink-0 gap-2 ps-2 font-mono text-[12px] tabular-nums">
                      <span className={diffStat.additions ? 'text-emerald-600 dark:text-emerald-400' : 'text-neutral-400'}>+{diffStat.additions.toLocaleString()}</span>
                      <span className={diffStat.deletions ? 'text-red-500 dark:text-red-400' : 'text-neutral-400'}>-{diffStat.deletions.toLocaleString()}</span>
                    </span>
                  ) : gitLoading || state.entries.length > 0 ? (
                    <span aria-hidden className="kv-skeleton ml-auto h-3 w-14 shrink-0 self-center rounded" />
                  ) : null}
                </span>
              </PanelRow>
            </PanelSection>
          ) : state.status === 'not_repo' ? (
            <PanelSection>
              <PanelRow
                icon={busy === 'init' ? <Loader2 className={`${PANEL_ICON} animate-spin`} /> : <GitBranch className={PANEL_ICON} />}
                disabled={Boolean(busy)}
                onClick={() => void initRepo()}
              >
                <span className="min-w-0 flex-1 truncate">{t.dockGitInitRepo}</span>
              </PanelRow>
            </PanelSection>
          ) : null)}

          {conversationSection}

          {roster && roster.agents.length > 0 && (
            <PanelSection>
              <div className="flex items-center justify-between px-2.5 pb-1 pt-1.5 text-[12px] font-medium text-neutral-400">
                <span>{t.workspaceSubagents}</span>
                {roster.runningCount > 0 && (
                  <span className="inline-flex items-center gap-1.5 font-normal text-emerald-600 dark:text-emerald-400">
                    <span aria-hidden className="size-1.5 animate-pulse rounded-full bg-emerald-500 motion-reduce:animate-none" />
                    {t.workspaceSubagentsRunningShort.replace('{n}', String(roster.runningCount))}
                  </span>
                )}
              </div>
              <div className="custom-scrollbar max-h-[13.5rem] overflow-y-auto">
                {roster.agents.map((agent) => (
                  <div key={agent.id} className="group/agent flex items-center">
                    <PanelRow
                      icon={<AgentAvatar agentId={agent.id} />}
                      title={agent.activity ?? undefined}
                      onClick={() => { setOpen(false); if (conversationId) requestDockSubagent({ conversationId, agentId: agent.id }) }}
                    >
                      <span className="min-w-0 flex-1 truncate">{agent.displayName || agent.id}</span>
                      <AgentStatus state={agent.status} zh={lang === 'zh'} />
                    </PanelRow>
                    {agent.status === 'running' && conversationId && (
                      <IconButton
                        label={t.workspaceStopSubagent.replace('{name}', agent.displayName || agent.id)}
                        size="sm"
                        variant="ghost"
                        className="shrink-0 opacity-0 transition-opacity focus-visible:opacity-100 group-hover/agent:opacity-100"
                        onClick={() => void run(async () => {
                          await subagentsApi.kill({ conversationId, agentId: agent.id, expectedGeneration: agent.generation })
                          refreshRoster()
                        })}
                      >
                        <Square size={12} strokeWidth={2} />
                      </IconButton>
                    )}
                  </div>
                ))}
              </div>
            </PanelSection>
          )}
        </div>,
        document.body)}

      {commitDialog && (
        <CommitDialog
          lang={lang}
          branch={state?.head ?? ''}
          fileCount={isRepo ? state.entries.length : 0}
          action={commitDialog}
          message={message}
          onMessage={setMessage}
          onSubmit={() => void submitCommit()}
          onClose={() => setCommitDialog(null)}
        />
      )}
      {branchDialog && historyBranches && conversationId && (
        <ConversationBranchDialog
          key={conversationId}
          conversationId={conversationId}
          disabled={historyBranches.disabled}
          lang={lang}
          onSwitch={historyBranches.onSwitch}
          onClose={() => setBranchDialog(false)}
        />
      )}
    </div>
  )
}

/** 对应 T3 的 Commit 对话框：分支、将提交的范围与提交信息，确认后执行。 */
function CommitDialog({ lang, branch, fileCount, action, message, onMessage, onSubmit, onClose }: {
  lang: Lang
  branch: string
  fileCount: number
  action: 'commit' | 'commit_push'
  message: string
  onMessage: (value: string) => void
  onSubmit: () => void
  onClose: () => void
}) {
  const t = i18n[lang]
  const [dialog, setDialog] = useState<HTMLDialogElement | null>(null)
  useEffect(() => { dialog?.showModal() }, [dialog])
  const title = action === 'commit_push' ? t.workspaceCommitPush : t.workspaceCommit
  return (
    <dialog ref={setDialog} className="kv-modal" aria-label={title} onCancel={(event) => { event.preventDefault(); onClose() }}>
      <form onSubmit={(event) => { event.preventDefault(); onSubmit() }}>
        <h2>{title}</h2>
        <p className="kv-market-muted">{t.dockGitBranchLabel}: {branch}</p>
        <p className="kv-market-muted">{t.workspaceCommitScope.replace('{n}', String(fileCount))}</p>
        <label className="kv-market-field"><span>{t.workspaceCommitMessage}</span>
          <Input autoFocus aria-label={t.workspaceCommitMessage} value={message} onChange={onMessage} />
        </label>
        <footer className="kv-market-dialog-actions">
          <Button variant="ghost" onClick={onClose}>{t.workspaceCancel}</Button>
          <Button type="submit" variant="primary" disabled={!message.trim()}>{title}</Button>
        </footer>
      </form>
    </dialog>
  )
}
