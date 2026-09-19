import { useCallback, useEffect, useRef, useState } from 'react'
import { FileSpreadsheet, Globe2, Monitor, RefreshCw } from 'lucide-react'
import { api, type ChatMcpServer, type ChatToolsConfig, type ControlToolStatus, type PluginStatus, type SkillMeta } from '../../api/tauri'
import { refreshSettings } from '../../api/settingsCache'
import { Button } from '../../components/Button'
import { Toggle } from '../components'
import type { Lang } from '../../components/i18n'

type NativeTool = 'cua' | 'playwright'
type Tool = NativeTool | 'ego-lite' | 'officecli'

type ControlDetectionSnapshot = {
  versions: Partial<Record<NativeTool, string>>
  updates: Partial<Record<NativeTool, ControlToolStatus>>
  skills: SkillMeta[]
  plugins: PluginStatus[]
  skillScanPaths: string[]
}

const CONTROL_CACHE_KEY = 'kivio.computer-control.detection.v1'
let detectionInFlight: { key: string; promise: Promise<ControlDetectionSnapshot> } | null = null

const CONTROL_TOOLS = [
  { id: 'cua' as const, kind: 'native' as const, group: 'desktop' as const, name: 'Cua Driver', skill: 'cua-driver', icon: Monitor },
  { id: 'playwright' as const, kind: 'native' as const, group: 'browser' as const, name: 'Playwright CLI', skill: 'playwright-cli', icon: Globe2 },
  { id: 'ego-lite' as const, kind: 'plugin' as const, group: 'browser' as const, name: 'ego lite', icon: Globe2 },
  { id: 'officecli' as const, kind: 'plugin' as const, group: 'document' as const, name: 'OfficeCLI', icon: FileSpreadsheet },
]

const CONTROL_GROUPS = [
  { id: 'desktop' as const, zh: '电脑操作', en: 'Computer control' },
  { id: 'browser' as const, zh: '浏览器操作', en: 'Browser control' },
  { id: 'document' as const, zh: '文档操作', en: 'Document control' },
]

const CUA_MCP_ID = 'computer-control-cua-driver'
const LEGACY_CUA_MCP_CONNECTOR_ID = 'plugin:cua-driver'

function isCuaMcp(server: ChatMcpServer): boolean {
  return server.id === CUA_MCP_ID
    || server.id === 'plugin-cua-driver'
    || server.connectorId === 'computer-control:cua'
    || server.connectorId === LEGACY_CUA_MCP_CONNECTOR_ID
}

function cuaMcpServer(current?: ChatMcpServer, enabled = true): ChatMcpServer {
  return {
    id: CUA_MCP_ID,
    name: 'Cua Driver',
    enabled,
    transport: 'stdio',
    url: '',
    command: current?.command || 'cua-driver',
    args: ['mcp'],
    env: current?.env ?? {},
    headers: {},
    cwd: current?.cwd ?? null,
    enabledTools: current?.enabledTools ?? [],
  }
}

function formatControlVersion(output: string): string {
  const match = output.match(/\bv?(\d+\.\d+(?:\.\d+)?(?:[-+][0-9A-Za-z.-]+)?)\b/i)
  return match ? `v${match[1]}` : output.trim()
}

function scanPathKey(paths: string[]): string {
  return JSON.stringify(paths)
}

function readDetectionCache(paths: string[]): ControlDetectionSnapshot | null {
  try {
    const raw = window.sessionStorage.getItem(CONTROL_CACHE_KEY)
    if (!raw) return null
    const parsed = JSON.parse(raw) as Partial<ControlDetectionSnapshot>
    if (!parsed.versions || !parsed.updates || !Array.isArray(parsed.skills) || !Array.isArray(parsed.plugins) || !Array.isArray(parsed.skillScanPaths)) {
      return null
    }
    if (scanPathKey(parsed.skillScanPaths) !== scanPathKey(paths)) return null
    return parsed as ControlDetectionSnapshot
  } catch {
    return null
  }
}

function writeDetectionCache(snapshot: ControlDetectionSnapshot): void {
  try {
    window.sessionStorage.setItem(CONTROL_CACHE_KEY, JSON.stringify(snapshot))
  } catch {
    // The page can still work without web storage; it will simply detect again next time.
  }
}

function updatePluginInDetectionCache(paths: string[], status: PluginStatus): void {
  const cached = readDetectionCache(paths)
  if (!cached) return
  writeDetectionCache({
    ...cached,
    plugins: cached.plugins.map(plugin => plugin.id === status.id ? status : plugin),
  })
}

function detectControls(skillScanPaths: string[]): Promise<ControlDetectionSnapshot> {
  const key = scanPathKey(skillScanPaths)
  if (detectionInFlight?.key === key) return detectionInFlight.promise

  const promise = Promise.all([
    Promise.all(CONTROL_TOOLS.filter(tool => tool.kind === 'native').map(async tool => {
      try {
        const status = await api.computerControlStatus(tool.id)
        return [tool.id, status] as const
      } catch {
        return [tool.id, null] as const
      }
    })),
    api.chatSkillsList(skillScanPaths).catch(() => ({ success: false, skills: [] as SkillMeta[] })),
    api.pluginsList().catch(() => [] as PluginStatus[]),
  ]).then(([cliResults, skillResult, plugins]) => {
    const updates = Object.fromEntries(cliResults.filter((entry): entry is readonly [NativeTool, ControlToolStatus] => entry[1] !== null))
    const snapshot: ControlDetectionSnapshot = {
      versions: Object.fromEntries(cliResults.map(([id, status]) => [id, status ? formatControlVersion(status.currentVersion) : ''])),
      updates,
      skills: skillResult.skills,
      plugins,
      skillScanPaths: [...skillScanPaths],
    }
    writeDetectionCache(snapshot)
    return snapshot
  }).finally(() => {
    if (detectionInFlight?.promise === promise) detectionInFlight = null
  })

  detectionInFlight = { key, promise }
  return promise
}

export function ComputerControlTab({ lang, tools, onChange }: {
  lang: Lang
  tools: ChatToolsConfig
  onChange: (updates: Partial<ChatToolsConfig>) => void
}) {
  const zh = lang === 'zh'
  const initialCache = useRef(readDetectionCache(tools.skillScanPaths)).current
  const [versions, setVersions] = useState<Partial<Record<NativeTool, string>>>(initialCache?.versions ?? {})
  const [updates, setUpdates] = useState<Partial<Record<NativeTool, ControlToolStatus>>>(initialCache?.updates ?? {})
  const [skills, setSkills] = useState<SkillMeta[]>(initialCache?.skills ?? [])
  const [plugins, setPlugins] = useState<PluginStatus[]>(initialCache?.plugins ?? [])
  const [loading, setLoading] = useState(initialCache === null)
  const [installing, setInstalling] = useState<Tool | null>(null)
  const [updating, setUpdating] = useState<NativeTool | null>(null)
  const [error, setError] = useState('')
  const latest = useRef({ tools, onChange })
  const mounted = useRef(false)
  latest.current = { tools, onChange }

  const reload = useCallback(async (showLoading: boolean) => {
    if (showLoading) setLoading(true)
    const snapshot = await detectControls(latest.current.tools.skillScanPaths)
    if (!mounted.current) return
    setVersions(snapshot.versions)
    setUpdates(snapshot.updates)
    setSkills(snapshot.skills)
    setPlugins(snapshot.plugins)
    setLoading(false)
  }, [])

  useEffect(() => {
    mounted.current = true
    if (!initialCache) void reload(true)
    return () => { mounted.current = false }
  }, [initialCache, reload])

  const setNativeControlEnabled = (tool: NativeTool, id: string, enabled: boolean) => {
    const { tools: current, onChange: change } = latest.current
    const disabled = new Set(current.disabledSkillIds ?? [])
    if (enabled) disabled.delete(id)
    else disabled.add(id)
    const updates: Partial<ChatToolsConfig> = {
      disabledSkillIds: [...disabled],
      ...(enabled ? {
        enabled: true,
        nativeTools: {
          ...current.nativeTools,
          skillRuntime: true,
          runCommand: true,
          readFile: true,
        },
      } : {}),
    }
    if (tool === 'cua') {
      const existing = current.servers.find(isCuaMcp)
      updates.servers = [
        ...current.servers.filter(server => !isCuaMcp(server)),
        cuaMcpServer(existing, enabled),
      ]
    }
    change(updates)
  }

  const install = async (tool: Tool) => {
    setInstalling(tool)
    setError('')
    try {
      if (tool === 'ego-lite' || tool === 'officecli') {
        const result = await api.pluginsRunOfficialInstall(tool)
        if (!mounted.current) return
        setPlugins(current => current.map(plugin => plugin.id === result.status.id ? result.status : plugin))
        await refreshSettings().catch(() => undefined)
      } else {
        const skill = await api.computerControlInstall(tool)
        if (!mounted.current) return
        setNativeControlEnabled(tool, skill.id, true)
      }
      await reload(false)
    } catch {
      if (mounted.current) setError(zh ? '安装失败，请稍后重试。' : 'Installation failed. Please try again.')
    } finally {
      if (mounted.current) setInstalling(null)
    }
  }

  const update = async (tool: NativeTool) => {
    setUpdating(tool)
    setError('')
    try {
      const skill = await api.computerControlUpdate(tool)
      if (!mounted.current) return
      setNativeControlEnabled(tool, skill.id, true)
      await reload(false)
    } catch (cause) {
      console.error('Failed to update computer-control tool:', cause)
      const detail = typeof cause === 'string'
        ? cause
        : cause instanceof Error
          ? cause.message
          : ''
      if (mounted.current) {
        setError(detail
          ? `${zh ? '更新失败' : 'Update failed'}：${detail.slice(0, 240)}`
          : (zh ? '更新失败，请稍后重试。' : 'Update failed. Please try again.'))
      }
    } finally {
      if (mounted.current) setUpdating(null)
    }
  }

  const setPluginEnabled = async (id: 'ego-lite' | 'officecli', enabled: boolean) => {
    setInstalling(id)
    setError('')
    try {
      const result = await api.pluginsSetEnabled(id, enabled)
      if (!mounted.current) return
      setPlugins(current => current.map(plugin => plugin.id === result.status.id ? result.status : plugin))
      updatePluginInDetectionCache(latest.current.tools.skillScanPaths, result.status)
      await refreshSettings().catch(() => undefined)
    } catch {
      if (mounted.current) setError(zh ? '更新失败，请稍后重试。' : 'Update failed. Please try again.')
    } finally {
      if (mounted.current) setInstalling(null)
    }
  }

  const runtimeEnabled = tools.enabled
    && tools.nativeTools.skillRuntime !== false
    && tools.nativeTools.runCommand === true
    && tools.nativeTools.readFile === true

  return (
    <>
      {CONTROL_GROUPS.map(group => (
        <section className="computer-control-section" key={group.id}>
          <h3 className="computer-control-heading">{zh ? group.zh : group.en}</h3>
          <div className="computer-control-card">
            {CONTROL_TOOLS.filter(tool => tool.group === group.id).map(tool => {
              const plugin = tool.kind === 'plugin' ? plugins.find(item => item.id === tool.id) : undefined
              const skill = tool.kind === 'native'
                ? skills.find(item => item.id === tool.skill || item.name === tool.skill)
                : undefined
              const mcp = tool.id === 'cua' ? tools.servers.find(isCuaMcp) : undefined
              const installed = tool.kind === 'plugin' ? plugin?.installed === true : undefined
              const skillCount = tool.kind === 'plugin' ? (installed ? plugin?.skillCount ?? 0 : 0) : (skill ? 1 : 0)
              const mcpCount = tool.kind === 'plugin' ? (installed ? plugin?.mcpCount ?? 0 : 0) : (mcp ? 1 : 0)
              const componentCounts = [
                skillCount > 0 ? `${skillCount} Skill` : '',
                mcpCount > 0 ? `${mcpCount} MCP` : '',
              ].filter(Boolean).join(' · ')
              const version = tool.kind === 'plugin'
                ? (plugin?.version ? formatControlVersion(plugin.version) : '')
                : versions[tool.id] ?? ''
              const updateStatus = tool.kind === 'native' ? updates[tool.id] : undefined
              const ready = tool.kind === 'plugin'
                ? installed === true
                : !!version && !!skill && (tool.id !== 'cua' || !!mcp)
              const enabled = tool.kind === 'plugin'
                ? plugin?.enabled === true
                : ready
                  && runtimeEnabled
                  && !(tools.disabledSkillIds ?? []).includes(skill!.id)
                  && (tool.id !== 'cua' || mcp?.enabled === true)
              const Icon = tool.icon
              const description = version
                ? `${version}${componentCounts ? ` · ${componentCounts}` : ''}`
                : (zh ? '未安装' : 'Not installed')

              return (
                <div className="computer-control-row" key={tool.id}>
                  <span className={`computer-control-icon computer-control-icon--${tool.id}`} aria-hidden="true">
                    <Icon size={18} strokeWidth={1.8} />
                  </span>
                  <div className="computer-control-copy">
                    <div className="computer-control-name">{tool.name}</div>
                    <div className="computer-control-description">
                      {!loading && <span className={`computer-control-status-dot ${ready ? 'is-ready' : ''}`} />}
                      {loading ? (zh ? '正在检测…' : 'Checking…') : description}
                    </div>
                  </div>
                  <div className="computer-control-action">
                    {loading ? (
                      <RefreshCw size={14} className="animate-spin text-neutral-400" aria-label={zh ? '正在检测' : 'Checking'} />
                    ) : ready ? (
                      <>
                        {tool.kind === 'native' && updateStatus?.updateAvailable && (
                          <Button
                            size="sm"
                            disabled={installing !== null || updating !== null}
                            title={updateStatus.latestVersion ? `${zh ? '最新版本' : 'Latest'} v${updateStatus.latestVersion}` : undefined}
                            onClick={() => { void update(tool.id) }}
                          >
                            {updating === tool.id && <RefreshCw size={12} className="animate-spin" />}
                            {updating === tool.id ? (zh ? '更新中…' : 'Updating…') : (zh ? '更新' : 'Update')}
                          </Button>
                        )}
                        <Toggle
                          checked={enabled}
                          disabled={installing !== null || updating !== null}
                          ariaLabel={`${tool.name} ${zh ? '控制' : 'control'}`}
                          onChange={value => {
                            if (tool.kind === 'plugin') void setPluginEnabled(tool.id, value)
                            else setNativeControlEnabled(tool.id, skill!.id, value)
                          }}
                        />
                      </>
                    ) : (
                      <Button
                        size="sm"
                        disabled={installing !== null || (tool.kind === 'plugin' && plugin?.canInstall !== true)}
                        onClick={() => { void install(tool.id) }}
                      >
                        {installing === tool.id && <RefreshCw size={12} className="animate-spin" />}
                        {installing === tool.id ? (zh ? '安装中…' : 'Installing…') : (zh ? '安装' : 'Install')}
                      </Button>
                    )}
                  </div>
                </div>
              )
            })}
          </div>
        </section>
      ))}
      {error && <p role="alert" className="computer-control-error">{error}</p>}
    </>
  )
}
