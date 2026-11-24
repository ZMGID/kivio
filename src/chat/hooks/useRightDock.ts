import { useCallback, useEffect, useState } from 'react'
import { peekSettings, subscribeSettings } from '../../api/settingsCache'
import { notifyError } from '../../components/appNoticeStore'
import { useT } from '../../components/i18n'
import type { AgentRuntimeConfig } from '../api'
import { dockApi } from '../dock/api'
import {
  onDockDiffPreviewRequest,
  onDockMarkdownPreviewRequest,
  onDockPreviewRequest,
} from '../dock/dockPreview'
import { resolveDockPreviewTarget } from '../dock/dockPreviewTarget'
import type { DockOpenTabRequest, DockPreviewRequest, DockRevealRequest, DockTab } from '../dock/RightDock'
import {
  getRememberedDockOpen,
  getRememberedDockTab,
  getRememberedDockWidth,
  getRememberedTreeExpanded,
  rememberDockOpen,
  rememberDockTab,
  rememberDockWidth,
  rememberTreeExpanded,
} from '../persistence'
import { formatComposerFileMention } from '../slashCommands'
import { insertTextIntoComposer } from '../composerInsert'
import { useDockSubagent } from './useDockSubagent'

export interface UseRightDockOptions {
  conversationId: string | null
  projectId: string | null
  /** 外部 agent 与内置 runtime 的实际写入目录不同，runtime 切换必须重解析 workdir。 */
  agentRuntimeKind: AgentRuntimeConfig['kind']
  worktreePath?: string | null
}

/**
 * 右侧 IDE dock 的页面级 owner：开合 / 宽度 / 页签 / 文件树展开态（均带 localStorage
 * 持久化）、跟随会话解析的 workdir，以及来自消息区工具卡片的预览 / 定位请求。
 * 工具卡片经 dockPreview 单监听信道到这里，再转成 RightDock 认得的请求对象。
 */
export function useRightDock({
  conversationId,
  projectId,
  agentRuntimeKind,
  worktreePath,
}: UseRightDockOptions) {
  const t = useT()
  const [open, setOpen] = useState(() => getRememberedDockOpen())
  const [width, setWidthState] = useState(() => getRememberedDockWidth())
  const [tab, setTabState] = useState<DockTab>(() => getRememberedDockTab())
  const [openTabRequest, setOpenTabRequest] = useState<DockOpenTabRequest | null>(null)
  const [resolvedWorkdir, setWorkdir] = useState('')
  const workdir = worktreePath || resolvedWorkdir
  const [workingRoot, setWorkingRoot] = useState(() =>
    peekSettings()?.chatTools.nativeTools.workingDirectory,
  )
  useEffect(() => subscribeSettings((settings) => {
    setWorkingRoot(settings.chatTools.nativeTools.workingDirectory)
  }), [])
  const [treeExpanded, setTreeExpandedState] = useState<string[]>([])
  const [reveal, setReveal] = useState<DockRevealRequest>(null)
  const [preview, setPreview] = useState<DockPreviewRequest>(null)

  // 文件树展开状态按 workdir 持久化，workdir 切换时重新载入。
  useEffect(() => {
    setTreeExpandedState(workdir ? getRememberedTreeExpanded(workdir) : [])
  }, [workdir])

  const showTab = useCallback((next: DockTab) => {
    setTabState(next)
    setOpenTabRequest({ tab: next })
    rememberDockTab(next)
    setOpen(true)
    rememberDockOpen(true)
  }, [])

  // 按请求对象身份确认，避免旧请求的确认清除更新的请求。
  const onOpenTabRequestHandled = useCallback((request: DockOpenTabRequest) => {
    setOpenTabRequest((current) => current === request ? null : current)
  }, [])

  const toggle = useCallback(() => {
    setOpenTabRequest(null)
    setOpen((prev) => {
      rememberDockOpen(!prev)
      return !prev
    })
  }, [])

  const close = useCallback(() => {
    setOpenTabRequest(null)
    setOpen(false)
    rememberDockOpen(false)
  }, [])

  /** 输入栏 Git 胶囊「在 Git 面板中打开」。 */
  const openGit = useCallback(() => showTab('git'), [showTab])
  /** 标题栏后台任务状态灯。 */
  const openTasks = useCallback(() => showTab('tasks'), [showTab])
  const subagent = useDockSubagent(conversationId, openTasks)

  const setWidth = useCallback((nextWidth: number) => {
    setWidthState(nextWidth)
    rememberDockWidth(nextWidth)
  }, [])

  const setTab = useCallback((next: DockTab) => {
    setOpenTabRequest(null)
    setTabState(next)
    rememberDockTab(next)
  }, [])

  const setTreeExpanded = useCallback((paths: string[]) => {
    setTreeExpandedState(paths)
    if (workdir) rememberTreeExpanded(workdir, paths)
  }, [workdir])

  /** Git 面板「在文件树中定位」：切到文件 tab 并展开定位。 */
  const revealInTree = useCallback((path: string) => {
    showTab('files')
    setReveal((prev) => ({ path, nonce: (prev?.nonce ?? 0) + 1 }))
  }, [showTab])

  // A click during a conversation switch must wait for that conversation's directory.
  useEffect(() => {
    let active = true
    let cwd = worktreePath ?? ''
    let resolving = !cwd
    let pending: (() => void) | undefined
    setPreview(null)
    setReveal(null)
    setWorkdir(cwd)
    if (resolving) {
      void dockApi.resolveCwd(conversationId, projectId).catch(() => '').then((resolved) => {
        if (!active) return
        resolving = false
        cwd = resolved
        setWorkdir(cwd)
        pending?.()
      })
    }
    const unsubscribe = onDockPreviewRequest((rawPath, link) => {
      if (link?.conversationId && link.conversationId !== conversationId) return false
      const openFile = () => {
        const target = resolveDockPreviewTarget(rawPath, cwd)
        if (!target) {
          notifyError(t.chatMsgOpenLinkFailed + rawPath)
          return
        }
        showTab('files')
        const path = target.revealRel
        if (path) setReveal((prev) => ({ path, nonce: (prev?.nonce ?? 0) + 1 }))
        setPreview((prev) => ({
          kind: 'file', ...target.request, line: link?.line,
          readOnly: !!link && path === null, nonce: (prev?.nonce ?? 0) + 1,
        }))
      }
      if (resolving) pending = openFile
      else openFile()
      return true
    })
    return () => { active = false; unsubscribe() }
  }, [conversationId, projectId, agentRuntimeKind, workingRoot, worktreePath, showTab, t.chatMsgOpenLinkFailed])

  // 工具卡片点 +N -N 徽标 → dock 侧栏渲染整份带色 diff。
  useEffect(() => onDockDiffPreviewRequest((payload) => {
    showTab('files')
    setPreview((prev) => ({ kind: 'diff', ...payload, nonce: (prev?.nonce ?? 0) + 1 }))
  }), [showTab])

  // claude 交计划（ExitPlanMode）→ dock 侧栏渲染整份计划。审批卡里那块 `max-h-40` 的
  // 灰框只够扫一眼，而「批不批这个计划」是要读完才能决定的。
  useEffect(() => onDockMarkdownPreviewRequest((payload) => {
    showTab('files')
    setPreview((prev) => ({ kind: 'markdown', ...payload, nonce: (prev?.nonce ?? 0) + 1 }))
  }), [showTab])

  /** 文件树「插入 @ 引用」：经 composerInsert 文本信道注入输入框正文。 */
  const insertMention = useCallback((path: string) => {
    insertTextIntoComposer(`${formatComposerFileMention(path)} `)
  }, [])

  return {
    open,
    width,
    tab,
    openTabRequest,
    onOpenTabRequestHandled,
    workdir,
    treeExpanded,
    reveal,
    preview,
    toggle,
    close,
    openGit,
    openTasks,
    subagent,
    setWidth,
    setTab,
    setTreeExpanded,
    revealInTree,
    insertMention,
  }
}
