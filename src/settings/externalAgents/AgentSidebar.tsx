import { ArrowUpCircle, RefreshCw } from 'lucide-react'
import type { DetectedExternalAgent } from '../../api/externalCliSettings'
import { Button, IconButton } from '../../components/Button'
import { i18n, type Lang } from '../../components/i18n'
import { getCliInstallJob } from '../cliInstallJobs'
import { SearchInput, Toggle } from '../components'
import { deriveCliMaintenance, getCliInstallInfo } from './cliMaintenance'
import { IconBox } from './shared'

export type AgentRow = {
  agent: DetectedExternalAgent
  disabled: boolean
  /** 第二行：它现在走哪个供应商 / 登没登录。只对已安装且启用的 CLI 有意义。 */
  summary: string
  /** 需要用户处理（例如未登录）。 */
  attention: boolean
}

/**
 * 左栏：全部 CLI，已安装的在前。
 *
 * 每行两层信息：名称 + 版本（有新版本多一个箭头），下面一行是它当前的状态。
 * 启用开关就在行上，不必点进去；停用和未安装的整行压暗但留在原位——
 * 拨一下开关就让这一行跳到别的分组里，用户会找不到它。
 */
export function AgentSidebar({
  lang,
  rows,
  selectedId,
  onSelect,
  onToggle,
  query,
  onQuery,
  scanning,
  scanFailed,
  onRescan,
  updatableIds,
  onUpdateAll,
}: {
  lang: Lang
  rows: AgentRow[]
  selectedId: string | null
  onSelect: (id: string) => void
  onToggle: (id: string, enabled: boolean) => void
  query: string
  onQuery: (value: string) => void
  scanning: boolean
  scanFailed: boolean
  onRescan: () => void
  /** 有现成安装计划、可以一键更新的 CLI。 */
  updatableIds: string[]
  onUpdateAll: () => void
}) {
  const t = i18n[lang]

  return (
    <nav className="kv-cli-nav" aria-label={t.externalAgentsSearch}>
      <div className="kv-cli-search">
        <SearchInput
          className="min-w-0 flex-1"
          value={query}
          onChange={onQuery}
          placeholder={t.externalAgentsSearch}
          aria-label={t.externalAgentsSearch}
          clearLabel={t.dialogClear}
        />
        <IconButton
          size="sm"
          variant="ghost"
          label={t.externalAgentsRescan}
          onClick={onRescan}
          disabled={scanning}
        >
          <RefreshCw size={13} className={scanning ? 'animate-spin' : ''} />
        </IconButton>
      </div>

      {updatableIds.length > 1 && (
        <div className="kv-cli-nav-updates">
          <span>{t.externalAgentsUpdatesCount.replace('{count}', String(updatableIds.length))}</span>
          <Button size="sm" variant="ghost" onClick={onUpdateAll}>
            {t.externalAgentsUpdateAll}
          </Button>
        </div>
      )}

      <div className="kv-cli-nav-list custom-scrollbar">
        {rows.map(({ agent, disabled, summary, attention }) => {
          const maintenance = deriveCliMaintenance(agent, getCliInstallInfo(agent.id))
          const running = getCliInstallJob(agent.id).running
          const selected = agent.id === selectedId
          const off = disabled || !agent.available
          return (
            <div
              key={agent.id}
              className={`kv-cli-nav-item ${selected ? 'active' : ''} ${off ? 'is-off' : ''}`}
            >
              <button
                type="button"
                className="kv-cli-nav-select"
                onClick={() => onSelect(agent.id)}
                aria-current={selected ? 'true' : undefined}
                data-tauri-drag-region="false"
              >
                <IconBox id={agent.id} size={28} />
                <span className="kv-cli-nav-text">
                  <span className="kv-cli-nav-title">
                    <span className="kv-cli-nav-name">{agent.name}</span>
                    {agent.available && maintenance.localVersion && (
                      <span className="kv-cli-nav-version">{maintenance.localVersion}</span>
                    )}
                    {running ? (
                      <span className="kv-cli-updating-dot" aria-label={t.externalAgentsInstalling} />
                    ) : (
                      !off &&
                      maintenance.state === 'outdated' && (
                        <span
                          className="kv-cli-nav-update"
                          title={t.externalAgentsUpdateAvailable.replace(
                            '{version}',
                            maintenance.latestVersion ?? '',
                          )}
                        >
                          <ArrowUpCircle size={13} aria-hidden="true" />
                        </span>
                      )
                    )}
                  </span>
                  <span className={`kv-cli-nav-sub ${attention ? 'warn' : ''}`}>{summary}</span>
                </span>
              </button>
              {agent.available && (
                <span className="kv-cli-nav-toggle" title={t.externalAgentsEnableDesc}>
                  <Toggle
                    checked={!disabled}
                    onChange={(enabled) => onToggle(agent.id, enabled)}
                    ariaLabel={`${agent.name} ${t.externalAgentsEnable}`}
                  />
                </span>
              )}
            </div>
          )
        })}
        {rows.length === 0 && (
          <p className="kv-cli-nav-empty">
            {scanning
              ? t.externalAgentsRescanning
              : scanFailed
                ? t.externalAgentsDetectFailed
                : t.externalAgentsNoMatch}
          </p>
        )}
      </div>
    </nav>
  )
}
