import { useState } from 'react'
import { Blocks, BookOpen, ChevronRight, ExternalLink, SlidersHorizontal } from 'lucide-react'
import type { DetectedExternalAgent } from '../../api/externalCliSettings'
import type { ExternalCliAgentConfig } from '../../api/tauri'
import { i18n, type Lang } from '../../components/i18n'
import { DshPluginsSettings } from '../DshPluginsSettings'
import { deriveCliMaintenance, useCliInstallInfo } from './cliMaintenance'
import { MaintenanceCard } from './MaintenanceCard'
import { ModelsSection } from './ModelsSection'
import { ProviderSection } from './ProviderSection'
import { RuntimeSection } from './RuntimeSection'
import { IconBox, Section } from './shared'

/** 通往二级页的入口行。 */
function SubpageEntry({
  icon,
  label,
  hint,
  action,
  onOpen,
}: {
  icon: React.ReactNode
  label: string
  hint: string
  action: string
  onOpen: () => void
}) {
  return (
    <button
      type="button"
      className="kv-row kv-cli-entry"
      onClick={onOpen}
      data-tauri-drag-region="false"
    >
      <span className="kv-cli-monogram" aria-hidden="true">
        {icon}
      </span>
      <span className="kv-row-text">
        <span className="kv-row-label">{label}</span>
        <span className="kv-row-desc">{hint}</span>
      </span>
      <span className="kv-subpage-entry-go">
        {action}
        <ChevronRight size={14} aria-hidden="true" />
      </span>
    </button>
  )
}

/**
 * 右栏：一个 CLI 的全部设置。各区顺序对每个 CLI 固定——
 * 状态 → 供应商 → 模型 → 扩展 → 运行环境。
 * 没装的 CLI 只留状态（含安装动作）和运行环境：装好或指对路径之前，其余配置都用不上。
 */
export function AgentDetail({
  agent,
  lang,
  config,
  onPatch,
  reloadAgents,
  onOpenPiExtensions,
  onOpenPiSkills,
}: {
  agent: DetectedExternalAgent
  lang: Lang
  config: ExternalCliAgentConfig
  onPatch: (patch: Partial<ExternalCliAgentConfig>) => void
  reloadAgents: (force?: boolean) => Promise<void>
  onOpenPiExtensions: () => void
  onOpenPiSkills: () => void
}) {
  const t = i18n[lang]
  const entry = useCliInstallInfo(agent.id)
  const { localVersion } = deriveCliMaintenance(agent, entry)
  const [showPlugins, setShowPlugins] = useState(false)

  if (showPlugins && agent.id === 'dsh') {
    return <DshPluginsSettings lang={lang} onBack={() => setShowPlugins(false)} />
  }

  return (
    <>
      <header className="kv-cli-head">
        <IconBox id={agent.id} size={40} />
        <div className="kv-cli-head-main">
          <h2 className="kv-cli-head-title">{agent.name}</h2>
          {localVersion && <span className="kv-cli-version">{localVersion}</span>}
        </div>
        {entry.info && (
          <a
            className="kv-cli-link"
            href={entry.info.docsUrl}
            target="_blank"
            rel="noreferrer"
            data-tauri-drag-region="false"
          >
            <ExternalLink size={12} />
            {t.externalAgentsDocs}
          </a>
        )}
      </header>

      <MaintenanceCard agent={agent} lang={lang} reloadAgents={reloadAgents} />

      {agent.available && (
        <ProviderSection
          lang={lang}
          agentId={agent.id}
          agentName={agent.name}
          providers={config.providers ?? []}
          nativeProviders={agent.nativeProviders ?? []}
          current={config.currentProvider ?? ''}
          onPatch={onPatch}
          reloadAgents={reloadAgents}
        />
      )}

      {agent.available && (
        <ModelsSection
          lang={lang}
          agentId={agent.id}
          customModels={config.customModels ?? []}
          onChange={(customModels) => onPatch({ customModels })}
        />
      )}

      {agent.available && agent.id === 'dsh' && (
        <Section title={t.externalAgentsExtensionsSection}>
          <div className="kv-cli-card">
            <SubpageEntry
              icon={<SlidersHorizontal size={13} />}
              label={t.externalAgentsDshPlugins}
              hint={t.externalAgentsDshPluginsHint}
              action={t.externalAgentsConfigure}
              onOpen={() => setShowPlugins(true)}
            />
          </div>
        </Section>
      )}
      {agent.available && agent.id === 'pi' && (
        <Section title={t.externalAgentsExtensionsSection}>
          <div className="kv-cli-card">
            <SubpageEntry
              icon={<Blocks size={13} />}
              label={t.externalAgentsPiExtensions}
              hint={t.externalAgentsPiExtensionsHint}
              action={t.externalAgentsManage}
              onOpen={onOpenPiExtensions}
            />
            <SubpageEntry
              icon={<BookOpen size={13} />}
              label={t.externalAgentsPiSkills}
              hint={t.externalAgentsPiSkillsHint}
              action={t.externalAgentsManage}
              onOpen={onOpenPiSkills}
            />
          </div>
        </Section>
      )}

      <RuntimeSection
        lang={lang}
        agentId={agent.id}
        detectedPath={agent.path ?? null}
        configDir={entry.info?.configDir ?? null}
        config={config}
        onPatch={onPatch}
      />
    </>
  )
}
