import { useState } from 'react'
import { act, fireEvent, render, screen, within } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { api, type Settings } from '../api/tauri'
import { i18n } from '../components/i18n'
import { makeProvider, makeSettings } from '../settings/tabs/testFixtures'
import { ProviderSetupPanel } from './ProviderSetupPanel'
import { HotkeyStep } from './steps/HotkeyStep'
import { DoneStep } from './steps/DoneStep'

vi.mock('../api/tauri', async () => {
  const actual = await vi.importActual<typeof import('../api/tauri')>('../api/tauri')
  return { ...actual, api: {
    ...actual.api,
    providerOAuthStart: vi.fn(async () => ({ loginId: 'login', userCode: 'CODE', verificationUrl: 'https://example.com/auth', interval: 3, expiresAt: 1900000000 })),
    providerOAuthPoll: vi.fn(async () => ({ status: 'authorized', interval: 3, auth: { provider: 'codex', credentialId: 'credential' } })),
    providerOAuthCancel: vi.fn(async () => {}),
    providerOAuthAccount: vi.fn(async () => ({ email: null, name: null, accountId: 'account' })),
    openExternal: vi.fn(async () => {}),
    testProviderConnection: vi.fn(async () => ({ success: true })),
  } }
})

afterEach(() => { vi.useRealTimers(); vi.clearAllMocks() })

describe('onboarding matches current capabilities', () => {
  it('authorizes an OAuth preset and tests with the unsaved credential reference', async () => {
    vi.useFakeTimers()
    function Setup() {
      const [settings, setSettings] = useState(makeSettings())
      return <ProviderSetupPanel t={i18n.zh} lang="zh" settings={settings} onChange={setSettings} />
    }
    render(<Setup />)
    fireEvent.click(screen.getByRole('button', { name: 'Codex OAuth' }))
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: '登录授权' })) })
    await act(async () => { await vi.advanceTimersByTimeAsync(3000) })
    expect(screen.getByText('已授权')).toBeInTheDocument()
    expect(screen.queryByText(/设置会自动保存/)).not.toBeInTheDocument()
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: '测试连接' })) })
    expect(api.testProviderConnection).toHaveBeenCalledWith(expect.any(String), expect.objectContaining({
      request: expect.objectContaining({ oauth: { provider: 'codex', credentialId: 'credential' } }),
      apiFormat: 'openai_responses',
    }))
  })

  it('records the chat shortcut and keeps explicitly cleared shortcuts empty', () => {
    let latest: Settings = makeSettings({ hotkey: '', screenshotTranslation: { ...makeSettings().screenshotTranslation, replaceHotkey: '' } })
    function Setup() {
      const [settings, setSettings] = useState(latest)
      return <HotkeyStep t={i18n.zh} settings={settings} onChange={next => { latest = next; setSettings(next) }} />
    }
    render(<Setup />)
    const row = screen.getByText(i18n.zh.chatHotkeyLabel).closest('.onboarding-form-row') as HTMLElement
    fireEvent.click(within(row).getByRole('button', { name: i18n.zh.hotkeyRecord }))
    fireEvent.keyDown(window, { key: 'u', code: 'KeyU', ctrlKey: true, shiftKey: true })
    expect(latest.chatHotkey).toBe('CommandOrControl+Shift+U')
    expect(latest.hotkey).toBe('')
    for (const label of [i18n.zh.onboardingHotkeyTranslator, i18n.zh.onboardingHotkeyReplace]) {
      const field = screen.getByText(label, { selector: '.onboarding-field-label' }).closest('.onboarding-form-row') as HTMLElement
      expect(within(field).queryByRole('button', { name: i18n.zh.hotkeyClear })).not.toBeInTheDocument()
    }
    fireEvent.click(within(row).getByRole('button', { name: i18n.zh.hotkeyClear }))
    expect(latest.chatHotkey).toBe('')
    expect(within(row).queryByRole('button', { name: i18n.zh.hotkeyClear })).not.toBeInTheDocument()
  })

  it('reports chat search as enabled independently from Lens search', () => {
    const settings = makeSettings({ providers: [makeProvider()] })
    settings.lens.webSearch = { enabled: false, provider: 'tavily', tavilyApiKey: 'key', exaApiKey: '', maxResults: 5, searchDepth: 'basic' }
    render(<DoneStep t={i18n.zh} settings={settings} />)
    const summary = within(screen.getByText(i18n.zh.onboardingDoneSectionModels).parentElement as HTMLElement)
    const chatRow = summary.getByText(`${i18n.zh.webSearchChatSection} · ${i18n.zh.onboardingDoneWebSearch}`).closest('.onboarding-summary-row') as HTMLElement
    const lensRow = summary.getByText(`${i18n.zh.webSearchLensSection} · ${i18n.zh.onboardingDoneWebSearch}`).closest('.onboarding-summary-row') as HTMLElement
    expect(within(chatRow).getByText(i18n.zh.enabled)).toBeInTheDocument()
    expect(within(lensRow).getByText(i18n.zh.onboardingDoneDisabled)).toBeInTheDocument()
  })
})
