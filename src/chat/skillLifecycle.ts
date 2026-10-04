import { homeDir } from '@tauri-apps/api/path'
import { open } from '@tauri-apps/plugin-dialog'
import { api, isTauriRuntime, type Settings, type SkillMeta } from '../api/tauri'
import { updateSettingsCached } from '../api/settingsCache'
import { confirmDialog } from '../components/dialogQueue'
import { i18n } from '../components/i18n'
import {
  buildClawHubDownloadUrl,
  resolveClawHubSkillOwner,
  type ClawHubSkillCard,
} from '../settings/public/skills'
import { createWindowStore } from '../utils/windowStore'

/** 本地 CLI 技能来源：只扫各家自己的目录。`~/.agents/skills` 由 Kivio 直接扫描，不必导入。 */
export const CLI_SKILL_SOURCES = [
  { key: 'claude', label: 'Claude Code', dirs: ['.claude/skills'] },
  { key: 'codex', label: 'Codex', dirs: ['.codex/skills'] },
  { key: 'opencode', label: 'OpenCode', dirs: ['.config/opencode/skills', '.opencode/skills'] },
  { key: 'pi', label: 'Pi', dirs: ['.pi/agent/skills'] },
] as const

type CliSkillKey = (typeof CLI_SKILL_SOURCES)[number]['key']
type CliSkillGroups = Record<CliSkillKey, SkillMeta[]>

type SkillEnableIntent = { skillId: string; enabled: boolean }

/** Store card slug paired with the inventory id returned by that install. */
type StoreSlugMark = { slug: string; skillId: string }

type SkillLifecycleState = {
  view: 'installed' | 'store' | 'import' | 'advanced'
  /** Increments only when the skill center actually changes tabs. */
  navigationEpoch: number
  skills: readonly SkillMeta[]
  skillsLoading: boolean
  listError: string
  actionError: string
  enabledPluginSkillIds: readonly string[]
  /**
   * Store cards installed in this window, keyed to the real inventory skill.
   * The badge is this relation plus `skills`; the slug is not the skill id.
   */
  installedSlugs: readonly StoreSlugMark[]
  storeInstallError: string
  /** In-flight operation identities. Navigation does not clear these. */
  busyKeys: readonly string[]
  urlDraft: string
  urlError: string
  urlDone: string
  cliSkills: CliSkillGroups | null
  cliSelectedIds: readonly string[]
  cliImportDone: string
  enableIntents: readonly SkillEnableIntent[]
  enableFailedIds: readonly string[]
  settledGeneration: number
  notifiedGeneration: number
  refreshedGeneration: number
}

const EMPTY_IDS: readonly string[] = []
const EMPTY_STORE_SLUGS: readonly StoreSlugMark[] = []

function emptySkillLifecycleState(): SkillLifecycleState {
  return {
    view: 'installed',
    navigationEpoch: 0,
    skills: [],
    skillsLoading: false,
    listError: '',
    actionError: '',
    enabledPluginSkillIds: EMPTY_IDS,
    installedSlugs: EMPTY_STORE_SLUGS,
    storeInstallError: '',
    busyKeys: EMPTY_IDS,
    urlDraft: '',
    urlError: '',
    urlDone: '',
    cliSkills: null,
    cliSelectedIds: EMPTY_IDS,
    cliImportDone: '',
    enableIntents: [],
    enableFailedIds: EMPTY_IDS,
    settledGeneration: 0,
    notifiedGeneration: 0,
    refreshedGeneration: 0,
  }
}

export const skillLifecycleStore = createWindowStore(emptySkillLifecycleState())

let flightEpoch = 0
let inventoryEpoch = 0
let enableTail: Promise<void> = Promise.resolve()
let inventoryScope: { scanPaths?: string[]; projectCwd?: string; listFailedLabel: string } = {
  listFailedLabel: i18n.zh.chatSkillListLoadFailed,
}

const settingsListeners = new Set<(settings: Settings) => void>()

function alive(epoch: number): boolean {
  return flightEpoch === epoch
}

function runKey(identity: string, epoch: number): string {
  return `${epoch}:${identity}`
}

function withId(ids: readonly string[], id: string): readonly string[] {
  return ids.includes(id) ? ids : [...ids, id]
}

function withoutId(ids: readonly string[], id: string): readonly string[] {
  return ids.includes(id) ? ids.filter((item) => item !== id) : ids
}

function rememberInstalledSlug(marks: readonly StoreSlugMark[], slug: string, skillId: string): readonly StoreSlugMark[] {
  if (marks.some((mark) => mark.slug === slug && mark.skillId === skillId)) return marks
  return [...marks.filter((mark) => mark.slug !== slug), { slug, skillId }]
}

function forgetInstalledSkill(marks: readonly StoreSlugMark[], skillId: string): readonly StoreSlugMark[] {
  if (!marks.some((mark) => mark.skillId === skillId)) return marks
  return marks.filter((mark) => mark.skillId !== skillId)
}

/** Store badge follows the inventory skill recorded at install, never `slug === id`. */
export function isStoreSlugInstalled(state: Pick<SkillLifecycleState, 'installedSlugs' | 'skills'>, slug: string): boolean {
  const skillId = state.installedSlugs.find((mark) => mark.slug === slug)?.skillId
  return Boolean(skillId && state.skills.some((skill) => skill.id === skillId))
}

function errorMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err)
}

function emptyCliGroups(): CliSkillGroups {
  return { claude: [], codex: [], opencode: [], pi: [] }
}

export function effectiveDisabledSkillIds(
  base: readonly string[] | undefined,
  intents: readonly SkillEnableIntent[],
): string[] {
  let disabled = [...(base ?? [])]
  for (const intent of intents) {
    disabled = intent.enabled
      ? disabled.filter((id) => id !== intent.skillId)
      : disabled.includes(intent.skillId) ? disabled : [...disabled, intent.skillId]
  }
  return disabled
}

function publishSettings(settings: Settings) {
  for (const listener of [...settingsListeners]) listener(settings)
}

export function subscribeSkillSettingsSaved(listener: (settings: Settings) => void): () => void {
  settingsListeners.add(listener)
  return () => { settingsListeners.delete(listener) }
}

function enqueueEnable(epoch: number, task: () => Promise<void>): Promise<void> {
  const run = enableTail.then(() => {
    if (!alive(epoch)) return
    return task()
  }, () => {
    if (!alive(epoch)) return
    return task()
  })
  enableTail = run.then(() => undefined, () => undefined)
  return run
}

function chooseCliSkills(groups: CliSkillGroups | null, selectedIds: readonly string[]): SkillMeta[] {
  if (!groups) return []
  const all = [...groups.claude, ...groups.codex, ...groups.opencode, ...groups.pi]
  return all.filter((skill) => selectedIds.includes(skill.id) && skill.path)
}

/** Authoritative installed-skill read. Later calls supersede earlier ones; unmount does not cancel. */
export function refreshSkillInventory(scope?: {
  scanPaths?: string[]
  projectCwd?: string | null
  listFailedLabel?: string
}): Promise<void> {
  if (scope) {
    if ('scanPaths' in scope) inventoryScope.scanPaths = scope.scanPaths
    if ('projectCwd' in scope) inventoryScope.projectCwd = scope.projectCwd || undefined
    if (scope.listFailedLabel) inventoryScope.listFailedLabel = scope.listFailedLabel
  }
  const epoch = ++inventoryEpoch
  const flight = flightEpoch
  const request = inventoryScope
  skillLifecycleStore.setState((state) => ({ ...state, skillsLoading: true, listError: '' }))
  return (async () => {
    try {
      let pluginIds = skillLifecycleStore.getSnapshot().enabledPluginSkillIds
      if (isTauriRuntime()) {
        try {
          const plugins = await api.pluginsListCached()
          if (!alive(flight) || epoch !== inventoryEpoch) return
          const ids: string[] = []
          for (const plugin of plugins) {
            if (!plugin.enabled) continue
            for (const skillId of plugin.skillIds ?? []) ids.push(skillId)
          }
          pluginIds = ids
        } catch {
          /* 插件列表失败不挡技能列表 */
        }
      }
      if (!alive(flight) || epoch !== inventoryEpoch) return
      const result = await api.chatSkillsList(request.scanPaths, request.projectCwd)
      if (!alive(flight) || epoch !== inventoryEpoch) return
      if (result.success) {
        skillLifecycleStore.setState((state) => ({
          ...state,
          skills: result.skills,
          skillsLoading: false,
          listError: '',
          enabledPluginSkillIds: pluginIds,
        }))
        return
      }
      skillLifecycleStore.setState((state) => ({
        ...state,
        skillsLoading: false,
        listError: result.error || request.listFailedLabel,
      }))
    } catch (err) {
      if (!alive(flight) || epoch !== inventoryEpoch) return
      skillLifecycleStore.setState((state) => ({
        ...state,
        skillsLoading: false,
        listError: errorMessage(err),
      }))
    }
  })()
}

async function refreshAfterMutation(epoch: number): Promise<boolean> {
  await refreshSkillInventory()
  if (!alive(epoch)) return false
  skillLifecycleStore.setState((state) => ({ ...state, settledGeneration: state.settledGeneration + 1 }))
  return true
}

export function setSkillView(view: SkillLifecycleState['view']) {
  skillLifecycleStore.setState((state) => state.view === view ? state : {
    ...state,
    view,
    navigationEpoch: state.navigationEpoch + 1,
  })
}

export function setSkillUrlDraft(url: string) {
  skillLifecycleStore.setState((state) => state.urlDraft === url ? state : { ...state, urlDraft: url })
}

export function toggleCliSkillSelected(id: string) {
  skillLifecycleStore.setState((state) => {
    const selected = state.cliSelectedIds.includes(id)
      ? state.cliSelectedIds.filter((item) => item !== id)
      : [...state.cliSelectedIds, id]
    return { ...state, cliSelectedIds: selected }
  })
}

export function setSkillActionError(message: string) {
  skillLifecycleStore.setState((state) => state.actionError === message ? state : { ...state, actionError: message })
}

export function markSkillInventoryNotified(generation: number) {
  skillLifecycleStore.setState((state) => {
    if (state.settledGeneration !== generation || state.notifiedGeneration === generation) return state
    return { ...state, notifiedGeneration: generation }
  })
}

export function markSkillInventoryRefreshed(generation: number) {
  skillLifecycleStore.setState((state) => {
    if (state.settledGeneration !== generation || state.refreshedGeneration === generation) return state
    return { ...state, refreshedGeneration: generation }
  })
}

export function installStoreSkill(card: ClawHubSkillCard, failedLabel: string): Promise<void> {
  const epoch = flightEpoch
  const identity = `store:${card.slug}`
  return skillLifecycleStore.run(runKey(identity, epoch), async () => {
    if (!alive(epoch)) return
    skillLifecycleStore.setState((state) => ({
      ...state,
      busyKeys: withId(state.busyKeys, identity),
      storeInstallError: '',
    }))
    try {
      const resolved = await resolveClawHubSkillOwner(card)
      if (!alive(epoch)) return
      const downloadUrl = resolved.downloadUrl ?? buildClawHubDownloadUrl(resolved.slug, resolved.ownerHandle)
      const result = await api.chatSkillsInstallFromUrl(downloadUrl)
      if (!alive(epoch)) return
      if (!result.success) throw new Error(result.error || failedLabel)
      await refreshSkillInventory()
      if (!alive(epoch)) return
      const skillId = result.skill?.id
      skillLifecycleStore.setState((state) => ({
        ...state,
        installedSlugs: skillId && state.skills.some((skill) => skill.id === skillId)
          ? rememberInstalledSlug(state.installedSlugs, card.slug, skillId)
          : state.installedSlugs,
        storeInstallError: '',
        settledGeneration: state.settledGeneration + 1,
      }))
    } catch (err) {
      if (!alive(epoch)) return
      skillLifecycleStore.setState((state) => ({ ...state, storeInstallError: errorMessage(err) }))
    } finally {
      if (alive(epoch)) skillLifecycleStore.setState((state) => ({ ...state, busyKeys: withoutId(state.busyKeys, identity) }))
    }
  })
}

export function installSkillFromUrl(labels: { failed: string; installed: string }): Promise<void> {
  const epoch = flightEpoch
  if (!skillLifecycleStore.getSnapshot().urlDraft.trim()) return Promise.resolve()
  return skillLifecycleStore.run(runKey('url', epoch), async () => {
    if (!alive(epoch)) return
    const value = skillLifecycleStore.getSnapshot().urlDraft.trim()
    if (!value) return
    skillLifecycleStore.setState((state) => ({
      ...state,
      busyKeys: withId(state.busyKeys, 'url'),
      urlError: '',
      urlDone: '',
    }))
    try {
      const result = await api.chatSkillsInstallFromUrl(value)
      if (!alive(epoch)) return
      if (!result.success) throw new Error(result.error || labels.failed)
      await refreshSkillInventory()
      if (!alive(epoch)) return
      skillLifecycleStore.setState((state) => ({
        ...state,
        urlDraft: state.urlDraft.trim() === value ? '' : state.urlDraft,
        urlDone: labels.installed,
        urlError: '',
        settledGeneration: state.settledGeneration + 1,
      }))
    } catch (err) {
      if (!alive(epoch)) return
      skillLifecycleStore.setState((state) => ({ ...state, urlError: errorMessage(err) }))
    } finally {
      if (alive(epoch)) skillLifecycleStore.setState((state) => ({ ...state, busyKeys: withoutId(state.busyKeys, 'url') }))
    }
  })
}

export function scanCliSkills(scanFailedLabel: string): Promise<void> {
  const epoch = flightEpoch
  return skillLifecycleStore.run(runKey('cli-scan', epoch), async () => {
    if (!alive(epoch)) return
    skillLifecycleStore.setState((state) => ({
      ...state,
      busyKeys: withId(state.busyKeys, 'cli-scan'),
      cliImportDone: '',
      actionError: '',
    }))
    try {
      const home = (await homeDir()).replace(/[/\\]+$/, '')
      if (!alive(epoch)) return
      const norm = (path: string) => path.replace(/\\/g, '/').toLowerCase()
      const piAgentDir = (await api.chatPiAgentDir())?.replace(/[/\\]+$/, '')
      const defaultPiDirs = CLI_SKILL_SOURCES
        .filter((source) => source.key === 'pi')
        .flatMap((source) => source.dirs.map((dir) => `${home}/${dir}`))
      const piSkillDirs = Array.from(new Set([
        ...(piAgentDir ? [`${piAgentDir}/skills`] : []),
        ...defaultPiDirs,
      ].map((dir) => dir.replace(/\\/g, '/'))))
      const sources = CLI_SKILL_SOURCES.map((source) => ({
        key: source.key,
        prefixes: source.key === 'pi'
          ? piSkillDirs.map(norm)
          : source.dirs.map((dir) => norm(`${home}/${dir}`)),
      }))
      const scanDirs = [
        ...CLI_SKILL_SOURCES.filter((source) => source.key !== 'pi')
          .flatMap((source) => source.dirs.map((dir) => `${home}/${dir}`)),
        ...piSkillDirs,
      ]
      const result = await api.chatSkillsList(scanDirs)
      if (!alive(epoch)) return
      if (!result.success) {
        skillLifecycleStore.setState((state) => ({
          ...state,
          actionError: result.error || scanFailedLabel,
          cliSkills: emptyCliGroups(),
          cliSelectedIds: EMPTY_IDS,
        }))
        return
      }
      const scanned = result.skills.filter((skill) => skill.source === 'external' && skill.path)
      const groups = emptyCliGroups()
      for (const skill of scanned) {
        const path = norm(skill.path as string)
        const source = sources.find((item) => item.prefixes.some((prefix) => path.startsWith(prefix)))
        if (source) groups[source.key].push(skill)
      }
      skillLifecycleStore.setState((state) => ({
        ...state,
        cliSkills: groups,
        cliSelectedIds: EMPTY_IDS,
      }))
    } catch (err) {
      if (!alive(epoch)) return
      skillLifecycleStore.setState((state) => ({ ...state, actionError: errorMessage(err) }))
    } finally {
      if (alive(epoch)) skillLifecycleStore.setState((state) => ({ ...state, busyKeys: withoutId(state.busyKeys, 'cli-scan') }))
    }
  })
}

export function importSelectedCliSkills(labels: {
  importNamedFailed: (name: string) => string
  done: (count: number) => string
}): Promise<void> {
  const epoch = flightEpoch
  const snapshot = skillLifecycleStore.getSnapshot()
  if (chooseCliSkills(snapshot.cliSkills, snapshot.cliSelectedIds).length === 0) return Promise.resolve()
  return skillLifecycleStore.run(runKey('cli-import', epoch), async () => {
    if (!alive(epoch)) return
    const current = skillLifecycleStore.getSnapshot()
    const chosen = chooseCliSkills(current.cliSkills, current.cliSelectedIds)
    if (chosen.length === 0) return
    skillLifecycleStore.setState((state) => ({
      ...state,
      busyKeys: withId(state.busyKeys, 'cli-import'),
      cliImportDone: '',
      actionError: '',
    }))
    let imported = 0
    let failure = ''
    try {
      for (const skill of chosen) {
        const folder = (skill.path as string).replace(/[/\\]+SKILL\.md$/i, '')
        const result = await api.chatSkillsImport(folder)
        if (!alive(epoch)) return
        if (result.success) imported += 1
        else failure = result.error || labels.importNamedFailed(skill.name)
      }
      await refreshSkillInventory()
      if (!alive(epoch)) return
      skillLifecycleStore.setState((state) => ({
        ...state,
        actionError: failure,
        cliImportDone: imported > 0 ? labels.done(imported) : state.cliImportDone,
        cliSkills: imported > 0 ? null : state.cliSkills,
        cliSelectedIds: imported > 0 ? EMPTY_IDS : state.cliSelectedIds,
        settledGeneration: state.settledGeneration + 1,
      }))
    } catch (err) {
      if (!alive(epoch)) return
      skillLifecycleStore.setState((state) => ({ ...state, actionError: errorMessage(err) }))
    } finally {
      if (alive(epoch)) skillLifecycleStore.setState((state) => ({ ...state, busyKeys: withoutId(state.busyKeys, 'cli-import') }))
    }
  })
}

async function importSkillPath(
  identity: 'import-folder' | 'import-zip',
  failedLabel: string,
  select: () => Promise<string | null>,
): Promise<void> {
  const epoch = flightEpoch
  return skillLifecycleStore.run(runKey(identity, epoch), async () => {
    if (!alive(epoch)) return
    skillLifecycleStore.setState((state) => ({
      ...state,
      busyKeys: withId(state.busyKeys, identity),
    }))
    try {
      const selected = await select()
      if (!alive(epoch)) return
      if (!selected) return
      skillLifecycleStore.setState((state) => ({ ...state, actionError: '' }))
      const result = await api.chatSkillsImport(selected)
      if (!alive(epoch)) return
      if (!result.success) {
        skillLifecycleStore.setState((state) => ({ ...state, actionError: result.error || failedLabel }))
        return
      }
      await refreshAfterMutation(epoch)
    } catch (err) {
      if (!alive(epoch)) return
      skillLifecycleStore.setState((state) => ({ ...state, actionError: errorMessage(err) }))
    } finally {
      if (alive(epoch)) skillLifecycleStore.setState((state) => ({ ...state, busyKeys: withoutId(state.busyKeys, identity) }))
    }
  })
}

export function importSkillFolder(failedLabel: string): Promise<void> {
  return importSkillPath('import-folder', failedLabel, async () => {
    const selected = await open({ directory: true, multiple: false })
    return typeof selected === 'string' ? selected : null
  })
}

export function importSkillZip(failedLabel: string): Promise<void> {
  return importSkillPath('import-zip', failedLabel, async () => {
    const selected = await open({
      directory: false,
      multiple: false,
      filters: [{ name: 'Skill Zip', extensions: ['zip'] }],
    })
    return typeof selected === 'string' ? selected : null
  })
}

export function deleteInstalledSkill(
  skill: SkillMeta,
  labels: { confirm: string; confirmLabel: string },
): Promise<void> {
  const epoch = flightEpoch
  const identity = `delete:${skill.id}`
  return skillLifecycleStore.run(runKey(identity, epoch), async () => {
    if (!alive(epoch)) return
    skillLifecycleStore.setState((state) => ({ ...state, busyKeys: withId(state.busyKeys, identity) }))
    try {
      const confirmed = await confirmDialog({ message: labels.confirm, confirmLabel: labels.confirmLabel, danger: true })
      if (!alive(epoch) || !confirmed) return
      skillLifecycleStore.setState((state) => ({ ...state, actionError: '' }))
      await api.chatSkillsUninstall(skill.id)
      if (!alive(epoch)) return
      if (!(await refreshAfterMutation(epoch))) return
      skillLifecycleStore.setState((state) => {
        if (state.skills.some((item) => item.id === skill.id)) return state
        const installedSlugs = forgetInstalledSkill(state.installedSlugs, skill.id)
        return installedSlugs === state.installedSlugs ? state : { ...state, installedSlugs }
      })
    } catch (err) {
      if (!alive(epoch)) return
      skillLifecycleStore.setState((state) => ({ ...state, actionError: errorMessage(err) }))
    } finally {
      if (alive(epoch)) skillLifecycleStore.setState((state) => ({ ...state, busyKeys: withoutId(state.busyKeys, identity) }))
    }
  })
}

export function setSkillEnabled(skillId: string, enabled: boolean): Promise<void> {
  const epoch = flightEpoch
  const identity = `enable:${skillId}`
  return skillLifecycleStore.run(runKey(identity, epoch), () => enqueueEnable(epoch, async () => {
    if (!alive(epoch)) return
    skillLifecycleStore.setState((state) => ({
      ...state,
      enableIntents: [
        ...state.enableIntents.filter((item) => item.skillId !== skillId),
        { skillId, enabled },
      ],
      enableFailedIds: withoutId(state.enableFailedIds, skillId),
      busyKeys: withId(state.busyKeys, identity),
      actionError: '',
    }))
    try {
      const saved = await updateSettingsCached((fresh) => ({
        ...fresh,
        chatTools: {
          ...fresh.chatTools,
          disabledSkillIds: effectiveDisabledSkillIds(
            fresh.chatTools.disabledSkillIds,
            skillLifecycleStore.getSnapshot().enableIntents,
          ),
        },
      }))
      if (!alive(epoch)) return
      publishSettings(saved)
      skillLifecycleStore.setState((state) => ({
        ...state,
        enableIntents: state.enableIntents.filter((item) => item.skillId !== skillId || item.enabled !== enabled),
        enableFailedIds: withoutId(state.enableFailedIds, skillId),
        actionError: '',
        settledGeneration: state.settledGeneration + 1,
      }))
    } catch (err) {
      if (!alive(epoch)) return
      skillLifecycleStore.setState((state) => ({
        ...state,
        actionError: errorMessage(err),
        enableFailedIds: withId(state.enableFailedIds, skillId),
      }))
    } finally {
      if (alive(epoch)) skillLifecycleStore.setState((state) => ({ ...state, busyKeys: withoutId(state.busyKeys, identity) }))
    }
  }))
}

export function retryFailedSkillEnables(): Promise<void> {
  const { enableFailedIds, enableIntents } = skillLifecycleStore.getSnapshot()
  const jobs = enableFailedIds.map((skillId) => {
    const intent = enableIntents.find((item) => item.skillId === skillId)
    return intent ? setSkillEnabled(intent.skillId, intent.enabled) : Promise.resolve()
  })
  return Promise.all(jobs).then(() => undefined)
}

/** Restore an empty snapshot. Call only after every in-flight command has settled. */
export function resetSkillLifecycleStoreForTests() {
  flightEpoch += 1
  inventoryEpoch += 1
  enableTail = Promise.resolve()
  inventoryScope = { listFailedLabel: i18n.zh.chatSkillListLoadFailed }
  settingsListeners.clear()
  skillLifecycleStore.setState(emptySkillLifecycleState())
}
