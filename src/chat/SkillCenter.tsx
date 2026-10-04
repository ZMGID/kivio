import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode, type CSSProperties } from 'react'
import {
  ChevronDown,
  Download,
  ExternalLink,
  FolderOpen,
  Plus,
  RefreshCw,
  Search,
  Sliders,
  Sparkles,
  Trash2,
  X,
} from 'lucide-react'
import { open } from '@tauri-apps/plugin-dialog'
import { ChatMarkdown } from './ChatMarkdown'
import {
  api,
  type ChatToolsConfig,
  type Settings,
  type SkillDetail,
  type SkillMeta,
} from '../api/tauri'
import { getSettingsCached, updateSettingsCached } from '../api/settingsCache'
import { Select, Toggle } from '../settings/public/controls'
import { useT, type I18n } from '../components/i18n'
import { Button, IconButton } from '../components/Button'
import { SkillStoreBrowser } from './SkillStoreBrowser'
import { DefaultSkillIcon, SkillIcon } from '../settings/public/icons'
import { useWindowStore } from '../utils/windowStore'
import {
  CLI_SKILL_SOURCES,
  deleteInstalledSkill,
  effectiveDisabledSkillIds,
  importSelectedCliSkills,
  importSkillFolder,
  importSkillZip,
  installSkillFromUrl,
  markSkillInventoryNotified,
  markSkillInventoryRefreshed,
  refreshSkillInventory,
  retryFailedSkillEnables,
  scanCliSkills,
  setSkillActionError,
  setSkillEnabled,
  setSkillUrlDraft,
  setSkillView,
  skillLifecycleStore,
  subscribeSkillSettingsSaved,
  toggleCliSkillSelected,
} from './skillLifecycle'

interface SkillCenterProps {
  heading?: ReactNode
  /** Skill 启用状态 / 列表变化后通知 Chat 刷新其技能列表 */
  onSkillsChanged?: () => void
  /** 当前对话工作目录：扫描项目 `.kivio/skills` 与 `.agents/skills` */
  projectCwd?: string
}

function isBuiltinSkill(skill: SkillMeta): boolean {
  return skill.source === 'builtin'
}

function isPluginSkill(skill: SkillMeta): boolean {
  return skill.source === 'plugin'
}

function isProjectSkill(skill: SkillMeta): boolean {
  return skill.source === 'project'
}

function isGlobalAgentsSkill(skill: SkillMeta): boolean {
  return skill.source === 'agents'
}

function canDeleteSkill(skill: SkillMeta): boolean {
  return skill.source === 'user'
}

function skillSourceLabel(skill: SkillMeta, t: I18n): string {
  if (skill.source === 'builtin') return t.chatSkillSourceBuiltin
  if (skill.source === 'plugin') return t.chatSkillSourcePlugin
  if (skill.source === 'external') return t.chatSkillSourceWorkspace
  if (skill.source === 'project') return t.chatSkillSourceProject
  if (skill.source === 'agents') return t.chatSkillSourceGlobal
  return t.chatSkillSourcePersonal
}

function ownsSkillPreview(ownership: {
  mounted: boolean
  request: number
  currentRequest: number
  navigationEpoch: number
  currentNavigationEpoch: number
  projectCwd: string | undefined
  currentProjectCwd: string | undefined
}): boolean {
  return ownership.mounted
    && ownership.request === ownership.currentRequest
    && ownership.navigationEpoch === ownership.currentNavigationEpoch
    && ownership.projectCwd === ownership.currentProjectCwd
}

function skillMatches(skill: SkillMeta, query: string): boolean {
  if (!query) return true
  return (
    skill.name.toLowerCase().includes(query) ||
    (skill.description ?? '').toLowerCase().includes(query)
  )
}

/** 自带样式的开关：明暗对比清晰，不依赖设置面板的 CSS 变量作用域 */
function SkillCard({
  skill,
  enabled,
  index,
  onToggleEnabled,
  onPreview,
  onDelete,
  manageLocked = false,
  toggleBusy = false,
  deleteBusy = false,
}: {
  skill: SkillMeta
  enabled: boolean
  /** 卡片入场 stagger 序号 */
  index: number
  onToggleEnabled: (skillId: string, enabled: boolean) => void
  onPreview: (skillId: string) => void
  /** 删除个人/导入技能（仅 source==='user' 显示）；不传则不显示删除 */
  onDelete?: (skill: SkillMeta) => void
  /** 插件附属：开关在插件页，此处只展示 */
  manageLocked?: boolean
  toggleBusy?: boolean
  deleteBusy?: boolean
}) {
  const t = useT()
  return (
    <div
      role="button"
      tabIndex={0}
      onClick={() => onPreview(skill.id)}
      onKeyDown={(event) => {
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault()
          onPreview(skill.id)
        }
      }}
      data-tauri-drag-region="false"
      title={t.chatSkillViewFull}
      style={{ '--chat-motion-delay': `${Math.min(index, 8) * 24}ms` } as CSSProperties}
      className={`chat-motion-fade-up group flex h-full min-w-0 cursor-pointer flex-col rounded-xl border p-3.5 text-left transition-[border-color,box-shadow,transform,background-color] duration-[var(--kv-dur-fast)] ease-[var(--kv-ease-standard)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-neutral-900/15 ${
        enabled
          ? 'border-neutral-200 bg-neutral-50 shadow-sm hover:-translate-y-0.5 hover:border-neutral-300 hover:shadow-md'
          : 'border-neutral-200/80 bg-neutral-50/60 hover:-translate-y-0.5 hover:border-neutral-300 hover:bg-neutral-50 hover:shadow-md'
      }`}
    >
      <div className="flex items-start justify-between gap-2">
        <span
          className={`grid size-10 shrink-0 place-items-center rounded-lg border transition-colors duration-[var(--kv-dur-fast)] ${
            enabled
              ? 'border-neutral-200 bg-neutral-50 text-neutral-600'
              : 'border-neutral-200/80 bg-neutral-100/80 text-neutral-400 group-hover:text-neutral-500 dark:text-neutral-600'
          }`}
        >
          <DefaultSkillIcon size={18} strokeWidth={1.75} />
        </span>
        {manageLocked ? (
          <span
            className="shrink-0 pt-0.5 text-[11px] text-neutral-400 dark:text-neutral-500"
            title={t.chatSkillPluginManageHint}
          >
            {enabled ? t.chatSkillPluginEnabled : t.chatSkillPluginDisabled}
          </span>
        ) : (
          <span
            className="shrink-0"
            onClick={(event) => event.stopPropagation()}
            onKeyDown={(event) => event.stopPropagation()}
          >
            <Toggle checked={enabled} disabled={toggleBusy} onChange={(next) => onToggleEnabled(skill.id, next)} ariaLabel={t.chatSkillEnableNamed.replace('{name}', skill.name)} />
          </span>
        )}
      </div>
      <div className="mt-2.5 min-w-0 flex-1">
        <div className={`truncate text-[13.5px] font-semibold leading-tight ${
          enabled ? 'text-neutral-950' : 'text-neutral-600 dark:text-neutral-400'
        }`}>
          {skill.name}
        </div>
        <p className="mt-1 line-clamp-2 text-[12px] leading-[1.45] text-neutral-500 dark:text-neutral-400">
          {skill.description || t.chatSkillNoDescription}
        </p>
      </div>
      <div className="mt-2.5 flex min-h-6 items-center gap-1 border-t border-neutral-100 pt-2 text-[11px] text-neutral-400 dark:text-neutral-500">
        <span className="truncate">{skillSourceLabel(skill, t)}</span>
        {onDelete && canDeleteSkill(skill) && !manageLocked ? (
          <span
            className="ml-auto shrink-0 opacity-0 transition-opacity duration-[var(--kv-dur-fast)] focus-within:opacity-100 group-hover:opacity-100"
            onClick={(event) => event.stopPropagation()}
            onKeyDown={(event) => event.stopPropagation()}
          >
            <IconButton
              size="sm"
              className="danger"
              disabled={deleteBusy}
              onClick={() => onDelete(skill)}
              label={t.chatSkillDeleteNamed.replace('{name}', skill.name)}
              title={t.chatSkillDelete}
            >
              <Trash2 size={14} strokeWidth={1.75} />
            </IconButton>
          </span>
        ) : null}
      </div>
    </div>
  )
}

function SkillSection({
  title,
  note,
  emptyText,
  skills,
  disabledSkillIds,
  onToggleEnabled,
  onPreview,
  onDelete,
  collapsible = false,
  defaultCollapsed = false,
  manageLocked = false,
  lockedActiveIds,
  enableBusyIds,
  deleteBusyIds,
}: {
  title: string
  note?: string
  emptyText: string
  skills: SkillMeta[]
  disabledSkillIds: string[]
  onToggleEnabled: (skillId: string, enabled: boolean) => void
  onPreview: (skillId: string) => void
  onDelete?: (skill: SkillMeta) => void
  collapsible?: boolean
  defaultCollapsed?: boolean
  manageLocked?: boolean
  lockedActiveIds?: Set<string>
  enableBusyIds?: readonly string[]
  deleteBusyIds?: readonly string[]
}) {
  const [collapsed, setCollapsed] = useState(collapsible && defaultCollapsed)
  const t = useT()
  const enabledCount = skills.filter((skill) =>
    manageLocked ? Boolean(lockedActiveIds?.has(skill.id)) : !disabledSkillIds.includes(skill.id),
  ).length
  return (
    <section className="space-y-2.5">
      <div
        className={`flex min-w-0 items-center gap-3 px-1 ${collapsible ? 'cursor-pointer select-none' : ''}`}
        onClick={collapsible ? () => setCollapsed((v) => !v) : undefined}
      >
        {collapsible && (
          <ChevronDown
            size={16}
            className={`shrink-0 text-neutral-400 transition-transform duration-[var(--kv-dur-fast)] ease-[var(--kv-ease-standard)] ${collapsed ? '-rotate-90' : ''}`}
          />
        )}
        <h3 className="text-[15px] font-semibold text-neutral-700">{title}</h3>
        <span className="text-[14px] font-medium text-neutral-400">{skills.length}</span>
        {collapsed && skills.length > 0 && (
          <span className="text-[12.5px] text-neutral-400">{t.chatSkillsEnabledCount.replace('{n}', String(enabledCount))}</span>
        )}
        {note && <span className="ml-auto truncate text-[12.5px] text-neutral-400">{note}</span>}
      </div>
      {collapsed ? null : skills.length === 0 ? (
        <div className="grid min-h-[72px] place-items-center rounded-md border border-dashed border-neutral-200 text-[13px] text-neutral-400">
          {emptyText}
        </div>
      ) : (
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {skills.map((skill, index) => (
            <SkillCard
              key={skill.id}
              skill={skill}
              index={index}
              enabled={
                manageLocked
                  ? Boolean(lockedActiveIds?.has(skill.id))
                  : !disabledSkillIds.includes(skill.id)
              }
              onToggleEnabled={onToggleEnabled}
              onPreview={onPreview}
              onDelete={onDelete}
              manageLocked={manageLocked}
              toggleBusy={enableBusyIds?.includes(skill.id)}
              deleteBusy={deleteBusyIds?.includes(skill.id)}
            />
          ))}
        </div>
      )}
    </section>
  )
}

function SkillUrlImport() {
  const t = useT()
  const [ops] = useWindowStore(skillLifecycleStore)
  const busy = ops.busyKeys.includes('url')
  const install = useCallback(() => {
    void installSkillFromUrl({ failed: t.chatSkillInstallFailed, installed: t.chatSkillInstalled })
  }, [t])
  return (
    <div className="rounded-md border border-neutral-200 p-3">
      <div className="mb-1.5 text-[13px] font-medium text-neutral-800">{t.chatSkillInstallFromUrl}</div>
      <p className="mb-2 text-[12px] text-neutral-500 dark:text-neutral-400">
        {t.chatSkillUrlImportHint}
      </p>
      <div className="flex items-center gap-2">
        <input
          type="text"
          value={ops.urlDraft}
          onChange={(e) => setSkillUrlDraft(e.target.value)}
          placeholder="https://github.com/owner/repo"
          className="h-9 w-full rounded-md border border-neutral-200 bg-neutral-50 px-2.5 font-mono text-[12.5px] text-neutral-800 outline-none focus:border-neutral-300"
          data-tauri-drag-region="false"
        />
        <Button onClick={install} disabled={busy || !ops.urlDraft.trim()} data-tauri-drag-region="false">
          {busy ? t.chatSkillInstalling : t.chatSkillInstall}
        </Button>
      </div>
      {ops.urlError && <div className="mt-2 text-[12px] text-red-600 dark:text-red-400">{ops.urlError}</div>}
      {ops.urlDone && <div className="mt-2 text-[12px] text-emerald-600 dark:text-emerald-400">{ops.urlDone}</div>}
    </div>
  )
}

export function SkillCenter({ onSkillsChanged, projectCwd, heading }: SkillCenterProps) {
  const t = useT()
  const [ops] = useWindowStore(skillLifecycleStore)
  const [settings, setSettings] = useState<Settings | null>(null)
  const [query, setQuery] = useState('')
  const view = ops.view
  const [selectedSkillPreview, setSelectedSkillPreview] = useState<SkillDetail | null>(null)

  const settingsRef = useRef<Settings | null>(null)
  const saveTimer = useRef<number | null>(null)
  const previewMountedRef = useRef(true)
  const previewRequestRef = useRef(0)
  const projectCwdRef = useRef(projectCwd)
  projectCwdRef.current = projectCwd
  const skills = ops.skills
  const skillsLoading = ops.skillsLoading
  const skillError = ops.actionError || ops.listError
  const cliSkills = ops.cliSkills
  const cliScanning = ops.busyKeys.includes('cli-scan')
  const cliImporting = ops.busyKeys.includes('cli-import')
  const cliImportDone = ops.cliImportDone
  const enabledPluginSkillIds = useMemo(() => new Set(ops.enabledPluginSkillIds), [ops.enabledPluginSkillIds])

  const chatTools = settings?.chatTools
  const disabledSkillIds = effectiveDisabledSkillIds(chatTools?.disabledSkillIds, ops.enableIntents)
  const skillScanPaths = chatTools?.skillScanPaths ?? []
  const enableBusyIds = ops.busyKeys.filter((key) => key.startsWith('enable:')).map((key) => key.slice('enable:'.length))
  const deleteBusyIds = ops.busyKeys.filter((key) => key.startsWith('delete:')).map((key) => key.slice('delete:'.length))

  const refreshChatSkills = useCallback((scanPaths?: string[]) => {
    return refreshSkillInventory({
      scanPaths: scanPaths ?? settingsRef.current?.chatTools?.skillScanPaths,
      projectCwd: projectCwd ?? null,
      listFailedLabel: t.chatSkillListLoadFailed,
    })
  }, [projectCwd, t])

  useEffect(() => {
    previewMountedRef.current = true
    return () => { previewMountedRef.current = false }
  }, [])

  useEffect(() => subscribeSkillSettingsSaved((saved) => {
    settingsRef.current = saved
    setSettings(saved)
  }), [])

  useEffect(() => {
    let mounted = true
    void (async () => {
      try {
        const loaded = await getSettingsCached()
        if (mounted) {
          settingsRef.current = loaded
          setSettings(loaded)
        }
        await refreshSkillInventory({
          scanPaths: loaded.chatTools.skillScanPaths,
          projectCwd: projectCwd ?? null,
          listFailedLabel: t.chatSkillListLoadFailed,
        })
      } catch (err) {
        if (mounted) setSkillActionError(err instanceof Error ? err.message : String(err))
      }
    })()
    return () => {
      mounted = false
      if (saveTimer.current) window.clearTimeout(saveTimer.current)
    }
  }, [projectCwd, t])

  const refreshingGeneration = useRef(0)
  useEffect(() => {
    const snap = skillLifecycleStore.getSnapshot()
    if (snap.settledGeneration === 0) return
    const generation = snap.settledGeneration
    if (snap.notifiedGeneration !== generation) {
      markSkillInventoryNotified(generation)
      const claimed = skillLifecycleStore.getSnapshot()
      if (claimed.notifiedGeneration === generation) onSkillsChanged?.()
    }
    const pending = skillLifecycleStore.getSnapshot()
    if (pending.refreshedGeneration === generation || refreshingGeneration.current === generation) return
    refreshingGeneration.current = generation
    let active = true
    void refreshSkillInventory({
      projectCwd: projectCwd ?? null,
      listFailedLabel: t.chatSkillListLoadFailed,
    }).then(() => {
      if (!active) return
      const latest = skillLifecycleStore.getSnapshot()
      if (latest.settledGeneration !== generation) return
      markSkillInventoryRefreshed(generation)
    })
    return () => { active = false }
  }, [onSkillsChanged, ops.notifiedGeneration, ops.refreshedGeneration, ops.settledGeneration, projectCwd, t])

  const flushSave = useCallback(async (next: Settings) => {
    try {
      // 只把技能页改过的字段盖到 fresh 上，避免把 MCP / 收藏 / 插件开关盖回旧值。
      // 启用状态由 setSkillEnabled 单独提交，这里不回写，免得并行保存把未完成的开关盖掉。
      const nextTools = next.chatTools
      const saved = await updateSettingsCached((fresh) => ({
        ...fresh,
        chatTools: {
          ...fresh.chatTools,
          skillScanPaths: nextTools.skillScanPaths,
          skillAutoMatch: nextTools.skillAutoMatch,
          skillFallbackMode: nextTools.skillFallbackMode,
        },
      }))
      settingsRef.current = saved
      setSettings(saved)
      onSkillsChanged?.()
    } catch (err) {
      setSkillActionError(err instanceof Error ? err.message : String(err))
    }
  }, [onSkillsChanged])

  // 更新 chatTools：本地立即生效，再持久化（文本类编辑防抖，开关/下拉立即保存）
  const persistChatTools = useCallback((updates: Partial<ChatToolsConfig>, debounce = false) => {
    setSettings((prev) => {
      if (!prev) return prev
      const next: Settings = {
        ...prev,
        chatTools: { ...prev.chatTools, ...updates },
      }
      settingsRef.current = next
      if (saveTimer.current) {
        window.clearTimeout(saveTimer.current)
        saveTimer.current = null
      }
      if (debounce) {
        saveTimer.current = window.setTimeout(() => {
          saveTimer.current = null
          void flushSave(next)
        }, 500)
      } else {
        void flushSave(next)
      }
      return next
    })
  }, [flushSave])

  const handleToggleSkillEnabled = useCallback((skillId: string, enabled: boolean) => {
    void setSkillEnabled(skillId, enabled)
  }, [])

  const handlePreviewSkill = useCallback(async (skillId: string) => {
    const request = ++previewRequestRef.current
    const navigationEpoch = skillLifecycleStore.getSnapshot().navigationEpoch
    const cwd = projectCwd
    setSkillActionError('')
    const baselineError = skillLifecycleStore.getSnapshot().actionError
    const owned = () => ownsSkillPreview({
      mounted: previewMountedRef.current,
      request,
      currentRequest: previewRequestRef.current,
      navigationEpoch,
      currentNavigationEpoch: skillLifecycleStore.getSnapshot().navigationEpoch,
      projectCwd: cwd,
      currentProjectCwd: projectCwdRef.current,
    })
    try {
      const result = await api.chatSkillsRead(skillId, cwd || undefined)
      if (!owned()) return
      if (result.success && result.skill) {
        setSelectedSkillPreview(result.skill)
        return
      }
      if (!owned() || skillLifecycleStore.getSnapshot().actionError !== baselineError) return
      setSkillActionError(result.error || t.chatSkillReadFailed)
    } catch (err) {
      if (!owned() || skillLifecycleStore.getSnapshot().actionError !== baselineError) return
      setSkillActionError(err instanceof Error ? err.message : String(err))
    }
  }, [projectCwd, t])

  const handleImportSkill = useCallback(() => {
    void importSkillFolder(t.chatSkillImportFailed)
  }, [t])

  const handleDeleteSkill = useCallback((skill: SkillMeta) => {
    void deleteInstalledSkill(skill, {
      confirm: t.chatSkillDeleteConfirm.replace('{name}', skill.name),
      confirmLabel: t.dialogDelete,
    })
  }, [t])

  const handleImportSkillZip = useCallback(() => {
    void importSkillZip(t.chatSkillImportFailed)
  }, [t])

  const handleOpenSkillFolder = useCallback(async () => {
    setSkillActionError('')
    try {
      const result = await api.chatSkillsOpenFolder()
      if (!result.success) {
        setSkillActionError(result.error || t.chatSkillOpenFolderFailed)
      }
    } catch (err) {
      setSkillActionError(err instanceof Error ? err.message : String(err))
    }
  }, [t])

  const handleCliScan = useCallback(() => {
    void scanCliSkills(t.chatSkillScanFailed)
  }, [t])

  const handleCliImportSelected = useCallback(() => {
    void importSelectedCliSkills({
      importNamedFailed: (name) => t.chatSkillImportNamedFailed.replace('{name}', name),
      done: (count) => t.chatSkillCliImportDone.replace('{n}', String(count)),
    })
  }, [t])

  const normalizedQuery = query.trim().toLowerCase()
  const builtinSkills = useMemo(
    () => skills.filter((skill) => isBuiltinSkill(skill) && skillMatches(skill, normalizedQuery)),
    [skills, normalizedQuery],
  )
  const pluginSkills = useMemo(
    () => skills.filter((skill) => isPluginSkill(skill) && skillMatches(skill, normalizedQuery)),
    [skills, normalizedQuery],
  )
  const projectSkills = useMemo(
    () => skills.filter((skill) => isProjectSkill(skill) && skillMatches(skill, normalizedQuery)),
    [skills, normalizedQuery],
  )
  const globalSkills = useMemo(
    () => skills.filter((skill) => isGlobalAgentsSkill(skill) && skillMatches(skill, normalizedQuery)),
    [skills, normalizedQuery],
  )
  const personalSkills = useMemo(
    () =>
      skills.filter(
        (skill) =>
          !isBuiltinSkill(skill)
          && !isPluginSkill(skill)
          && !isProjectSkill(skill)
          && !isGlobalAgentsSkill(skill)
          && skillMatches(skill, normalizedQuery),
      ),
    [skills, normalizedQuery],
  )

  return (
    <div className="assistant-center-root flex h-full min-h-0 flex-col text-neutral-900">
      {/* 顶栏：与聊天主区同底色、无分隔；可拖拽，右侧避开窗口按钮 */}

      {/* 内容区：直接坐在白底上，与聊天主区无缝 */}
      <main className={heading ? "kv-market-scroll custom-scrollbar" : "custom-scrollbar min-h-0 flex-1 overflow-y-auto"}>
          <div className={heading ? "w-full pb-4" : "mx-auto w-full max-w-[1040px] px-9 pb-10 pt-7"}>
            {/* 头部：标题 + 副标题 + 图标动作 */}
            <div className="border-b border-neutral-200 pb-5">
              {heading ?? (<h1 className="flex items-center gap-2.5 text-[28px] font-semibold tracking-normal text-neutral-950">
                <SkillIcon size={24} className="text-neutral-500" />
                Skill
              </h1>)}
              <div className="mt-3.5 flex min-w-0 items-center gap-4">
              <p className="min-w-0 flex-1 text-[14px] leading-relaxed text-neutral-500 dark:text-neutral-400">
                {t.chatSkillPageSubtitle}
              </p>
              <div className="flex shrink-0 items-center gap-0.5">
                <IconButton
                  size="lg"
                  label={t.chatSkillImportFolder}
                  onClick={() => void handleImportSkill()}
                  disabled={ops.busyKeys.includes('import-folder')}
                  data-tauri-drag-region="false"
                >
                  <FolderOpen size={17} />
                </IconButton>
                <IconButton
                  size="lg"
                  label={t.chatSkillImportZip}
                  onClick={() => void handleImportSkillZip()}
                  disabled={ops.busyKeys.includes('import-zip')}
                  data-tauri-drag-region="false"
                >
                  <Download size={17} />
                </IconButton>
                <IconButton
                  size="lg"
                  label={t.chatSkillOpenSkillFolder}
                  onClick={() => void handleOpenSkillFolder()}
                  data-tauri-drag-region="false"
                >
                  <ExternalLink size={17} />
                </IconButton>
                <IconButton
                  size="lg"
                  label={t.chatSkillRefreshList}
                  onClick={() => void refreshChatSkills()}
                  disabled={skillsLoading}
                  data-tauri-drag-region="false"
                >
                  <RefreshCw size={17} className={skillsLoading ? 'animate-spin' : ''} />
                </IconButton>
              </div>
            </div>
          </div>

          {/* Tab 行 */}
          <div className="mt-5 flex items-center gap-1 border-b border-neutral-200">
            {([['installed', t.chatSkillTabInstalled], ['store', t.chatSkillTabStore], ['import', t.chatSkillTabImport], ['advanced', t.chatSkillTabAdvanced]] as const).map(([id, label]) => (
              <button
                key={id}
                type="button"
                onClick={() => setSkillView(id)}
                data-tauri-drag-region="false"
                className={`relative px-3 py-2 text-[13px] font-medium transition-colors ${
                  view === id
                    ? 'text-neutral-900'
                    : 'text-neutral-500 hover:text-neutral-800 dark:text-neutral-400'
                }`}
              >
                {label}
                {id === 'installed' && skills.length > 0 && (
                  <span className="ml-1.5 text-[11px] tabular-nums text-neutral-400">{skills.length}</span>
                )}
                {view === id && (
                  <span className="chat-motion-tab-underline absolute inset-x-2 -bottom-px h-0.5 rounded-full bg-accent" />
                )}
              </button>
            ))}
          </div>

          {view === 'store' ? (
            <div key="store" className="chat-motion-tab-in mt-5 flex min-h-[420px] flex-col">
              <SkillStoreBrowser />
            </div>
          ) : view === 'import' ? (
            <div key="import" className="chat-motion-tab-in mt-5 space-y-4">
              <div className="flex flex-wrap gap-2">
                <Button onClick={() => void handleImportSkill()} disabled={ops.busyKeys.includes('import-folder')} data-tauri-drag-region="false">
                  <FolderOpen size={14} />
                  {t.chatSkillImportFolder}
                </Button>
                <Button onClick={() => void handleImportSkillZip()} disabled={ops.busyKeys.includes('import-zip')} data-tauri-drag-region="false">
                  <Download size={14} />
                  {t.chatSkillImportZip}
                </Button>
                <Button onClick={() => void handleOpenSkillFolder()} data-tauri-drag-region="false">
                  <ExternalLink size={14} />
                  {t.chatSkillOpenSkillFolder}
                </Button>
              </div>
              <SkillUrlImport />
              <div className="rounded-md border border-neutral-200 p-3">
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <div className="mb-1.5 text-[13px] font-medium text-neutral-800">{t.chatSkillImportFromCli}</div>
                    <p className="text-[12px] text-neutral-500 dark:text-neutral-400">
                      {t.chatSkillCliImportHint}
                    </p>
                  </div>
                  <Button onClick={() => void handleCliScan()} disabled={cliScanning} data-tauri-drag-region="false">
                    <Search size={14} />
                    {cliScanning ? t.chatSkillScanning : t.chatSkillScan}
                  </Button>
                </div>

                {cliSkills && (() => {
                  const total = cliSkills.claude.length + cliSkills.codex.length + cliSkills.opencode.length + cliSkills.pi.length
                  return (
                    <div className="mt-3 space-y-3">
                      {total === 0 ? (
                        <div className="rounded-md border border-dashed border-neutral-200 px-3 py-2 text-[11.5px] text-neutral-400">
                          {t.chatSkillCliNoSkillsFound}
                        </div>
                      ) : (
                        <>
                          {CLI_SKILL_SOURCES.map((source) => {
                            const group = cliSkills[source.key]
                            if (group.length === 0) return null
                            return (
                              <div key={source.key}>
                                <div className="mb-1.5 text-[12px] font-medium text-neutral-600">{source.label}</div>
                                <div className="overflow-hidden rounded-md border border-neutral-200 [&>*+*]:border-t [&>*+*]:border-neutral-100 dark:[&>*+*]:border-neutral-800/70">
                                  {group.map((skill) => (
                                    <label
                                      key={skill.id}
                                      className="flex cursor-pointer items-center gap-2.5 px-3 py-2"
                                      data-tauri-drag-region="false"
                                    >
                                      <input
                                        type="checkbox"
                                        checked={ops.cliSelectedIds.includes(skill.id)}
                                        onChange={() => toggleCliSkillSelected(skill.id)}
                                        className="size-3.5 shrink-0 accent-[var(--accent)]"
                                      />
                                      <div className="min-w-0 flex-1">
                                        <div className="truncate text-[12.5px] font-medium text-neutral-800">{skill.name}</div>
                                        <div className="truncate text-[11px] text-neutral-400">{skill.description || t.chatSkillNoDescription}</div>
                                      </div>
                                    </label>
                                  ))}
                                </div>
                              </div>
                            )
                          })}
                          <Button
                            onClick={() => void handleCliImportSelected()}
                            disabled={cliImporting || ops.cliSelectedIds.length === 0}
                            data-tauri-drag-region="false"
                          >
                            {cliImporting ? t.chatSkillImporting : t.chatSkillImportSelected.replace('{n}', String(ops.cliSelectedIds.length))}
                          </Button>
                        </>
                      )}
                    </div>
                  )
                })()}

                {cliImportDone && (
                  <div className="mt-2 text-[12px] text-emerald-600 dark:text-emerald-400">{cliImportDone}</div>
                )}
              </div>
              {skillError && (
                <div className="flex items-center justify-between gap-3 rounded-md border border-red-200 bg-red-50 px-3 py-2 text-[12px] text-red-700 dark:border-red-900/60 dark:bg-red-950/30 dark:text-red-300">
                  <span>{skillError}</span>
                  {ops.enableFailedIds.length > 0 && (
                    <Button size="sm" onClick={() => void retryFailedSkillEnables()}>{t.chatRetry}</Button>
                  )}
                </div>
              )}
            </div>
          ) : view === 'advanced' ? (
          <section key="advanced" className="chat-motion-tab-in mt-5 overflow-hidden rounded-md border border-neutral-200">
            <div className="flex w-full items-center gap-2 px-4 py-3">
              <Sliders size={15} className="shrink-0 text-neutral-400" />
              <span className="text-[13px] font-semibold text-neutral-800">{t.chatSkillTabAdvanced}</span>
              <span className="text-[12px] text-neutral-400">{t.chatSkillAdvancedSubtitle}</span>
            </div>
            <div>
              <div className="space-y-5 border-t border-neutral-200 px-4 py-4">
                <div className="flex items-start justify-between gap-4">
                  <div className="min-w-0">
                    <div className="text-[13px] font-medium text-neutral-800">{t.chatSkillAutoMatch}</div>
                    <p className="mt-0.5 text-[12px] text-neutral-500 dark:text-neutral-400">
                      {t.chatSkillAutoMatchHint}
                    </p>
                  </div>
                  <Toggle
                    checked={chatTools?.skillAutoMatch !== false}
                    onChange={(skillAutoMatch) => persistChatTools({ skillAutoMatch })}
                    ariaLabel={t.chatSkillAutoMatch}
                  />
                </div>

                <div className="min-w-0">
                  <div className="min-w-0">
                    <div className="mb-1.5 text-[13px] font-medium text-neutral-800">
                      {t.chatSkillFallbackMode}
                    </div>
                    <Select
                      value={chatTools?.skillFallbackMode || 'progressive'}
                      onChange={(value) => persistChatTools({ skillFallbackMode: value })}
                      options={[
                        { value: 'progressive', label: t.chatSkillFallbackProgressive },
                        { value: 'skill_md_only', label: t.chatSkillFallbackSkillMdOnly },
                        { value: 'legacy_full_body', label: t.chatSkillFallbackLegacyFullBody },
                      ]}
                    />
                  </div>
                </div>

                <div className="min-w-0">
                  <div className="mb-1.5 text-[13px] font-medium text-neutral-800">{t.chatSkillExtraScanPaths}</div>
                  <div className="space-y-1.5">
                    {skillScanPaths.map((path, index) => (
                      <div key={`${path}-${index}`} className="flex items-center gap-1.5">
                        <input
                          type="text"
                          value={path}
                          onChange={(event) => {
                            const next = [...skillScanPaths]
                            next[index] = event.target.value
                            persistChatTools({ skillScanPaths: next }, true)
                          }}
                          placeholder="/path/to/skills"
                          className="h-9 w-full rounded-md border border-neutral-200 bg-neutral-50 px-2.5 font-mono text-[12.5px] text-neutral-800 outline-none focus:border-neutral-300"
                          data-tauri-drag-region="false"
                        />
                        <IconButton
                          size="lg"
                          variant="danger"
                          label={t.chatSkillRemovePath}
                          onClick={() => {
                            const next = skillScanPaths.filter((_, i) => i !== index)
                            persistChatTools({ skillScanPaths: next })
                            void refreshChatSkills(next)
                          }}
                          data-tauri-drag-region="false"
                        >
                          <Trash2 size={14} />
                        </IconButton>
                      </div>
                    ))}
                    <Button
                      onClick={async () => {
                        const selected = await open({ directory: true, multiple: false })
                        if (typeof selected === 'string') {
                          const next = [...skillScanPaths, selected]
                          persistChatTools({ skillScanPaths: next })
                          void refreshChatSkills(next)
                        }
                      }}
                      data-tauri-drag-region="false"
                    >
                      <Plus size={13} />
                      {t.chatSkillAddScanPath}
                    </Button>
                  </div>
                </div>
              </div>
            </div>
          </section>
          ) : (
          <div key="installed" className="chat-motion-tab-in">
          {/* 搜索 */}
          <div className="relative mt-6">
            <Search size={16} className="pointer-events-none absolute left-3.5 top-1/2 -translate-y-1/2 text-neutral-400" />
            <input
              type="text"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={t.chatSkillSearchPlaceholder}
              className="h-10 w-full rounded-md border border-neutral-200 bg-neutral-50 pl-10 pr-4 text-[14px] outline-none placeholder:text-neutral-400 focus:border-neutral-300 text-neutral-900"
              data-tauri-drag-region="false"
            />
          </div>

          {skillError && (
            <div className="mt-4 flex items-center justify-between gap-3 rounded-md border border-red-200 bg-red-50 px-3 py-2 text-[12px] text-red-700 dark:border-red-900/60 dark:bg-red-950/30 dark:text-red-300">
              <span>{skillError}</span>
              {ops.enableFailedIds.length > 0 && (
                <Button size="sm" onClick={() => void retryFailedSkillEnables()}>{t.chatRetry}</Button>
              )}
            </div>
          )}

          {/* 技能列表 */}
          <div className="mt-6 space-y-5">
            {skillsLoading && skills.length === 0 ? (
              <div className="grid min-h-[220px] place-items-center text-[13px] text-neutral-400">{t.chatSkillLoading}</div>
            ) : (
              <>
                {(projectSkills.length > 0 || Boolean(projectCwd)) && (
                  <SkillSection
                    title={t.chatSkillSectionProject}
                    note={t.chatSkillProjectNote}
                    emptyText={normalizedQuery ? t.chatSkillNoMatchingSkills : t.chatSkillNoProjectSkills}
                    skills={projectSkills}
                    disabledSkillIds={disabledSkillIds}
                    onToggleEnabled={handleToggleSkillEnabled}
                    enableBusyIds={enableBusyIds}
                    onPreview={handlePreviewSkill}
                  />
                )}
                <SkillSection
                  title={t.chatSkillSectionWorkspacePersonal}
                  note={t.chatSkillPersonalNote}
                  emptyText={normalizedQuery ? t.chatSkillNoMatchingSkills : t.chatSkillNoImportedSkills}
                  skills={personalSkills}
                  disabledSkillIds={disabledSkillIds}
                  onToggleEnabled={handleToggleSkillEnabled}
                  onPreview={handlePreviewSkill}
                  onDelete={handleDeleteSkill}
                  enableBusyIds={enableBusyIds}
                  deleteBusyIds={deleteBusyIds}
                />
                <SkillSection
                  title={t.chatSkillSectionPlugin}
                  note={t.chatSkillPluginNote}
                  emptyText={normalizedQuery ? t.chatSkillNoMatchingPlugin : t.chatSkillNoPluginSkills}
                  skills={pluginSkills}
                  disabledSkillIds={disabledSkillIds}
                  onToggleEnabled={handleToggleSkillEnabled}
                  onPreview={handlePreviewSkill}
                  manageLocked
                  lockedActiveIds={enabledPluginSkillIds}
                />
                {(globalSkills.length > 0 || normalizedQuery) && (
                  <SkillSection
                    title={t.chatSkillSectionGlobal}
                    note={t.chatSkillGlobalNote}
                    emptyText={t.chatSkillNoMatchingSkills}
                    skills={globalSkills}
                    disabledSkillIds={disabledSkillIds}
                    onToggleEnabled={handleToggleSkillEnabled}
                    enableBusyIds={enableBusyIds}
                    onPreview={handlePreviewSkill}
                  />
                )}
                <SkillSection
                  title={t.chatSkillSectionBuiltin}
                  note={t.chatSkillBuiltinNote}
                  emptyText={normalizedQuery ? t.chatSkillNoMatchingBuiltin : t.chatSkillNoBuiltin}
                  skills={builtinSkills}
                  disabledSkillIds={disabledSkillIds}
                  onToggleEnabled={handleToggleSkillEnabled}
                  enableBusyIds={enableBusyIds}
                  onPreview={handlePreviewSkill}
                  collapsible
                  defaultCollapsed
                />
              </>
            )}
          </div>
          </div>
          )}
        </div>
      </main>

      {/* 预览弹窗 */}
      {selectedSkillPreview && (
        <div
          className="chat-motion-fade fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-6"
          data-tauri-drag-region="false"
          onClick={() => setSelectedSkillPreview(null)}
        >
          <div
            role="dialog"
            aria-modal="true"
            aria-labelledby="skill-preview-title"
            className="chat-motion-modal-in flex max-h-[80vh] w-full max-w-[640px] flex-col gap-3 overflow-hidden rounded-2xl border border-neutral-200 bg-neutral-50 p-5 shadow-2xl"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="flex items-start gap-2">
              <Sparkles size={16} className="mt-0.5 shrink-0 text-accent" />
              <div className="min-w-0 flex-1">
                <h3 id="skill-preview-title" className="truncate text-[15px] font-semibold text-neutral-900">
                  {selectedSkillPreview.name}
                </h3>
                <p className="mt-0.5 text-[12.5px] text-neutral-500 dark:text-neutral-400">
                  {selectedSkillPreview.description}
                </p>
              </div>
              <IconButton
                size="sm"
                label={t.chatWinClose}
                onClick={() => setSelectedSkillPreview(null)}
                data-tauri-drag-region="false"
              >
                <X size={14} />
              </IconButton>
            </div>
            {selectedSkillPreview.recommendedTools.length > 0 && (
              <div className="flex flex-wrap gap-1.5">
                {selectedSkillPreview.recommendedTools.map((tool) => (
                  <span
                    key={tool}
                    className="rounded-md bg-neutral-100 px-2 py-0.5 text-[11.5px] text-neutral-600"
                  >
                    {tool}
                  </span>
                ))}
              </div>
            )}
            <div className="custom-scrollbar max-h-[52vh] overflow-y-auto rounded-lg border border-neutral-200 bg-neutral-50 p-3">
              <ChatMarkdown content={selectedSkillPreview.body} />
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
