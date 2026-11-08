// 标题栏工作区面板，参照 T3 Code 的 ThreadDetailsPanel：
// 段一 = 项目行 / Open in 拆分按钮；段二 = 分支 / Git 主操作拆分按钮 / Changes（仅项目对话）；
// 段三 = 历史分支 / 任务；段四 = 子代理。
// Git 数据来自 useGitBadge（只在面板可见时订阅），主操作规则见 workspaceGitAction。
// 位置：浮层贴触发按钮右下展开；内联常驻时贴对话区右缘，并通过 --chat-details-inset 让出内容宽度。
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { Activity, ArrowDown, ArrowUp, Check, CloudDownload, CloudUpload, Code2, FileDiff, FolderOpen, GitBranch, GitCommitHorizontal, GitFork, Loader2, Monitor, SlidersHorizontal, Square, SquareMenu } from 'lucide-react'
import { IconButton } from '../../components/Button'
import { notifyError } from '../../components/appNoticeStore'
import { i18n, type Lang } from '../../components/i18n'
import type { GitBranchesResult } from '../../api/dockContracts'
import { getRememberedEditor, getWorkspaceHiddenItems, getWorkspacePanelOpen, rememberEditor, rememberWorkspaceHiddenItems, rememberWorkspacePanelOpen } from '../persistence'
import { chatApi, type ConversationBranch } from '../api'
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
import { defaultCommitMessage, resolveGitMenu, resolveGitQuickAction, type GitAction, type GitHint } from './workspaceGitAction'
import { WorkspaceBranchPicker, type BranchPick } from './WorkspaceBranchPicker'
import { WorkspaceItemsMenu } from './WorkspaceItemsMenu'
import { WORKSPACE_ITEMS, type WorkspaceItemId } from './workspaceItems'

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
  /** 空白新对话：内联常驻默认不展开，用户点开后才显示。 */
  blank?: boolean
}

const EDITOR_LABELS: Record<string, string> = { vscode: 'VS Code', cursor: 'Cursor', zed: 'Zed', windsurf: 'Windsurf' }
const ITEM = 'kv-menu-row w-full text-neutral-800 transition-colors hover:bg-neutral-100 disabled:opacity-50'
/** 动作结果在按钮原位停留的时长。 */
const SUCCESS_VISIBLE_MS = 4000
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
  blank = false,
}: WorkspaceActionsMenuProps) {
  const t = i18n[lang]
  const inline = useInlineRoom(inlineAllowed && !dockOpen, conversationId)
  // 用户按对话的展开/收起选择；没选过时，空白新对话默认不展开（T3 也让空草稿保持安静）。
  const [choice, setChoiceState] = useState<boolean | null>(() => conversationId ? getWorkspacePanelOpen(conversationId) : null)
  useEffect(() => { setChoiceState(conversationId ? getWorkspacePanelOpen(conversationId) : null) }, [conversationId])
  const setChoice = useCallback((next: boolean) => {
    setChoiceState(next)
    // 只记内联常驻的选择；浮层是临时的，不影响下次。
    if (conversationId && inline) rememberWorkspacePanelOpen(conversationId, next)
  }, [conversationId, inline])
  const open = inline ? (choice ?? !blank) : choice === true
  const presence = usePresence(open)
  const setOpen = (next: boolean) => { if (next || !inline) setChoice(next) }
  const showGit = Boolean(projectName)
  const { state, diffStat, loading: gitLoading, applyMutationState, refresh } = useGitBadge(open && showGit ? workdir : '', true)
  const tasks = useBackgroundTasks(conversationId)
  const runningTasks = partitionTasks(tasks).running.length
  const [place, setPlace] = useState<{ top: number; right: number }>({ top: 0, right: 8 })
  const wrapRef = useRef<HTMLDivElement>(null)
  const panelRef = useRef<HTMLDivElement>(null)
  const [busy, setBusy] = useState<GitAction | 'branch' | 'init' | null>(null)
  /** 动作完成后在按钮原位显示几秒结果，代替弹窗/通知（参照 T3 的 inline success）。 */
  const [done, setDone] = useState<string | null>(null)
  useEffect(() => {
    if (!done) return
    const timer = window.setTimeout(() => setDone(null), SUCCESS_VISIBLE_MS)
    return () => window.clearTimeout(timer)
  }, [done])
  const [editors, setEditors] = useState<string[] | null>(null)
  const [branchData, setBranchData] = useState<GitBranchesResult | null>(null)
  const [hiddenItems, setHiddenItems] = useState<string[]>(() => getWorkspaceHiddenItems())
  const [itemsMenu, setItemsMenu] = useState<{ x: number; y: number } | null>(null)
  const show = Object.fromEntries(WORKSPACE_ITEMS.map((id) => [id, !hiddenItems.includes(id)])) as Record<WorkspaceItemId, boolean>
  const allHidden = WORKSPACE_ITEMS.every((id) => !show[id])
  const toggleItem = (id: WorkspaceItemId) => {
    setHiddenItems((prev) => {
      const next = prev.includes(id) ? prev.filter((item) => item !== id) : [...prev, id]
      rememberWorkspaceHiddenItems(next)
      return next
    })
  }
  const [historyList, setHistoryList] = useState<ConversationBranch[] | null>(null)
  const [historySwitching, setHistorySwitching] = useState(false)
  const { roster, refresh: refreshRoster } = useSubagentRoster(open ? conversationId : null)

  const historyEnabled = Boolean(historyBranches)
  // 历史分支随面板打开读取，作为下拉直接切换，不再弹对话框。
  useEffect(() => {
    setHistoryList(null)
    if (!open || !conversationId || !historyEnabled) return
    let cancelled = false
    chatApi.getBranches(conversationId).then((list) => { if (!cancelled) setHistoryList(list) }).catch(() => { if (!cancelled) setHistoryList([]) })
    return () => { cancelled = true }
  }, [open, conversationId, historyEnabled])

  // 浮层在切到内联模式时作废，避免回到浮层模式后自己弹出来。
  useEffect(() => { if (!inline) setChoiceState((prev) => (prev === true ? null : prev)) }, [inline])
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
  }, [open, inline, setChoice])

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
      if (!cancelled) setBranchData(result)
    }).catch(() => {
      if (!cancelled) setBranchData({ branches: [], remotes: [], remoteBranches: [], defaultBranch: '' })
    })
    return () => { cancelled = true }
  }, [workdir])
  useEffect(() => {
    if (!open || !isRepo) return
    return loadBranches()
  }, [open, isRepo, head, loadBranches])
  useEffect(() => { setBranchData(null) }, [workdir])

  const name = projectName || workdir.split(/[\\/]/).filter(Boolean).pop() || workdir
  const editorList = editors ?? []
  // PATH 探测为空时仍显示 VS Code 行：真的找不到时由后端报错提示。
  const preferred = editorList.find((id) => id === getRememberedEditor()) ?? editorList[0] ?? 'vscode'
  const editorName = (id: string) => EDITOR_LABELS[id] ?? id
  const remotesLoaded = branchData !== null
  const hasRemote = (branchData?.remotes.length ?? 0) > 0
  const quick = resolveGitQuickAction(isRepo ? state : null, hasRemote, Boolean(busy) || !remotesLoaded)
  const menuState = resolveGitMenu(isRepo ? state : null, hasRemote, Boolean(busy) || !remotesLoaded)
  const gitPending = showGit && (!state || (isRepo && !remotesLoaded))
  const syncMeta = isRepo && (state.ahead > 0 || state.behind > 0) ? (
    <span className="inline-flex items-center gap-1.5" title={t.workspaceSyncTitle.replace('{ahead}', String(state.ahead)).replace('{behind}', String(state.behind))}>
      {state.ahead > 0 && <span className="inline-flex items-center"><ArrowUp className="size-3" />{state.ahead}</span>}
      {state.behind > 0 && <span className="inline-flex items-center"><ArrowDown className="size-3" />{state.behind}</span>}
    </span>
  ) : null
  const actionLabel: Record<GitAction, string> = {
    commit: t.workspaceCommit,
    commit_push: t.workspaceCommitPush,
    push: t.workspacePush,
    pull: t.workspacePull,
  }
  const busyLabel: Record<GitAction, string> = {
    commit: t.workspaceCommitting,
    commit_push: t.workspaceCommittingPushing,
    push: t.workspacePushing,
    pull: t.workspacePulling,
  }
  const doneLabel: Record<GitAction, string> = {
    commit: t.workspaceCommitted,
    commit_push: t.workspaceCommittedPushed,
    push: t.workspacePushed,
    pull: t.workspacePulled,
  }
  const hintLabel: Record<GitHint, string> = {
    busy: '',
    unavailable: t.workspaceHintUnavailable,
    detached: t.workspaceHintDetached,
    noRemote: t.workspaceHintNoRemote,
    diverged: t.workspaceHintDiverged,
    upToDate: t.workspaceHintUpToDate,
  }
  /** 行内放得下的短说明，完整原因在悬停提示里。 */
  const hintShort: Record<GitHint, string> = {
    busy: '',
    unavailable: t.workspaceShortUnavailable,
    detached: t.workspaceShortDetached,
    noRemote: t.workspaceShortNoRemote,
    diverged: t.workspaceShortDiverged,
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
    void run(() => dockApi.openInEditor(workdir, editor))
  }

  const pickBranch = (pick: BranchPick) => {
    if (busy) return
    setBusy('branch')
    void run(async () => {
      const result = pick.kind === 'create'
        ? await dockApi.gitCreateBranch(workdir, pick.name)
        : await dockApi.gitSwitchBranch(workdir, pick.name)
      applyMutationState(result.state)
      if (!result.ok) throw new Error(result.message)
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
    setDone(null)
    await run(async () => {
      if (action === 'pull') {
        const pulled = await dockApi.gitPull(workdir)
        applyMutationState(pulled.state)
        if (!pulled.ok) throw new Error(pulled.message)
      } else {
        if (action !== 'push') {
          // 和 T3 一样直接执行、不弹窗：提交信息按改动文件自动生成。
          const entries = isRepo ? state.entries : []
          const staged = await dockApi.gitStageAll(workdir)
          if (!staged.ok) throw new Error(staged.message)
          const committed = await dockApi.gitCommit(workdir, defaultCommitMessage(entries))
          applyMutationState(committed.state)
          if (!committed.ok) throw new Error(committed.message)
        }
        if (action !== 'commit') {
          const pushed = await dockApi.gitPush(workdir)
          applyMutationState(pushed.state)
          if (!pushed.ok) throw new Error(pushed.message)
        }
      }
      setDone(doneLabel[action])
      void refresh({ silent: true })
      loadBranches()
    })
    setBusy(null)
  }

  const switchHistory = (branch: ConversationBranch) => {
    if (!historyBranches || branch.active || historySwitching) return
    setHistorySwitching(true)
    void historyBranches.onSwitch(branch.id).then((applied) => {
      if (!applied || !conversationId) return
      return chatApi.getBranches(conversationId).then(setHistoryList)
    }).catch((err) => notifyError(err instanceof Error ? err.message : String(err)))
      .finally(() => setHistorySwitching(false))
  }
  const activeHistory = historyList?.find((branch) => branch.active)

  const tasksLabel = runningTasks > 0 ? t.chatBgJobsRunning.replace('{n}', String(runningTasks)) : t.dockTabTasks
  const showHistoryRow = show.history && Boolean(historyBranches) && (historyList?.length ?? 0) > 1
  const conversationSection = conversationId && (showHistoryRow || show.tasks) ? (
    <PanelSection>
      {showHistoryRow && historyBranches && (
        <SplitRow
          icon={historySwitching ? <Loader2 className={`${PANEL_ICON} animate-spin`} /> : <GitFork className={PANEL_ICON} />}
          label={activeHistory?.label ?? t.workspaceHistoryBranches}
          meta={t.workspaceHistoryCount.replace('{n}', String(historyList?.length ?? 0))}
          disabled={historyBranches.disabled || historySwitching}
          menuLabel={t.workspaceHistoryBranches}
          menu={(close) => (historyList ?? []).map((branch) => (
            <button key={branch.id} type="button" role="menuitemradio" aria-checked={branch.active} disabled={branch.active || historyBranches.disabled} className={`${ITEM} ${branch.active ? 'font-medium disabled:opacity-100' : ''}`} onClick={() => { close(); switchHistory(branch) }}>
              <span className="min-w-0 flex-1 truncate">{branch.label}</span>
              {branch.active && <Check size={13} strokeWidth={2} className="shrink-0 text-neutral-500" />}
            </button>
          ))}
        />
      )}
      {show.tasks && <PanelRow
        icon={<Activity className={`${PANEL_ICON} ${runningTasks > 0 ? '!text-emerald-500' : ''}`} />}
        onClick={onOpenTasks ? () => { setOpen(false); onOpenTasks() } : undefined}
      >
        <span className="min-w-0 flex-1 truncate">{t.dockTabTasks}</span>
        {runningTasks > 0 && (
          <span key={runningTasks} title={tasksLabel} aria-label={tasksLabel} className="chat-motion-pop shrink-0 rounded-full bg-emerald-500/10 px-1.5 text-[11px] font-medium tabular-nums text-emerald-600 dark:text-emerald-400">
            {runningTasks}
          </span>
        )}
      </PanelRow>}
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
          onContextMenu={(event) => {
            // 参照 T3：右键面板空白处选择显示哪些项；输入框和选中的文字保留系统菜单。
            const target = event.target as Element
            if (target.closest('input, textarea') || !window.getSelection()?.isCollapsed) return
            event.preventDefault()
            setItemsMenu({ x: event.clientX, y: event.clientY })
          }}
        >
          {itemsMenu && (
            <WorkspaceItemsMenu
              at={itemsMenu}
              lang={lang}
              hidden={hiddenItems}
              onToggle={toggleItem}
              onClose={() => setItemsMenu(null)}
            />
          )}
          {allHidden && (
            <PanelSection>
              <PanelRow
                icon={<SlidersHorizontal className={PANEL_ICON} />}
                onClick={() => {
                  const rect = panelRef.current?.getBoundingClientRect()
                  setItemsMenu({ x: rect?.left ?? 0, y: (rect?.top ?? 0) + 36 })
                }}
              >
                <span className="min-w-0 flex-1 truncate">{t.workspaceCustomize}</span>
              </PanelRow>
            </PanelSection>
          )}
          {(show.openIn || show.project) && <PanelSection>
            {show.project && <PanelRow
              icon={<Monitor className={PANEL_ICON} />}
              title={workdir}
              trailing={<FolderOpen className={PANEL_ICON} />}
              onClick={() => void run(() => dockApi.fsOpenPath(workdir, '', 'open'))}
            >
              <span className="min-w-0 flex-1 truncate">{name}</span>
            </PanelRow>}
            {show.openIn && <SplitRow
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
            />}
          </PanelSection>}

          {gitPending && (show.branch || show.gitActions || show.changes) && (
            <PanelSection>
              <div role="status" aria-label={t.dockLoading}>
                <PanelSkeletonRows widths={[72, 120, 96]} />
              </div>
            </PanelSection>
          )}

          {showGit && state && !gitPending && (isRepo ? ((show.branch || show.gitActions || show.changes) &&
            <PanelSection>
              {show.branch && <SplitRow
                icon={busy === 'branch' ? <Loader2 className={`${PANEL_ICON} animate-spin`} /> : <GitBranch className={PANEL_ICON} />}
                label={state.head || 'HEAD'}
                meta={syncMeta}
                disabled={Boolean(busy)}
                menuLabel={t.dockGitBranchLabel}
                menu={(close) => (
                  <WorkspaceBranchPicker
                    data={branchData}
                    lang={lang}
                    disabled={Boolean(busy)}
                    onPick={(pick) => { close(); pickBranch(pick) }}
                  />
                )}
              />}
              {show.gitActions && <SplitRow
                icon={done ? <Check className={`${PANEL_ICON} !text-emerald-500`} /> : busy && busy !== 'branch' && busy !== 'init' ? <Loader2 className={`${PANEL_ICON} animate-spin`} /> : quick.action === 'pull' ? <CloudDownload className={PANEL_ICON} /> : quick.action === 'commit' ? <GitCommitHorizontal className={PANEL_ICON} /> : <CloudUpload className={PANEL_ICON} />}
                label={done ? <span role="status" aria-live="polite">{done}</span> : busy && busy !== 'branch' && busy !== 'init' ? busyLabel[busy] : actionLabel[quick.action]}
                meta={!done && quick.kind === 'hint' && quick.hint !== 'busy' && !busy ? hintShort[quick.hint] : undefined}
                disabled={Boolean(done) || quick.kind === 'hint'}
                hint={quick.kind === 'hint' ? hintLabel[quick.hint] || undefined : undefined}
                onPrimary={() => { if (quick.kind === 'run') void gitAction(quick.action) }}
                menuLabel={t.workspaceGitOptions}
                menu={(close) => (
                  <>
                    <button type="button" role="menuitem" disabled={!menuState.commit} className={ITEM} onClick={() => { close(); void gitAction('commit') }}>
                      <GitCommitHorizontal size={14} strokeWidth={1.8} className="shrink-0" />
                      <span className="min-w-0 flex-1">{t.workspaceCommit}</span>
                    </button>
                    <button type="button" role="menuitem" disabled={!menuState.commit_push} className={ITEM} onClick={() => { close(); void gitAction('commit_push') }}>
                      <CloudUpload size={14} strokeWidth={1.8} className="shrink-0" />
                      <span className="min-w-0 flex-1">{t.workspaceCommitPush}</span>
                    </button>
                    <button type="button" role="menuitem" disabled={!menuState.push} className={ITEM} onClick={() => { close(); void gitAction('push') }}>
                      <CloudUpload size={14} strokeWidth={1.8} className="shrink-0" />
                      <span className="min-w-0 flex-1">{t.workspacePush}</span>
                    </button>
                    <button type="button" role="menuitem" disabled={!menuState.pull} className={ITEM} onClick={() => { close(); void gitAction('pull') }}>
                      <CloudDownload size={14} strokeWidth={1.8} className="shrink-0" />
                      <span className="min-w-0 flex-1">{t.workspacePull}</span>
                    </button>
                    {remotesLoaded && !hasRemote && (
                      <div className="px-3 py-1.5 text-[11px] text-neutral-400">{t.workspaceHintNoRemote}</div>
                    )}
                  </>
                )}
              />}
              {show.changes && <PanelRow icon={<FileDiff className={PANEL_ICON} />} onClick={onOpenGitPanel ? () => { setOpen(false); onOpenGitPanel() } : undefined}>
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
              </PanelRow>}
            </PanelSection>
          ) : state.status === 'not_repo' && show.gitActions ? (
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

          {show.subagents && roster && roster.agents.length > 0 && (
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
    </div>
  )
}
