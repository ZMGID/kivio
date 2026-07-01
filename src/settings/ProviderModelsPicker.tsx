import { useMemo, useState } from 'react'
import { createPortal } from 'react-dom'
import { ChevronDown, Minus, Plus, RefreshCw, Search, X } from 'lucide-react'
import type { ModelProvider } from '../api/tauri'
import { ModelIcon } from '../chat/ModelIcon'
import { Input } from './components'

type Lang = 'zh' | 'zh-TW' | 'en'

export type ProviderModelsPickerLabels = {
  title: string
  searchPlaceholder: string
  fetchModels: string
  fetching: string
  addModel: string
  manualAddModel: string
  noModels: string
  noSearchResults: string
  enabled: string
  addAllModels: string
  close: string
}

type ProviderModelsPickerProps = {
  provider: ModelProvider
  lang: Lang
  labels: ProviderModelsPickerLabels
  fetching: boolean
  onClose: () => void
  onFetch: () => void
  onAdd: (model: string) => void
  onAddAll: (models: string[]) => void
  onRemove: (model: string) => void
}

function modelKey(model: string) {
  return model.toLowerCase()
}

export function ProviderModelsPicker({
  provider,
  lang,
  labels,
  fetching,
  onClose,
  onFetch,
  onAdd,
  onAddAll,
  onRemove,
}: ProviderModelsPickerProps) {
  const [query, setQuery] = useState('')
  const [groupOpen, setGroupOpen] = useState(true)
  const [manualOpen, setManualOpen] = useState(false)
  const [manualValue, setManualValue] = useState('')

  const enabledSet = useMemo(
    () => new Set(provider.enabledModels.map(modelKey)),
    [provider.enabledModels],
  )

  const allModels = useMemo(() => {
    const seen = new Set<string>()
    const merged: string[] = []
    for (const model of [...provider.availableModels, ...provider.enabledModels]) {
      const key = modelKey(model)
      if (!model.trim() || seen.has(key)) continue
      seen.add(key)
      merged.push(model)
    }
    return merged.sort((a, b) => a.localeCompare(b, undefined, { sensitivity: 'base' }))
  }, [provider.availableModels, provider.enabledModels])

  const filteredModels = useMemo(() => {
    const q = query.trim().toLowerCase()
    if (!q) return allModels
    return allModels.filter((model) => model.toLowerCase().includes(q))
  }, [allModels, query])

  const addableModels = useMemo(
    () => filteredModels.filter((model) => !enabledSet.has(modelKey(model))),
    [enabledSet, filteredModels],
  )

  const submitManual = () => {
    const value = manualValue.trim()
    if (!value) return
    onAdd(value)
    setManualValue('')
    setManualOpen(false)
  }

  return createPortal(
    <div
      className="kv-modal-backdrop kv-modal-backdrop--portal"
      data-tauri-drag-region="false"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose()
      }}
    >
      <div
        className="kv kv-modal kv-model-picker"
        role="dialog"
        aria-modal="true"
        aria-labelledby="kv-model-picker-title"
        data-tauri-drag-region="false"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="kv-model-picker-header">
          <div className="kv-model-picker-title-row">
            <h3 id="kv-model-picker-title" className="kv-model-picker-title">
              {labels.title}
            </h3>
            <span className="kv-tag">{provider.enabledModels.length}</span>
          </div>
          <button
            type="button"
            className="kv-icon-btn"
            onClick={onClose}
            data-tauri-drag-region="false"
            aria-label={labels.close}
          >
            <X size={14} />
          </button>
        </div>

        <div className="kv-model-picker-search">
          <Search size={14} className="kv-model-picker-search-icon" />
          <Input
            value={query}
            onChange={setQuery}
            placeholder={labels.searchPlaceholder}
            mono={false}
          />
        </div>

        <div className="kv-model-picker-toolbar">
          <button
            type="button"
            className="kv-btn sm kv-model-picker-fetch"
            onClick={onFetch}
            disabled={fetching}
            data-tauri-drag-region="false"
          >
            <RefreshCw size={12} className={fetching ? 'animate-spin' : ''} />
            {fetching ? labels.fetching : labels.fetchModels}
          </button>
          <button
            type="button"
            className="kv-btn sm kv-model-picker-add-toggle"
            onClick={() => setManualOpen((open) => !open)}
            data-tauri-drag-region="false"
            aria-expanded={manualOpen}
            aria-label={labels.addModel}
          >
            <Plus size={14} strokeWidth={2.25} />
          </button>
        </div>

        {manualOpen && (
          <div className="kv-model-picker-manual">
            <Input
              className="!text-[12px]"
              value={manualValue}
              onChange={setManualValue}
              placeholder={labels.manualAddModel}
              mono
              onKeyDown={(e: React.KeyboardEvent<HTMLInputElement>) => {
                if (e.key !== 'Enter') return
                if (e.nativeEvent.isComposing || e.keyCode === 229) return
                submitManual()
              }}
            />
            <button
              type="button"
              className="kv-btn sm"
              onClick={submitManual}
              data-tauri-drag-region="false"
            >
              {labels.addModel}
            </button>
          </div>
        )}

        <div className="kv-model-picker-body custom-scrollbar">
          <div className="kv-model-picker-group-head">
            <button
              type="button"
              className="kv-model-picker-group-toggle"
              onClick={() => setGroupOpen((open) => !open)}
              data-tauri-drag-region="false"
            >
              <ChevronDown
                size={14}
                className={`kv-model-picker-chevron ${groupOpen ? 'open' : ''}`}
              />
              <span className="kv-model-picker-group-name truncate">
                {provider.name || provider.id}
              </span>
              <span className="kv-tag">{filteredModels.length}</span>
            </button>
            {addableModels.length > 0 && (
              <button
                type="button"
                className="kv-model-picker-row-btn add shrink-0"
                onClick={() => onAddAll(addableModels)}
                data-tauri-drag-region="false"
                aria-label={labels.addAllModels}
                title={labels.addAllModels}
              >
                <Plus size={14} strokeWidth={2.25} />
              </button>
            )}
          </div>

          {groupOpen && (
            <ul className="kv-model-picker-list">
              {fetching && filteredModels.length === 0 && (
                <li className="kv-model-picker-empty">{labels.fetching}</li>
              )}
              {!fetching && filteredModels.length === 0 && (
                <li className="kv-model-picker-empty">
                  {query.trim() ? labels.noSearchResults : labels.noModels}
                </li>
              )}
              {filteredModels.map((model) => {
                const isEnabled = enabledSet.has(modelKey(model))
                return (
                  <li key={model} className="kv-model-picker-row">
                    <ModelIcon model={model} size={16} />
                    <span className="kv-model-picker-row-name" title={model}>
                      {model}
                    </span>
                    {isEnabled ? (
                      <span className="kv-tag ok shrink-0">{labels.enabled}</span>
                    ) : null}
                    <button
                      type="button"
                      className={`kv-model-picker-row-btn ${isEnabled ? 'remove' : 'add'}`}
                      onClick={() => (isEnabled ? onRemove(model) : onAdd(model))}
                      data-tauri-drag-region="false"
                      aria-label={
                        isEnabled
                           ? (lang.startsWith('zh') ? `移除 ${model}` : `Remove ${model}`)
                           : (lang.startsWith('zh') ? `新增 ${model}` : `Add ${model}`)
                      }
                    >
                      {isEnabled ? <Minus size={14} /> : <Plus size={14} strokeWidth={2.25} />}
                    </button>
                  </li>
                )
              })}
            </ul>
          )}
        </div>
      </div>
    </div>,
    document.body,
  )
}
