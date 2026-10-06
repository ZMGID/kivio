const settings = {
  providers: [
    { id: 'study-fixture', name: 'Simulated provider', enabled: true, apiKeys: [], baseUrl: '', apiFormat: 'openai_chat', request: {}, availableModels: ['test-text', 'test-vision'], enabledModels: ['test-text', 'test-vision'], modelOverrides: { 'test-text': { capabilities: { vision: false } }, 'test-vision': { capabilities: { vision: true } } } },
  ],
  defaultModels: { chat: { providerId: 'study-fixture', model: 'test-text' } },
}
export const getSettingsCached = async () => settings
export const subscribeSettings = () => () => {}
