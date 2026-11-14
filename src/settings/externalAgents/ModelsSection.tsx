import { useEffect, useRef, useState } from 'react'
import { Plus, X } from 'lucide-react'
import {
  externalCliSettingsApi,
  type DetectedExternalAgent,
} from '../../api/externalCliSettings'
import type { ExternalCliAgentConfig } from '../../api/tauri'
import { Button, IconButton } from '../../components/Button'
import { i18n, type Lang } from '../../components/i18n'
import { Input, SearchInput } from '../components'
import { Section } from './shared'

/** 模型多到这个数才给筛选框：三五个模型配一个搜索框只是占地方。 */
const FILTER_THRESHOLD = 8

type CustomModel = NonNullable<ExternalCliAgentConfig['customModels']>[number]
type ProbedModel = DetectedExternalAgent['models'][number]

function contextLabel(model: ProbedModel): string | null {
  const tokens = model.contextWindowTokens ?? model.context_window_tokens
  if (!tokens) return null
  if (tokens >= 1_000_000) return `${+(tokens / 1_000_000).toFixed(1)}M`
  return `${Math.round(tokens / 1000)}K`
}

/**
 * 「模型」：探测到的和手动添加的排成一张表，底下一行直接加新的。
 * 后端返回的探测列表里已经并入了保存过的自定义模型，这里按 id 去重，自定义的只出现一次。
 */
export function ModelsSection({
  lang,
  agentId,
  customModels,
  onChange,
}: {
  lang: Lang
  agentId: string
  customModels: CustomModel[]
  onChange: (models: CustomModel[]) => void
}) {
  const t = i18n[lang]
  const [probed, setProbed] = useState<ProbedModel[]>([])
  const [query, setQuery] = useState('')
  const [newId, setNewId] = useState('')
  const [newLabel, setNewLabel] = useState('')
  const idInput = useRef<HTMLInputElement | null>(null)

  useEffect(() => {
    let cancelled = false
    void externalCliSettingsApi
      .detectExternalAgentModels(agentId, null, false)
      .then(({ models }) => {
        if (!cancelled) setProbed(models)
      })
      .catch(() => {
        // 探测失败时这里就只剩自定义模型；失败原因在聊天里选模型时会报。
      })
    return () => {
      cancelled = true
    }
  }, [agentId])

  const custom = customModels.filter((model) => model.id.trim())
  const customIds = new Set(custom.map((model) => model.id))
  const builtIn = probed.filter((model) => !customIds.has(model.id))
  const total = builtIn.length + custom.length
  const needle = query.trim().toLowerCase()
  const matches = (id: string, label: string) =>
    !needle || id.toLowerCase().includes(needle) || label.toLowerCase().includes(needle)
  const shownBuiltIn = builtIn.filter((model) => matches(model.id, model.label))
  const shownCustom = custom.filter((model) => matches(model.id, model.label))

  const candidate = newId.trim()
  const duplicate = Boolean(candidate) && (customIds.has(candidate) || builtIn.some((m) => m.id === candidate))
  const add = () => {
    if (!candidate || duplicate) return
    onChange([...custom, { id: candidate, label: newLabel.trim() }])
    setNewId('')
    setNewLabel('')
    idInput.current?.focus()
  }
  const submitOnEnter = (event: React.KeyboardEvent) => {
    if (event.key === 'Enter') add()
  }

  return (
    <Section
      title={t.externalAgentsModelsSection}
      desc={t.externalAgentsModelsSummary
        .replace('{probed}', String(builtIn.length))
        .replace('{custom}', String(custom.length))}
      actions={
        total > FILTER_THRESHOLD && (
          <SearchInput
            className="kv-cli-model-filter"
            value={query}
            onChange={setQuery}
            placeholder={t.externalAgentsModelsFilter}
            aria-label={t.externalAgentsModelsFilter}
            clearLabel={t.dialogClear}
          />
        )
      }
    >
      <div className="kv-cli-card kv-cli-models">
        {total > 0 && (
          <ul className="kv-cli-model-list custom-scrollbar">
            {shownBuiltIn.map((model) => (
              <li key={model.id} className="kv-cli-model">
                <code className="kv-cli-model-id">{model.id}</code>
                {model.label && model.label !== model.id && (
                  <span className="kv-cli-model-label">{model.label}</span>
                )}
                {contextLabel(model) && <span className="kv-tag">{contextLabel(model)}</span>}
              </li>
            ))}
            {shownCustom.map((model) => (
              <li key={`custom-${model.id}`} className="kv-cli-model">
                <code className="kv-cli-model-id">{model.id}</code>
                {model.label && model.label !== model.id && (
                  <span className="kv-cli-model-label">{model.label}</span>
                )}
                <span className="kv-tag accent">{t.externalAgentsModelsCustom}</span>
                <IconButton
                  size="xs"
                  variant="ghost"
                  label={`${t.externalAgentsRemove} ${model.id}`}
                  onClick={() => onChange(custom.filter((item) => item.id !== model.id))}
                >
                  <X size={13} />
                </IconButton>
              </li>
            ))}
            {shownBuiltIn.length + shownCustom.length === 0 && (
              <li className="kv-cli-model-empty">{t.externalAgentsModelsNoMatch}</li>
            )}
          </ul>
        )}
        <div className="kv-cli-model-add">
          <Input
            ref={idInput}
            value={newId}
            onChange={setNewId}
            onKeyDown={submitOnEnter}
            placeholder={t.externalAgentsModelId}
            aria-label={t.externalAgentsModelId}
            aria-invalid={duplicate || undefined}
            title={duplicate ? t.externalAgentsModelDuplicate : undefined}
            spellCheck={false}
            mono
          />
          <Input
            value={newLabel}
            onChange={setNewLabel}
            onKeyDown={submitOnEnter}
            placeholder={t.externalAgentsModelLabel}
            aria-label={t.externalAgentsModelLabel}
          />
          <Button size="sm" onClick={add} disabled={!candidate || duplicate}>
            <Plus size={12} />
            {t.externalAgentsModelAdd}
          </Button>
        </div>
        <p className="kv-row-desc">
          {duplicate ? t.externalAgentsModelDuplicate : t.externalAgentsModelCustomHint}
        </p>
      </div>
    </Section>
  )
}
