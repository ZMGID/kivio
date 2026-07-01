import { useState, useEffect, useCallback } from 'react'
import { ArrowLeft, RotateCcw } from 'lucide-react'
import type { ModelInfo } from '../api/tauri'
import { resolveModelInfo, matchModel } from '../data/modelMatching'
import { Toggle, Input } from '../settings/components'

type Lang = 'zh' | 'zh-TW' | 'en'

type ModelDetailDrawerProps = {
  modelName: string
  overrides?: Record<string, ModelInfo>
  lang: Lang
  onClose: () => void
  onSave: (modelName: string, info: ModelInfo) => void
  onReset: (modelName: string) => void
}

function deepEqual(a: unknown, b: unknown): boolean {
  return JSON.stringify(a) === JSON.stringify(b)
}

export function ModelDetailDrawer({
  modelName,
  overrides,
  lang,
  onClose,
  onSave,
  onReset,
}: ModelDetailDrawerProps) {
  const resolved = resolveModelInfo(modelName, overrides)
  const dbDefaults = matchModel(modelName)
  const hasOverride = !!overrides?.[modelName]

  const [form, setForm] = useState<ModelInfo>(resolved)

  useEffect(() => {
    setForm(resolveModelInfo(modelName, overrides))
  }, [modelName, overrides])

  const updateField = useCallback(<K extends keyof ModelInfo>(key: K, value: ModelInfo[K]) => {
    setForm(prev => ({ ...prev, [key]: value }))
  }, [])

  const updateCapability = useCallback((key: keyof NonNullable<ModelInfo['capabilities']>, value: boolean) => {
    setForm(prev => ({
      ...prev,
      capabilities: { ...prev.capabilities, [key]: value },
    }))
  }, [])

  const updatePricing = useCallback((key: keyof NonNullable<ModelInfo['pricing']>, value: string) => {
    const num = value === '' ? undefined : Number(value)
    setForm(prev => ({
      ...prev,
      pricing: { ...prev.pricing, [key]: num },
    }))
  }, [])

  const handleSave = useCallback(() => {
    onSave(modelName, form)
  }, [modelName, form, onSave])

  const handleReset = useCallback(() => {
    onReset(modelName)
    if (dbDefaults) {
      setForm(dbDefaults)
    }
  }, [modelName, onReset, dbDefaults])

  const isDirty = !deepEqual(form, resolved)

  const t = {
    title: lang.startsWith('zh') ? '模型詳情' : 'Model Details',
    back: lang.startsWith('zh') ? '返回' : 'Back',
    displayName: lang.startsWith('zh') ? '顯示名稱' : 'Display Name',
    contextWindow: lang.startsWith('zh') ? '上下文長度' : 'Context Window',
    maxOutput: lang.startsWith('zh') ? '最大輸出' : 'Max Output',
    capabilities: lang.startsWith('zh') ? '功能' : 'Capabilities',
    vision: lang.startsWith('zh') ? '圖像輸入' : 'Image Input',
    functionCalling: lang.startsWith('zh') ? '工具呼叫' : 'Tool Calling',
    reasoning: lang.startsWith('zh') ? '推理模式' : 'Reasoning',
    streaming: lang.startsWith('zh') ? '串流輸出' : 'Streaming',
    webSearch: lang.startsWith('zh') ? '網路搜尋' : 'Web Search',
    imageGeneration: lang.startsWith('zh') ? '生圖' : 'Image Generation',
    pricing: lang.startsWith('zh') ? '定價 (per 1M tokens, USD)' : 'Pricing (per 1M tokens, USD)',
    input: lang.startsWith('zh') ? '輸入' : 'Input',
    output: lang.startsWith('zh') ? '輸出' : 'Output',
    cachedInput: lang.startsWith('zh') ? '快取輸入' : 'Cached Input',
    save: lang.startsWith('zh') ? '儲存' : 'Save',
    reset: lang.startsWith('zh') ? '重設為預設值' : 'Reset to Defaults',
    noDatabase: lang.startsWith('zh') ? '未在資料庫中找到此模型，可手動填寫參數。' : 'Model not found in database. You can fill in parameters manually.',
  }

  return (
    <div
      className="kv-modal-backdrop"
      data-tauri-drag-region="false"
      onMouseDown={(e) => { if (e.target === e.currentTarget) onClose() }}
    >
      <div className="kv-drawer" data-tauri-drag-region="false" onMouseDown={(e) => e.stopPropagation()}>
        <div className="kv-drawer-header">
          <button
            type="button"
            className="kv-icon-btn"
            onClick={onClose}
            data-tauri-drag-region="false"
            aria-label={t.back}
          >
            <ArrowLeft size={14} />
          </button>
          <span className="kv-drawer-title truncate">{modelName}</span>
          <span style={{ width: 28 }} />
        </div>

        <div className="kv-drawer-body custom-scrollbar">
          {!dbDefaults && (
            <p className="kv-drawer-hint">{t.noDatabase}</p>
          )}

          <div className="kv-drawer-section">
            <label className="kv-drawer-label">{t.displayName}</label>
            <Input
              value={form.displayName || ''}
              onChange={(v) => updateField('displayName', v || undefined)}
              placeholder={modelName}
              mono
            />
          </div>

          <div className="kv-drawer-row">
            <div className="kv-drawer-section flex-1">
              <label className="kv-drawer-label">{t.contextWindow}</label>
              <Input
                type="number"
                value={form.contextWindow?.toString() || ''}
                onChange={(v) => updateField('contextWindow', v ? Number(v) : undefined)}
                placeholder="-"
              />
            </div>
            <div className="kv-drawer-section flex-1">
              <label className="kv-drawer-label">{t.maxOutput}</label>
              <Input
                type="number"
                value={form.maxOutput?.toString() || ''}
                onChange={(v) => updateField('maxOutput', v ? Number(v) : undefined)}
                placeholder="-"
              />
            </div>
          </div>

          <div className="kv-drawer-section">
            <label className="kv-drawer-label">{t.capabilities}</label>
            <div className="kv-drawer-toggles">
              <CapabilityToggle label={t.vision} checked={form.capabilities?.vision ?? false} onChange={(v) => updateCapability('vision', v)} />
              <CapabilityToggle label={t.functionCalling} checked={form.capabilities?.functionCalling ?? false} onChange={(v) => updateCapability('functionCalling', v)} />
              <CapabilityToggle label={t.reasoning} checked={form.capabilities?.reasoning ?? false} onChange={(v) => updateCapability('reasoning', v)} />
              <CapabilityToggle label={t.streaming} checked={form.capabilities?.streaming ?? false} onChange={(v) => updateCapability('streaming', v)} />
              <CapabilityToggle label={t.webSearch} checked={form.capabilities?.webSearch ?? false} onChange={(v) => updateCapability('webSearch', v)} />
              <CapabilityToggle label={t.imageGeneration} checked={form.capabilities?.imageGeneration ?? false} onChange={(v) => updateCapability('imageGeneration', v)} />
            </div>
          </div>

          <div className="kv-drawer-section">
            <label className="kv-drawer-label">{t.pricing}</label>
            <div className="kv-drawer-row">
              <div className="kv-drawer-section flex-1">
                <label className="kv-drawer-sublabel">{t.input}</label>
                <Input
                  type="number"
                  value={form.pricing?.input?.toString() || ''}
                  onChange={(v) => updatePricing('input', v)}
                  placeholder="0.00"
                />
              </div>
              <div className="kv-drawer-section flex-1">
                <label className="kv-drawer-sublabel">{t.output}</label>
                <Input
                  type="number"
                  value={form.pricing?.output?.toString() || ''}
                  onChange={(v) => updatePricing('output', v)}
                  placeholder="0.00"
                />
              </div>
              <div className="kv-drawer-section flex-1">
                <label className="kv-drawer-sublabel">{t.cachedInput}</label>
                <Input
                  type="number"
                  value={form.pricing?.cachedInput?.toString() || ''}
                  onChange={(v) => updatePricing('cachedInput', v)}
                  placeholder="-"
                />
              </div>
            </div>
          </div>
        </div>

        <div className="kv-drawer-footer">
          {hasOverride && (
            <button
              type="button"
              className="kv-btn ghost"
              onClick={handleReset}
              data-tauri-drag-region="false"
            >
              <RotateCcw size={12} />
              {t.reset}
            </button>
          )}
          <div className="flex-1" />
          <button
            type="button"
            className="kv-btn primary"
            onClick={handleSave}
            disabled={!isDirty}
            data-tauri-drag-region="false"
          >
            {t.save}
          </button>
        </div>
      </div>
    </div>
  )
}

function CapabilityToggle({ label, checked, onChange }: {
  label: string
  checked: boolean
  onChange: (v: boolean) => void
}) {
  return (
    <div className="kv-drawer-toggle-row">
      <span className="kv-drawer-toggle-label">{label}</span>
      <Toggle checked={checked} onChange={onChange} />
    </div>
  )
}
