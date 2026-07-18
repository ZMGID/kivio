import { useCallback, useEffect, useMemo, useState } from 'react'
import { RefreshCw } from 'lucide-react'
import type { AgentRuntimeConfig, ChatConfig } from '../api/tauri'
import { AgentIcon } from '../chat/AgentIcon'
import { chatApi, type DetectedExternalAgent } from '../chat/api'
import { Select, SettingRow, SettingsGroup } from './components'
import { i18n, type Lang } from './i18n'
import { Button } from '../components/Button'

const BUILTIN_RUNTIME: AgentRuntimeConfig = {
  kind: 'builtin',
  externalAgentId: null,
  externalModel: null,
  externalReasoning: null,
}

function normalizeRuntime(raw?: AgentRuntimeConfig | null): AgentRuntimeConfig {
  if (!raw) return { ...BUILTIN_RUNTIME }
  const kind = raw.kind === 'external' ? 'external' : 'builtin'
  return {
    kind,
    externalAgentId: raw.externalAgentId ?? raw.external_agent_id ?? null,
    externalModel: raw.externalModel ?? raw.external_model ?? null,
    externalReasoning: raw.externalReasoning ?? raw.external_reasoning ?? null,
  }
}

function authLabel(agent: DetectedExternalAgent, lang: Lang): string {
  const t = i18n[lang]
  const status = agent.authStatus ?? agent.auth_status
  if (status === 'ok') return t.externalAgentsAuthOk
  if (status === 'auth_required') return t.externalAgentsAuthRequired
  return t.externalAgentsAuthUnknown
}

interface ExternalAgentsSettingsProps {
  lang: Lang
  chatConfig: ChatConfig
  onChatChange: (patch: Partial<ChatConfig>) => void
}

export function ExternalAgentsSettings({
  lang,
  chatConfig,
  onChatChange,
}: ExternalAgentsSettingsProps) {
  const t = i18n[lang]
  const [agents, setAgents] = useState<DetectedExternalAgent[]>([])
  const [scanning, setScanning] = useState(false)
  const [expandedId, setExpandedId] = useState<string | null>(null)

  const runtime = useMemo(
    () => normalizeRuntime(chatConfig.defaultAgentRuntime),
    [chatConfig.defaultAgentRuntime],
  )
  const usesExternal = runtime.kind === 'external'

  const loadAgents = useCallback(async (force = false) => {
    setScanning(true)
    try {
      const list = await chatApi.detectExternalAgents(force)
      setAgents(list)
    } catch (err) {
      console.error('[ExternalAgentsSettings] detect failed:', err)
      setAgents([])
    } finally {
      setScanning(false)
    }
  }, [])

  useEffect(() => {
    void loadAgents()
  }, [loadAgents])

  const selectedAgent = agents.find((item) => item.id === runtime.externalAgentId)
  const availableAgents = agents.filter((item) => item.available)
  const installedCount = availableAgents.length

  const updateRuntime = (next: AgentRuntimeConfig) => {
    onChatChange({ defaultAgentRuntime: next })
  }

  const selectBuiltin = () => {
    updateRuntime({ ...BUILTIN_RUNTIME })
  }

  const selectExternalAgent = (agentId: string) => {
    const agent = agents.find((item) => item.id === agentId)
    if (!agent?.available) return
    updateRuntime({
      kind: 'external',
      externalAgentId: agentId,
      externalModel: agent.models[0]?.id ?? 'default',
      externalReasoning: null,
    })
  }

  const modelOptions = (selectedAgent?.models ?? [{ id: 'default', label: 'Default' }]).map(
    (model) => ({ value: model.id, label: model.label }),
  )
  const reasoningOptions = (selectedAgent?.reasoningOptions
    ?? selectedAgent?.reasoning_options
    ?? []
  ).map((option) => ({ value: option.id, label: option.label }))

  return (
    <>
      <SettingsGroup title={t.externalAgentsDefaultSection}>
        <div className="mb-4 flex flex-wrap gap-2">
          <Button
            size="sm"
            variant={!usesExternal ? 'primary' : 'default'}
            onClick={selectBuiltin}
          >
            {t.externalAgentsModeBuiltin}
          </Button>
          <Button
            size="sm"
            variant={usesExternal ? 'primary' : 'default'}
            onClick={() => {
              if (usesExternal && runtime.externalAgentId) return
              const first = availableAgents[0]
              if (first) selectExternalAgent(first.id)
            }}
            disabled={installedCount === 0}
          >
            {t.externalAgentsModeExternal}
          </Button>
        </div>

        {usesExternal && (
          <>
            <SettingRow label={t.externalAgentsDefaultAgent}>
              <Select
                className="min-w-[180px]"
                value={runtime.externalAgentId ?? ''}
                onChange={selectExternalAgent}
                options={
                  availableAgents.length > 0
                    ? availableAgents.map((agent) => ({
                        value: agent.id,
                        label: agent.name,
                      }))
                    : [{ value: '', label: t.externalAgentsNoAvailable }]
                }
              />
            </SettingRow>
            {selectedAgent && (
              <SettingRow label={t.externalAgentsDefaultModel}>
                <Select
                  className="min-w-[180px]"
                  value={runtime.externalModel ?? 'default'}
                  onChange={(externalModel) =>
                    updateRuntime({
                      ...runtime,
                      kind: 'external',
                      externalModel,
                    })
                  }
                  options={modelOptions}
                />
              </SettingRow>
            )}
            {reasoningOptions.length > 0 && (
              <SettingRow label={t.externalAgentsDefaultReasoning}>
                <Select
                  className="min-w-[180px]"
                  value={runtime.externalReasoning ?? 'default'}
                  onChange={(externalReasoning) =>
                    updateRuntime({
                      ...runtime,
                      kind: 'external',
                      externalReasoning,
                    })
                  }
                  options={reasoningOptions}
                />
              </SettingRow>
            )}
          </>
        )}
      </SettingsGroup>

      <SettingsGroup title={t.externalAgentsDetectSection}>
        <div className="mb-3 flex flex-wrap items-center justify-between gap-2 px-1">
          <Button
            size="sm"
            onClick={() => void loadAgents(true)}
            disabled={scanning}
            data-tauri-drag-region="false"
          >
            <RefreshCw size={12} className={scanning ? 'animate-spin' : ''} />
            {scanning ? t.externalAgentsRescanning : t.externalAgentsRescan}
          </Button>
        </div>

        {agents.length === 0 && !scanning ? (
          <div className="rounded-xl border border-dashed border-neutral-200 px-4 py-6 text-center dark:border-neutral-700">
            <p className="text-[13px] font-medium text-neutral-800 dark:text-neutral-100">
              {t.externalAgentsNoAvailable}
            </p>
            <p className="kv-row-desc mt-1">{t.externalAgentsNoAvailableHint}</p>
          </div>
        ) : (
          <div className="flex flex-col gap-2">
            {agents.map((agent) => {
              const expanded = expandedId === agent.id
              const modelPreview = agent.models
                .slice(0, 6)
                .map((model) => model.id)
                .join(', ')
              return (
                <div
                  key={agent.id}
                  className="rounded-xl border border-neutral-200/90 bg-white px-3 py-3 dark:border-neutral-700 dark:bg-neutral-950/40"
                >
                  <button
                    type="button"
                    className="flex w-full items-start gap-3 text-left"
                    onClick={() => setExpandedId(expanded ? null : agent.id)}
                  >
                    <AgentIcon id={agent.id} size={28} />
                    <div className="min-w-0 flex-1">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="text-[14px] font-medium text-neutral-900 dark:text-neutral-50">
                          {agent.name}
                        </span>
                        <span className={`kv-tag ${agent.available ? 'ok' : ''}`}>
                          {agent.available
                            ? t.externalAgentsInstalled
                            : t.externalAgentsNotInstalled}
                        </span>
                        {agent.available && (
                          <span className="kv-row-desc text-[11px]">
                            {t.externalAgentsModelsCount.replace(
                              '{count}',
                              String(agent.models.length),
                            )}
                          </span>
                        )}
                      </div>
                      {agent.available && (
                        <p className="kv-row-desc mt-1 truncate">
                          {authLabel(agent, lang)}
                          {agent.version ? ` · ${agent.version}` : ''}
                        </p>
                      )}
                      {agent.id === 'cursor' && agent.available && (
                        <p className="kv-row-desc mt-1">{t.externalAgentsCursorToolLimit}</p>
                      )}
                    </div>
                  </button>
                  {expanded && (
                    <div className="mt-3 border-t border-neutral-100 pt-3 text-[12px] text-neutral-600 dark:border-neutral-800 dark:text-neutral-300">
                      {agent.path && (
                        <p className="break-all">
                          <span className="font-medium">{t.externalAgentsPath}: </span>
                          {agent.path}
                        </p>
                      )}
                      {agent.version && (
                        <p className="mt-1">
                          <span className="font-medium">{t.externalAgentsVersion}: </span>
                          {agent.version}
                        </p>
                      )}
                      {modelPreview && (
                        <p className="mt-1 break-all">
                          <span className="font-medium">{t.externalAgentsDefaultModel}: </span>
                          {modelPreview}
                          {agent.models.length > 6 ? '…' : ''}
                        </p>
                      )}
                    </div>
                  )}
                </div>
              )
            })}
          </div>
        )}
      </SettingsGroup>
    </>
  )
}
