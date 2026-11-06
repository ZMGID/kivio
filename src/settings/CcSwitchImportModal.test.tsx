import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { externalCliSettingsApi, type ImportedCcSwitchProvider } from '../api/externalCliSettings'
import { i18n } from '../components/i18n'
import { CcSwitchImportModal } from './CcSwitchImportModal'

vi.mock('../api/externalCliSettings', () => ({
  externalCliSettingsApi: { externalCliScanCcSwitch: vi.fn(), externalCliImportCcSwitch: vi.fn() },
}))

const selected: ImportedCcSwitchProvider = {
  agentId: 'claude', id: 'selected', name: 'Selected provider', remark: '',
  hasApiKey: true, isCurrent: false, env: [{ key: 'API_KEY', value: 'selected-test-key' }],
  configToml: '', authJson: '',
}
const scan = vi.mocked(externalCliSettingsApi.externalCliScanCcSwitch)
const importSelected = vi.mocked(externalCliSettingsApi.externalCliImportCcSwitch)

beforeEach(() => {
  scan.mockReset().mockResolvedValue({ providers: [
    { agentId: 'claude', id: 'selected', name: 'Selected provider', remark: '', hasApiKey: true, isCurrent: false },
    { agentId: 'claude', id: 'unselected', name: 'Unselected provider', remark: '', hasApiKey: true, isCurrent: false },
  ], skipped: 0 })
  importSelected.mockReset().mockResolvedValue([selected])
})

it('reads only selected provider details and permits retry after a read failure', async () => {
  const onImport = vi.fn()
  importSelected.mockRejectedValueOnce(new Error('source unavailable'))
  render(<CcSwitchImportModal lang="zh" agentId="claude" existingIds={[]} onImport={onImport} onClose={vi.fn()} />)
  fireEvent.click(await screen.findByRole('checkbox', { name: /Unselected provider/ }))
  const button = screen.getByRole('button', { name: i18n.zh.externalAgentsProviderImportConfirm.replace('{count}', '1') })
  fireEvent.click(button)
  await screen.findByText('Error: source unavailable')
  expect(onImport).not.toHaveBeenCalled()
  fireEvent.click(button)
  await waitFor(() => expect(onImport).toHaveBeenCalledWith([selected]))
  expect(importSelected).toHaveBeenLastCalledWith([{ agentId: 'claude', providerId: 'selected' }])
  expect(screen.queryByText('selected-test-key')).not.toBeInTheDocument()
})

it('does not apply a selected import after the picker has closed', async () => {
  let complete!: (providers: ImportedCcSwitchProvider[]) => void
  importSelected.mockReturnValue(new Promise((resolve) => { complete = resolve }))
  const onImport = vi.fn()
  const view = render(<CcSwitchImportModal lang="zh" agentId="claude" existingIds={[]} onImport={onImport} onClose={vi.fn()} />)
  await screen.findByRole('checkbox', { name: /Selected provider/ })
  fireEvent.click(screen.getByRole('button', { name: i18n.zh.externalAgentsProviderImportConfirm.replace('{count}', '2') }))
  view.unmount()
  await act(async () => { complete([selected]) })
  expect(onImport).not.toHaveBeenCalled()
})
