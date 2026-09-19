import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { getSettingsSnapshotCached, updateSettingsCached } from '../api/settingsCache'
import { OnboardingShell } from './OnboardingShell'

vi.mock('../api/settingsCache', () => ({
  getSettingsSnapshotCached: vi.fn(),
  saveSettingsSnapshotCached: vi.fn(),
  updateSettingsCached: vi.fn(),
}))

const load = vi.mocked(getSettingsSnapshotCached)
const update = vi.mocked(updateSettingsCached)

beforeEach(() => {
  load.mockReset()
  update.mockReset()
  load.mockRejectedValue(new Error('load failed'))
  vi.spyOn(console, 'error').mockImplementation(() => {})
})

describe('OnboardingShell load failure', () => {
  it('only leaves after the narrow skipped write succeeds', async () => {
    const onSkip = vi.fn()
    const onSettingsChange = vi.fn()
    update.mockResolvedValue({} as never)
    render(<OnboardingShell onComplete={vi.fn()} onSkip={onSkip} onSettingsChange={onSettingsChange} />)
    fireEvent.click(await screen.findByRole('button', { name: '跳过引导' }))
    await waitFor(() => expect(onSkip).toHaveBeenCalledOnce())
    expect(onSettingsChange).toHaveBeenCalledOnce()
    expect(update).toHaveBeenCalledOnce()
    expect(load).toHaveBeenCalledOnce()
  })

  it('keeps the error visible and stays put when the skipped write fails', async () => {
    const onSkip = vi.fn()
    update.mockRejectedValue(new Error('save failed'))
    render(<OnboardingShell onComplete={vi.fn()} onSkip={onSkip} />)
    fireEvent.click(await screen.findByRole('button', { name: '跳过引导' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('save failed')
    expect(onSkip).not.toHaveBeenCalled()
  })
})
