import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  externalCliSettingsApi,
  onExternalAgentsUpdated,
  type DetectedExternalAgent,
} from '../api/externalCliSettings'
import type { ExternalCliAgentConfig, Settings as SettingsData } from '../api/tauri'
import { i18n, type Lang } from '../components/i18n'
import { AgentDetail } from './externalAgents/AgentDetail'
import { AgentSidebar, type AgentRow } from './externalAgents/AgentSidebar'
import {
  checkCliInstallInfoBatch,
  deriveCliMaintenance,
  getCliInstallInfo,
  runCliInstall,
  useCliMaintenanceRevision,
} from './externalAgents/cliMaintenance'
import { PiExtensionsSettings } from './PiExtensionsSettings'
import { PiSkillsSettings } from './PiSkillsSettings'

const EMPTY_CONFIG: ExternalCliAgentConfig = {}

interface ExternalAgentsSettingsProps {
  lang: Lang
  settings: SettingsData
  updateChat: (updates: Partial<NonNullable<SettingsData['chat']>>) => void
}

/**
 * 「本地 CLI 管理」页：左栏选 CLI，右栏是它的状态、供应商、模型、扩展和运行环境。
 * 具体各区见 `externalAgents/`；这里只负责探测列表、选中项，以及把改动写回设置草稿。
 */
export function ExternalAgentsSettings({ lang, settings, updateChat }: ExternalAgentsSettingsProps) {
  const t = i18n[lang]
  const [agents, setAgents] = useState<DetectedExternalAgent[]>([])
  const [scanning, setScanning] = useState(false)
  const [scanFailed, setScanFailed] = useState(false)
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [query, setQuery] = useState('')
  const [subpage, setSubpage] = useState<'pi-extensions' | 'pi-skills' | null>(null)
  useCliMaintenanceRevision()

  const overrides = useMemo(
    () => settings.chat?.externalCliAgents ?? {},
    [settings.chat?.externalCliAgents],
  )
  const isDisabled = useCallback(
    (agent: DetectedExternalAgent) => overrides[agent.id]?.disabled ?? agent.disabled ?? false,
    [overrides],
  )

  const loadAgents = useCallback(async (force = false) => {
    setScanning(true)
    try {
      const list = await externalCliSettingsApi.detectExternalAgents(force)
      setAgents(list)
      setScanFailed(false)
      setSelectedId((prev) => prev ?? list.find((a) => a.available)?.id ?? list[0]?.id ?? null)
    } catch (err) {
      // 保留上一次的列表：一次扫描失败不该把用户正在看的 CLI 清空。
      console.error('[ExternalAgentsSettings] detect failed:', err)
      setScanFailed(true)
    } finally {
      setScanning(false)
    }
  }, [])

  useEffect(() => {
    void loadAgents()
  }, [loadAgents])

  // 首屏拿的是落盘快照，后台重探完会推一条新列表过来。
  useEffect(() => {
    let unlisten: (() => void) | null = null
    let cancelled = false
    void onExternalAgentsUpdated((list) => setAgents(list)).then((un) => {
      if (cancelled) un()
      else unlisten = un
    })
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [])

  // 在用的 CLI 都查一遍版本，左栏才能标出谁有更新。没装的、停用的等选中时再查。
  const activeIds = useMemo(
    () => agents.filter((agent) => agent.available && !isDisabled(agent)).map((agent) => agent.id),
    [agents, isDisabled],
  )
  const activeKey = activeIds.join(',')
  useEffect(() => {
    if (activeKey) void checkCliInstallInfoBatch(activeKey.split(','))
  }, [activeKey])

  // 已安装的排前面，其余保持探测顺序。停用的不挪位置：开关就在行上，拨一下不该让它跳走。
  const rows = useMemo<AgentRow[]>(() => {
    const needle = query.trim().toLowerCase()
    const summarize = (agent: DetectedExternalAgent, disabled: boolean) => {
      if (!agent.available) return { summary: t.externalAgentsNotInstalled, attention: false }
      if (disabled) return { summary: t.externalAgentsDisabledGroup, attention: false }
      const config = overrides[agent.id]
      const provider = config?.providers?.find(
        (item) => item.id === config.currentProvider && !item.disabled,
      )
      if (provider) return { summary: provider.name, attention: false }
      const auth = agent.authStatus ?? agent.auth_status
      if (auth === 'auth_required') return { summary: t.externalAgentsSignedOut, attention: true }
      return {
        summary: auth === 'ok' ? t.externalAgentsSignedIn : t.externalAgentsOwnConfig,
        attention: false,
      }
    }
    return agents
      .filter(
        (agent) =>
          !needle || agent.name.toLowerCase().includes(needle) || agent.id.includes(needle),
      )
      .map((agent) => {
        const disabled = isDisabled(agent)
        return { agent, disabled, ...summarize(agent, disabled) }
      })
      .sort((a, b) => Number(b.agent.available) - Number(a.agent.available))
  }, [agents, isDisabled, overrides, query, t])

  const updatableIds = agents
    .filter((agent) => activeIds.includes(agent.id))
    .filter((agent) => {
      const maintenance = deriveCliMaintenance(agent, getCliInstallInfo(agent.id))
      return maintenance.state === 'outdated' && maintenance.actionable
    })
    .map((agent) => agent.id)

  const rescan = async () => {
    await loadAgents(true)
    await checkCliInstallInfoBatch(activeIds, true)
  }
  const updateAll = async () => {
    // 逐个来：并行跑多个 npm 全局安装会互相抢锁。
    for (const id of updatableIds) await runCliInstall(id, loadAgents)
  }

  const patchOverride = useCallback(
    (agentId: string, patch: Partial<ExternalCliAgentConfig>) => {
      const current = settings.chat?.externalCliAgents ?? {}
      updateChat({
        externalCliAgents: {
          ...current,
          [agentId]: { ...(current[agentId] ?? EMPTY_CONFIG), ...patch },
        },
      })
    },
    [settings.chat?.externalCliAgents, updateChat],
  )

  if (subpage === 'pi-extensions') {
    return <PiExtensionsSettings lang={lang} onBack={() => setSubpage(null)} />
  }
  if (subpage === 'pi-skills') {
    return <PiSkillsSettings lang={lang} onBack={() => setSubpage(null)} />
  }

  const selected = agents.find((agent) => agent.id === selectedId) ?? null

  return (
    <div className="kv-providers-root">
      <div className="kv-cli">
        <AgentSidebar
          lang={lang}
          rows={rows}
          selectedId={selectedId}
          onSelect={setSelectedId}
          onToggle={(id, enabled) => patchOverride(id, { disabled: !enabled })}
          query={query}
          onQuery={setQuery}
          scanning={scanning}
          scanFailed={scanFailed}
          onRescan={() => void rescan()}
          updatableIds={updatableIds}
          onUpdateAll={() => void updateAll()}
        />
        <div className="kv-cli-detail">
          {selected ? (
            <AgentDetail
              key={selected.id}
              agent={selected}
              lang={lang}
              config={overrides[selected.id] ?? EMPTY_CONFIG}
              onPatch={(patch) => patchOverride(selected.id, patch)}
              reloadAgents={loadAgents}
              onOpenPiExtensions={() => setSubpage('pi-extensions')}
              onOpenPiSkills={() => setSubpage('pi-skills')}
            />
          ) : (
            <p className="kv-cli-nav-empty">
              {scanning ? t.externalAgentsRescanning : t.externalAgentsSelectHint}
            </p>
          )}
        </div>
      </div>
    </div>
  )
}
