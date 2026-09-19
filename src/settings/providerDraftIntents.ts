import type { ModelInfo, ModelProvider, Settings } from '../api/tauri'
import type { ProviderPreset } from './providerPresets'
import { createProviderRequestDraft } from './public/providerDraft'
import { isProviderEnabled } from './utils'

export type ProviderDraftIntent =
  | { type: 'update'; id: string; updates: Partial<ModelProvider> }
  | { type: 'icon'; id: string; iconKey: string }
  | { type: 'reorder'; fromId: string; toId: string }
  | { type: 'add'; id: string; preset?: ProviderPreset }
  | { type: 'delete'; id: string }
  | { type: 'add-model'; id: string; model: string }
  | { type: 'add-models'; id: string; models: string[] }
  | { type: 'remove-model'; id: string; model: string }
  | { type: 'save-override'; id: string; model: string; info: ModelInfo }
  | { type: 'reset-override'; id: string; model: string }

function clearDefaultModelProvider(
  defaultModels: Settings['defaultModels'],
  providerId: string,
): Settings['defaultModels'] {
  return {
    chat: defaultModels.chat.providerId === providerId ? { providerId: '', model: '' } : defaultModels.chat,
    vision: defaultModels.vision.providerId === providerId ? { providerId: '', model: '' } : defaultModels.vision,
    videoAnalysis: defaultModels.videoAnalysis.providerId === providerId ? { providerId: '', model: '' } : defaultModels.videoAnalysis,
    titleSummary: defaultModels.titleSummary.providerId === providerId ? { providerId: '', model: '' } : defaultModels.titleSummary,
    compression: defaultModels.compression.providerId === providerId ? { providerId: '', model: '' } : defaultModels.compression,
    imageGeneration: defaultModels.imageGeneration.providerId === providerId ? { providerId: '', model: '' } : defaultModels.imageGeneration,
    promptOptimize: defaultModels.promptOptimize.providerId === providerId ? { providerId: '', model: '' } : defaultModels.promptOptimize,
    advisor: defaultModels.advisor.providerId === providerId ? { providerId: '', model: '' } : defaultModels.advisor,
  }
}

function resolveDefaultModelsAfterModelRemoval(
  defaultModels: Settings['defaultModels'],
  providerId: string,
  resolveAfterRemoval: (currentModel: string) => string,
): Settings['defaultModels'] {
  return {
    chat: defaultModels.chat.providerId === providerId ? { ...defaultModels.chat, model: resolveAfterRemoval(defaultModels.chat.model) } : defaultModels.chat,
    vision: defaultModels.vision.providerId === providerId ? { ...defaultModels.vision, model: resolveAfterRemoval(defaultModels.vision.model) } : defaultModels.vision,
    videoAnalysis: defaultModels.videoAnalysis.providerId === providerId ? { ...defaultModels.videoAnalysis, model: resolveAfterRemoval(defaultModels.videoAnalysis.model) } : defaultModels.videoAnalysis,
    titleSummary: defaultModels.titleSummary.providerId === providerId ? { ...defaultModels.titleSummary, model: resolveAfterRemoval(defaultModels.titleSummary.model) } : defaultModels.titleSummary,
    compression: defaultModels.compression.providerId === providerId ? { ...defaultModels.compression, model: resolveAfterRemoval(defaultModels.compression.model) } : defaultModels.compression,
    imageGeneration: defaultModels.imageGeneration.providerId === providerId ? { ...defaultModels.imageGeneration, model: resolveAfterRemoval(defaultModels.imageGeneration.model) } : defaultModels.imageGeneration,
    promptOptimize: defaultModels.promptOptimize.providerId === providerId ? { ...defaultModels.promptOptimize, model: resolveAfterRemoval(defaultModels.promptOptimize.model) } : defaultModels.promptOptimize,
    advisor: defaultModels.advisor.providerId === providerId ? { ...defaultModels.advisor, model: resolveAfterRemoval(defaultModels.advisor.model) } : defaultModels.advisor,
  }
}

function updateProvider(settings: Settings, id: string, updates: Partial<ModelProvider>): Settings {
  if (!settings.providers.some((provider) => provider.id === id)) return settings
  return {
    ...settings,
    providers: settings.providers.map((provider) => provider.id === id ? { ...provider, ...updates } : provider),
  }
}

function resolveProvider(providers: ModelProvider[], providerId: string): ModelProvider | undefined {
  const matched = providers.find((provider) => provider.id === providerId)
  if (matched && isProviderEnabled(matched)) return matched
  return providers.find(isProviderEnabled) ?? providers[0]
}

function resolveModel(provider: ModelProvider | undefined, currentModel: string): string {
  if (!provider || provider.enabledModels.includes(currentModel)) return currentModel
  return provider.enabledModels[0] || currentModel
}

/** One pure Interface for provider/model draft edits, including dependent selections. */
export function applyProviderDraftIntent(settings: Settings, intent: ProviderDraftIntent): Settings {
  switch (intent.type) {
    case 'update': return updateProvider(settings, intent.id, intent.updates)
    case 'icon': {
      const providerIcons = { ...(settings.providerIcons ?? {}) }
      if (intent.iconKey) providerIcons[intent.id] = intent.iconKey
      else delete providerIcons[intent.id]
      return { ...settings, providerIcons }
    }
    case 'reorder': {
      if (intent.fromId === intent.toId) return settings
      const fromIndex = settings.providers.findIndex((provider) => provider.id === intent.fromId)
      const toIndex = settings.providers.findIndex((provider) => provider.id === intent.toId)
      if (fromIndex < 0 || toIndex < 0) return settings
      const providers = [...settings.providers]
      const [moved] = providers.splice(fromIndex, 1)
      providers.splice(toIndex, 0, moved)
      return { ...settings, providers }
    }
    case 'add': {
      if (settings.providers.some((provider) => provider.id === intent.id)) return settings
      const preset = intent.preset
      const provider: ModelProvider = {
        id: intent.id,
        name: preset?.name ?? 'New Provider',
        apiKeys: [],
        baseUrl: preset?.baseUrl ?? 'https://api.openai.com/v1',
        availableModels: [],
        enabledModels: [],
        enabled: true,
        apiFormat: preset?.apiFormat ?? 'openai_chat',
        request: createProviderRequestDraft(preset?.oauth),
      }
      return { ...settings, providers: [...settings.providers, provider] }
    }
    case 'delete': {
      if (!settings.providers.some((provider) => provider.id === intent.id)) return settings
      const providers = settings.providers.filter((provider) => provider.id !== intent.id)
      const translatorProvider = resolveProvider(providers, settings.translatorProviderId)
      const screenshotProvider = resolveProvider(providers, settings.screenshotTranslation?.providerId || '')
      const lensHadOwnProvider = !!settings.lens?.providerId
      const lensProvider = lensHadOwnProvider
        ? resolveProvider(providers, settings.lens?.providerId || '')
        : undefined
      const deletedProviderWasChatModel = settings.defaultModels.chat.providerId === intent.id
        || settings.chatProviderId === intent.id
      const defaultModels = clearDefaultModelProvider(settings.defaultModels, intent.id)
      const providerIcons = { ...(settings.providerIcons ?? {}) }
      delete providerIcons[intent.id]
      return {
        ...settings,
        providers,
        providerIcons,
        translatorProviderId: translatorProvider?.id ?? '',
        translatorModel: resolveModel(translatorProvider, settings.translatorModel),
        defaultModels,
        screenshotTranslation: {
          ...settings.screenshotTranslation,
          providerId: screenshotProvider?.id ?? '',
          model: resolveModel(screenshotProvider, settings.screenshotTranslation?.model || ''),
        },
        ...(lensHadOwnProvider ? {
          lens: {
            ...settings.lens,
            providerId: lensProvider?.id ?? '',
            model: resolveModel(lensProvider, settings.lens?.model || ''),
          },
        } : {}),
        chatProviderId: deletedProviderWasChatModel ? '' : settings.chatProviderId,
        chatModel: deletedProviderWasChatModel ? '' : settings.chatModel,
      }
    }
    case 'add-model': {
      const provider = settings.providers.find((item) => item.id === intent.id)
      const model = intent.model.trim()
      if (!provider || !model || provider.enabledModels.includes(model)) return settings
      return updateProvider(settings, intent.id, { enabledModels: [...provider.enabledModels, model] })
    }
    case 'add-models': {
      const provider = settings.providers.find((item) => item.id === intent.id)
      if (!provider) return settings
      const enabledKeys = new Set(provider.enabledModels.map((model) => model.toLowerCase()))
      const seen = new Set<string>()
      const nextModels: string[] = []
      for (const candidate of intent.models) {
        const model = candidate.trim()
        const key = model.toLowerCase()
        if (!model || enabledKeys.has(key) || seen.has(key)) continue
        seen.add(key)
        nextModels.push(model)
      }
      if (!nextModels.length) return settings
      return updateProvider(settings, intent.id, { enabledModels: [...provider.enabledModels, ...nextModels] })
    }
    case 'remove-model': {
      const provider = settings.providers.find((item) => item.id === intent.id)
      if (!provider) return settings
      const nextEnabledModels = provider.enabledModels.filter((model) => model !== intent.model)
      const resolveAfterRemoval = (currentModel: string) => currentModel === intent.model
        ? nextEnabledModels[0] || '' : currentModel
      const defaultModels = resolveDefaultModelsAfterModelRemoval(
        settings.defaultModels,
        intent.id,
        resolveAfterRemoval,
      )
      return {
        ...settings,
        providers: settings.providers.map((item) => item.id === intent.id
          ? { ...item, enabledModels: nextEnabledModels } : item),
        translatorModel: settings.translatorProviderId === intent.id
          ? resolveAfterRemoval(settings.translatorModel) : settings.translatorModel,
        defaultModels,
        chatProviderId: defaultModels.chat.providerId,
        chatModel: defaultModels.chat.model,
        screenshotTranslation: settings.screenshotTranslation.providerId === intent.id
          ? { ...settings.screenshotTranslation, model: resolveAfterRemoval(settings.screenshotTranslation.model) }
          : settings.screenshotTranslation,
        lens: settings.lens?.providerId === intent.id
          ? { ...settings.lens, model: resolveAfterRemoval(settings.lens.model || '') }
          : settings.lens,
      }
    }
    case 'save-override': {
      const provider = settings.providers.find((item) => item.id === intent.id)
      if (!provider) return settings
      return updateProvider(settings, intent.id, {
        modelOverrides: { ...provider.modelOverrides, [intent.model]: intent.info },
      })
    }
    case 'reset-override': {
      const provider = settings.providers.find((item) => item.id === intent.id)
      if (!provider?.modelOverrides?.[intent.model]) return settings
      const modelOverrides = { ...provider.modelOverrides }
      delete modelOverrides[intent.model]
      return updateProvider(settings, intent.id, { modelOverrides })
    }
  }
}
