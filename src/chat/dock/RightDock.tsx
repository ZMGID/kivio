// Right Dock 容器：tab 条（文件树 / Git / PR / 终端 / 任务）+ 常驻面板 + 左缘拖拽调宽 + 折叠滑出。
// 宽度通过 CSS 变量 --chat-dock-width 直写（拖拽过程不触发 React 重渲），松手才持久化。
import { memo, useCallback, useEffect, useLayoutEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import { createPortal } from 'react-dom'
import { Activity, FolderTree, GitBranch, GitPullRequest, Plus, Terminal, X } from 'lucide-react'
import { i18n, type Lang } from '../../components/i18n'
import { IconButton } from '../../components/Button'
import { FileTreePanel } from './FileTreePanel'
import { GitPanel } from './GitPanel'
import { PullRequestsPanel } from './PullRequestsPanel'
import { TerminalPanel } from './TerminalPanel'
import { BackgroundTasksPanel } from './BackgroundTasksPanel'
import { SidebarToggleIcon } from '../SidebarToggleIcon'
import { SubagentSidebar } from '../AgentHub'
import type { DockSubagent } from './dockPreview'

export type DockTab = 'files' | 'git' | 'pulls' | 'terminal' | 'tasks'
export type DockOpenTabRequest = { tab: DockTab }

// 启动页入口：单键直达（仅启动页可见时生效，输入框内不拦截）。
const DOCK_ENTRIES = [
  { tab: 'files', key: 'f', icon: FolderTree, label: 'dockTabFiles' },
  { tab: 'git', key: 'g', icon: GitBranch, label: 'dockTabGit' },
  { tab: 'pulls', key: 'p', icon: GitPullRequest, label: 'dockTabPullRequests' },
  { tab: 'terminal', key: 't', icon: Terminal, label: 'dockTabTerminal' },
  { tab: 'tasks', key: 'k', icon: Activity, label: 'dockTabTasks' },
] as const

export const DOCK_MIN_WIDTH = 320
export const DOCK_MAX_WIDTH = 560
export const DOCK_DEFAULT_WIDTH = 360

export type DockRevealRequest = {
  path: string
  nonce: number
} | null

/** 工具卡片 → 右侧栏预览：点文件名预览文件（workdir 可以不同于文件树，如写到桌面的
 *  文件用其所在目录）；点 +N -N 徽标预览整份带色 diff；claude 提交计划时预览整份计划。 */
export type DockPreviewRequest =
  | { kind: 'file'; workdir: string; path: string; nonce: number }
  | { kind: 'diff'; title: string; patch: string; nonce: number }
  | { kind: 'markdown'; title: string; text: string; nonce: number }
  | null

type RightDockProps = {
  open: boolean
  /**
   * 工具栏并入窗口标题栏带（Windows / Linux）。为 true 时只往 `toolbarHost` 里 portal，绝不回退到 Dock 内联；
   * 标题栏在 Dock 收起时会卸载宿主节点，若按「宿主为空」判断会在淡出中的面板里冒出一行内联工具栏。
   */
  toolbarInTitlebar?: boolean
  toolbarHost?: HTMLDivElement | null
  width: number
  activeTab: DockTab
  openTabRequest: DockOpenTabRequest | null
  workdir: string
  lang: Lang
  /** 当前对话 id（任务页按对话隔离）。null = 还没有对话。 */
  conversationId: string | null
  treeExpanded: string[]
  revealRequest: DockRevealRequest
  previewRequest: DockPreviewRequest
  subagent?: DockSubagent | null
  onBackFromSubagent?: () => void
  onToggleTab: (tab: DockTab) => void
  onOpenTabRequestHandled: (request: DockOpenTabRequest) => void
  onWidthChange: (width: number) => void
  onClose: () => void
  onTreeExpandedChange: (paths: string[]) => void
  onInsertMention?: (path: string) => void
  onRevealInTree: (path: string) => void
}

export const RightDock = memo(function RightDock({
  open,
  toolbarInTitlebar = false,
  toolbarHost,
  width,
  activeTab,
  openTabRequest,
  workdir,
  lang,
  conversationId,
  treeExpanded,
  revealRequest,
  previewRequest,
  subagent,
  onBackFromSubagent,
  onToggleTab,
  onOpenTabRequestHandled,
  onWidthChange,
  onClose,
  onTreeExpandedChange,
  onInsertMention,
  onRevealInTree,
}: RightDockProps) {
  const t = i18n[lang]
  const shellRef = useRef<HTMLElement>(null)
  const dragStateRef = useRef<{ startX: number; startWidth: number; width: number; raf: number } | null>(null)

  const handleResizeStart = useCallback(
    (e: ReactPointerEvent<HTMLDivElement>) => {
      e.preventDefault()
      const startWidth = shellRef.current?.getBoundingClientRect().width ?? width
      dragStateRef.current = { startX: e.clientX, startWidth, width: startWidth, raf: 0 }

      const applyWidth = (nextWidth: number) => {
        shellRef.current?.style.setProperty('--chat-dock-width', `${nextWidth}px`)
        toolbarHost?.parentElement?.style.setProperty('--chat-dock-width', `${nextWidth}px`)
      }

      const onMove = (moveEvent: PointerEvent) => {
        const state = dragStateRef.current
        if (!state) return
        // 左缘手柄：向左拖变宽。
        const delta = state.startX - moveEvent.clientX
        const nextWidth = Math.min(DOCK_MAX_WIDTH, Math.max(DOCK_MIN_WIDTH, Math.round(state.startWidth + delta)))
        state.width = nextWidth
        if (!state.raf) {
          state.raf = window.requestAnimationFrame(() => {
            state.raf = 0
            applyWidth(state.width)
          })
        }
      }
      const onUp = () => {
        window.removeEventListener('pointermove', onMove)
        window.removeEventListener('pointerup', onUp)
        const state = dragStateRef.current
        dragStateRef.current = null
        if (!state) return
        if (state.raf) window.cancelAnimationFrame(state.raf)
        applyWidth(state.width)
        if (state.width !== Math.round(startWidth)) onWidthChange(state.width)
      }
      window.addEventListener('pointermove', onMove)
      window.addEventListener('pointerup', onUp)
    },
    [width, onWidthChange, toolbarHost],
  )

  // 启动页：没有已打开面板，或点了 + 时显示；此时所有面板都不算活动。
  const [openTabs, setOpenTabs] = useState<DockTab[]>([])
  const [launcher, setLauncher] = useState(false)
  const showLauncher = launcher || openTabs.length === 0
  const shown = showLauncher ? null : activeTab
  const filesActive = open && shown === 'files'
  const gitActive = open && shown === 'git'
  const pullsActive = open && shown === 'pulls'
  const terminalActive = open && shown === 'terminal'
  const selectedSubagent = open && shown === 'tasks' && subagent?.conversationId === conversationId ? subagent : null
  // 代码提前加载，PTY 和画布只在首次真正打开终端时创建，之后切标签保留会话。
  // Dock 折叠时即使记住了终端标签，也不能提前启动 shell。
  const [terminalMounted, setTerminalMounted] = useState(() => terminalActive)
  useEffect(() => {
    if (terminalActive) setTerminalMounted(true)
  }, [terminalActive])
  // 折叠后仍挂载（用于滑出动画），卡片被负 margin 推到窗口外；不设 inert 的话 Tab 会聚焦到
  // 屏外按钮，浏览器为了露出焦点把 .chat-window-shell 横向滚走一个面板宽度。同 Sidebar 的做法。
  useLayoutEffect(() => {
    const el = shellRef.current
    if (el) el.inert = !open
  }, [open])
  const chooseEntry = useCallback(
    (tab: DockTab) => {
      setOpenTabs((prev) => (prev.includes(tab) ? prev : [...prev, tab]))
      setLauncher(false)
      onToggleTab(tab)
    },
    [onToggleTab],
  )
  const closeTab = (tab: DockTab) => {
    const next = openTabs.filter((x) => x !== tab)
    setOpenTabs(next)
    if (activeTab === tab && next.length) onToggleTab(next[next.length - 1])
  }
  useEffect(() => {
    if (!openTabRequest) return
    if (open && activeTab === openTabRequest.tab) {
      setOpenTabs((prev) => (prev.includes(activeTab) ? prev : [...prev, activeTab]))
      setLauncher(false)
    }
    onOpenTabRequestHandled(openTabRequest)
  }, [activeTab, open, openTabRequest, onOpenTabRequestHandled])
  useEffect(() => {
    if (!open || !showLauncher) return
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey) return
      if ((e.target as HTMLElement | null)?.closest('input,textarea,[contenteditable]')) return
      const hit = DOCK_ENTRIES.find((entry) => entry.key === e.key.toLowerCase())
      if (!hit) return
      e.preventDefault()
      chooseEntry(hit.tab)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [open, showLauncher, chooseEntry])
  const toolbar = (
      /* 原生标题栏（macOS）下工具栏留在 Dock 内，与主区 52px 顶栏同高、同右缩进，
         开合按钮从主区顶栏移到这里时位置不变。并入标题栏带时沿用带的高度。 */
      <div
        className={`flex shrink-0 items-center gap-1 ${toolbarInTitlebar ? 'min-h-10 px-2 py-1' : 'h-[52px] pl-2 pr-3'}`}
        data-tauri-drag-region
      >
        <IconButton label={t.dockNewTab} size="sm" variant="ghost" onClick={() => setLauncher(true)}>
          <Plus size={14} />
        </IconButton>
        <div className="custom-scrollbar flex min-w-0 flex-1 items-center gap-1 overflow-x-auto">
        {openTabs.map((tab) => {
          const entry = DOCK_ENTRIES.find((x) => x.tab === tab)!
          const Icon = entry.icon
          return (
            <div
              key={tab}
              className={`flex shrink-0 items-center gap-1 rounded-lg pl-2.5 pr-1 text-[12.5px] transition-colors ${
                shown === tab
                  ? 'bg-[var(--accent-soft)] font-medium text-neutral-900 dark:text-neutral-100'
                  : 'text-neutral-500 hover:bg-neutral-500/8 hover:text-neutral-700 dark:text-neutral-400'
              }`}
            >
              <button type="button" className="flex items-center gap-1.5 py-1.5" onClick={() => chooseEntry(tab)}>
                <Icon size={13} strokeWidth={1.75} />
                {t[entry.label]}
              </button>
              <button
                type="button"
                aria-label={t.dockClose}
                className="rounded p-1 opacity-60 hover:opacity-100"
                onClick={() => closeTab(tab)}
              >
                <X size={11} />
              </button>
            </div>
          )
        })}
        </div>
        {!toolbarInTitlebar && <IconButton label={t.dockClose} size="sm" variant="ghost" className="group" aria-expanded={open} onClick={onClose}>
          <SidebarToggleIcon side="right" open={open} />
        </IconButton>}
      </div>
  )
  return (
    <aside
      ref={shellRef}
      className={`chat-dock-shell relative flex min-h-0 shrink-0 flex-col overflow-hidden${open ? '' : ' is-collapsed'}`}
      style={{ ['--chat-dock-width' as string]: `${width}px` }}
      aria-hidden={!open}
    >
      {/* 左缘拖拽手柄 */}
      <div
        className="absolute bottom-0 left-0 top-0 z-20 w-[6px] cursor-col-resize"
        onPointerDown={handleResizeStart}
      />

      {toolbarInTitlebar ? (open && toolbarHost && createPortal(toolbar, toolbarHost)) : toolbar}

      {showLauncher && (
        <div className="chat-dock-launcher custom-scrollbar">
          <div className="chat-dock-launcher-content">
            <h2 className="chat-dock-launcher-title">{t.dockLauncherTitle}</h2>
            {DOCK_ENTRIES.map(({ tab, key, icon: Icon, label }) => (
              <button
                key={tab}
                type="button"
                className="chat-dock-launcher-entry"
                aria-keyshortcuts={key}
                onClick={() => chooseEntry(tab)}
              >
                <Icon size={16} strokeWidth={1.75} aria-hidden="true" />
                <span>{t[label]}</span>
                <kbd>{key}</kbd>
              </button>
            ))}
          </div>
        </div>
      )}

      {/* 文件 / Git / PR / 终端保留面板；后台任务仅在任务页可见时读取。 */}
      <div className="flex min-h-0 flex-1 flex-col" hidden={shown !== 'files'}>
        <FileTreePanel
          workdir={workdir}
          active={filesActive}
          lang={lang}
          expandedPaths={treeExpanded}
          onExpandedChange={onTreeExpandedChange}
          revealPath={revealRequest?.path ?? null}
          revealNonce={revealRequest?.nonce ?? 0}
          previewRequest={previewRequest}
          onInsertMention={onInsertMention}
        />
      </div>
      <div className="flex min-h-0 flex-1 flex-col" hidden={shown !== 'git'}>
        <GitPanel workdir={workdir} active={gitActive} lang={lang} onRevealInTree={onRevealInTree} />
      </div>
      <div className="flex min-h-0 flex-1 flex-col" hidden={shown !== 'pulls'}>
        <PullRequestsPanel workdir={workdir} active={pullsActive} lang={lang} />
      </div>
      <div className="flex min-h-0 flex-1 flex-col" hidden={shown !== 'terminal'}>
        {terminalMounted && (
          <TerminalPanel workdir={workdir} active={terminalActive} lang={lang} />
        )}
      </div>
      <div data-task-scroll className="flex min-h-0 flex-1 flex-col overflow-y-auto custom-scrollbar" hidden={shown !== 'tasks'}>
        {selectedSubagent ? <SubagentSidebar key={JSON.stringify([selectedSubagent.conversationId, selectedSubagent.agentId])}
          target={selectedSubagent} onBack={onBackFromSubagent} /> : <BackgroundTasksPanel
          active={open && shown === 'tasks'}
          lang={lang}
          conversationId={conversationId}
        />}
      </div>
    </aside>
  )
})
