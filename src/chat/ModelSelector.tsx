import { useState, useEffect } from 'react'
import { ChevronDown } from 'lucide-react'
import { api, type ModelProvider } from '../api/tauri'

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

  useEffect(() => {
    loadProviders()
  }, [])

  const loadProviders = async () => {
    try {
      const settings = await api.getSettings()
      setProviders(settings.providers || [])
    } catch (err) {
      console.error('Failed to load providers:', err)
    }
  }

  const currentProvider = providers.find((p) => p.id === currentProviderId)
  const displayName = currentModel || currentProvider?.enabledModels[0] || '选择模型'

  return (
    <div className="relative">
      <button
        onClick={() => setOpen(!open)}
        className="flex items-center gap-2 px-3 py-1.5 rounded-lg hover:bg-neutral-100 dark:hover:bg-neutral-800 transition-colors"
      >
        <span className="text-sm font-medium text-neutral-700 dark:text-neutral-300">
          {displayName}
        </span>
        <ChevronDown
          size={16}
          className={`text-neutral-500 transition-transform ${open ? 'rotate-180' : ''}`}
        />
      </button>

      {/* 下拉菜单 */}
      {open && (
        <>
          {/* 背景遮罩 */}
          <div className="fixed inset-0 z-10" onClick={() => setOpen(false)} />

          {/* 菜单内容 */}
          <div className="absolute top-full mt-2 left-0 min-w-[200px] bg-white dark:bg-neutral-800 rounded-xl shadow-lg border border-neutral-200 dark:border-neutral-700 z-20 max-h-[400px] overflow-y-auto">
            {providers.map((provider) => (
              <div key={provider.id} className="p-2">
                <div className="text-xs font-semibold text-neutral-500 dark:text-neutral-400 px-3 py-2">
                  {provider.name}
                </div>
                {(provider.enabledModels.length > 0
                  ? provider.enabledModels
                  : provider.availableModels
                ).map((model) => (
                  <button
                    key={model}
                    onClick={() => {
                      onModelChange(provider.id, model)
                      setOpen(false)
                    }}
                    className={`w-full text-left px-3 py-2 rounded-lg text-sm transition-colors ${
                      currentProviderId === provider.id && currentModel === model
                        ? 'bg-blue-50 dark:bg-blue-900/20 text-blue-600 dark:text-blue-400'
                        : 'hover:bg-neutral-100 dark:hover:bg-neutral-700 text-neutral-700 dark:text-neutral-300'
                    }`}
                  >
                    {model}
                  </button>
                ))}
              </div>
            ))}

            {providers.length === 0 && (
              <div className="p-4 text-sm text-neutral-500 text-center">暂无可用模型</div>
            )}
          </div>
        </>
      )}
    </div>
  )
}
