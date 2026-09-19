import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import type { SettingsSnapshot } from '../api/tauri'
import { i18n } from '../components/i18n'
import { makeSettings } from './tabs/testFixtures'
import { SettingsShell } from './SettingsShell'

const canonical: SettingsSnapshot = {
  settings: makeSettings({ settingsLanguage: 'zh', chat: { defaultLanguage: 'zh' } as never }),
  version: { epoch: 'test', revision: 1 },
}
const pendingSave = new Promise<SettingsSnapshot>(() => {})

vi.mock('../api/settingsCache', () => ({
  peekSettingsSnapshot: () => canonical,
  getSettingsSnapshotCached: async () => canonical,
  refreshSettingsSnapshot: async () => canonical,
  saveSettingsSnapshotCached: () => pendingSave,
  subscribeSettingsSnapshot: () => () => {},
  importSettingsSnapshotCached: async () => canonical,
  updateSettingsCached: vi.fn(),
}))

vi.mock('../api/tauri', async (importOriginal) => {
  const original = await importOriginal<typeof import('../api/tauri')>()
  return {
    ...original,
    api: {
      ...original.api,
      getAppVersion: async () => 'test',
      getDefaultPromptTemplates: async () => ({}),
      listSystemFonts: async () => [],
      getPermissionStatus: async () => ({ platform: 'windows', accessibility: true, screenRecording: true }),
      onHotkeyWarning: async () => () => {},
      onUpdateAvailable: async () => () => {},
      onReplaceTranslationPackProgress: async () => () => {},
    },
  }
})

describe('SettingsShell session center language slot', () => {
  it('renders the unsaved language draft before the canonical snapshot changes', async () => {
    const user = userEvent.setup()
    const renderSessionCenter = (lang: 'zh' | 'en') => <p data-testid="session-language">{lang}</p>
    render(
      <SettingsShell
        variant="embedded"
        onClose={vi.fn()}
        onSettingsChange={vi.fn()}
        renderSessionCenter={renderSessionCenter}
        renderPluginCenter={() => null}
        renderReleaseNotes={() => null}
      />,
    )

    await user.click(await screen.findByRole('button', { name: '中文' }))
    await user.click(screen.getByRole('option', { name: 'English' }))
    await user.click(screen.getByRole('button', { name: i18n.en.tabSessions }))

    expect(screen.getByTestId('session-language')).toHaveTextContent('en')
    expect(canonical.settings.settingsLanguage).toBe('zh')
  })
})
