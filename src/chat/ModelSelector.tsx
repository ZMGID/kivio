import { useCallback, useEffect, useState } from 'react'
import { ChevronDown } from 'lucide-react'
import { api, type ModelProvider } from '../api/tauri'
import { chatTitlebarPillButtonClass } from './platform'

interface ModelSelectorProps {
  currentProviderId: string
  currentModel: string
  onModelChange: (providerId: string, model: string) => void
}

export function ModelSelector({
  currentProviderId,
  currentModel,
  onModelChange,
}: ModelSelectorProps) {
  const [open, setOpen] = useState(false)
  const [providers, setProviders] = useState<ModelProvider[]>([])

  const loadProviders = useCallback(async () => {
    try {
      const settings = await api.getSettings()
      setProviders(settings.providers || [])
    } catch (err) {
      console.error('Failed to load providers:', err)
      setProviders([
        {
          id: currentProviderId || 'dev-provider',
          name: 'Preview',
          apiKeys: [],
          baseUrl: '',
          availableModels: currentModel ? [currentModel] : ['dev-model'],
          enabledModels: currentModel ? [currentModel] : ['dev-model'],
          supportsTools: true,
          apiFormat: 'openai',
        },
      ])
    }
  }, [currentModel, currentProviderId])

  useEffect(() => {
    loadProviders()
  }, [loadProviders])

  const currentProvider = providers.find((p) => p.id === currentProviderId)
  const displayName = currentModel || currentProvider?.enabledModels[0] || '选择模型'

  return (
    <div className="relative" data-tauri-drag-region="false">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className={chatTitlebarPillButtonClass}
      >
        <span className="max-w-[200px] truncate font-medium text-neutral-800 dark:text-neutral-200">
          {displayName}
        </span>
        <ChevronDown
          size={15}
          className={`shrink-0 text-neutral-400 transition-transform ${open ? 'rotate-180' : ''}`}
        />
      </button>

      {open && (
        <>
          <div className="fixed inset-0 z-10" onClick={() => setOpen(false)} aria-hidden />
          <div className="absolute left-0 top-full z-20 mt-2 max-h-[min(400px,60vh)] min-w-[240px] overflow-y-auto rounded-2xl border border-neutral-200/90 bg-white py-1 shadow-lg dark:border-neutral-700 dark:bg-neutral-900">
            {providers.map((provider) => (
              <div key={provider.id} className="px-1 py-1">
                <div className="px-3 py-1.5 text-[11px] font-semibold uppercase tracking-wide text-neutral-400">
                  {provider.name}
                </div>
                {(provider.enabledModels.length > 0
                  ? provider.enabledModels
                  : provider.availableModels
                ).map((model) => (
                  <button
                    key={model}
                    type="button"
                    onClick={() => {
                      onModelChange(provider.id, model)
                      setOpen(false)
                    }}
                    className={`w-full rounded-lg px-3 py-2 text-left text-[13px] transition-colors ${
                      currentProviderId === provider.id && currentModel === model
                        ? 'bg-neutral-100 font-medium text-neutral-900 dark:bg-neutral-800 dark:text-neutral-100'
                        : 'text-neutral-700 hover:bg-neutral-50 dark:text-neutral-300 dark:hover:bg-neutral-800/80'
                    }`}
                  >
                    {model}
                  </button>
                ))}
              </div>
            ))}
            {providers.length === 0 && (
              <div className="px-4 py-6 text-center text-sm text-neutral-500">暂无可用模型</div>
            )}
          </div>
        </>
      )}
    </div>
  )
}
