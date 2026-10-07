const enabledModels = new URLSearchParams(window.location.search).get('models') === 'text-only'
  ? ['test-text']
  : ['test-text', 'test-vision', 'test-vision-alt']
const settings = {
  providers: [
    { id: 'study-fixture', name: 'Simulated provider', enabled: true, apiKeys: [], baseUrl: '', apiFormat: 'openai_chat', request: {}, availableModels: ['test-text', 'test-vision', 'test-vision-alt'], enabledModels, modelOverrides: { 'test-text': { capabilities: { vision: false } }, 'test-vision': { capabilities: { vision: true } }, 'test-vision-alt': { capabilities: { vision: true } } } },
  ],
  // Study must choose an eligible image model even when regular chat uses text only.
  defaultModels: { chat: { providerId: 'study-fixture', model: 'test-text' } },
}
export const getSettingsCached = async () => settings
export const subscribeSettings = () => () => {}
