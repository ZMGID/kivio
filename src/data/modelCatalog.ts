import type { ModelInfo, ModelProvider } from '../api/tauri'

/** Import advertised capabilities while preserving explicit per-model settings. */
export function applyModelCatalog(provider: ModelProvider, catalog: {
  models: string[]
  capabilities: Record<string, NonNullable<ModelInfo['capabilities']>>
}): Pick<ModelProvider, 'availableModels' | 'modelOverrides'> {
  const modelOverrides = { ...provider.modelOverrides }
  for (const [id, capabilities] of Object.entries(catalog.capabilities)) {
    const existing = modelOverrides[id]
    modelOverrides[id] = { ...existing, advertisedVideoInput: capabilities.videoInput }
  }
  return { availableModels: catalog.models, modelOverrides }
}
