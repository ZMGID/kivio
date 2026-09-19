import { useCallback, useEffect, useState } from 'react'
import { api, type ChatMcpServer, type ChatToolDefinition } from '../../api/tauri'
import { getSettingsCached, subscribeSettings, updateSettingsCached } from '../../api/settingsCache'
import { hasEnabledNativeBuiltinTool, hasEnabledSkillRuntime } from '../../api/chatTools'
import { isPluginManagedServer, preservePluginManagedServers } from '../../settings/public/connectors'
import { isTauriRuntime } from '../utils'
import { scheduleIdleTask } from '../idleTask'
import { useTauriEvent } from './useTauriEvent'

export const DEFAULT_APPROVAL_POLICY = 'readonly_auto_sensitive_confirm'

export interface UseChatToolIndicatorOptions {
  /** 审批策略 / MCP 开关落盘后通知宿主（App 级 settings 变更回调）。 */
  onSettingsChange: () => void
}

/**
 * 输入栏工具指示器与顶栏权限胶囊的数据 owner：从 settings 读一份「工具能力快照」
 * （启用的 MCP server、原生工具开关、审批策略、禁用 Skill）以及供应商能力表
 * （apiFormat / baseUrl / OAuth 类型，用于判断内置联网搜索能不能选），再经
 * `chat_mcp_list_tools(cached_only)` 被动读工具目录 —— 不连接服务器、不延长空闲寿命。
 *
 * 写入路径（审批策略、MCP 开关）一律读-改-写：后端 OAuth 刷新会改写 servers[].auth，
 * 拿缓存快照整体保存会把新 token 覆盖回旧值。
 */
export function useChatToolIndicator({ onSettingsChange }: UseChatToolIndicatorOptions) {
  const [enabledTools, setEnabledTools] = useState<ChatToolDefinition[]>([])
  const [mcpServers, setMcpServers] = useState<ChatMcpServer[]>([])
  const [webSearchEnabled, setWebSearchEnabled] = useState(true)
  const [providerApiFormats, setProviderApiFormats] = useState<Record<string, string>>({})
  const [providerOAuthTypes, setProviderOAuthTypes] = useState<Record<string, string>>({})
  const [providerBaseUrls, setProviderBaseUrls] = useState<Record<string, string>>({})
  const [enabledToolCount, setEnabledToolCount] = useState<number | null>(null)
  const [toolDiscoveryPending, setToolDiscoveryPending] = useState(true)
  const [toolsDisabledReason, setToolsDisabledReason] = useState('')
  const [toolsRequested, setToolsRequested] = useState(false)
  const [approvalPolicy, setApprovalPolicyState] = useState(DEFAULT_APPROVAL_POLICY)
  const [disabledSkillIds, setDisabledSkillIds] = useState<string[]>([])

  const resetToolCatalog = useCallback((reason = '') => {
    setToolDiscoveryPending(false)
    setEnabledTools([])
    setEnabledToolCount(null)
    setToolsDisabledReason(reason)
    setToolsRequested(false)
  }, [])

  const refresh = useCallback(async () => {
    setToolDiscoveryPending(true)
    setEnabledToolCount(null)
    setToolsDisabledReason('')
    if (!isTauriRuntime()) {
      resetToolCatalog()
      setApprovalPolicyState(DEFAULT_APPROVAL_POLICY)
      setMcpServers([])
      return
    }
    try {
      const settings = await getSettingsCached()
      const chatTools = settings.chatTools
      setMcpServers(chatTools?.servers ?? [])
      setWebSearchEnabled(chatTools?.nativeTools?.webSearch !== false)
      setProviderOAuthTypes(
        Object.fromEntries(settings.providers.map((p) => [p.id, p.request.oauth?.provider ?? ''])),
      )
      setProviderApiFormats(
        Object.fromEntries((settings.providers ?? []).map((p) => [p.id, p.apiFormat ?? ''])),
      )
      setProviderBaseUrls(
        Object.fromEntries((settings.providers ?? []).map((p) => [p.id, p.baseUrl ?? ''])),
      )
      setApprovalPolicyState(chatTools?.approvalPolicy || DEFAULT_APPROVAL_POLICY)
      const nextDisabledSkillIds = chatTools?.disabledSkillIds ?? []
      setDisabledSkillIds((prev) =>
        prev.length === nextDisabledSkillIds.length
        && prev.every((id, index) => id === nextDisabledSkillIds[index])
          ? prev
          : nextDisabledSkillIds,
      )
      if (!chatTools) {
        resetToolCatalog()
        setApprovalPolicyState(DEFAULT_APPROVAL_POLICY)
        return
      }
      const anyMcpEnabled = chatTools.enabled && chatTools.servers.some((server) => server.enabled)
      const anyNativeEnabled = hasEnabledNativeBuiltinTool(chatTools.nativeTools)
      const skillRuntimeEnabled = hasEnabledSkillRuntime(chatTools.nativeTools)
      const requested = anyMcpEnabled || anyNativeEnabled || skillRuntimeEnabled
      setToolsRequested(requested)
      if (!requested) {
        setToolDiscoveryPending(false)
        setEnabledTools([])
        setEnabledToolCount(null)
        setToolsDisabledReason('')
        return
      }
      const result = await api.chatMcpListTools(true)
      const tools = result.success ? result.tools : []
      setEnabledTools(tools)
      setToolDiscoveryPending(Boolean(result.discoveryPending))
      setEnabledToolCount(result.discoveryPending ? null : tools.length)
      setToolsDisabledReason(result.success ? '' : result.error || '工具不可用')
    } catch (err) {
      resetToolCatalog(err instanceof Error ? err.message : String(err))
      setApprovalPolicyState(DEFAULT_APPROVAL_POLICY)
    }
  }, [resetToolCatalog])

  const setApprovalPolicy = useCallback(async (nextApprovalPolicy: string) => {
    setApprovalPolicyState(nextApprovalPolicy)
    try {
      await updateSettingsCached((settings) => ({
        ...settings,
        chatTools: {
          ...settings.chatTools,
          approvalPolicy: nextApprovalPolicy,
        },
      }))
      onSettingsChange()
    } catch (err) {
      console.error('Failed to update approval policy:', err)
      void refresh()
    }
  }, [onSettingsChange, refresh])

  const toggleMcpServer = useCallback(async (serverId: string) => {
    try {
      const current = mcpServers.find((server) => server.id === serverId)
      // 插件托管的 server 只走「扩展 → 插件」开关。
      if (!current || isPluginManagedServer(current)) return
      const desiredEnabled = !current.enabled
      const servers = preservePluginManagedServers(
        mcpServers,
        mcpServers.map((server) =>
          server.id === serverId ? { ...server, enabled: desiredEnabled } : server,
        ),
      )
      // 乐观更新本地列表（开关即时反馈），保存后由 refresh 校正。
      setMcpServers(servers)
      await updateSettingsCached((fresh) => {
        const currentServers = fresh.chatTools?.servers ?? []
        const currentServer = currentServers.find((server) => server.id === serverId)
        if (!currentServer || isPluginManagedServer(currentServer)) return fresh
        const nextServers = preservePluginManagedServers(
          currentServers,
          currentServers.map((server) => (
            server.id === serverId ? { ...server, enabled: desiredEnabled } : server
          )),
        )
        return { ...fresh, chatTools: { ...fresh.chatTools, servers: nextServers } }
      })
      onSettingsChange()
      await refresh()
    } catch (err) {
      console.error('Failed to toggle MCP server:', err)
      void refresh()
    }
  }, [mcpServers, onSettingsChange, refresh])

  useTauriEvent(api.onMcpServerState, (event) => {
    if (event.state.kind !== 'connecting') void refresh()
  }, [refresh])

  // 首次读延后到空闲：挂载帧先给会话页；工具目录本来就是被动缓存读。
  useEffect(() => scheduleIdleTask(() => { void refresh() }, 1500), [refresh])

  // 其他窗口 / 设置页改了 server 列表，MCP 开关列表立即跟上（工具目录仍等 server 状态事件）。
  useEffect(() => subscribeSettings((next) => {
    setMcpServers(next.chatTools?.servers ?? [])
  }), [])

  return {
    enabledTools,
    mcpServers,
    webSearchEnabled,
    providerApiFormats,
    providerOAuthTypes,
    providerBaseUrls,
    enabledToolCount,
    toolDiscoveryPending,
    toolsDisabledReason,
    toolsRequested,
    approvalPolicy,
    disabledSkillIds,
    refresh,
    setApprovalPolicy,
    toggleMcpServer,
  }
}
