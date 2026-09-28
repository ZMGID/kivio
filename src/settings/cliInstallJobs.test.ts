import { describe, expect, it, vi, beforeEach } from 'vitest'
import { onExternalCliInstallLog } from '../api/externalCliSettings'
import {
  getCliInstallJob,
  resetCliInstallJobsForTests,
  startCliInstall,
} from './cliInstallJobs'

vi.mock('../api/externalCliSettings', () => ({
  onExternalCliInstallLog: vi.fn().mockResolvedValue(() => {}),
}))

const mockOnInstallLog = vi.mocked(onExternalCliInstallLog)

describe('cliInstallJobs', () => {
  beforeEach(() => {
    resetCliInstallJobsForTests()
    mockOnInstallLog.mockReset()
    mockOnInstallLog.mockResolvedValue(() => {})
  })

  it('marks the job running before install() is awaited', async () => {
    let finish = () => {}
    const started = startCliInstall('claude', {
      install: () =>
        new Promise<void>((resolve) => {
          finish = resolve
        }),
    })
    expect(getCliInstallJob('claude').running).toBe(true)
    await Promise.resolve()
    finish()
    await started
    expect(getCliInstallJob('claude')).toEqual({
      running: false,
      log: [],
      result: 'ok',
    })
  })

  it('appends log lines from the install event stream', async () => {
    let emit: Parameters<typeof mockOnInstallLog>[0] | undefined
    mockOnInstallLog.mockImplementation(async (handler) => {
      emit = handler
      return () => {}
    })
    let finish = () => {}
    const started = startCliInstall('claude', {
      install: () =>
        new Promise<void>((resolve) => {
          finish = resolve
        }),
    })
    await Promise.resolve()
    emit?.({ agentId: 'claude', line: '$ npm', done: false, success: false })
    emit?.({ agentId: 'codex', line: 'ignore me', done: false, success: false })
    expect(getCliInstallJob('claude').log).toEqual(['$ npm'])
    finish()
    await started
  })

  it('does not start a second install while one is in flight', async () => {
    let finish = () => {}
    const install = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve
        }),
    )
    const first = startCliInstall('claude', { install })
    await Promise.resolve()
    expect(install).toHaveBeenCalledTimes(1)
    await startCliInstall('claude', { install })
    expect(install).toHaveBeenCalledTimes(1)
    finish()
    await first
  })

  it('keeps Pi running through startup verification and allows retry after failure', async () => {
    let emit: Parameters<typeof mockOnInstallLog>[0] | undefined
    const unlisten = vi.fn()
    mockOnInstallLog.mockImplementation(async (handler) => {
      emit = handler
      return unlisten
    })
    let rejectInstall: (error: Error) => void = () => {}
    const afterDone = vi.fn().mockResolvedValue(undefined)
    const started = startCliInstall('pi', {
      install: () => new Promise<void>((_resolve, reject) => { rejectInstall = reject }),
      afterDone,
    })
    await Promise.resolve()
    emit?.({ agentId: 'pi', line: 'added 144 packages', done: false, success: false })
    expect(getCliInstallJob('pi')).toMatchObject({ running: true, result: null })

    emit?.({ agentId: 'pi', line: 'Pi startup verification failed', done: false, success: false })
    emit?.({ agentId: 'pi', line: null, done: true, success: false })
    rejectInstall(new Error('Pi startup verification failed'))
    await started
    expect(getCliInstallJob('pi')).toMatchObject({ running: false, result: 'fail' })
    expect(getCliInstallJob('pi').log).toContain('Pi startup verification failed')
    expect(unlisten).toHaveBeenCalledOnce()
    expect(afterDone).toHaveBeenCalledWith('pi')

    await startCliInstall('pi', {
      install: async () => {
        emit?.({ agentId: 'pi', line: 'Pi 0.87.1 verified', done: false, success: false })
        emit?.({ agentId: 'pi', line: null, done: true, success: true })
      },
    })
    expect(getCliInstallJob('pi')).toEqual({
      running: false,
      result: 'ok',
      log: ['Pi 0.87.1 verified'],
    })
  })
})
