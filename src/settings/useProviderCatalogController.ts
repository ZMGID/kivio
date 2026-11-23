import { useCallback, useEffect, useRef, useState } from 'react'
import { api, type ModelProvider } from '../api/tauri'
import { applyModelCatalog } from '../data/modelCatalog'
import { stableStringify } from './utils'

export interface ProviderCatalogPort {
  fetch: typeof api.fetchModelCatalog
  getProvider(id: string): ModelProvider | null | undefined
  apply(id: string, updates: ReturnType<typeof applyModelCatalog>): void
}

function connectionIdentity(provider: ModelProvider): string {
  return stableStringify({
    id: provider.id,
    baseUrl: provider.baseUrl,
    apiKeys: provider.apiKeys,
    activeKeyIndex: provider.activeKeyIndex,
    apiFormat: provider.apiFormat,
    request: provider.request,
  })
}

/** Fetches a model catalog against one connection identity and applies it to the latest draft. */
export function useProviderCatalogController(port: ProviderCatalogPort) {
  const portRef = useRef(port)
  portRef.current = port
  const live = useRef(true)
  const inFlight = useRef(new Map<string, { identity: string; request: Promise<boolean> }>())
  const [fetchingProviderIds, setFetchingProviderIds] = useState<ReadonlySet<string>>(new Set())
  const [error, setError] = useState('')

  const fetchModels = useCallback((providerId: string): Promise<boolean> => {
    const provider = portRef.current.getProvider(providerId)
    if (!provider) return Promise.resolve(false)
    const identity = connectionIdentity(provider)
    const pending = inFlight.current.get(providerId)
    if (pending?.identity === identity) return pending.request

    setError('')
    const request = portRef.current.fetch(providerId, {
      id: provider.id,
      baseUrl: provider.baseUrl,
      apiKeys: provider.apiKeys,
      activeKeyIndex: provider.activeKeyIndex,
      apiFormat: provider.apiFormat,
      request: provider.request,
    }).then(catalog => {
      const latest = portRef.current.getProvider(providerId)
      if (!live.current || !latest || inFlight.current.get(providerId)?.request !== request || connectionIdentity(latest) !== identity) return false
      portRef.current.apply(providerId, applyModelCatalog(latest, catalog))
      return true
    }).catch((failure: unknown) => {
      const latest = portRef.current.getProvider(providerId)
      if (live.current && latest && inFlight.current.get(providerId)?.request === request && connectionIdentity(latest) === identity) {
        setError(failure instanceof Error ? failure.message : String(failure))
      }
      return false
    }).finally(() => {
      if (inFlight.current.get(providerId)?.request !== request) return
      inFlight.current.delete(providerId)
      if (live.current) setFetchingProviderIds(new Set(inFlight.current.keys()))
    })
    inFlight.current.set(providerId, { identity, request })
    setFetchingProviderIds(new Set(inFlight.current.keys()))
    return request
  }, [])

  useEffect(() => {
    live.current = true
    return () => { live.current = false }
  }, [])

  return { fetchingProviderIds, error, fetchModels }
}
