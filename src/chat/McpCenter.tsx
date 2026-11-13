import { FolderIcon as FolderOpen } from '../components/FileIcon'
import { ErrorNotice } from '../components/AppNotice'
// MCP 整页（Chat 窗口「插件 → MCP」）。已安装按来源分组（个人 / 连接器 / 网络搜索 / 插件），
// 与 Skill 页同一套「凡是已加载的都列出来」；插件与网络搜索 MCP 只读（开关在各自设置页）。

import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode, type CSSProperties } from 'react'
import { ChevronDown, FileJson, Loader2, Plus, RefreshCw, Search, Terminal, Trash2 } from 'lucide-react'
import { McpIcon } from '../settings/public/icons'
import { McpServerIcon } from './McpServerIcon'
import { useLang, useT } from '../components/i18n'
import { open } from '@tauri-apps/plugin-dialog'
import {
  api,
  type ChatMcpServer,
  type ChatNativeToolsConfig,
  type ChatToolsConfig,
  type CliImportScan,
  type McpServerStatePayload,
  type McpServerIcon as McpIconImage,
  type McpOAuthClient,
  type Settings,
} from '../api/tauri'
import { peekSettings, refreshSettings, subscribeSettings, updateSettingsCached } from '../api/settingsCache'
import { Toggle, Select, Input, TextArea } from '../settings/public/controls'
import { Button, IconButton } from '../components/Button'
import { confirmDialog } from '../components/dialogQueue'
import { McpRegistryBrowser } from './McpRegistryBrowser'
import { SubagentRoleSettings } from './SubagentRoleSettings'
import { AddMenuButton, CenterEmpty, CenterHeader, CenterSection, CenterTabs } from './market/PluginCenterParts'
import {
  argsToText,
  CHAT_TOOL_TIMEOUT_PRESETS_MS,
  clampMcpIdleTimeoutMs,
  clampTaskMaxConcurrency,
  clampToolTimeoutMs,
  envToText,
  formatToolTimeoutLabel,
  MCP_IDLE_TIMEOUT_PRESETS_MS,
  TASK_MAX_CONCURRENCY_PRESETS,
  textToArgs,
  textToEnv,
} from '../settings/public/mcpTools'
import { adoptFreshPluginManagedServers, isPluginManagedServer, preservePluginManagedServers, isBuiltinGithubOAuth, useConnectorOAuth, OAuthDeviceDialog } from '../settings/public/connectors'
import {
  buildInstalledMcpList,
  entriesOfKind,
  filterInstalledMcpList,
  TINYFISH_MCP_ID,
  type McpInstalledEntry,
  type McpInstalledKind,
} from './mcpInstalledList'

type TestFeedback = { ok: boolean; message: string; icons?: McpIconImage[] }
type OAuthClientDraft = { clientId: string; clientSecret?: string; scopesText?: string }

export function McpCenter({ heading, conversationId }: { heading?: ReactNode; conversationId?: string } = {}) {
  const t = useT()
  const lang = useLang()
  const { connect: connectOAuth, prompt: devicePrompt, cancel: cancelOAuth } = useConnectorOAuth()
  const [settings, setSettings] = useState<Settings | null>(null)
  const [states, setStates] = useState<Record<string, McpServerStatePayload>>({})
  const [view, setView] = useState<'installed' | 'store' | 'import' | 'advanced'>('installed')
  const [error, setError] = useState('')
  const [loading, setLoading] = useState(true)
  const [expandedId, setExpandedId] = useState<string | null>(null)
  const [testingId, setTestingId] = useState<string | null>(null)
  const [oauthId, setOauthId] = useState<string | null>(null)
  const [oauthClients, setOauthClients] = useState<Record<string, OAuthClientDraft>>({})
  const [oauthAppOpen, setOauthAppOpen] = useState<Record<string, boolean>>({})
  const [testFeedback, setTestFeedback] = useState<Record<string, TestFeedback>>({})
  const [cliScan, setCliScan] = useState<CliImportScan | null>(null)
  const [cliScanning, setCliScanning] = useState(false)
  const [cliSelected, setCliSelected] = useState<Set<string>>(new Set())
  const [cliImportDone, setCliImportDone] = useState('')
  const [query, setQuery] = useState('')
  const [modelAllowDraft, setModelAllowDraft] = useState<string | null>(null)
  const settingsRef = useRef<Settings | null>(null)

  const chatTools = settings?.chatTools
  const servers = useMemo(() => chatTools?.servers ?? [], [chatTools?.servers])
  const nativeTools = chatTools?.nativeTools
  const installedEntries = useMemo(
    () => buildInstalledMcpList(servers, settings?.lens?.webSearch),
    [servers, settings?.lens?.webSearch],
  )
  const listedEntries = useMemo(
    () => filterInstalledMcpList(installedEntries, query),
    [installedEntries, query],
  )

  const NATIVE_TOOLS: Array<{ key: keyof ChatNativeToolsConfig; label: string; defaultOn?: boolean }> = [
    { key: 'readFile', label: t.chatMcpNativeReadFile },
    { key: 'writeFile', label: t.chatMcpNativeWriteFile },
    { key: 'editFile', label: t.chatMcpNativeEditFile },
    { key: 'runCommand', label: t.chatMcpNativeRunCommand },
    { key: 'skillRuntime', label: t.chatMcpNativeSkillRuntime, defaultOn: true },
    { key: 'webSearch', label: t.chatMcpNativeWebSearch },
    { key: 'webFetch', label: t.chatMcpNativeWebFetch },
    { key: 'automation', label: t.chatMcpNativeAutomation, defaultOn: true },
    { key: 'scheduledTasks', label: t.chatMcpNativeScheduledTasks, defaultOn: true },
  ]

  const subagentText = lang === 'zh' ? {
    title: '子代理',
    file: '允许从文件读取任务', fileDesc: '允许 taskFile 使用本地文件作为任务内容。',
    structured: '结构化结果', structuredDesc: '允许子代理通过 yield 返回结构化结果。',
    effort: '允许任务指定推理强度', effortDesc: '允许每个任务选择模型支持的推理强度。',
    isolation: '允许工作区隔离', isolationDesc: '允许任务在独立 Git 工作树中运行。',
    prewalk: '任务预先规划', prewalkDesc: '内置 task 角色先规划，再使用可用的 SMOL 模型执行。',
    fresh: '工作池使用新代理', freshDesc: '为工作池中的每项任务使用新代理。',
    model: '按任务选择模型', modelDesc: '仅可选择已在模型设置中授权的模型。',
    off: '关闭', eval: '仅 eval', on: '所有任务',
    effortCap: '最高推理强度', noLimit: '不限制',
    allow: '可选模型范围', allowDesc: '以逗号分隔 provider/model；留空时不额外限制已授权模型。',
  } : {
    title: 'Subagents',
    file: 'Allow task files', fileDesc: 'Allow taskFile to load task instructions from a local file.',
    structured: 'Structured results', structuredDesc: 'Allow subagents to return structured results with yield.',
    effort: 'Allow per-task effort', effortDesc: 'Allow each task to select a supported reasoning effort.',
    isolation: 'Allow workspace isolation', isolationDesc: 'Allow tasks to run in a separate Git worktree.',
    prewalk: 'Plan before implementation', prewalkDesc: 'The built-in task role plans first, then uses SMOL for implementation when available.',
    fresh: 'Use fresh workpool agents', freshDesc: 'Use a new agent for each workpool item.',
    model: 'Per-task model selection', modelDesc: 'Only models already authorized in model settings can be selected.',
    off: 'Off', eval: 'Eval only', on: 'All tasks',
    effortCap: 'Maximum reasoning effort', noLimit: 'No limit',
    allow: 'Allowed model scope', allowDesc: 'Separate provider/model identities with commas. Leave empty to allow all authorized models.',
  }
  const subagentToggles = [
    { key: 'taskFileEnabled', label: subagentText.file, description: subagentText.fileDesc },
    { key: 'taskStructured', label: subagentText.structured, description: subagentText.structuredDesc },
    { key: 'taskEnableEffort', label: subagentText.effort, description: subagentText.effortDesc },
    { key: 'taskIsolationEnabled', label: subagentText.isolation, description: subagentText.isolationDesc },
    { key: 'taskPrewalk', label: subagentText.prewalk, description: subagentText.prewalkDesc },
    { key: 'taskEvalWorkpoolFreshAgents', label: subagentText.fresh, description: subagentText.freshDesc },
  ] as const

  const loadSettings = useCallback(async () => {
    try {
      const cached = peekSettings()
      if (cached) {
        settingsRef.current = cached
        setSettings(cached)
        setLoading(false)
      }
      const loaded = await refreshSettings()
      settingsRef.current = loaded
      setSettings(loaded)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    void loadSettings()
  }, [loadSettings])

  useEffect(() => {
    return subscribeSettings((fresh) => {
      setSettings((prev) => {
        if (!prev) return fresh
        const prevServers = prev.chatTools.servers
        const nextServers = adoptFreshPluginManagedServers(prevServers, fresh.chatTools.servers)
        const { taskDisabledAgents, taskAgentModelOverrides, taskAgentPrewalk, taskAgentAdvisor } = fresh.chatTools
        if (nextServers === prevServers && prev.providers === fresh.providers
          && prev.chatTools.taskDisabledAgents === taskDisabledAgents && prev.chatTools.taskAgentModelOverrides === taskAgentModelOverrides
          && prev.chatTools.taskAgentPrewalk === taskAgentPrewalk && prev.chatTools.taskAgentAdvisor === taskAgentAdvisor) return prev
        const next = {
          ...prev,
          providers: fresh.providers,
          chatTools: { ...prev.chatTools, servers: nextServers, taskDisabledAgents, taskAgentModelOverrides, taskAgentPrewalk, taskAgentAdvisor },
        }
        settingsRef.current = next
        return next
      })
    })
  }, [])

  // 连接状态：订阅推送 + 已启用服务器初次快照
  useEffect(() => {
    let unlisten: (() => void) | undefined
    let active = true
    const revisions = new Map<string, number>()
    const refresh = async (serverId: string) => {
      const revision = (revisions.get(serverId) ?? 0) + 1
      revisions.set(serverId, revision)
      try {
        const status = await api.chatMcpServerStatus(serverId)
        if (active && revisions.get(serverId) === revision) {
          setStates(prev => ({ ...prev, [serverId]: { ...status, serverId } }))
        }
      } catch { /* Keep the last connection event when the snapshot is unavailable. */ }
    }
    void api.onMcpServerState((payload) => {
      if (!active) return
      revisions.set(payload.serverId, (revisions.get(payload.serverId) ?? 0) + 1)
      setStates((prev) => ({ ...prev, [payload.serverId]: payload }))
      setTestFeedback(prev => {
        const feedback = prev[payload.serverId]
        return feedback?.icons ? { ...prev, [payload.serverId]: { ...feedback, icons: undefined } } : prev
      })
      if (payload.state.kind === 'connected') void refresh(payload.serverId)
    }).then((fn) => {
      if (active) unlisten = fn
      else fn()
    })
    installedEntries.forEach(({ server }) => {
      if (!server.enabled) return
      void refresh(server.id)
    })
    return () => { active = false; unlisten?.() }
  }, [installedEntries])

  // 非服务器 chatTools 字段（内置工具 / 运行参数）：本地立即生效 + 持久化，保住后端刷新的 servers。
  const persistChatTools = useCallback((updates: Partial<ChatToolsConfig>) => {
    setSettings((prev) => {
      if (!prev) return prev
      const next: Settings = { ...prev, chatTools: { ...prev.chatTools, ...updates } }
      settingsRef.current = next
      return next
    })
    void (async () => {
      try {
        const saved = await updateSettingsCached((fresh) => ({
          ...fresh,
          chatTools: { ...fresh.chatTools, ...updates },
        }))
        settingsRef.current = saved
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err))
      }
    })()
  }, [])

  const updateNativeTools = useCallback((updates: Partial<ChatNativeToolsConfig>) => {
    const base = settingsRef.current?.chatTools.nativeTools
    if (!base) return
    persistChatTools({ nativeTools: { ...base, ...updates } })
  }, [persistChatTools])

  function acceptRoleSettings(saved: Settings) {
    setSettings(current => {
      if (!current) { settingsRef.current = saved; return saved }
      const { taskDisabledAgents, taskAgentModelOverrides, taskAgentPrewalk, taskAgentAdvisor } = saved.chatTools
      const next = {
        ...current,
        chatTools: { ...current.chatTools, taskDisabledAgents, taskAgentModelOverrides, taskAgentPrewalk, taskAgentAdvisor },
      }
      settingsRef.current = next
      return next
    })
  }

  // 变更服务器：先读后端 fresh（保住后端 OAuth 刷新过的 token），再按 id 施加改动后整存。
  const mutateServers = useCallback(async (fn: (servers: ChatMcpServer[]) => ChatMcpServer[]) => {
    try {
      const saved = await updateSettingsCached((fresh) => {
        const prevServers = fresh.chatTools.servers
        const nextServers = preservePluginManagedServers(prevServers, fn(prevServers))
        return { ...fresh, chatTools: { ...fresh.chatTools, servers: nextServers } }
      })
      settingsRef.current = saved
      setSettings(saved)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    }
  }, [])

  const updateServer = useCallback((id: string, updates: Partial<ChatMcpServer>) => {
    setTestFeedback(prev => { const next = { ...prev }; delete next[id]; return next })
    void mutateServers((list) => list.map((s) => {
      if (s.id !== id) return s
      if (updates.url !== undefined && updates.url !== s.url) {
        const headers = Object.fromEntries(Object.entries(s.headers).filter(([key]) => key.toLowerCase() !== 'authorization'))
        return { ...s, ...updates, auth: undefined, headers }
      }
      return { ...s, ...updates }
    }))
  }, [mutateServers])

  // 启用即连：先落设置（后端按 settings 判定 eligible），再后台预热该 server。
  // 状态点随 onMcpServerState 推送变绿/红，无需手动「测试连接」或发起对话。
  const toggleServerEnabled = useCallback((id: string, enabled: boolean) => {
    const current = settingsRef.current?.chatTools?.servers?.find((s) => s.id === id)
    if (current && isPluginManagedServer(current)) return
    void mutateServers((list) => list.map((s) => (s.id === id ? { ...s, enabled } : s))).then(() => {
      if (enabled) void api.chatMcpWarmup([id])
    })
  }, [mutateServers])

  const handleInstall = useCallback((server: ChatMcpServer) => {
    void mutateServers((list) => [...list, server])
  }, [mutateServers])

  const handleImportJson = useCallback(async () => {
    try {
      const selected = await open({ directory: false, multiple: false, filters: [{ name: 'MCP JSON', extensions: ['json'] }] })
      if (typeof selected !== 'string') return
      const result = await api.chatMcpImportJson(selected)
      if (!result.success) {
        setError(result.error || t.chatMcpImportJsonFailed)
        return
      }
      await mutateServers((list) => [...list, ...result.servers])
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    }
  }, [mutateServers, t])

  // 从本地 CLI（Claude Code / Codex / OpenCode / Pi）扫描已配置的 MCP 服务器。
  const handleCliScan = useCallback(async () => {
    setCliScanning(true)
    setCliImportDone('')
    setError('')
    try {
      const scan = await api.chatCliImportScan()
      setCliScan(scan)
      // 默认不勾，由用户自己选。
      setCliSelected(new Set())
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setCliScanning(false)
    }
  }, [])

  const toggleCliSelected = useCallback((id: string) => {
    setCliSelected((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }, [])

  // 导入选中项 = 复制成 Kivio 自己的 ChatMcpServer（enabled:false，走现有安装流程）。
  const handleCliImportSelected = useCallback(async () => {
    if (!cliScan) return
    const all = [...cliScan.claude.servers, ...cliScan.codex.servers, ...cliScan.opencode.servers, ...cliScan.pi.servers]
    const chosen = all.filter((server) => cliSelected.has(server.id))
    if (chosen.length === 0) return
    await mutateServers((list) => [...list, ...chosen])
    setCliImportDone(t.chatMcpCliImportDone.replace('{n}', String(chosen.length)))
    setCliScan(null)
    setCliSelected(new Set())
    setView('installed')
  }, [cliScan, cliSelected, mutateServers, t])

  const handleTest = useCallback(async (server: ChatMcpServer) => {
    if (!chatTools) return
    setTestingId(server.id)
    setTestFeedback((prev) => {
      const next = { ...prev }
      delete next[server.id]
      return next
    })
    try {
      const result = await api.chatMcpTestServer(server, chatTools.toolTimeoutMs)
      const current = settingsRef.current?.chatTools.servers.find(item => item.id === server.id)
      if (!current || current.url !== server.url || current.command !== server.command
        || current.transport !== server.transport || current.args.join('\0') !== server.args.join('\0')) return
      setTestFeedback((prev) => ({
        ...prev,
        [server.id]: result.success
          ? { ok: true, message: t.chatMcpTestConnected.replace('{n}', String(result.tools.length)), icons: result.icons }
          : { ok: false, message: result.error || t.chatMcpTestFailed },
      }))
    } catch (err) {
      setTestFeedback((prev) => ({ ...prev, [server.id]: { ok: false, message: err instanceof Error ? err.message : String(err) } }))
    } finally {
      setTestingId(null)
    }
  }, [chatTools, t])

  // OAuth applications are scoped to this server URL, not just its editable row id.
  const oauthClientFor = (server: ChatMcpServer): OAuthClientDraft =>
    oauthClients[`${server.id}:${server.url}`] ?? {
      clientId: server.auth?.clientId ?? '',
      clientSecret: server.auth?.clientSecret,
      scopesText: server.auth?.scopes?.join(' '),
    }

  // OAuth 授权复用连接器的已注册应用 / DCR + PKCE 流程。
  const handleOauth = useCallback(async (entry: McpInstalledEntry) => {
    const server = entry.server
    const url = (server.url || '').trim()
    if (!url) return
    setOauthId(server.id)
    try {
      const draft = oauthClients[`${server.id}:${server.url}`]
      const client: McpOAuthClient | undefined = draft ? {
        clientId: draft.clientId.trim(), clientSecret: draft.clientSecret,
        scopes: draft.scopesText?.trim() ? draft.scopesText.trim().split(/\s+/) : undefined,
      } : (!isBuiltinGithubOAuth(url) && server.auth?.clientId ? {
        clientId: server.auth.clientId, clientSecret: server.auth.clientSecret, scopes: server.auth.scopes,
      } : undefined)
      const authed = await connectOAuth({ url, name: server.name, client })
      if (!authed) return
      const authorization = authed.headers?.Authorization
      let serverToTest: ChatMcpServer
      if (entry.kind === 'websearch' && server.id === TINYFISH_MCP_ID) {
        const saved = await updateSettingsCached((fresh) => {
          const currentWebSearch = fresh.lens?.webSearch
          if (!currentWebSearch) throw new Error(t.chatMcpTestFailed)
          if ((currentWebSearch.tinyfishMcpUrl?.trim() || 'https://agent.tinyfish.ai/mcp') !== url) throw new Error(t.chatMcpOauthChanged)
          return {
            ...fresh,
            lens: {
              ...fresh.lens,
              webSearch: {
                ...currentWebSearch,
                tinyfishMcpAuth: authed.auth ?? null,
              },
            },
          }
        })
        settingsRef.current = saved
        setSettings(saved)
        serverToTest = { ...server, auth: authed.auth, headers: authed.headers }
      } else {
        const saved = await updateSettingsCached((fresh) => {
          const current = fresh.chatTools.servers.find((s) => s.id === server.id)
          if (!current || current.url.trim() !== url) throw new Error(t.chatMcpOauthChanged)
          const headers = { ...current.headers }
          if (authorization) {
            for (const key of Object.keys(headers)) if (key.toLowerCase() === 'authorization') delete headers[key]
            headers.Authorization = authorization
          }
          const servers = fresh.chatTools.servers.map((s) => s.id === server.id ? { ...s, auth: authed.auth, headers } : s)
          return { ...fresh, chatTools: { ...fresh.chatTools, servers: preservePluginManagedServers(fresh.chatTools.servers, servers) } }
        })
        settingsRef.current = saved
        setSettings(saved)
        serverToTest = saved.chatTools.servers.find((s) => s.id === server.id)!
      }
      await handleTest(serverToTest)
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err)
      const requiresClient = message.startsWith('OAUTH_CLIENT_REQUIRED:')
      if (requiresClient) setOauthAppOpen((prev) => ({ ...prev, [`${server.id}:${server.url}`]: true }))
      setTestFeedback((prev) => ({ ...prev, [server.id]: { ok: false, message: requiresClient ? t.chatMcpOauthClientRequired : message } }))
    } finally {
      setOauthId(null)
    }
  }, [connectOAuth, handleTest, oauthClients, t])

  const lockedNote = (kind: McpInstalledKind) =>
    kind === 'plugin' ? t.chatMcpPluginNote
    : kind === 'connector' ? t.chatMcpConnectorNote
    : kind === 'websearch' ? t.chatMcpWebSearchNote
    : undefined

  const sourceTag = (kind: McpInstalledKind) =>
    kind === 'plugin' ? t.chatSkillSourcePlugin
    : kind === 'connector' ? t.chatMcpSourceConnector
    : kind === 'websearch' ? t.chatMcpSourceWebSearch
    : null

  // 手动添加：建一条停用的 stdio 空白配置并直接展开编辑。
  const handleAddManual = useCallback(() => {
    const id = `manual-${typeof crypto !== 'undefined' && 'randomUUID' in crypto ? crypto.randomUUID() : Date.now().toString(36)}`
    const server: ChatMcpServer = {
      id, name: t.chatMcpNewServerName, enabled: false, transport: 'stdio',
      url: '', command: '', args: [], env: {}, headers: {}, cwd: null, enabledTools: [],
    }
    setQuery('')
    setView('installed')
    setExpandedId(id)
    void mutateServers((list) => [...list, server])
  }, [mutateServers, t])

  const handleDeleteServer = useCallback(async (server: ChatMcpServer) => {
    const ok = await confirmDialog({
      title: t.chatMcpDeleteTitle,
      message: t.chatMcpDeleteConfirm.replace('{name}', server.name),
      confirmLabel: t.chatDelete,
      danger: true,
    })
    if (!ok) return
    setExpandedId((current) => (current === server.id ? null : current))
    void mutateServers((list) => list.filter((s) => s.id !== server.id))
  }, [mutateServers, t])

  const handleStartCliImport = useCallback(() => {
    setView('import')
    void handleCliScan()
  }, [handleCliScan])

  const statusView = (server: ChatMcpServer) => {
    if (!server.enabled) return { dot: 'bg-neutral-300 dark:bg-neutral-600', label: t.chatMcpDisabled, error: '' }
    const state = states[server.id]?.state
    const kind = state?.kind ?? 'disconnected'
    if (kind === 'connected') return { dot: 'bg-emerald-500', label: t.chatMcpStatusConnected, error: '' }
    if (kind === 'connecting') return { dot: 'bg-amber-500 animate-pulse', label: t.chatMcpStatusConnecting, error: '' }
    if (kind === 'error') {
      return { dot: 'bg-red-500', label: t.chatMcpStatusError, error: state?.kind === 'error' ? state.message : '' }
    }
    return { dot: 'bg-neutral-300 dark:bg-neutral-600', label: t.chatMcpStatusDisconnected, error: '' }
  }

  const renderServerRow = (entry: McpInstalledEntry, idx: number) => {
    const { server, kind, manageLocked } = entry
    const expanded = expandedId === server.id
    const isHttp = server.transport === 'streamable_http'
    const feedback = testFeedback[server.id]
    const tag = sourceTag(kind)
    const note = lockedNote(kind)
    const status = statusView(server)
    const address = isHttp ? server.url : [server.command, ...server.args].filter(Boolean).join(' ')
    const oauthKey = `${server.id}:${server.url}`
    const actions = (
      <div className="kv-mcp-actions">
        <Button size="sm" onClick={() => void handleTest(server)} disabled={testingId === server.id}>
          {testingId === server.id && <Loader2 size={12} className="animate-spin" />}
          {t.chatMcpTestConnection}
        </Button>
        {isHttp && (
          <Button size="sm" variant="ghost" onClick={() => void handleOauth(entry)} disabled={oauthId !== null}>
            {oauthId === server.id && <Loader2 size={12} className="animate-spin" />}
            {t.chatMcpOauthAuthorize}
          </Button>
        )}
        {feedback && (
          <span className={`kv-mcp-feedback ${feedback.ok ? 'is-ok' : 'is-error'}`}>{feedback.message}</span>
        )}
        {!manageLocked && (
          <Button size="sm" variant="danger" className="is-end" onClick={() => void handleDeleteServer(server)}>
            <Trash2 size={13} />
            {t.chatMcpDeleteTitle}
          </Button>
        )}
      </div>
    )
    return (
      <div
        key={server.id}
        style={{ '--chat-motion-delay': `${Math.min(idx, 8) * 24}ms` } as CSSProperties}
        className={`kv-mcp-row chat-motion-fade-up${expanded ? ' is-open' : ''}`}
      >
        <div className="kv-mcp-row-head">
          <button
            type="button"
            className="kv-mcp-row-main"
            aria-expanded={expanded}
            onClick={() => setExpandedId(expanded ? null : server.id)}
            data-tauri-drag-region="false"
          >
            <span className="kv-mcp-row-icon">
              <McpServerIcon icons={feedback?.icons ?? states[server.id]?.icons} />
              <span className={`kv-mcp-row-dot ${status.dot}`} aria-hidden="true" />
            </span>
            <span className="kv-mcp-row-text">
              <span className="kv-mcp-row-title">
                <strong>{server.name}</strong>
                <span className="kv-skill-chip">{isHttp ? 'HTTP' : 'stdio'}</span>
                {tag && <span className="kv-skill-chip">{tag}</span>}
              </span>
              <span className="kv-mcp-row-sub">
                <span className={`kv-mcp-row-status${status.error ? ' is-error' : ''}`} title={status.error || undefined}>{status.label}</span>
                {address && <code>{address}</code>}
              </span>
            </span>
            <ChevronDown size={15} className="kv-mcp-row-chevron" aria-hidden="true" />
          </button>
          <span className="kv-mcp-row-actions" title={manageLocked ? note : undefined}>
            <Toggle
              checked={server.enabled}
              disabled={manageLocked}
              onChange={(enabled) => toggleServerEnabled(server.id, enabled)}
              ariaLabel={manageLocked ? note : t.chatMcpEnableNamed.replace('{name}', server.name)}
            />
          </span>
        </div>

        {expanded && (
          <div className="kv-mcp-row-body chat-motion-search-reveal">
            {status.error && <p className="kv-mcp-feedback is-error">{status.error}</p>}
            {manageLocked ? (
              <>
                {note && <p className="kv-mcp-note">{note}</p>}
                {address && <code className="kv-skill-detail-path">{address}</code>}
              </>
            ) : (
              <div className="kv-mcp-form">
                <label className="kv-mcp-field">
                  {t.chatMcpName}
                  <Input value={server.name} onChange={(name) => updateServer(server.id, { name })} />
                </label>
                <div className="kv-mcp-field">
                  {t.chatMcpTransport}
                  <Select
                    value={isHttp ? 'streamable_http' : 'stdio'}
                    onChange={(transport) => updateServer(server.id, { transport })}
                    ariaLabel={t.chatMcpTransport}
                    options={[{ value: 'stdio', label: t.chatMcpTransportStdio }, { value: 'streamable_http', label: t.chatMcpTransportHttp }]}
                  />
                </div>
                {isHttp ? (
                  <>
                    <label className="kv-mcp-field is-wide">
                      URL
                      <Input mono value={server.url} onChange={(url) => updateServer(server.id, { url })} placeholder="https://example.com/mcp" />
                    </label>
                    <label className="kv-mcp-field is-wide">
                      {t.chatMcpHeadersLabel}
                      <TextArea mono rows={2} maxRows={8} value={envToText(server.headers)} onChange={(text) => updateServer(server.id, { headers: textToEnv(text) })} />
                    </label>
                  </>
                ) : (
                  <>
                    <label className="kv-mcp-field is-wide">
                      {t.chatMcpCommand}
                      <Input mono value={server.command} onChange={(command) => updateServer(server.id, { command })} placeholder="npx" />
                    </label>
                    <label className="kv-mcp-field">
                      {t.chatMcpArgsLabel}
                      <TextArea mono rows={2} maxRows={8} value={argsToText(server.args)} onChange={(text) => updateServer(server.id, { args: textToArgs(text) })} />
                    </label>
                    <label className="kv-mcp-field">
                      {t.chatMcpEnvLabel}
                      <TextArea mono rows={2} maxRows={8} value={envToText(server.env)} onChange={(text) => updateServer(server.id, { env: textToEnv(text) })} />
                    </label>
                  </>
                )}
              </div>
            )}
            {isHttp && (
              <>
                {isBuiltinGithubOAuth(server.url) && <p className="kv-mcp-note">{t.chatMcpGithubBuiltin}</p>}
                <details
                  className="kv-mcp-oauth"
                  open={oauthAppOpen[oauthKey] ?? false}
                  onToggle={(event) => {
                    const open = event.currentTarget.open
                    setOauthAppOpen((prev) => prev[oauthKey] === open ? prev : { ...prev, [oauthKey]: open })
                  }}
                >
                  <summary>{isBuiltinGithubOAuth(server.url) ? t.chatMcpOauthAdvancedApp : t.chatMcpOauthApp}</summary>
                  <p className="kv-mcp-note mb-2.5">{t.chatMcpOauthAppHint}</p>
                  <div className="kv-mcp-oauth-fields">
                    {(['clientId', 'clientSecret', 'scopesText'] as const).map((field) => (
                      <label key={field} className="kv-mcp-field">
                        {field === 'clientId' ? 'Client ID' : field === 'clientSecret' ? 'Client Secret' : 'Scopes'}
                        <Input
                          type={field === 'clientSecret' ? 'password' : 'text'}
                          autoComplete="off"
                          disabled={oauthId === server.id}
                          value={oauthClientFor(server)[field] ?? ''}
                          placeholder={field === 'scopesText' ? t.chatMcpOauthScopes : field === 'clientSecret' ? t.chatMcpOauthOptional : ''}
                          onChange={(text) => {
                            setOauthClients((prev) => ({ ...prev, [oauthKey]: { ...oauthClientFor(server), [field]: text } }))
                          }}
                        />
                      </label>
                    ))}
                  </div>
                </details>
              </>
            )}
            {actions}
          </div>
        )}
      </div>
    )
  }

  const renderRuntimeSelect = (
    label: string,
    value: string,
    onChange: (value: string) => void,
    options: Array<{ value: string; label: string }>,
    desc?: string,
  ) => (
    <div className="flex h-full flex-col">
      <div className="mb-2">
        <div className="text-[13px] font-medium text-neutral-800">{label}</div>
        {desc && <p className="mt-0.5 text-[12px] text-neutral-500">{desc}</p>}
      </div>
      <div className="mt-auto">
        <Select className="w-full" value={value} onChange={onChange} options={options} ariaLabel={label} />
      </div>
    </div>
  )

  const mcpOff = Boolean(settings) && !(chatTools?.enabled ?? false) && installedEntries.length > 0

  const installedView = (
    <div key="installed" className="chat-motion-tab-in">
      {mcpOff && (
        <div className="kv-center-banner">
          <span>{t.chatMcpOffBanner}</span>
          <Button size="sm" variant="primary" onClick={() => persistChatTools({ enabled: true })}>{t.chatMcpTurnOn}</Button>
        </div>
      )}
      {installedEntries.length > 0 && (
        <div className="kv-center-toolbar">
          <label className="kv-center-search">
            <Input value={query} onChange={setQuery} placeholder={t.chatMcpSearchPlaceholder} aria-label={t.chatMcpSearchPlaceholder} />
          </label>
        </div>
      )}
      {loading ? (
        <div className="space-y-2" aria-busy="true">
          {Array.from({ length: 3 }, (_, i) => <div key={i} className="kv-skeleton kv-market-skeleton" />)}
        </div>
      ) : installedEntries.length === 0 ? (
        <CenterEmpty
          icon={<McpIcon size={22} />}
          title={t.chatMcpEmptyTitle}
          description={t.chatMcpEmptyDesc}
          actions={<>
            <Button variant="primary" onClick={() => setView('store')}>{t.chatMcpBrowseStore}</Button>
            <Button onClick={handleAddManual}><Plus size={14} />{t.chatMcpAddManual}</Button>
            <Button variant="ghost" onClick={() => void handleImportJson()}>{t.chatMcpAddImportJson}</Button>
          </>}
        />
      ) : listedEntries.length === 0 ? (
        <div className="kv-market-empty">
          <p>{t.chatMcpNoMatch}</p>
          <Button size="sm" onClick={() => setQuery('')}>{t.chatSkillClearSearch}</Button>
        </div>
      ) : (
        <div className="kv-center-sections">
          {([
            ['user', t.chatMcpSectionUser, t.chatMcpUserNote],
            ['connector', t.chatMcpSectionConnector, t.chatMcpConnectorNote],
            ['websearch', t.chatMcpSectionWebSearch, t.chatMcpWebSearchNote],
            ['plugin', t.chatMcpSectionPlugin, t.chatMcpPluginNote],
          ] as const).map(([kind, title, note]) => {
            const rows = entriesOfKind(listedEntries, kind)
            if (rows.length === 0) return null
            const enabledCount = rows.filter((entry) => entry.server.enabled).length
            return (
              <CenterSection
                key={kind}
                title={title}
                count={rows.length}
                meta={t.chatSkillsEnabledCount.replace('{n}', String(enabledCount))}
                note={note}
              >
                <div className="kv-mcp-list">
                  {rows.map((entry, idx) => renderServerRow(entry, idx))}
                </div>
              </CenterSection>
            )
          })}
        </div>
      )}
    </div>
  )

  const importView = (
    <div key="import" className="chat-motion-tab-in kv-center-cards">
      <section className="kv-center-card">
        <div className="kv-center-card-head">
          <span className="kv-market-icon" aria-hidden="true"><FileJson size={17} /></span>
          <div>
            <strong>{t.chatMcpImportJsonTitle}</strong>
            <p>{t.chatMcpImportJsonDesc}</p>
          </div>
          <Button onClick={() => void handleImportJson()}>
            <FolderOpen open size={14} />
            {t.chatMcpImportJsonPick}
          </Button>
        </div>
      </section>

      <section className="kv-center-card">
        <div className="kv-center-card-head">
          <span className="kv-market-icon" aria-hidden="true"><Terminal size={17} /></span>
          <div>
            <strong>{t.chatMcpImportCliTitle}</strong>
            <p>{t.chatMcpImportCliDesc}</p>
          </div>
          <Button onClick={() => void handleCliScan()} disabled={cliScanning}>
            {cliScanning ? <Loader2 size={14} className="animate-spin" /> : <Search size={14} />}
            {t.chatMcpScan}
          </Button>
        </div>

        {cliScan && (
          <div className="kv-center-card-body">
            {([['claude', 'Claude Code'], ['codex', 'Codex'], ['opencode', 'OpenCode'], ['pi', 'Pi']] as const).map(([key, label]) => {
              const group = cliScan[key]
              return (
                <div key={key} className="kv-center-picklist-group">
                  <div className="kv-center-picklist-title">
                    {label}
                    {!group.available ? <small>{t.chatMcpCliNotDetected}</small> : group.servers.length === 0 ? <small>{t.chatMcpCliNoServers}</small> : <small>{group.servers.length}</small>}
                  </div>
                  {group.available && group.servers.length > 0 && (
                    <div className="kv-center-picklist">
                      {group.servers.map((server) => {
                        const isHttp = server.transport === 'streamable_http'
                        return (
                          <label key={server.id} data-tauri-drag-region="false">
                            <input type="checkbox" checked={cliSelected.has(server.id)} onChange={() => toggleCliSelected(server.id)} />
                            <span>
                              <strong>{server.name}</strong>
                              <small>{isHttp ? server.url : [server.command, ...server.args].filter(Boolean).join(' ')}</small>
                            </span>
                            <span className="kv-skill-chip flex-none">{isHttp ? 'HTTP' : 'stdio'}</span>
                          </label>
                        )
                      })}
                    </div>
                  )}
                </div>
              )
            })}
            <div className="kv-mcp-actions mt-3">
              <Button variant="primary" onClick={() => void handleCliImportSelected()} disabled={cliSelected.size === 0}>
                {t.chatMcpCliImportSelected.replace('{n}', String(cliSelected.size))}
              </Button>
            </div>
          </div>
        )}

        {cliImportDone && <div className="kv-center-done">{cliImportDone}</div>}
      </section>
    </div>
  )

  return (
    <div className="assistant-center-root kv-market flex h-full min-h-0 flex-col" data-tauri-drag-region="false">
      <OAuthDeviceDialog prompt={devicePrompt} onCancel={cancelOAuth} lang={lang} />

      <main className={heading ? 'kv-market-scroll custom-scrollbar' : 'kv-market-scroll custom-scrollbar pt-7'}>
        <div className="w-full pb-4">
          <CenterHeader
            heading={heading ?? (
              <h1 className="flex items-center gap-2.5">
                <McpIcon size={24} className="text-[var(--color-muted-foreground)]" />
                MCP
              </h1>
            )}
            subtitle={t.chatMcpSubtitle}
            actions={<>
              <IconButton size="md" label={t.chatMcpRefresh} onClick={() => void loadSettings()}>
                <RefreshCw size={16} />
              </IconButton>
              <AddMenuButton
                label={t.chatCenterAdd}
                items={[
                  { key: 'store', label: t.chatMcpAddFromStore, icon: <Search size={14} />, onSelect: () => setView('store') },
                  { key: 'manual', label: t.chatMcpAddManual, icon: <Plus size={14} />, onSelect: handleAddManual },
                  { key: 'json', label: t.chatMcpAddImportJson, icon: <FileJson size={14} />, onSelect: () => void handleImportJson() },
                  { key: 'cli', label: t.chatMcpAddFromCli, icon: <Terminal size={14} />, onSelect: handleStartCliImport },
                ]}
              />
            </>}
          />

          <CenterTabs
            value={view}
            onChange={setView}
            tabs={[
              { id: 'installed', label: t.chatMcpTabInstalled, count: installedEntries.length },
              { id: 'store', label: t.chatMcpTabStore },
              { id: 'import', label: t.chatMcpTabImport },
              { id: 'advanced', label: t.chatMcpTabAdvanced },
            ]}
          />

          {error && <ErrorNotice message={error} />}

          {view === 'store' ? (
            <div key="store" className="chat-motion-tab-in flex min-h-[420px] flex-col">
              <McpRegistryBrowser existingServers={servers} onInstall={handleInstall} />
            </div>
          ) : view === 'import' ? importView : view === 'advanced' ? (
            <div key="advanced" className="chat-motion-tab-in mt-5 space-y-6">
              <section>
                <div className="mb-2 text-[13px] font-semibold text-neutral-800">{t.chatMcpNativeToolsTitle}</div>
                <p className="mb-3 text-[12px] text-neutral-500">
                  {t.chatMcpNativeToolsDesc}
                </p>
                <div className="overflow-hidden rounded-md border border-neutral-200 [&>*+*]:border-t [&>*+*]:border-neutral-100 dark:[&>*+*]:border-neutral-800/70">
                  {NATIVE_TOOLS.map((tool) => (
                    <div key={tool.key} className="flex items-center justify-between px-4 py-2.5">
                      <span className="text-[13px] text-neutral-800">{tool.label}</span>
                      <Toggle
                        checked={tool.defaultOn ? nativeTools?.[tool.key] !== false : nativeTools?.[tool.key] === true}
                        onChange={(checked) => updateNativeTools({ [tool.key]: checked } as Partial<ChatNativeToolsConfig>)}
                      />
                    </div>
                  ))}
                </div>
              </section>

              <section>
                <div className="mb-3 text-[13px] font-semibold text-neutral-800">{t.chatMcpToolRuntimeTitle}</div>
                <div className="flex items-center justify-between rounded-md border border-neutral-200 px-4 py-3">
                  <span className="text-[13px] text-neutral-800">{t.chatMcpEnableMcp}</span>
                  <Toggle checked={chatTools?.enabled ?? false} onChange={(enabled) => persistChatTools({ enabled })} />
                </div>
                <div className="mt-4 grid grid-cols-[repeat(auto-fit,minmax(190px,1fr))] items-stretch gap-x-4 gap-y-5">
                  {renderRuntimeSelect(t.chatMcpApprovalPolicy, chatTools?.approvalPolicy || 'auto', (approvalPolicy) => persistChatTools({ approvalPolicy }), [
                    { value: 'readonly_auto_sensitive_confirm', label: t.chatMcpApprovalOnce },
                    { value: 'always_confirm', label: t.chatMcpApprovalAlways },
                    { value: 'auto', label: t.chatMcpApprovalAuto },
                  ])}
                  {renderRuntimeSelect(
                    t.chatMcpTaskMaxConcurrency,
                    String(clampTaskMaxConcurrency(chatTools?.taskMaxConcurrency)),
                    (value) => persistChatTools({ taskMaxConcurrency: clampTaskMaxConcurrency(value) }),
                    TASK_MAX_CONCURRENCY_PRESETS.map((n) => ({ value: String(n), label: n === 0 ? t.chatMcpTaskConcurrencyUnlimited : String(n) })),
                  )}
                  {renderRuntimeSelect(
                    t.chatMcpToolTimeout,
                    String(clampToolTimeoutMs(chatTools?.toolTimeoutMs)),
                    (value) => persistChatTools({ toolTimeoutMs: clampToolTimeoutMs(value) }),
                    CHAT_TOOL_TIMEOUT_PRESETS_MS.map((ms) => ({ value: String(ms), label: formatToolTimeoutLabel(ms, lang) })),
                    t.chatMcpToolTimeoutDesc,
                  )}
                  {renderRuntimeSelect(
                    t.chatMcpIdleTimeout,
                    String(clampMcpIdleTimeoutMs(chatTools?.mcpIdleTimeoutMs)),
                    (value) => persistChatTools({ mcpIdleTimeoutMs: clampMcpIdleTimeoutMs(value) }),
                    MCP_IDLE_TIMEOUT_PRESETS_MS.map((ms) => ({ value: String(ms), label: formatToolTimeoutLabel(ms, lang) })),
                    t.chatMcpIdleTimeoutDesc,
                  )}
                </div>
                <div className="mt-6">
                  <div className="mb-3 text-[13px] font-semibold text-neutral-800">{subagentText.title}</div>
                  <div className="overflow-hidden rounded-md border border-neutral-200 [&>*+*]:border-t [&>*+*]:border-neutral-100 dark:[&>*+*]:border-neutral-800/70">
                    {subagentToggles.map(({ key, label, description }) => (
                      <div key={key} className="flex items-center justify-between gap-4 px-4 py-2.5">
                        <div>
                          <div className="text-[13px] text-neutral-800">{label}</div>
                          <p className="mt-0.5 text-[12px] text-neutral-500">{description}</p>
                        </div>
                        <Toggle ariaLabel={label} checked={chatTools?.[key] ?? false} onChange={(checked) => persistChatTools({ [key]: checked })} />
                      </div>
                    ))}
                  </div>
                  <div className="mt-4 grid grid-cols-[repeat(auto-fit,minmax(190px,1fr))] items-stretch gap-x-4 gap-y-5">
                    {renderRuntimeSelect(subagentText.model, chatTools?.taskPerCallModel ?? 'off', (value) => {
                      if (value === 'off' || value === 'eval' || value === 'on') persistChatTools({ taskPerCallModel: value })
                    }, [
                      { value: 'off', label: subagentText.off },
                      { value: 'eval', label: subagentText.eval },
                      { value: 'on', label: subagentText.on },
                    ], subagentText.modelDesc)}
                    {renderRuntimeSelect(subagentText.effortCap, chatTools?.taskMaxEffort ?? '', (value) => persistChatTools({ taskMaxEffort: value || null }), [
                      { value: '', label: subagentText.noLimit },
                      ...['off', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max'].map((value) => ({ value, label: value })),
                    ])}
                  </div>
                  <div className="mt-4">
                    <div className="mb-2">
                      <div className="text-[13px] font-medium text-neutral-800">{subagentText.allow}</div>
                      <p className="mt-0.5 text-[12px] text-neutral-500">{subagentText.allowDesc}</p>
                    </div>
                    <Input
                      aria-label={subagentText.allow}
                      mono
                      placeholder="provider/model, provider/model"
                      value={modelAllowDraft ?? (chatTools?.taskPerCallModelAllow ?? []).join(', ')}
                      onChange={setModelAllowDraft}
                      onBlur={() => {
                        if (modelAllowDraft === null) return
                        persistChatTools({ taskPerCallModelAllow: modelAllowDraft.split(',').map((model) => model.trim()).filter(Boolean) })
                        setModelAllowDraft(null)
                      }}
                      onKeyDown={(event) => { if (event.key === 'Enter') event.currentTarget.blur() }}
                    />
                  </div>
                  {settings && <SubagentRoleSettings key={conversationId} conversationId={conversationId} settings={settings} onSaved={acceptRoleSettings} />}
                </div>
              </section>
            </div>
          ) : installedView}
        </div>
      </main>
    </div>
  )
}
