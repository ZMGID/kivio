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
  const inFlight = useRef(false)
  const [fetchingProviderId, setFetchingProviderId] = useState<string | null>(null)
  const [error, setError] = useState('')

  const fetchModels = useCallback(async (providerId: string): Promise<void> => {
    if (inFlight.current) return
    const provider = portRef.current.getProvider(providerId)
    if (!provider) return
    inFlight.current = true
    setFetchingProviderId(providerId)
    setError('')
    const identity = connectionIdentity(provider)
    try {
      const catalog = await portRef.current.fetch(providerId, {
        id: provider.id,
        baseUrl: provider.baseUrl,
        apiKeys: provider.apiKeys,
        activeKeyIndex: provider.activeKeyIndex,
        apiFormat: provider.apiFormat,
        request: provider.request,
      })
      if (!live.current) return
      const latest = portRef.current.getProvider(providerId)
      if (!latest || connectionIdentity(latest) !== identity) return
      portRef.current.apply(providerId, applyModelCatalog(latest, catalog))
    } catch (failure) {
      const latest = portRef.current.getProvider(providerId)
      if (live.current && latest && connectionIdentity(latest) === identity) {
        setError(failure instanceof Error ? failure.message : String(failure))
      }
    } finally {
      inFlight.current = false
      if (live.current) setFetchingProviderId(null)
    }
  }, [])

  useEffect(() => {
    live.current = true
    return () => { live.current = false }
  }, [])

  return { fetchingProviderId, error, fetchModels }
}
