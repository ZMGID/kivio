import { useEffect, useRef } from 'react'
import {
  ArrowUpCircle,
  CheckCircle2,
  CircleAlert,
  CircleDashed,
  Copy,
  ExternalLink,
  Info,
  Loader2,
  RefreshCw,
  UserRound,
} from 'lucide-react'
import type { DetectedExternalAgent } from '../../api/externalCliSettings'
import { Button, IconButton } from '../../components/Button'
import { i18n, type Lang } from '../../components/i18n'
import { copyToClipboard } from '../../utils/clipboard'
import { clearCliInstallJob } from '../cliInstallJobs'
import {
  checkCliInstallInfo,
  deriveCliMaintenance,
  runCliInstall,
  useCliInstallInfo,
  useCliInstallJob,
  type CliMaintenanceState,
} from './cliMaintenance'
import { Row } from './shared'
import { useCliAccount } from './useCliAccount'

type Strings = (typeof i18n)[Lang]

const STATE_TONE: Record<CliMaintenanceState, '' | 'ok' | 'warn'> = {
  checking: '',
  'check-failed': 'warn',
  'not-installed': '',
  broken: 'warn',
  'latest-unknown': 'warn',
  outdated: 'warn',
  current: 'ok',
}

function StateIcon({ state }: { state: CliMaintenanceState }) {
  if (state === 'checking') return <Loader2 size={15} className="animate-spin" />
  if (state === 'current') return <CheckCircle2 size={15} />
  if (state === 'outdated') return <ArrowUpCircle size={15} />
  if (state === 'not-installed') return <CircleDashed size={15} />
  return <CircleAlert size={15} />
}

function stateHeadline(state: CliMaintenanceState, latestVersion: string | null, t: Strings): string {
  switch (state) {
    case 'checking':
      return t.externalAgentsCheckingVersion
    case 'check-failed':
      return t.externalAgentsVersionCheckFailed
    case 'not-installed':
      return t.externalAgentsNotInstalled
    case 'broken':
      return t.externalAgentsLocalVersionUnknown
    case 'latest-unknown':
      return t.externalAgentsLatestVersionUnknown
    case 'outdated':
      return t.externalAgentsUpdateAvailable.replace('{version}', latestVersion ?? '')
    case 'current':
      return t.externalAgentsUpToDate
  }
}

function installDoneLabel(agentId: string, stillBroken: boolean, t: Strings): string {
  if (stillBroken) {
    return agentId === 'dsh'
      ? t.externalAgentsInstallNeedsRepair
      : t.externalAgentsInstallNeedsRepairGeneric
  }
  return agentId === 'dsh' ? t.externalAgentsInstallDoneDsh : t.externalAgentsInstallDone
}

/**
 * 详情最上面那张卡：这个 CLI 现在是什么状态、要不要动手、登的是哪个账号。
 *
 * 版本状态、安装动作、它为什么不能自动更新，原先分散成徽章 / 提示段落 / 动作条三处，
 * 这里收成一行：左边说状态和原因，右边放唯一的那个动作。
 */
export function MaintenanceCard({
  agent,
  lang,
  reloadAgents,
}: {
  agent: DetectedExternalAgent
  lang: Lang
  reloadAgents: (force?: boolean) => Promise<void>
}) {
  const t = i18n[lang]
  const entry = useCliInstallInfo(agent.id)
  const job = useCliInstallJob(agent.id)
  const { info, checking, checkedAt } = entry
  const { running, log, result } = job
  const maintenance = deriveCliMaintenance(agent, entry)
  const { state, latestVersion, localVersion, actionable } = maintenance
  const logRef = useRef<HTMLPreElement | null>(null)
  const rawAuth = agent.authStatus ?? agent.auth_status
  const authStatus = rawAuth === 'ok' || rawAuth === 'auth_required' ? rawAuth : null

  // 首次打开复用刚查过的结果；之后探测到的路径 / 版本变了（改了自定义路径、在外面升了级），
  // 手上的安装计划就是针对旧二进制的，必须重查。
  const checkedOnce = useRef(false)
  useEffect(() => {
    void checkCliInstallInfo(agent.id, checkedOnce.current)
    checkedOnce.current = true
  }, [agent.id, agent.path, agent.version, agent.available])

  useEffect(() => {
    const node = logRef.current
    if (node && typeof node.scrollTo === 'function') node.scrollTo({ top: node.scrollHeight })
  }, [log])

  // 成功提示停留片刻就收掉；失败的留着，日志是用户排查的唯一线索。
  useEffect(() => {
    if (result !== 'ok' || running) return
    const timer = window.setTimeout(() => clearCliInstallJob(agent.id), 4000)
    return () => window.clearTimeout(timer)
  }, [agent.id, result, running])

  const needsAction = state === 'not-installed' || state === 'broken' || state === 'outdated'
  // 能自动执行时把将要跑的命令原样摆出来（见下方命令条）；不能执行时说为什么不能。
  // 不需要动手就什么都不说——「无法验证 npm 安装归属」对一个已是最新的 CLI 只是噪音。
  const current = localVersion ? t.externalAgentsCurrentVersion.replace('{version}', localVersion) : null
  const detail = !needsAction
    ? state === 'latest-unknown'
      ? current
      : null
    : info?.planId
      ? current
      : info?.manualReason ?? null
  const command = needsAction && info?.planId ? info.command : null
  const checkLabel = checkedAt
    ? `${t.externalAgentsCheckUpdate} · ${t.externalAgentsLastChecked.replace(
        '{time}',
        new Date(checkedAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }),
      )}`
    : t.externalAgentsCheckUpdate
  const notes = [
    agent.id === 'dsh' && !agent.available ? t.externalAgentsDshInstallHint : null,
    agent.id === 'antigravity' ? t.externalAgentsAntigravityNote : null,
  ].filter((note) => note !== null)

  return (
    <div className="kv-cli-card kv-cli-status">
      <Row
        lead={
          <span className={`kv-cli-status-icon ${STATE_TONE[state]}`} aria-hidden="true">
            <StateIcon state={state} />
          </span>
        }
        label={stateHeadline(state, latestVersion, t)}
        desc={detail}
      >
        {(actionable || running) && (
          <Button
            size="sm"
            variant="primary"
            onClick={() => void runCliInstall(agent.id, reloadAgents)}
            disabled={running || checking}
          >
            {running
              ? t.externalAgentsInstalling
              : agent.available
                ? t.externalAgentsUpdate
                : t.externalAgentsInstall}
          </Button>
        )}
        {needsAction && !actionable && !running && info && (
          <a
            className="kv-btn sm"
            href={info.docsUrl}
            target="_blank"
            rel="noreferrer"
            data-tauri-drag-region="false"
          >
            <ExternalLink size={12} />
            {t.externalAgentsDocs}
          </a>
        )}
        <IconButton
          size="sm"
          variant="ghost"
          label={checkLabel}
          onClick={() => void checkCliInstallInfo(agent.id, true)}
          disabled={checking || running}
        >
          <RefreshCw size={13} className={checking ? 'animate-spin' : ''} />
        </IconButton>
      </Row>

      {command && !running && (
        <div className="kv-row-stack kv-cli-command">
          <code title={command}>{command}</code>
          <IconButton
            size="xs"
            variant="ghost"
            label={t.externalAgentsCopyCommand}
            onClick={() => void copyToClipboard(command)}
          >
            <Copy size={12} />
          </IconButton>
        </div>
      )}

      {(running || result) && (
        <div className="kv-row-stack kv-cli-job">
          <div
            className={`kv-cli-job-status ${
              running ? 'running' : result === 'ok' ? (state === 'broken' ? 'warn' : 'ok') : 'fail'
            }`}
            role="status"
          >
            {running
              ? t.externalAgentsInstalling
              : result === 'ok'
                ? installDoneLabel(agent.id, state === 'broken', t)
                : t.externalAgentsInstallFailed}
          </div>
          {(running || result === 'fail') && (
            <pre ref={logRef} className="kv-cli-log" aria-label={t.externalAgentsInstallLog}>
              {log.length > 0 ? log.join('\n') : t.externalAgentsInstalling}
            </pre>
          )}
        </div>
      )}

      {agent.available &&
        (agent.capabilities.account === 'native' ? (
          <AccountRow agentId={agent.id} t={t} />
        ) : (
          authStatus && (
            <Row
              lead={
                <span
                  className={authStatus === 'ok' ? 'kv-cli-status-icon' : 'kv-cli-status-icon warn'}
                  aria-hidden="true"
                >
                  <UserRound size={15} />
                </span>
              }
              label={t.externalAgentsAccount}
              desc={authStatus === 'ok' ? t.externalAgentsSignedIn : t.externalAgentsSignedOutHint}
            />
          )
        ))}

      {notes.map((note) => (
        <div className="kv-row kv-cli-note" key={note}>
          <Info size={13} aria-hidden="true" />
          <p className="kv-row-desc">{note}</p>
        </div>
      ))}
    </div>
  )
}

function AccountRow({ agentId, t }: { agentId: string; t: Strings }) {
  const { account, loading, error, refresh } = useCliAccount(agentId)
  const identity = [account?.account?.email, account?.account?.plan].filter(Boolean).join(' · ')
  const desc = error
    ? error
    : identity ||
      (loading && !account
        ? t.externalAgentsAccountLoading
        : account?.account?.type ?? t.externalAgentsAccountUnknown)
  return (
    <Row
      lead={
        <span className="kv-cli-status-icon" aria-hidden="true">
          <UserRound size={15} />
        </span>
      }
      label={t.externalAgentsAccount}
      desc={desc}
    >
      <IconButton
        size="sm"
        variant="ghost"
        label={t.externalAgentsAccountRefresh}
        onClick={() => void refresh()}
        disabled={loading}
      >
        <RefreshCw size={13} className={loading ? 'animate-spin' : ''} />
      </IconButton>
    </Row>
  )
}
