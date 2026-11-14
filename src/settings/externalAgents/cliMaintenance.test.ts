import { beforeEach, describe, expect, it, vi } from 'vitest'
import { externalCliSettingsApi, type ExternalCliInstallInfo } from '../../api/externalCliSettings'
import { resetCliInstallJobsForTests } from '../cliInstallJobs'
import {
  checkCliInstallInfo,
  checkCliInstallInfoBatch,
  deriveCliMaintenance,
  getCliInstallInfo,
  resetCliInstallInfoForTests,
  runCliInstall,
  type CliInstallInfoEntry,
} from './cliMaintenance'
import { isCliVersionOutdated } from './cliVersions'

vi.mock('../../api/externalCliSettings', () => ({
  externalCliSettingsApi: {
    externalCliInstallInfo: vi.fn(),
    externalCliInstall: vi.fn(),
  },
  onExternalCliInstallLog: vi.fn().mockResolvedValue(() => {}),
}))

const mockInfo = vi.mocked(externalCliSettingsApi.externalCliInstallInfo)
const mockInstall = vi.mocked(externalCliSettingsApi.externalCliInstall)

function info(patch: Partial<ExternalCliInstallInfo> = {}): ExternalCliInstallInfo {
  return {
    agentId: 'claude',
    localVersion: '1.0.0',
    latestVersion: '1.1.0',
    updateAvailable: true,
    command: 'npm install',
    docsUrl: 'https://docs',
    configDir: null,
    planId: 'plan-1',
    manualReason: null,
    binaryPath: '/bin/claude',
    targetVersion: '1.1.0',
    environment: 'native',
    source: 'npm',
    ...patch,
  }
}

function entry(value: ExternalCliInstallInfo | null, failed = false): CliInstallInfoEntry {
  return { info: value, checking: false, checkedAt: 1, failed }
}

describe('isCliVersionOutdated', () => {
  it('orders release, prerelease and unparsable versions', () => {
    expect(isCliVersionOutdated('1.0.0', '1.0.1')).toBe(true)
    expect(isCliVersionOutdated('1.2.0', '1.2.0')).toBe(false)
    expect(isCliVersionOutdated('0.1.0-rc.6', '0.1.0-rc.7')).toBe(true)
    expect(isCliVersionOutdated('0.1.0-rc.7', '0.1.0')).toBe(true)
    expect(isCliVersionOutdated('0.1.0', '0.1.0-rc.7')).toBe(false)
    expect(isCliVersionOutdated('codex-cli 0.167.0', '0.169.0')).toBe(true)
    expect(isCliVersionOutdated('nightly', '1.0.0')).toBeNull()
  })
})

describe('deriveCliMaintenance', () => {
  const installed = { available: true, version: '1.0.0' }

  it('distinguishes a pending first check from a failed one', () => {
    expect(deriveCliMaintenance(installed, entry(null)).state).toBe('checking')
    expect(deriveCliMaintenance(installed, entry(null, true)).state).toBe('check-failed')
  })

  it('only calls an update actionable when a plan exists', () => {
    expect(deriveCliMaintenance(installed, entry(info()))).toMatchObject({
      state: 'outdated',
      actionable: true,
    })
    expect(deriveCliMaintenance(installed, entry(info({ planId: null })))).toMatchObject({
      state: 'outdated',
      actionable: false,
    })
  })

  it('never offers an action for a CLI that is already current', () => {
    const current = info({ localVersion: '1.1.0', updateAvailable: false })
    expect(deriveCliMaintenance(installed, entry(current))).toMatchObject({
      state: 'current',
      actionable: false,
    })
  })

  it('falls back to the detected version when install info has none', () => {
    const result = deriveCliMaintenance(
      installed,
      entry(info({ localVersion: null, updateAvailable: false })),
    )
    expect(result).toMatchObject({ state: 'outdated', localVersion: '1.0.0' })
  })

  it('reports a present binary with no readable version as broken', () => {
    const result = deriveCliMaintenance(
      { available: true, version: null },
      entry(info({ localVersion: null, updateAvailable: false })),
    )
    expect(result).toMatchObject({ state: 'broken', actionable: true })
  })

  it('reports a missing binary as not installed', () => {
    const result = deriveCliMaintenance({ available: false }, entry(info({ localVersion: null })))
    expect(result).toMatchObject({ state: 'not-installed', actionable: true })
  })
})

describe('install info store', () => {
  beforeEach(() => {
    vi.useRealTimers()
    resetCliInstallInfoForTests()
    resetCliInstallJobsForTests()
    mockInfo.mockReset()
    mockInstall.mockReset()
    mockInfo.mockResolvedValue(info())
    mockInstall.mockResolvedValue()
  })

  it('reuses a fresh result and refetches on demand', async () => {
    await checkCliInstallInfo('claude')
    await checkCliInstallInfo('claude')
    expect(mockInfo).toHaveBeenCalledTimes(1)

    await checkCliInstallInfo('claude', true)
    expect(mockInfo).toHaveBeenCalledTimes(2)
  })

  it('shares one request between concurrent checks', async () => {
    await Promise.all([checkCliInstallInfo('claude', true), checkCliInstallInfo('claude', true)])
    expect(mockInfo).toHaveBeenCalledTimes(1)
  })

  it('retries a failed check instead of caching the failure', async () => {
    mockInfo.mockRejectedValueOnce(new Error('offline'))
    await checkCliInstallInfo('claude')
    expect(getCliInstallInfo('claude')).toMatchObject({ info: null, failed: true })

    await checkCliInstallInfo('claude')
    expect(getCliInstallInfo('claude')).toMatchObject({ failed: false })
    expect(getCliInstallInfo('claude').info?.planId).toBe('plan-1')
  })

  it('checks every requested CLI in a batch', async () => {
    await checkCliInstallInfoBatch(['a', 'b', 'c', 'd', 'e'])
    expect(mockInfo.mock.calls.map(([id]) => id).sort()).toEqual(['a', 'b', 'c', 'd', 'e'])
  })

  it('swaps a stale plan for a new one before installing', async () => {
    vi.useFakeTimers()
    await checkCliInstallInfo('claude')
    // 后端的计划 10 分钟过期：放久了的那份不能直接拿去执行。
    vi.advanceTimersByTime(9 * 60_000)
    mockInfo.mockResolvedValue(info({ planId: 'plan-2' }))
    const reload = vi.fn().mockResolvedValue(undefined)

    await runCliInstall('claude', reload)

    expect(mockInstall).toHaveBeenCalledWith('claude', 'plan-2')
    expect(reload).toHaveBeenCalledWith(true)
  })

  it('does nothing when there is no plan to run', async () => {
    mockInfo.mockResolvedValue(info({ planId: null }))
    await checkCliInstallInfo('claude')

    await runCliInstall('claude', vi.fn())

    expect(mockInstall).not.toHaveBeenCalled()
  })
})
