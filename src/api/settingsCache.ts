import {
  api,
  isSettingsVersionConflict,
  type Settings,
  type SettingsChangedEvent,
  type SettingsSnapshot,
  type SettingsVersion,
} from './tauri'

/**
 * Per-webview cache of the backend-owned canonical settings snapshot.
 *
 * Settings and their version are deliberately inseparable here. Full writes
 * must provide the version the edited value was read from; this module never
 * borrows a newer cache version to endorse an older full Settings object.
 */
let cached: SettingsSnapshot | null = null
let inflight: Promise<SettingsSnapshot> | null = null
let nextRequestSequence = 0
let lastAcceptedRequestSequence = 0

type SettingsListener = (settings: Settings) => void
type SettingsSnapshotListener = (snapshot: SettingsSnapshot) => void
const listeners = new Set<SettingsListener>()
const snapshotListeners = new Set<SettingsSnapshotListener>()

function sameVersion(a: SettingsVersion, b: SettingsVersion): boolean {
  return a.epoch === b.epoch && a.revision === b.revision
}

function isEventAhead(event: SettingsChangedEvent): boolean {
  if (!cached) return true
  if (event.version.epoch !== cached.version.epoch) return true
  return event.version.revision > cached.version.revision
}

function notifySettingsUpdated(snapshot: SettingsSnapshot): void {
  for (const listener of [...snapshotListeners]) {
    try {
      listener(snapshot)
    } catch (error) {
      console.error('[settingsCache] snapshot listener failed', error)
    }
  }
  for (const listener of [...listeners]) {
    try {
      listener(snapshot.settings)
    } catch (error) {
      console.error('[settingsCache] listener failed', error)
    }
  }
}

/**
 * Accept only monotonic snapshots. Revisions are comparable within an epoch.
 * Across backend restarts, request order prevents an old in-flight response
 * from replacing a snapshot fetched from the new process.
 */
function acceptSnapshot(
  incoming: SettingsSnapshot,
  requestSequence: number,
  notify: boolean,
): SettingsSnapshot {
  const current = cached
  let accept = current === null
  if (current) {
    if (incoming.version.epoch === current.version.epoch) {
      accept = incoming.version.revision > current.version.revision
    } else {
      accept = requestSequence >= lastAcceptedRequestSequence
    }
  }

  if (accept) {
    cached = incoming
    lastAcceptedRequestSequence = Math.max(lastAcceptedRequestSequence, requestSequence)
    if (notify) notifySettingsUpdated(incoming)
  } else if (current && sameVersion(incoming.version, current.version)) {
    // A later request confirming the same version still supersedes older
    // cross-epoch requests, even though it does not need to notify consumers.
    lastAcceptedRequestSequence = Math.max(lastAcceptedRequestSequence, requestSequence)
  }
  return cached ?? incoming
}

function beginRequest(): number {
  nextRequestSequence += 1
  return nextRequestSequence
}

/** Subscribe to Settings-only compatibility updates. */
export function subscribeSettings(listener: SettingsListener): () => void {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}

/** Subscribe to canonical values together with their exact backend version. */
export function subscribeSettingsSnapshot(listener: SettingsSnapshotListener): () => void {
  snapshotListeners.add(listener)
  return () => { snapshotListeners.delete(listener) }
}

export function peekSettingsSnapshot(): SettingsSnapshot | null {
  return cached
}

/** Settings-only compatibility view. Treat the returned object as read-only. */
export function peekSettings(): Settings | null {
  return cached?.settings ?? null
}

/** Cached snapshot read; concurrent cold reads share one IPC request. */
export function getSettingsSnapshotCached(): Promise<SettingsSnapshot> {
  if (cached) return Promise.resolve(cached)
  if (inflight) return inflight
  const requestSequence = beginRequest()
  const request = api.getSettings()
    .then((snapshot) => acceptSnapshot(snapshot, requestSequence, false))
    .finally(() => {
      if (inflight === request) inflight = null
    })
  inflight = request
  return request
}

export async function getSettingsCached(): Promise<Settings> {
  return (await getSettingsSnapshotCached()).settings
}

/** Force an authoritative read. Failure leaves the previous cache intact. */
export function refreshSettingsSnapshot(): Promise<SettingsSnapshot> {
  const requestSequence = beginRequest()
  return api.getSettings().then((snapshot) => acceptSnapshot(snapshot, requestSequence, true))
}

export async function refreshSettings(): Promise<Settings> {
  return (await refreshSettingsSnapshot()).settings
}

/**
 * Subscribe once per webview. Versioned events carry no settings or secrets;
 * an ahead event triggers an authoritative read. Focus/visibility refreshes
 * recover from a notification missed while a window was suspended.
 */
export async function startBackendSettingsSync(): Promise<() => void> {
  let stopped = false
  const refresh = () => {
    if (stopped) return
    void refreshSettingsSnapshot().catch((error) => {
      console.error('[settingsCache] backend refresh failed', error)
    })
  }
  const unlisten = await api.onKivioSettingsChanged((event) => {
    if (!stopped && isEventAhead(event)) refresh()
  })
  const onFocus = () => refresh()
  const onVisibilityChange = () => {
    if (typeof document === 'undefined' || document.visibilityState === 'visible') refresh()
  }
  if (typeof window !== 'undefined') window.addEventListener('focus', onFocus)
  if (typeof document !== 'undefined') document.addEventListener('visibilitychange', onVisibilityChange)
  return () => {
    stopped = true
    unlisten()
    if (typeof window !== 'undefined') window.removeEventListener('focus', onFocus)
    if (typeof document !== 'undefined') document.removeEventListener('visibilitychange', onVisibilityChange)
  }
}

/** Commit a full object against the exact version from which it was edited. */
export async function saveSettingsSnapshotCached(
  settings: Settings,
  expectedVersion: SettingsVersion,
): Promise<SettingsSnapshot> {
  const requestSequence = beginRequest()
  const saved = await api.saveSettings(settings, expectedVersion)
  return acceptSnapshot(saved, requestSequence, true)
}

/** Settings-only compatibility result; expectedVersion remains mandatory. */
export async function saveSettingsCached(
  settings: Settings,
  expectedVersion: SettingsVersion,
): Promise<Settings> {
  return (await saveSettingsSnapshotCached(settings, expectedVersion)).settings
}

/** Import is a full replacement and therefore validates the version captured
 * before the file-picking/import operation started. */
export async function importSettingsSnapshotCached(
  path: string,
  expectedVersion: SettingsVersion,
): Promise<SettingsSnapshot> {
  const requestSequence = beginRequest()
  const imported = await api.importSettings(path, expectedVersion)
  return acceptSnapshot(imported, requestSequence, true)
}

export async function importSettingsCached(
  path: string,
  expectedVersion: SettingsVersion,
): Promise<Settings> {
  return (await importSettingsSnapshotCached(path, expectedVersion)).settings
}

export type UpdateSettingsCachedOptions = {
  /** Number of fresh-read/reapply attempts after the initial CAS conflict. */
  maxConflictRetries?: number
}

/**
 * Safely perform a narrow read-modify-write operation. `mutate` must be pure:
 * on a version conflict it is re-run against one fresh canonical snapshot.
 * Long-lived editors must use their own three-way merge controller instead.
 */
export async function updateSettingsCached(
  mutate: (current: Settings) => Settings,
  options: UpdateSettingsCachedOptions = {},
): Promise<Settings> {
  const maxConflictRetries = Math.max(0, options.maxConflictRetries ?? 1)
  let base = await refreshSettingsSnapshot()
  for (let conflicts = 0; conflicts <= maxConflictRetries; conflicts += 1) {
    const next = mutate(base.settings)
    try {
      return await saveSettingsCached(next, base.version)
    } catch (error) {
      if (!isSettingsVersionConflict(error) || conflicts >= maxConflictRetries) throw error
      base = await refreshSettingsSnapshot()
    }
  }
  throw new Error('unreachable settings update state')
}

/** Lightweight backend mutations return the new canonical snapshot/version. */
export async function setFavoriteModelsCached(models: string[]): Promise<void> {
  const requestSequence = beginRequest()
  const saved = await api.setFavoriteModels(models)
  acceptSnapshot(saved, requestSequence, true)
}

export async function setTranslateCardSizeCached(width: number): Promise<void> {
  const requestSequence = beginRequest()
  const saved = await api.setTranslateCardSize(width)
  acceptSnapshot(saved, requestSequence, true)
}

/** Test-only reset. */
export function __resetSettingsCacheForTest(): void {
  cached = null
  inflight = null
  nextRequestSequence = 0
  lastAcceptedRequestSequence = 0
  listeners.clear()
  snapshotListeners.clear()
}
