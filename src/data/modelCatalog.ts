import type { ModelProvider, api } from '../api/tauri'

/** Import advertised capabilities while preserving explicit per-model settings. */
export function applyModelCatalog(provider: ModelProvider, catalog: Awaited<ReturnType<typeof api.fetchModelCatalog>>): Pick<ModelProvider, 'availableModels' | 'modelOverrides'> {
  const modelOverrides = { ...provider.modelOverrides }
  for (const [id, capabilities] of Object.entries(catalog.capabilities)) {
    const existing = modelOverrides[id]
    modelOverrides[id] = { ...existing, advertisedVideoInput: capabilities.videoInput }
  }
  for (const [id, displayName] of Object.entries(catalog.displayNames ?? {})) {
    modelOverrides[id] = { ...modelOverrides[id], advertisedDisplayName: displayName }
  }
  return { availableModels: catalog.models, modelOverrides }
}

/** Plan model IDs are used for requests; OpenAI supplies names for people. */
export function providerModelLabel(model: string, provider?: ModelProvider): string {
  if (provider?.request?.oauth?.provider !== 'chatgpt') return model
  const info = provider.modelOverrides?.[model]
  return info?.displayName || info?.advertisedDisplayName || model
}
