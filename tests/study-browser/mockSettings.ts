import { api, type SettingsSnapshot } from '../../src/api/tauri'
import { makeProvider, makeSettings } from '../../src/settings/tabs/testFixtures'

export function installSettingsFixture() {
  const query = new URLSearchParams(location.search)
  const enabledModels = query.get('models') === 'text-only' ? ['test-text'] : ['test-text', 'test-vision', 'test-vision-alt']
  const settings = makeSettings({
    theme: 'light', themeColor: 'neutral', favoriteModels: [], settingsLanguage: query.get('lang') === 'en' ? 'en' : 'zh', onboardingStatus: 'completed',
    providers: query.get('models') === 'none' ? [] : [makeProvider({ id: 'study-fixture', name: 'Simulated provider', apiKeys: [], baseUrl: '', availableModels: enabledModels, enabledModels,
      modelOverrides: { 'test-text': { capabilities: { vision: false } }, 'test-vision': { capabilities: { vision: true } }, 'test-vision-alt': { capabilities: { vision: true } } },
    })],
    chat: { maxOutputTokens: 2048, streamEnabled: true, defaultLanguage: 'zh', defaultAgentRuntime: { kind: 'builtin' } },
    chatMemory: { enabled: false, toolWriteConfirm: true },
  })
  settings.defaultModels.chat = { providerId: 'study-fixture', model: 'test-text' }
  let snapshot: SettingsSnapshot = { settings, version: { epoch: 'simulated-browser-settings', revision: 1 } }
  api.getSettings = async () => structuredClone(snapshot)
  api.saveSettings = async next => { snapshot = { settings: structuredClone(next), version: { ...snapshot.version, revision: snapshot.version.revision + 1 } }; return structuredClone(snapshot) }
  api.setFavoriteModels = async models => api.saveSettings({ ...snapshot.settings, favoriteModels: models }, snapshot.version)
  api.reasoningEffortsForModel = async () => []
}
