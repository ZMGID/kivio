import { useEffect, useMemo, useSyncExternalStore } from 'react'
import {
  externalCliSettingsApi,
  onExternalCliAccountUpdated,
  type AccountSnapshotMode,
  type CliAccountScope,
  type DetectedExternalAgent,
  type ExternalCliAccount,
} from './externalCliSettings'
import { peekSettings, subscribeSettings } from './settingsCache'
import type { Settings } from './tauri'

type View = { account: ExternalCliAccount | null; loading: boolean; error: string | null }
const EMPTY: View = { account: null, loading: false, error: null }
const MAX_SCOPES = 64
const entries = new Map<string, ReturnType<typeof createEntry>>()
let connection: Promise<unknown> | undefined

function connect() {
  if (!connection) {
    subscribeSettings(settings => {
      for (const entry of entries.values()) entry.updateConfiguration(settings)
    })
    window.addEventListener('focus', () => {
      for (const entry of entries.values()) entry.onFocus()
    })
    connection = onExternalCliAccountUpdated(id => {
      for (const entry of entries.values()) if (entry.agentId === id) entry.accountUpdated()
    }).catch(() => {})
  }
  return connection
}

function createEntry(agentId: string, target: CliAccountScope) {
  const listeners = new Set<() => void>()
  const scope = (settings: Settings | null) => [
    settings?.chat?.externalCliAgents?.[agentId],
    settings?.chatTools?.nativeTools?.workingDirectory?.trim() || null,
  ] as const
  let configuration = scope(peekSettings())
  let view: View = EMPTY
  let cacheInvalidated = false
  let nextCheckAt = 0
  let scopeRevision = 0
  let focusRead: Promise<void> | undefined
  let sequence = 0
  let applied = 0
  let pending: Promise<void> | undefined
  let initialCacheRead: Promise<void> | undefined

  const update = (changes: Partial<View>) => {
    view = { ...view, ...changes }
    listeners.forEach(listener => listener())
  }

  const load = (mode: AccountSnapshotMode): Promise<void> => {
    if (mode !== 'cached' && pending) return pending
    if (mode === 'automatic' && Date.now() < nextCheckAt) return Promise.resolve()
    const request = ++sequence
    const revision = scopeRevision
    const task = Promise.resolve().then(async () => {
      try {
        const account = await externalCliSettingsApi.getExternalCliAccount(agentId, target, mode)
        if (revision !== scopeRevision) return
        if (mode === 'cached' && !account) {
          if (request >= applied && view.account) {
            // The backend can no longer associate the displayed reading with this login.
            scopeRevision += 1
            pending = undefined
            initialCacheRead = undefined
            nextCheckAt = 0
            update(EMPTY)
            if (listeners.size) void load('automatic')
          }
          return
        }
        const previousCheck = view.account?.nextRefreshAt
        const currentCheck = account?.nextRefreshAt
        if (mode !== 'cached' && previousCheck != null && currentCheck != null && previousCheck !== currentCheck) {
          if (currentCheck < previousCheck) return
        } else if (request < applied) return
        applied = Math.max(request, applied)
        nextCheckAt = account?.nextRefreshAt ?? Date.now() + 300_000
        update({ account, error: account?.refreshError ?? null })
      } catch (error) {
        if (revision !== scopeRevision || request < applied || mode === 'cached') return
        applied = request
        nextCheckAt = Date.now() + 30_000
        update({ error: error instanceof Error ? error.message : String(error) })
        if (mode === 'refresh') throw error
      } finally {
        if (pending === task) {
          pending = undefined
          update({ loading: false })
        }
      }
    })
    if (mode !== 'cached') {
      pending = task
      update({ loading: true })
    }
    return task
  }

  return {
    agentId,
    load,
    accountUpdated() {
      if (listeners.size) void load('cached')
      else cacheInvalidated = true
    },
    getSnapshot: () => view,
    onFocus() {
      if (!listeners.size) {
        cacheInvalidated = true
        return
      }
      if (focusRead) return
      // A login can change in a terminal even while its quota reading is still fresh.
      focusRead = (view.account ? load('cached') : Promise.resolve())
        .then(() => { if (listeners.size) void load('automatic') })
        .finally(() => { focusRead = undefined })
    },
    evictable: () => listeners.size === 0 && !pending,
    updateConfiguration(settings: Settings | null) {
      const next = scope(settings)
      if (next[1] === configuration[1] && JSON.stringify(next[0]) === JSON.stringify(configuration[0])) return
      configuration = next
      cacheInvalidated = false
      scopeRevision += 1
      nextCheckAt = 0
      pending = undefined
      initialCacheRead = undefined
      update(EMPTY)
      if (listeners.size) void load('automatic')
    },
    subscribe(listener: () => void) {
      listeners.add(listener)
      void connect().then(async () => {
        if (!listeners.has(listener)) return
        if (cacheInvalidated) {
          cacheInvalidated = false
          nextCheckAt = 0
          initialCacheRead = load('cached')
          await initialCacheRead
        } else if (!target && !view.account) {
          initialCacheRead ??= load('cached')
          await initialCacheRead
        }
        if (listeners.has(listener)) void load('automatic')
      })
      return () => { listeners.delete(listener) }
    },
  }
}

function accountEntry(agentId: string, target: CliAccountScope = null) {
  const key = JSON.stringify([agentId, target])
  let entry = entries.get(key)
  if (entry) return entry
  if (entries.size >= MAX_SCOPES) {
    for (const [oldKey, oldEntry] of entries) {
      if (!oldEntry.evictable()) continue
      entries.delete(oldKey)
      if (entries.size < MAX_SCOPES) break
    }
  }
  entry = createEntry(agentId, target)
  entries.set(key, entry)
  return entry
}

const subscribeEmpty = () => () => {}
const getEmpty = () => EMPTY

/** A webview retains the last published view. The desktop still owns probe freshness and identity. */
export function useCliAccount(agentId: string | null | undefined, target?: CliAccountScope, conversationId?: string) {
  const entry = useMemo(() => agentId ? accountEntry(agentId, target) : null, [agentId, target])
  useEffect(() => {
    // A newly opened conversation can share a workspace entry. Verify the login
    // while its existing reading is visible, including when a draft is first saved.
    if (conversationId && entry?.getSnapshot().account) entry.onFocus()
  }, [entry, conversationId])
  const view = useSyncExternalStore(entry?.subscribe ?? subscribeEmpty, entry?.getSnapshot ?? getEmpty, getEmpty)
  return useMemo(() => ({
    ...view,
    refresh: () => entry?.load('refresh').catch(() => {}) ?? Promise.resolve(),
  }), [entry, view])
}

/** Subscribe the limits page to its installed CLIs as one list, sharing the individual account caches. */
export function useCliAccounts(agents: DetectedExternalAgent[]) {
  const store = useMemo(() => {
    const sources = agents.map(agent => ({ agent, entry: accountEntry(agent.id) }))
    const read = () => sources.map(({ agent, entry }) => {
      const view = entry.getSnapshot()
      return { agent, view, checking: view === EMPTY || view.loading }
    })
    let snapshot = read()
    return {
      subscribe(listener: () => void) {
        const subscriptions = sources.map(({ entry }) => entry.subscribe(listener))
        return () => subscriptions.forEach(unsubscribe => unsubscribe())
      },
      getSnapshot() {
        if (sources.some(({ entry }, i) => entry.getSnapshot() !== snapshot[i].view)) snapshot = read()
        return snapshot
      },
    }
  }, [agents])
  return useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot)
}

export function refreshCliAccount(agentId: string) {
  return accountEntry(agentId).load('refresh')
}
