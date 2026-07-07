import { useState, useEffect, useCallback } from 'react'
import { ArrowLeft, RotateCcw } from 'lucide-react'
import type { ModelInfo } from '../api/tauri'
import { resolveModelInfo, matchModel } from '../data/modelMatching'
import { Toggle, Input } from '../settings/components'
import { Button, IconButton } from './Button'

type Lang = 'zh' | 'en'

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
    title: lang === 'zh' ? '模型详情' : 'Model Details',
    back: lang === 'zh' ? '返回' : 'Back',
    displayName: lang === 'zh' ? '显示名称' : 'Display Name',
    contextWindow: lang === 'zh' ? '上下文长度' : 'Context Window',
    maxOutput: lang === 'zh' ? '最大输出' : 'Max Output',
    capabilities: lang === 'zh' ? '功能' : 'Capabilities',
    vision: lang === 'zh' ? '图像输入' : 'Image Input',
    functionCalling: lang === 'zh' ? '工具调用' : 'Tool Calling',
    reasoning: lang === 'zh' ? '推理模式' : 'Reasoning',
    streaming: lang === 'zh' ? '流式输出' : 'Streaming',
    webSearch: lang === 'zh' ? '网络搜索' : 'Web Search',
    imageGeneration: lang === 'zh' ? '生图' : 'Image Generation',
    pricing: lang === 'zh' ? '定价 (per 1M tokens, USD)' : 'Pricing (per 1M tokens, USD)',
    input: lang === 'zh' ? '输入' : 'Input',
    output: lang === 'zh' ? '输出' : 'Output',
    cachedInput: lang === 'zh' ? '缓存输入' : 'Cached Input',
    save: lang === 'zh' ? '保存' : 'Save',
    reset: lang === 'zh' ? '重置为默认值' : 'Reset to Defaults',
    noDatabase: lang === 'zh' ? '未在数据库中找到此模型，可手动填写参数。' : 'Model not found in database. You can fill in parameters manually.',
  }

  return (
    <div
      className="kv-modal-backdrop"
      data-tauri-drag-region="false"
      onMouseDown={(e) => { if (e.target === e.currentTarget) onClose() }}
    >
      <div className="kv-drawer" data-tauri-drag-region="false" onMouseDown={(e) => e.stopPropagation()}>
        <div className="kv-drawer-header">
          <IconButton
            size="xs"
            onClick={onClose}
            data-tauri-drag-region="false"
            label={t.back}
          >
            <ArrowLeft size={14} />
          </IconButton>
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
            <Button
              variant="ghost"
              onClick={handleReset}
              data-tauri-drag-region="false"
            >
              <RotateCcw size={12} />
              {t.reset}
            </Button>
          )}
          <div className="flex-1" />
          <Button
            variant="primary"
            onClick={handleSave}
            disabled={!isDirty}
            data-tauri-drag-region="false"
          >
            {t.save}
          </Button>
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
