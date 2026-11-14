import { useSyncExternalStore } from 'react'
import {
  externalCliSettingsApi,
  type DetectedExternalAgent,
  type ExternalCliInstallInfo,
} from '../../api/externalCliSettings'
import {
  getCliInstallJob,
  startCliInstall,
  subscribeCliInstallJobs,
  type CliInstallJob,
} from '../cliInstallJobs'
import { isCliVersionOutdated } from './cliVersions'

/**
 * 每个 CLI 的版本 / 安装计划快照。
 *
 * 放在组件树外面有两个原因：左栏要给每一行标「可更新」，不能只有选中的那个知道；
 * 来回切换 CLI 也不该每次都重新打一遍 registry。
 */
export type CliInstallInfoEntry = {
  info: ExternalCliInstallInfo | null
  checking: boolean
  /** 最近一次检查完成的时间；从未检查过为 `null`。 */
  checkedAt: number | null
  failed: boolean
}

const EMPTY_ENTRY: CliInstallInfoEntry = { info: null, checking: false, checkedAt: null, failed: false }
/** 这段时间内重复打开同一个 CLI 不再重查。 */
const FRESH_MS = 5 * 60_000
/** 后端的安装计划 10 分钟过期；留出余量，超过就先换一份新的再执行。 */
const PLAN_MAX_AGE_MS = 8 * 60_000
const BATCH_CONCURRENCY = 3

const entries = new Map<string, CliInstallInfoEntry>()
const pending = new Map<string, Promise<void>>()
const listeners = new Set<() => void>()
let revision = 0

function write(agentId: string, next: CliInstallInfoEntry) {
  entries.set(agentId, next)
  revision += 1
  for (const listener of listeners) listener()
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

export function getCliInstallInfo(agentId: string): CliInstallInfoEntry {
  return entries.get(agentId) ?? EMPTY_ENTRY
}

/** 查一次版本与安装计划。`force` 为 false 时，刚查过的直接复用。 */
export function checkCliInstallInfo(agentId: string, force = false): Promise<void> {
  const inFlight = pending.get(agentId)
  if (inFlight) return inFlight
  const current = getCliInstallInfo(agentId)
  if (
    !force &&
    !current.failed &&
    current.checkedAt !== null &&
    Date.now() - current.checkedAt < FRESH_MS
  ) {
    return Promise.resolve()
  }
  write(agentId, { ...current, checking: true })
  const task = (async () => {
    let info: ExternalCliInstallInfo | null = null
    try {
      info = await externalCliSettingsApi.externalCliInstallInfo(agentId)
    } catch {
      info = null
    }
    pending.delete(agentId)
    write(agentId, { info, checking: false, checkedAt: Date.now(), failed: info === null })
  })()
  pending.set(agentId, task)
  return task
}

/** 批量检查，限并发：每个 CLI 都要打一次 registry，十个一起发没必要。 */
export async function checkCliInstallInfoBatch(agentIds: string[], force = false): Promise<void> {
  const queue = [...agentIds]
  const worker = async () => {
    for (let id = queue.shift(); id !== undefined; id = queue.shift()) {
      await checkCliInstallInfo(id, force)
    }
  }
  await Promise.all(Array.from({ length: Math.min(BATCH_CONCURRENCY, queue.length) }, worker))
}

/** Tests only. */
export function resetCliInstallInfoForTests() {
  entries.clear()
  pending.clear()
  revision += 1
}

export function useCliInstallInfo(agentId: string): CliInstallInfoEntry {
  return useSyncExternalStore(subscribe, () => getCliInstallInfo(agentId))
}

export function useCliInstallJob(agentId: string): CliInstallJob {
  return useSyncExternalStore(subscribeCliInstallJobs, () => getCliInstallJob(agentId))
}

let jobsRevision = 0
subscribeCliInstallJobs(() => {
  jobsRevision += 1
})

/** 左栏用：任意一个 CLI 的版本信息或安装任务变了都重渲染。 */
export function useCliMaintenanceRevision(): number {
  const info = useSyncExternalStore(subscribe, () => revision)
  const jobs = useSyncExternalStore(subscribeCliInstallJobs, () => jobsRevision)
  return info + jobs
}

export type CliMaintenanceState =
  /** 还没有任何一次检查结果。 */
  | 'checking'
  | 'check-failed'
  | 'not-installed'
  /** 文件在，但读不出版本：通常是装了一半或依赖坏了。 */
  | 'broken'
  | 'latest-unknown'
  | 'outdated'
  | 'current'

export type CliMaintenance = {
  state: CliMaintenanceState
  localVersion: string | null
  latestVersion: string | null
  /** 有可以直接执行的安装计划，且当前状态确实需要它。 */
  actionable: boolean
}

export function deriveCliMaintenance(
  agent: Pick<DetectedExternalAgent, 'available' | 'version'>,
  entry: CliInstallInfoEntry,
): CliMaintenance {
  const { info } = entry
  const localVersion = info?.localVersion ?? agent.version ?? null
  const latestVersion = info?.latestVersion ?? null
  const outdated =
    (localVersion && latestVersion ? isCliVersionOutdated(localVersion, latestVersion) : null) ??
    info?.updateAvailable ??
    false
  let state: CliMaintenanceState
  if (!info) state = entry.failed ? 'check-failed' : 'checking'
  else if (!agent.available) state = 'not-installed'
  else if (!localVersion) state = 'broken'
  else if (!latestVersion) state = 'latest-unknown'
  else state = outdated ? 'outdated' : 'current'
  const needsAction = state === 'not-installed' || state === 'broken' || state === 'outdated'
  return {
    state,
    localVersion,
    latestVersion,
    actionable: needsAction && Boolean(info?.planId),
  }
}

/**
 * 执行安装 / 更新。计划太旧就先换一份，免得点下去才被后端以「已过期」拒绝。
 * 任务本身挂在 `cliInstallJobs` 上，切走详情不会丢进度。
 */
export async function runCliInstall(
  agentId: string,
  reloadAgents: (force?: boolean) => Promise<void>,
): Promise<void> {
  if (getCliInstallJob(agentId).running) return
  const { checkedAt } = getCliInstallInfo(agentId)
  if (checkedAt === null || Date.now() - checkedAt > PLAN_MAX_AGE_MS) {
    await checkCliInstallInfo(agentId, true)
  }
  const planId = getCliInstallInfo(agentId).info?.planId
  if (!planId) return
  await startCliInstall(agentId, planId, {
    install: (id, plan) => externalCliSettingsApi.externalCliInstall(id, plan),
    afterDone: async () => {
      // 计划是一次性的，执行完必须重查，否则下一次点击拿到的是已消费的计划。
      await checkCliInstallInfo(agentId, true)
      await reloadAgents(true)
    },
  })
}
