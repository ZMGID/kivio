import { useSyncExternalStore } from 'react'
import { extensionsApi } from '../api/extensions'
import { EXTENSION_API_VERSION, type ExtensionManifest, type ExtensionModule, type ExtensionPageProps, type ExtensionAPI, type ExtensionHookMap, type HookHandler } from '../../extensions/sdk'

const manifests = import.meta.glob<ExtensionManifest>('../../extensions/*/manifest.json', { eager: true, import: 'default' })
const loaders = import.meta.glob<{ default: ExtensionModule }>('../../extensions/*/index.tsx')
export const catalog = Object.entries(manifests).map(([path, manifest]) => {
  if (!/^[a-z][a-z0-9-]*$/.test(manifest.id) || manifest.apiVersion !== EXTENSION_API_VERSION)
    throw new Error(`不兼容的扩展：${manifest.id}`)
  const load = loaders[path.replace('manifest.json', 'index.tsx')]
  if (!load) throw new Error(`扩展缺少入口：${manifest.id}`)
  return { manifest, load }
})
if (new Set(catalog.map(p => p.manifest.id)).size !== catalog.length) throw new Error('扩展 ID 重复')
type HostActions = Pick<ExtensionPageProps, 'onCreateConversation' | 'onActivateConversation' | 'onSend'>
type Instance = {
  boundHost?: { original: HostActions; actions: HostActions }
  module: ExtensionModule
  abort: AbortController
  hooks: { [K in keyof ExtensionHookMap]: Set<HookHandler<K>> }
}
let state = { ready: false, enabled: [] as string[], busy: false, error: '', revision: 0 }
const subscribers = new Set<() => void>()
const instances = new Map<string, Instance>()
let queue = Promise.resolve()
let initialized: Promise<void> | undefined
let unsubscribe: (() => void) | undefined
function publish(patch: Partial<typeof state>) {
  state = { ...state, ...patch, revision: state.revision + 1 }
  subscribers.forEach(listener => listener())
}
function serialize(action: () => Promise<void>) {
  const result = queue.then(action)
  queue = result.catch(() => {})
  return result
}
function dispose(id: string) {
  const instance = instances.get(id)
  if (!instance) return
  instance.abort.abort()
  for (const handlers of Object.values(instance.hooks)) handlers.clear()
  instances.delete(id)
}
async function emit<K extends keyof ExtensionHookMap>(id: string, name: K, event: ExtensionHookMap[K]['event']) {
  const instance = instances.get(id)
  if (!instance) return
  for (const handler of [...instance.hooks[name]]) {
    if (instance.abort.signal.aborted) throw new Error('扩展上下文已失效')
    const result = await handler(Object.freeze(event), { extensionId: id, signal: instance.abort.signal })
    if (result?.cancel) throw new Error(result.reason)
  }
}
async function load(id: string) {
  if (instances.has(id)) return
  const entry = catalog.find(p => p.manifest.id === id)
  if (!entry) return
  const module = (await entry.load()).default
  const instance: Instance = { module, abort: new AbortController(), hooks: { beforeDisable: new Set(), activated: new Set(), deactivated: new Set() } }
  const api: ExtensionAPI = {
    context: Object.freeze({ extensionId: id, signal: instance.abort.signal }),
    on(name, handler) {
      if (instance.abort.signal.aborted) throw new Error('扩展上下文已失效')
      instance.hooks[name].add(handler)
      return () => { instance.hooks[name].delete(handler) }
    },
  }
  instances.set(id, instance)
  try {
    await module.activate(api)
  } catch (error) { dispose(id); throw error }
}
async function reconcile() {
  const enabled = await extensionsApi.list()
  const errors: string[] = []
  for (const id of instances.keys()) if (!enabled.includes(id)) dispose(id)
  for (const id of enabled) {
    try {
      if (!instances.has(id)) {
        await load(id)
        await emit(id, 'activated', { extensionId: id, reason: 'restore' })
      }
    }
    catch (error) { dispose(id); errors.push(`${id}: ${String(error)}`) }
  }
  publish({ enabled, ready: true, ...(errors.length ? { error: errors.join('\n') } : {}) })
}
export function initializeExtensions() {
  if (!initialized) initialized = serialize(async () => {
    try {
      unsubscribe ??= await extensionsApi.subscribe(() => { void serialize(reconcile).catch(error => publish({ error: error instanceof Error ? error.message : String(error) })) })
      await reconcile()
    } catch (error) { initialized = undefined; publish({ ready: true, error: error instanceof Error ? error.message : String(error) }); throw error }
  })
  return initialized
}
export function setExtensionEnabled(id: string, enabled: boolean) {
  return serialize(async () => {
    if (!catalog.some(p => p.manifest.id === id)) throw new Error('未知扩展')
    publish({ busy: true, error: '' })
    try {
      if (enabled) {
        // Load/registration failure must not leave native services or persisted activation behind.
        const wasEnabled = state.enabled.includes(id)
        await load(id)
        try {
          const next = await extensionsApi.setEnabled(id, true)
          if (!wasEnabled) await emit(id, 'activated', { extensionId: id, reason: 'enable' })
          publish({ enabled: next })
        } catch (error) {
          if (!wasEnabled) {
            try { publish({ enabled: await extensionsApi.setEnabled(id, false) }) }
            finally { dispose(id) }
          }
          throw error
        }
      } else {
        await emit(id, 'beforeDisable', { extensionId: id, reason: 'user' })
        publish({ enabled: await extensionsApi.setEnabled(id, false) })
        try { await emit(id, 'deactivated', { extensionId: id, reason: 'user' }) }
        finally { dispose(id) }
      }
    } catch (error) { publish({ error: error instanceof Error ? error.message : String(error) }); throw error }
    finally { publish({ busy: false }) }
  })
}
export function useExtensions() {
  return useSyncExternalStore(listener => { subscribers.add(listener); return () => { subscribers.delete(listener) } }, () => state)
}
export function extensionPage(id: string) { return instances.get(id)?.module.Page }
export function extensionIdForPath(path: string): string | null {
  if (path.startsWith('chat/extensions/')) return path.slice('chat/extensions/'.length)
  return catalog.find(p => p.manifest.legacyRoutes?.includes(path))?.manifest.id ?? null
}

export function useExtensionPath() {
  return useSyncExternalStore(listener => {
    window.addEventListener('hashchange', listener)
    return () => window.removeEventListener('hashchange', listener)
  }, () => window.location.hash.replace(/^#/, '').split('?')[0])
}

if (import.meta.hot) import.meta.hot.dispose(() => {
  unsubscribe?.()
  for (const id of instances.keys()) dispose(id)
})

export function clearExtensionError() {
  if (state.error) publish({ error: '' })
}

/** Bind asynchronous host operations to this activation, not just its package ID. */
export function bindExtensionHost(id: string, host: HostActions): HostActions {
  const instance = instances.get(id)
  if (!instance) throw new Error('扩展尚未启用')
  if (instance.boundHost && instance.boundHost.original.onCreateConversation === host.onCreateConversation
    && instance.boundHost.original.onActivateConversation === host.onActivateConversation
    && instance.boundHost.original.onSend === host.onSend) return instance.boundHost.actions
  const current = () => {
    if (!instance || instance.abort.signal.aborted || instances.get(id) !== instance || !state.enabled.includes(id) || state.busy)
      throw new Error('扩展上下文已失效，请重新打开扩展')
  }
  const actions: HostActions = {
    onCreateConversation: async title => {
      current()
      const conversation = await host.onCreateConversation(title)
      current()
      return conversation
    },
    onActivateConversation: async conversationId => {
      current()
      await host.onActivateConversation(conversationId)
      current()
    },
    onSend: async (...args) => {
      current()
      return host.onSend(...args)
    },
  }
  instance.boundHost = { original: { onCreateConversation: host.onCreateConversation, onActivateConversation: host.onActivateConversation, onSend: host.onSend }, actions }
  return actions
}
