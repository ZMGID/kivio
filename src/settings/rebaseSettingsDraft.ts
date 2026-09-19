import type { Settings } from '../api/tauri'
import { adoptFreshPluginManagedServers, isPluginManagedServer } from './connectorCatalog'
import { stableStringify } from './utils'

function same(a: unknown, b: unknown): boolean {
  return stableStringify(a) === stableStringify(b)
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

export type SettingsMergeConflict = {
  path: string
  base: unknown
  local: unknown
  remote: unknown
}

export type SettingsEditorState = {
  /** Last canonical value confirmed by the backend. Never contains UI-only placeholders. */
  canonical: Settings
  /** Draft shape already acknowledged by a successful save, including UI-only placeholders. */
  acknowledgedDraft: Settings
  /** Current editable value. Differences from acknowledgedDraft are unsaved user edits. */
  draft: Settings
  conflicts: SettingsMergeConflict[]
}

type MergeResult = { value: unknown; conflicts: SettingsMergeConflict[] }

const KEYED_COLLECTION_PATHS = new Set(['providers', 'chatTools.servers'])

function keyedId(value: unknown): string | null {
  if (!isPlainObject(value) || typeof value.id !== 'string' || !value.id) return null
  return value.id
}

function mergeKeyedCollection(
  base: unknown[],
  local: unknown[],
  remote: unknown[],
  path: string,
): MergeResult | null {
  const all = [...base, ...local, ...remote]
  if (all.some((item) => keyedId(item) === null)) return null
  const baseById = new Map(base.map((item) => [keyedId(item)!, item]))
  const localById = new Map(local.map((item) => [keyedId(item)!, item]))
  const remoteById = new Map(remote.map((item) => [keyedId(item)!, item]))
  const order = [...remote.map((item) => keyedId(item)!), ...local.map((item) => keyedId(item)!)]
    .filter((id, index, ids) => ids.indexOf(id) === index)
  const value: unknown[] = []
  const conflicts: SettingsMergeConflict[] = []

  for (const id of order) {
    const baseValue = baseById.get(id)
    const localHas = localById.has(id)
    const remoteHas = remoteById.has(id)
    const localValue = localById.get(id)
    const remoteValue = remoteById.get(id)
    const itemPath = `${path}.${id}`

    if (!localHas && !remoteHas) continue
    if (!baseById.has(id) && (!localHas || !remoteHas)) {
      // Absence on the other side is not a deletion when this id is new.
      value.push(localHas ? localValue : remoteValue)
      continue
    }
    if (!localHas) {
      if (baseValue !== undefined && !same(remoteValue, baseValue)) {
        conflicts.push({ path: itemPath, base: baseValue, local: undefined, remote: remoteValue })
      }
      continue
    }
    if (!remoteHas) {
      if (same(localValue, baseValue)) continue
      conflicts.push({ path: itemPath, base: baseValue, local: localValue, remote: undefined })
      value.push(localValue)
      continue
    }

    const merged = mergeValue(baseValue, localValue, remoteValue, itemPath)
    value.push(merged.value)
    conflicts.push(...merged.conflicts)
  }
  return { value, conflicts }
}

function mergeFavoriteModels(base: unknown[], local: unknown[], remote: unknown[]): unknown[] | null {
  if (![...base, ...local, ...remote].every((item) => typeof item === 'string')) return null
  const baseSet = new Set(base as string[])
  const localSet = new Set(local as string[])
  const remoteSet = new Set(remote as string[])
  const ids = new Set([...baseSet, ...localSet, ...remoteSet])
  const included = new Set<string>()
  for (const id of ids) {
    const was = baseSet.has(id)
    const here = localSet.has(id)
    const there = remoteSet.has(id)
    if (here === was ? there : here) included.add(id)
  }
  return [...(remote as string[]), ...(local as string[])]
    .filter((id, index, order) => included.has(id) && order.indexOf(id) === index)
}

function mergeValue(base: unknown, local: unknown, remote: unknown, path: string): MergeResult {
  if (same(local, base)) return { value: remote, conflicts: [] }
  if (same(remote, base) || same(local, remote)) return { value: local, conflicts: [] }

  if (Array.isArray(base) && Array.isArray(local) && Array.isArray(remote)) {
    if (KEYED_COLLECTION_PATHS.has(path)) {
      const keyed = mergeKeyedCollection(base, local, remote, path)
      if (keyed) return keyed
    }
    if (path === 'favoriteModels') {
      const favorites = mergeFavoriteModels(base, local, remote)
      if (favorites) return { value: favorites, conflicts: [] }
    }
  }

  if (isPlainObject(base) && isPlainObject(local) && isPlainObject(remote)) {
    const keys = new Set([...Object.keys(base), ...Object.keys(local), ...Object.keys(remote)])
    const value: Record<string, unknown> = {}
    const conflicts: SettingsMergeConflict[] = []
    for (const key of keys) {
      const merged = mergeValue(base[key], local[key], remote[key], path ? `${path}.${key}` : key)
      if (merged.value !== undefined) value[key] = merged.value
      conflicts.push(...merged.conflicts)
    }
    return { value, conflicts }
  }

  return {
    value: local,
    conflicts: [{ path, base, local, remote }],
  }
}

function containsPlaceholder(value: unknown): boolean {
  if (value === '') return true
  if (Array.isArray(value)) return value.some(containsPlaceholder)
  if (!isPlainObject(value)) return false
  return Object.entries(value).some(([key, child]) => key === '' || containsPlaceholder(child))
}

/** Restore only incomplete UI rows removed by backend sanitization; valid persisted fields stay canonical. */
function restorePlaceholders(canonical: unknown, draft: unknown): unknown {
  if (Array.isArray(canonical) && Array.isArray(draft)) {
    const canonicalIds = new Set(canonical.map(keyedId).filter((id): id is string => id !== null))
    if (canonicalIds.size > 0 || draft.some((item) => keyedId(item) !== null)) {
      const canonicalById = new Map(canonical.map((item) => [keyedId(item), item]))
      return draft.flatMap((item) => {
        const id = keyedId(item)
        if (id !== null && canonicalById.has(id)) {
          return [restorePlaceholders(canonicalById.get(id), item)]
        }
        return containsPlaceholder(item) ? [item] : []
      }).concat(canonical.filter((item) => {
        const id = keyedId(item)
        return id === null || !draft.some((candidate) => keyedId(candidate) === id)
      }))
    }
    const restored = [...canonical]
    for (const item of draft) {
      if (containsPlaceholder(item) && !restored.some((candidate) => same(candidate, item))) restored.push(item)
    }
    return restored
  }
  if (isPlainObject(canonical) && isPlainObject(draft)) {
    const value: Record<string, unknown> = { ...canonical }
    for (const [key, child] of Object.entries(draft)) {
      if (key in canonical) value[key] = restorePlaceholders(canonical[key], child)
      else if (key === '' || containsPlaceholder(child)) value[key] = child
    }
    return value
  }
  return canonical
}

export function createSettingsEditorState(
  canonical: Settings,
  draft: Settings = canonical,
): SettingsEditorState {
  return { canonical, acknowledgedDraft: draft, draft, conflicts: [] }
}

function valueAtConflictPath(settings: Settings, path: string): unknown {
  let current: unknown = settings
  for (const segment of path.split('.').filter(Boolean)) {
    if (Array.isArray(current)) {
      current = current.find((item) => keyedId(item) === segment)
    } else if (isPlainObject(current)) {
      current = current[segment]
    } else {
      return undefined
    }
  }
  return current
}

/** Update the editable buffer and clear only conflicts whose local value the user changed. */
export function updateSettingsEditorDraft(
  state: SettingsEditorState,
  draft: Settings,
): SettingsEditorState {
  return {
    ...state,
    draft,
    conflicts: state.conflicts.filter((conflict) => (
      same(valueAtConflictPath(draft, conflict.path), conflict.local)
    )),
  }
}

/** Atomically advances the canonical and acknowledged baselines before replaying unsaved edits. */
export function receiveSettingsSnapshot(
  state: SettingsEditorState,
  fresh: Settings,
): SettingsEditorState {
  const acknowledged = mergeValue(state.canonical, state.acknowledgedDraft, fresh, '')
  const current = mergeValue(state.acknowledgedDraft, state.draft, acknowledged.value, '')
  const acknowledgedDraft = acknowledged.value as Settings
  const draft = current.value as Settings
  const pluginIds = new Set(
    [
      ...(state.canonical.chatTools?.servers ?? []),
      ...(state.acknowledgedDraft.chatTools?.servers ?? []),
      ...(state.draft.chatTools?.servers ?? []),
      ...(fresh.chatTools?.servers ?? []),
    ].filter(isPluginManagedServer).map((server) => server.id),
  )
  const adoptPlugins = (value: Settings): Settings => {
    if (!value.chatTools) return value
    const servers = adoptFreshPluginManagedServers(
      value.chatTools.servers ?? [],
      fresh.chatTools?.servers ?? [],
    )
    return servers === value.chatTools.servers
      ? value
      : { ...value, chatTools: { ...value.chatTools, servers } }
  }
  // Advancing the baseline does not resolve an earlier disagreement. Keep it
  // until the user edits that value or the remote value converges with it.
  const conflictsByPath = new Map<string, SettingsMergeConflict>()
  for (const conflict of state.conflicts) {
    const local = valueAtConflictPath(draft, conflict.path)
    const remote = valueAtConflictPath(fresh, conflict.path)
    if (!same(local, remote)) conflictsByPath.set(conflict.path, { ...conflict, local, remote })
  }
  for (const conflict of [...acknowledged.conflicts, ...current.conflicts]) {
    conflictsByPath.set(conflict.path, conflict)
  }
  const conflicts = [...conflictsByPath.values()].filter((conflict) => (
    ![...pluginIds].some((id) => conflict.path === `chatTools.servers.${id}`
      || conflict.path.startsWith(`chatTools.servers.${id}.`))
  ))
  return {
    canonical: fresh,
    acknowledgedDraft: adoptPlugins(acknowledgedDraft),
    draft: adoptPlugins(draft),
    conflicts,
  }
}

/** Applies a successful canonical save response and replays edits made while the request was in flight. */
export function acceptSettingsSave(
  submitted: Settings,
  saved: Settings,
  latest: Settings,
): SettingsEditorState {
  const acknowledged = restorePlaceholders(saved, submitted) as Settings
  const replayed = mergeValue(submitted, latest, acknowledged, '')
  return {
    canonical: saved,
    acknowledgedDraft: acknowledged,
    draft: restorePlaceholders(replayed.value, latest) as Settings,
    conflicts: replayed.conflicts,
  }
}
