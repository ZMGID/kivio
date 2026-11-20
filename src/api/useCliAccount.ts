import { useMemo, useSyncExternalStore } from 'react'
import {
  externalCliSettingsApi,
  onExternalCliAccountUpdated,
  type AccountSnapshotMode,
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
      for (const entry of entries.values()) if (entry.visible()) void entry.load('automatic')
    })
    connection = onExternalCliAccountUpdated(id => {
      for (const entry of entries.values()) if (entry.agentId === id) void entry.load('cached')
    }).catch(() => {})
  }
  return connection
}

function createEntry(agentId: string, conversationId?: string | null) {
  const listeners = new Set<() => void>()
  const scope = (settings: Settings | null) => [
    settings?.chat?.externalCliAgents?.[agentId],
    settings?.chatTools?.nativeTools?.workingDirectory?.trim() || null,
  ] as const
  let configuration = scope(peekSettings())
  let view: View = EMPTY
  let nextCheckAt = 0
  let configurationRevision = 0
  let sequence = 0
  let applied = 0
  let pending: Promise<void> | undefined

  const update = (changes: Partial<View>) => {
    view = { ...view, ...changes }
    listeners.forEach(listener => listener())
  }

  const load = (mode: AccountSnapshotMode): Promise<void> => {
    if (mode !== 'cached' && pending) return pending
    if (mode === 'automatic' && Date.now() < nextCheckAt) return Promise.resolve()
    const request = ++sequence
    const revision = configurationRevision
    const task = Promise.resolve().then(async () => {
      try {
        const account = await externalCliSettingsApi.getExternalCliAccount(agentId, conversationId, mode)
        if (revision !== configurationRevision || (mode === 'cached' && !account)) return
        const previousCheck = view.account?.nextRefreshAt
        const currentCheck = account?.nextRefreshAt
        if (previousCheck != null && currentCheck != null && previousCheck !== currentCheck) {
          if (currentCheck < previousCheck) return
        } else if (request < applied) return
        applied = Math.max(request, applied)
        nextCheckAt = account?.nextRefreshAt ?? Date.now() + 300_000
        update({ account, error: account?.refreshError ?? null })
      } catch (error) {
        if (revision !== configurationRevision || request < applied || mode === 'cached') return
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
    getSnapshot: () => view,
    visible: () => listeners.size !== 0,
    evictable: () => listeners.size === 0 && !pending,
    updateConfiguration(settings: Settings | null) {
      const next = scope(settings)
      if (next[1] === configuration[1] && JSON.stringify(next[0]) === JSON.stringify(configuration[0])) return
      configuration = next
      configurationRevision += 1
      nextCheckAt = 0
      pending = undefined
      update(EMPTY)
      if (listeners.size) void load('automatic')
    },
    subscribe(listener: () => void) {
      listeners.add(listener)
      void connect().then(() => {
        if (listeners.has(listener)) void load('automatic')
      })
      return () => { listeners.delete(listener) }
    },
  }
}

function accountEntry(agentId: string, conversationId?: string | null) {
  const project = conversationId ?? null
  const key = JSON.stringify([agentId, project])
  let entry = entries.get(key)
  if (entry) return entry
  if (entries.size >= MAX_SCOPES) {
    for (const [oldKey, oldEntry] of entries) {
      if (!oldEntry.evictable()) continue
      entries.delete(oldKey)
      if (entries.size < MAX_SCOPES) break
    }
  }
  entry = createEntry(agentId, project)
  entries.set(key, entry)
  return entry
}

const subscribeEmpty = () => () => {}
const getEmpty = () => EMPTY

/** A webview retains the last published view. The desktop still owns probe freshness and identity. */
export function useCliAccount(agentId: string | null | undefined, conversationId?: string | null) {
  const entry = useMemo(() => agentId ? accountEntry(agentId, conversationId) : null, [agentId, conversationId])
  const view = useSyncExternalStore(entry?.subscribe ?? subscribeEmpty, entry?.getSnapshot ?? getEmpty, getEmpty)
  return useMemo(() => ({
    ...view,
    refresh: () => entry?.load('refresh').catch(() => {}) ?? Promise.resolve(),
  }), [entry, view])
}

export function refreshCliAccount(agentId: string) {
  return accountEntry(agentId).load('refresh')
}
