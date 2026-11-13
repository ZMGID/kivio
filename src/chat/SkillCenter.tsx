import { FolderIcon as FolderOpen } from '../components/FileIcon'
import { useCallback, useEffect, useId, useMemo, useRef, useState, type ReactNode, type CSSProperties } from 'react'
import { Download, ExternalLink, Link2, Plus, RefreshCw, Search, Terminal, Trash2, X } from 'lucide-react'
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
import { Input, Select, Toggle } from '../settings/public/controls'
import { useT, type I18n } from '../components/i18n'
import { Button, IconButton } from '../components/Button'
import { SkillStoreBrowser } from './SkillStoreBrowser'
import { DefaultSkillIcon, SkillIcon } from '../settings/public/icons'
import { useWindowStore } from '../utils/windowStore'
import { AddMenuButton, CenterEmpty, CenterHeader, CenterSection, CenterTabs } from './market/PluginCenterParts'
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

type SkillSectionId = 'project' | 'personal' | 'plugin' | 'global' | 'builtin'
type SkillFilter = 'all' | 'on' | 'off'

/** 详情弹窗退场动画（--kv-dur-fast）的兜底时长：animationend 未触发时仍会卸载。 */
const DIALOG_EXIT_MS = 200

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

/** 一行一个技能：左侧打开详情，右侧开关（插件附属技能只显示状态）。 */
function SkillRow({
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
  /** 入场 stagger 序号 */
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
  const description = skill.description || t.chatSkillNoDescription
  return (
    <div
      className={`kv-skill-row chat-motion-fade-up${enabled ? '' : ' is-off'}`}
      style={{ '--chat-motion-delay': `${Math.min(index, 10) * 18}ms` } as CSSProperties}
      data-tauri-drag-region="false"
    >
      <button type="button" className="kv-skill-row-main" onClick={() => onPreview(skill.id)} title={description}>
        <span className="kv-market-icon" aria-hidden="true">
          <DefaultSkillIcon size={18} strokeWidth={1.75} />
        </span>
        <span className="kv-skill-row-text">
          <strong>{skill.name}</strong>
          <span>{description}</span>
        </span>
      </button>
      <span className="kv-skill-row-actions">
        {onDelete && canDeleteSkill(skill) && !manageLocked && (
          <span className="kv-skill-row-reveal">
            <IconButton
              size="sm"
              variant="danger"
              disabled={deleteBusy}
              onClick={() => onDelete(skill)}
              label={t.chatSkillDeleteNamed.replace('{name}', skill.name)}
              title={t.chatSkillDelete}
            >
              <Trash2 size={14} strokeWidth={1.75} />
            </IconButton>
          </span>
        )}
        {manageLocked ? (
          <span className="kv-skill-row-state" title={t.chatSkillPluginManageHint}>
            {enabled ? t.chatSkillPluginEnabled : t.chatSkillPluginDisabled}
          </span>
        ) : (
          <Toggle
            checked={enabled}
            disabled={toggleBusy}
            onChange={(next) => onToggleEnabled(skill.id, next)}
            ariaLabel={t.chatSkillEnableNamed.replace('{name}', skill.name)}
          />
        )}
      </span>
    </div>
  )
}

/** 技能详情：完整说明、来源与位置，并能就地启用 / 删除。 */
function SkillDetailDialog({
  skill,
  enabled,
  manageLocked,
  toggleBusy,
  deleteBusy,
  onToggleEnabled,
  onDelete,
  onClose,
}: {
  skill: SkillDetail
  enabled: boolean
  manageLocked: boolean
  toggleBusy: boolean
  deleteBusy: boolean
  onToggleEnabled: (skillId: string, enabled: boolean) => void
  onDelete?: (skill: SkillMeta) => void
  onClose: () => void
}) {
  const t = useT()
  const titleId = useId()
  const dialogRef = useRef<HTMLDialogElement>(null)
  const [closing, setClosing] = useState(false)
  const onCloseRef = useRef(onClose)
  onCloseRef.current = onClose

  useEffect(() => {
    const dialog = dialogRef.current
    if (!dialog) return
    // 记下原焦点，关闭后还回去（原生 <dialog> 卸载时不会自动还焦点）。
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null
    if (!dialog.open) {
      if (typeof dialog.showModal === 'function') dialog.showModal()
      else dialog.setAttribute('open', '')
    }
    return () => {
      if (typeof dialog.close === 'function') dialog.close()
      else dialog.removeAttribute('open')
      if (previous?.isConnected) previous.focus()
    }
  }, [])

  useEffect(() => {
    if (!closing) return
    const timer = window.setTimeout(() => onCloseRef.current(), DIALOG_EXIT_MS)
    return () => window.clearTimeout(timer)
  }, [closing])

  const close = () => setClosing(true)

  return (
    <dialog
      ref={dialogRef}
      className={`kv-modal kv-skill-detail${closing ? ' is-closing' : ''}`}
      aria-labelledby={titleId}
      data-tauri-drag-region="false"
      onCancel={(event) => {
        event.preventDefault()
        close()
      }}
      onKeyDown={(event) => event.stopPropagation()}
      onClick={(event) => {
        // 点到遮罩（对话框矩形之外）关闭。
        if (event.target !== event.currentTarget) return
        const rect = event.currentTarget.getBoundingClientRect()
        const inside = event.clientX >= rect.left && event.clientX <= rect.right
          && event.clientY >= rect.top && event.clientY <= rect.bottom
        if (!inside) close()
      }}
      onAnimationEnd={(event) => {
        if (closing && event.target === event.currentTarget) onCloseRef.current()
      }}
    >
      <header className="kv-skill-detail-head">
        <span className="kv-market-icon" aria-hidden="true">
          <DefaultSkillIcon size={18} strokeWidth={1.75} />
        </span>
        <div>
          <h2 id={titleId}>{skill.name}</h2>
          {skill.description && <p>{skill.description}</p>}
        </div>
        <IconButton size="sm" label={t.chatWinClose} onClick={close}>
          <X size={15} />
        </IconButton>
      </header>
      <div className="kv-skill-detail-meta">
        <span className="kv-skill-chip">{skillSourceLabel(skill, t)}</span>
        {skill.disableModelInvocation && <span className="kv-skill-chip">{t.chatSkillManualOnly}</span>}
        {skill.path && <span className="kv-skill-detail-path" title={skill.path}>{skill.path}</span>}
      </div>
      {skill.recommendedTools.length > 0 && (
        <div className="kv-skill-detail-meta" aria-label={t.chatSkillDetailTools}>
          <span>{t.chatSkillDetailTools}</span>
          {skill.recommendedTools.map((tool) => (
            <span key={tool} className="kv-skill-chip">{tool}</span>
          ))}
        </div>
      )}
      <div className="kv-skill-detail-body custom-scrollbar">
        <ChatMarkdown content={skill.body} />
      </div>
      <footer className="kv-skill-detail-foot">
        {onDelete && canDeleteSkill(skill) && !manageLocked && (
          <Button variant="danger" size="sm" disabled={deleteBusy} onClick={() => onDelete(skill)}>
            <Trash2 size={13} />
            {t.chatSkillDelete}
          </Button>
        )}
        {manageLocked ? (
          <span className="kv-market-load" title={t.chatSkillPluginManageHint}>
            {enabled ? t.chatSkillPluginEnabled : t.chatSkillPluginDisabled}
          </span>
        ) : (
          <label className="kv-market-load">
            <span>{t.chatSkillDetailEnable}</span>
            <Toggle
              checked={enabled}
              disabled={toggleBusy}
              onChange={(next) => onToggleEnabled(skill.id, next)}
              ariaLabel={t.chatSkillEnableNamed.replace('{name}', skill.name)}
            />
          </label>
        )}
      </footer>
    </dialog>
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
    <section className="kv-center-card">
      <div className="kv-center-card-head">
        <span className="kv-market-icon" aria-hidden="true"><Link2 size={17} /></span>
        <div>
          <strong>{t.chatSkillInstallFromUrl}</strong>
          <p>{t.chatSkillUrlImportHint}</p>
        </div>
      </div>
      <form
        className="kv-center-card-body kv-center-card-row"
        onSubmit={(event) => {
          event.preventDefault()
          if (!busy && ops.urlDraft.trim()) install()
        }}
      >
        <Input
          mono
          value={ops.urlDraft}
          onChange={setSkillUrlDraft}
          placeholder="https://github.com/owner/repo"
          aria-label={t.chatSkillInstallFromUrl}
        />
        <Button type="submit" variant="primary" disabled={busy || !ops.urlDraft.trim()}>
          {busy ? t.chatSkillInstalling : t.chatSkillInstall}
        </Button>
      </form>
      {ops.urlError && <div className="kv-mcp-feedback is-error kv-center-done">{ops.urlError}</div>}
      {ops.urlDone && <div className="kv-center-done">{ops.urlDone}</div>}
    </section>
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

  const [filter, setFilter] = useState<SkillFilter>('all')
  const [collapsedSections, setCollapsedSections] = useState<Partial<Record<SkillSectionId, boolean>>>({ builtin: true })

  const isSkillEnabled = (skill: SkillMeta) =>
    isPluginSkill(skill) ? enabledPluginSkillIds.has(skill.id) : !disabledSkillIds.includes(skill.id)
  const passesFilter = (skill: SkillMeta) => filter === 'all' || (filter === 'on') === isSkillEnabled(skill)
  const matchingSkills = skills.filter((skill) => skillMatches(skill, normalizedQuery))
  const matchingEnabled = matchingSkills.filter(isSkillEnabled).length
  const narrowing = Boolean(normalizedQuery) || filter !== 'all'

  const sectionDefs: Array<{
    id: SkillSectionId
    title: string
    note: string
    emptyText: string
    items: SkillMeta[]
    show: boolean
    locked?: boolean
    deletable?: boolean
    emptyAction?: ReactNode
  }> = [
    {
      id: 'project',
      title: t.chatSkillSectionProject,
      note: t.chatSkillProjectNote,
      emptyText: t.chatSkillNoProjectSkills,
      items: projectSkills,
      show: projectSkills.length > 0 || Boolean(projectCwd),
    },
    {
      id: 'personal',
      title: t.chatSkillSectionWorkspacePersonal,
      note: t.chatSkillPersonalNote,
      emptyText: t.chatSkillNoImportedSkills,
      items: personalSkills,
      show: true,
      deletable: true,
      emptyAction: (
        <Button size="sm" variant="ghost" onClick={() => setSkillView('store')}>{t.chatSkillBrowseStore}</Button>
      ),
    },
    {
      id: 'plugin',
      title: t.chatSkillSectionPlugin,
      note: t.chatSkillPluginNote,
      emptyText: t.chatSkillNoPluginSkills,
      items: pluginSkills,
      show: true,
      locked: true,
    },
    {
      id: 'global',
      title: t.chatSkillSectionGlobal,
      note: t.chatSkillGlobalNote,
      emptyText: t.chatSkillNoMatchingSkills,
      items: globalSkills,
      show: globalSkills.length > 0,
    },
    {
      id: 'builtin',
      title: t.chatSkillSectionBuiltin,
      note: t.chatSkillBuiltinNote,
      emptyText: t.chatSkillNoBuiltin,
      items: builtinSkills,
      show: true,
    },
  ]
  const visibleSections = sectionDefs
    .filter((section) => section.show)
    .map((section) => ({ ...section, visible: section.items.filter(passesFilter) }))
    .filter((section) => !narrowing || section.visible.length > 0)
  // 有内容的分组在前，空分组收成一行排在最后，不再把真正的技能挤到首屏以下。
  const orderedSections = narrowing ? visibleSections : [
    ...visibleSections.filter((section) => section.visible.length > 0),
    ...visibleSections.filter((section) => section.visible.length === 0),
  ]

  const previewMeta = selectedSkillPreview ? skills.find((skill) => skill.id === selectedSkillPreview.id) : undefined
  const closePreview = useCallback(() => setSelectedSkillPreview(null), [])

  const errorBanner = skillError ? (
    <div className="kv-center-banner" role="alert">
      <span className="text-[var(--danger)]">{skillError}</span>
      {ops.enableFailedIds.length > 0 && (
        <Button size="sm" onClick={() => void retryFailedSkillEnables()}>{t.chatRetry}</Button>
      )}
    </div>
  ) : null

  const installedView = (
    <div key="installed" className="chat-motion-tab-in">
      <div className="kv-center-toolbar">
        <label className="kv-center-search">
          <Input
            value={query}
            onChange={setQuery}
            placeholder={t.chatSkillSearchPlaceholder}
            aria-label={t.chatSkillSearchPlaceholder}
          />
        </label>
        <div className="kv-plugin-segments" role="group" aria-label={t.chatCenterFilterAll} data-tauri-drag-region="false">
          {([
            ['all', t.chatCenterFilterAll, matchingSkills.length],
            ['on', t.chatCenterFilterEnabled, matchingEnabled],
            ['off', t.chatCenterFilterDisabled, matchingSkills.length - matchingEnabled],
          ] as const).map(([id, label, count]) => (
            <button key={id} type="button" className="kv-plugin-segment" aria-pressed={filter === id} onClick={() => setFilter(id)}>
              {label}
              <span className="kv-plugin-segment-count">{count}</span>
            </button>
          ))}
        </div>
      </div>

      {errorBanner}

      {skillsLoading && skills.length === 0 ? (
        <div className="space-y-2" aria-busy="true">
          {Array.from({ length: 4 }, (_, i) => <div key={i} className="kv-skeleton kv-market-skeleton" />)}
        </div>
      ) : skills.length === 0 ? (
        <CenterEmpty
          icon={<SkillIcon size={22} />}
          title={t.chatSkillEmptyTitle}
          description={t.chatSkillEmptyDesc}
          actions={<>
            <Button variant="primary" onClick={() => setSkillView('store')}>{t.chatSkillBrowseStore}</Button>
            <Button onClick={() => void handleImportSkill()} disabled={ops.busyKeys.includes('import-folder')}>
              <FolderOpen open size={14} />
              {t.chatSkillImportFolder}
            </Button>
          </>}
        />
      ) : orderedSections.length === 0 ? (
        <div className="kv-market-empty">
          <p>{normalizedQuery ? t.chatSkillNoMatchQuery.replace('{q}', query.trim()) : t.chatSkillNoMatchFilter}</p>
          <Button size="sm" onClick={() => { setQuery(''); setFilter('all') }}>{t.chatSkillClearSearch}</Button>
        </div>
      ) : (
        <div className="kv-center-sections">
          {orderedSections.map((section) => {
            const collapsed = !narrowing && Boolean(collapsedSections[section.id])
            const enabledCount = section.items.filter(isSkillEnabled).length
            return (
              <CenterSection
                key={section.id}
                title={section.title}
                count={section.visible.length}
                meta={section.items.length > 0 ? t.chatSkillsEnabledCount.replace('{n}', String(enabledCount)) : undefined}
                note={section.note}
                collapsed={collapsed}
                onToggle={!narrowing && section.visible.length > 0
                  ? () => setCollapsedSections((prev) => ({ ...prev, [section.id]: !collapsed }))
                  : undefined}
                empty={<>
                  <span>{section.emptyText}</span>
                  {section.emptyAction}
                </>}
              >
                <div className="kv-market-rows">
                  {section.visible.map((skill, index) => (
                    <SkillRow
                      key={skill.id}
                      skill={skill}
                      index={index}
                      enabled={isSkillEnabled(skill)}
                      onToggleEnabled={handleToggleSkillEnabled}
                      onPreview={handlePreviewSkill}
                      onDelete={section.deletable ? handleDeleteSkill : undefined}
                      manageLocked={section.locked}
                      toggleBusy={enableBusyIds.includes(skill.id)}
                      deleteBusy={deleteBusyIds.includes(skill.id)}
                    />
                  ))}
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
          <span className="kv-market-icon" aria-hidden="true"><FolderOpen open size={17} /></span>
          <div>
            <strong>{t.chatSkillTabImport}</strong>
            <p>{t.chatSkillImportFolderDesc}</p>
          </div>
        </div>
        <div className="kv-center-card-body kv-mcp-actions">
          <Button onClick={() => void handleImportSkill()} disabled={ops.busyKeys.includes('import-folder')}>
            <FolderOpen open size={14} />
            {t.chatSkillImportFolder}
          </Button>
          <Button onClick={() => void handleImportSkillZip()} disabled={ops.busyKeys.includes('import-zip')}>
            <Download size={14} />
            {t.chatSkillImportZip}
          </Button>
          <Button variant="ghost" className="is-end" onClick={() => void handleOpenSkillFolder()}>
            <ExternalLink size={14} />
            {t.chatSkillOpenSkillFolder}
          </Button>
        </div>
      </section>

      <SkillUrlImport />

      <section className="kv-center-card">
        <div className="kv-center-card-head">
          <span className="kv-market-icon" aria-hidden="true"><Terminal size={17} /></span>
          <div>
            <strong>{t.chatSkillImportFromCli}</strong>
            <p>{t.chatSkillCliImportHint}</p>
          </div>
          <Button onClick={() => void handleCliScan()} disabled={cliScanning}>
            <Search size={14} />
            {cliScanning ? t.chatSkillScanning : t.chatSkillScan}
          </Button>
        </div>

        {cliSkills && (() => {
          const total = cliSkills.claude.length + cliSkills.codex.length + cliSkills.opencode.length + cliSkills.pi.length
          if (total === 0) return <div className="kv-center-done">{t.chatSkillCliNoSkillsFound}</div>
          return (
            <div className="kv-center-card-body">
              {CLI_SKILL_SOURCES.map((source) => {
                const group = cliSkills[source.key]
                if (group.length === 0) return null
                return (
                  <div key={source.key} className="kv-center-picklist-group">
                    <div className="kv-center-picklist-title">{source.label}<small>{group.length}</small></div>
                    <div className="kv-center-picklist">
                      {group.map((skill) => (
                        <label key={skill.id} data-tauri-drag-region="false">
                          <input
                            type="checkbox"
                            className="kv-checkbox"
                            checked={ops.cliSelectedIds.includes(skill.id)}
                            onChange={() => toggleCliSkillSelected(skill.id)}
                          />
                          <span>
                            <strong>{skill.name}</strong>
                            <small>{skill.description || t.chatSkillNoDescription}</small>
                          </span>
                        </label>
                      ))}
                    </div>
                  </div>
                )
              })}
              <div className="kv-mcp-actions mt-3">
                <Button
                  variant="primary"
                  onClick={() => void handleCliImportSelected()}
                  disabled={cliImporting || ops.cliSelectedIds.length === 0}
                >
                  {cliImporting ? t.chatSkillImporting : t.chatSkillImportSelected.replace('{n}', String(ops.cliSelectedIds.length))}
                </Button>
              </div>
            </div>
          )
        })()}

        {cliImportDone && <div className="kv-center-done">{cliImportDone}</div>}
      </section>
      {errorBanner}
    </div>
  )

  const advancedView = (
    <div key="advanced" className="chat-motion-tab-in kv-center-cards">
      <p className="kv-mcp-note">{t.chatSkillAdvancedSubtitle}</p>
      <section className="kv-center-card">
        <div className="kv-center-card-head">
          <div>
            <strong>{t.chatSkillAutoMatch}</strong>
            <p>{t.chatSkillAutoMatchHint}</p>
          </div>
          <Toggle
            checked={chatTools?.skillAutoMatch !== false}
            onChange={(skillAutoMatch) => persistChatTools({ skillAutoMatch })}
            ariaLabel={t.chatSkillAutoMatch}
          />
        </div>
      </section>
      <section className="kv-center-card">
        <div className="kv-center-card-head">
          <div>
            <strong>{t.chatSkillFallbackMode}</strong>
          </div>
        </div>
        <div className="kv-center-card-body">
          <Select
            value={chatTools?.skillFallbackMode || 'progressive'}
            onChange={(value) => persistChatTools({ skillFallbackMode: value })}
            ariaLabel={t.chatSkillFallbackMode}
            options={[
              { value: 'progressive', label: t.chatSkillFallbackProgressive },
              { value: 'skill_md_only', label: t.chatSkillFallbackSkillMdOnly },
              { value: 'legacy_full_body', label: t.chatSkillFallbackLegacyFullBody },
            ]}
          />
        </div>
      </section>
      <section className="kv-center-card">
        <div className="kv-center-card-head">
          <div>
            <strong>{t.chatSkillExtraScanPaths}</strong>
          </div>
        </div>
        <div className="kv-center-card-body space-y-1.5">
          {skillScanPaths.map((path, index) => (
            <div key={`${path}-${index}`} className="kv-center-card-row">
              <Input
                mono
                value={path}
                onChange={(value) => {
                  const next = [...skillScanPaths]
                  next[index] = value
                  persistChatTools({ skillScanPaths: next }, true)
                }}
                placeholder="/path/to/skills"
                aria-label={t.chatSkillExtraScanPaths}
              />
              <IconButton
                size="md"
                variant="danger"
                label={t.chatSkillRemovePath}
                onClick={() => {
                  const next = skillScanPaths.filter((_, i) => i !== index)
                  persistChatTools({ skillScanPaths: next })
                  void refreshChatSkills(next)
                }}
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
          >
            <Plus size={13} />
            {t.chatSkillAddScanPath}
          </Button>
        </div>
      </section>
    </div>
  )

  return (
    <div className="assistant-center-root kv-market flex h-full min-h-0 flex-col" data-tauri-drag-region="false">
      <main className={heading ? 'kv-market-scroll custom-scrollbar' : 'kv-market-scroll custom-scrollbar pt-7'}>
        <div className="w-full pb-4">
          <CenterHeader
            heading={heading ?? (
              <h1 className="flex items-center gap-2.5">
                <SkillIcon size={24} className="text-[var(--color-muted-foreground)]" />
                Skill
              </h1>
            )}
            subtitle={t.chatSkillPageSubtitle}
            actions={<>
              <IconButton size="md" label={t.chatSkillOpenSkillFolder} onClick={() => void handleOpenSkillFolder()}>
                <ExternalLink size={16} />
              </IconButton>
              <IconButton size="md" label={t.chatSkillRefreshList} onClick={() => void refreshChatSkills()} disabled={skillsLoading}>
                <RefreshCw size={16} className={skillsLoading ? 'animate-spin' : ''} />
              </IconButton>
              <AddMenuButton
                label={t.chatCenterAdd}
                items={[
                  { key: 'folder', label: t.chatSkillImportFolder, icon: <FolderOpen open size={14} />, disabled: ops.busyKeys.includes('import-folder'), onSelect: () => void handleImportSkill() },
                  { key: 'zip', label: t.chatSkillImportZip, icon: <Download size={14} />, disabled: ops.busyKeys.includes('import-zip'), onSelect: () => void handleImportSkillZip() },
                  { key: 'url', label: t.chatSkillAddFromUrl, icon: <Link2 size={14} />, onSelect: () => setSkillView('import') },
                  { key: 'cli', label: t.chatSkillAddFromCli, icon: <Terminal size={14} />, onSelect: () => { setSkillView('import'); void handleCliScan() } },
                  { key: 'store', label: t.chatSkillBrowseStore, icon: <Search size={14} />, onSelect: () => setSkillView('store') },
                ]}
              />
            </>}
          />

          <CenterTabs
            value={view}
            onChange={setSkillView}
            tabs={[
              { id: 'installed', label: t.chatSkillTabInstalled, count: skills.length },
              { id: 'store', label: t.chatSkillTabStore },
              { id: 'import', label: t.chatSkillTabImport },
              { id: 'advanced', label: t.chatSkillTabAdvanced },
            ]}
          />

          {view === 'store' ? (
            <div key="store" className="chat-motion-tab-in flex min-h-[420px] flex-col">
              <SkillStoreBrowser />
            </div>
          ) : view === 'import' ? importView : view === 'advanced' ? advancedView : installedView}
        </div>
      </main>

      {selectedSkillPreview && previewMeta && (
        <SkillDetailDialog
          key={selectedSkillPreview.id}
          skill={selectedSkillPreview}
          enabled={isSkillEnabled(previewMeta)}
          manageLocked={isPluginSkill(previewMeta)}
          toggleBusy={enableBusyIds.includes(previewMeta.id)}
          deleteBusy={deleteBusyIds.includes(previewMeta.id)}
          onToggleEnabled={handleToggleSkillEnabled}
          onDelete={canDeleteSkill(previewMeta) ? handleDeleteSkill : undefined}
          onClose={closePreview}
        />
      )}
    </div>
  )
}
