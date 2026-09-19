import { useCallback, useEffect, useState, type MutableRefObject } from 'react'
import type { AgentRuntimeConfig } from '../api'
import { dockApi } from '../dock/api'
import {
  onDockDiffPreviewRequest,
  onDockMarkdownPreviewRequest,
  onDockPreviewRequest,
  onDockSubAgentRequest,
} from '../dock/dockPreview'
import { resolveDockPreviewTarget } from '../dock/dockPreviewTarget'
import type { DockPreviewRequest, DockRevealRequest, DockTab } from '../dock/RightDock'
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
import { insertTextIntoComposer } from '../composerInsert'

export type DockSubAgentRequest = { conversationId: string; agentId: string; nonce: number } | null

export interface UseRightDockOptions {
  conversationId: string | null
  projectId: string | null
  /** 外部 agent 与内置 runtime 的实际写入目录不同，runtime 切换必须重解析 workdir。 */
  agentRuntimeKind: AgentRuntimeConfig['kind']
  /** 子代理面板请求只对当前会话生效；读 ref 而不是值，避免会话切换重订阅信道。 */
  currentConversationIdRef: MutableRefObject<string | null>
}

/**
 * 右侧 IDE dock 的页面级 owner：开合 / 宽度 / 页签 / 文件树展开态（均带 localStorage
 * 持久化）、跟随会话解析的 workdir，以及来自消息区工具卡片的预览 / 定位 / 子代理请求。
 * 工具卡片经 dockPreview 单监听信道到这里，再转成 RightDock 认得的请求对象。
 */
export function useRightDock({
  conversationId,
  projectId,
  agentRuntimeKind,
  currentConversationIdRef,
}: UseRightDockOptions) {
  const [open, setOpen] = useState(() => getRememberedDockOpen())
  const [width, setWidthState] = useState(() => getRememberedDockWidth())
  const [tab, setTabState] = useState<DockTab>(() => getRememberedDockTab())
  const [workdir, setWorkdir] = useState('')
  const [treeExpanded, setTreeExpandedState] = useState<string[]>([])
  const [reveal, setReveal] = useState<DockRevealRequest>(null)
  const [subAgentRequest, setSubAgentRequest] = useState<DockSubAgentRequest>(null)
  const [preview, setPreview] = useState<DockPreviewRequest>(null)

  // 工作目录跟随当前会话 / 选中项目 / agent runtime 变化，由后端 dock_resolve_cwd 解析。
  useEffect(() => {
    if (!conversationId && !projectId) {
      setWorkdir('')
      return
    }
    let cancelled = false
    dockApi
      .resolveCwd(conversationId, projectId)
      .then((cwd) => {
        if (!cancelled) setWorkdir(cwd)
      })
      .catch(() => {
        if (!cancelled) setWorkdir('')
      })
    return () => {
      cancelled = true
    }
  }, [conversationId, projectId, agentRuntimeKind])

  // 文件树展开状态按 workdir 持久化，workdir 切换时重新载入。
  useEffect(() => {
    setTreeExpandedState(workdir ? getRememberedTreeExpanded(workdir) : [])
  }, [workdir])

  const showTab = useCallback((next: DockTab) => {
    setTabState(next)
    rememberDockTab(next)
    setOpen(true)
    rememberDockOpen(true)
  }, [])

  const toggle = useCallback(() => {
    setOpen((prev) => {
      rememberDockOpen(!prev)
      return !prev
    })
  }, [])

  const close = useCallback(() => {
    setOpen(false)
    rememberDockOpen(false)
  }, [])

  /** 输入栏 Git 胶囊「在 Git 面板中打开」。 */
  const openGit = useCallback(() => showTab('git'), [showTab])
  /** 标题栏后台任务状态灯 / 子代理指示器。 */
  const openTasks = useCallback(() => showTab('tasks'), [showTab])

  useEffect(() => onDockSubAgentRequest((target) => {
    if (target.conversationId !== currentConversationIdRef.current) return
    openTasks()
    setSubAgentRequest((previous) => ({ ...target, nonce: (previous?.nonce ?? 0) + 1 }))
  }), [currentConversationIdRef, openTasks])

  const setWidth = useCallback((nextWidth: number) => {
    setWidthState(nextWidth)
    rememberDockWidth(nextWidth)
  }, [])

  const setTab = useCallback((next: DockTab) => {
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

  // 工具卡片点文件名 → dock 查看器预览（解析规则见 dockPreviewTarget）。
  useEffect(() => onDockPreviewRequest((rawPath) => {
    const target = resolveDockPreviewTarget(rawPath, workdir)
    if (!target) return
    showTab('files')
    if (target.revealRel) {
      const revealRel = target.revealRel
      setReveal((prev) => ({ path: revealRel, nonce: (prev?.nonce ?? 0) + 1 }))
    }
    setPreview((prev) => ({ kind: 'file', ...target.request, nonce: (prev?.nonce ?? 0) + 1 }))
  }), [showTab, workdir])

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
    insertTextIntoComposer(`@${path} `)
  }, [])

  return {
    open,
    width,
    tab,
    workdir,
    treeExpanded,
    reveal,
    preview,
    subAgentRequest,
    toggle,
    close,
    openGit,
    openTasks,
    setWidth,
    setTab,
    setTreeExpanded,
    revealInTree,
    insertMention,
  }
}
