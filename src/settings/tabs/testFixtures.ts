import type { ChatToolsConfig, Settings as SettingsData, ModelProvider, ProviderRequestConfig } from '../../api/tauri'
import { createProviderRequestDraft } from '../public/providerDraft'

/** Representative component input; not a persistence-default implementation. */
export function makeChatToolsFixture(overrides: Partial<ChatToolsConfig> = {}): ChatToolsConfig {
  return {
    enabled: false,
    servers: [],
    hooks: [],
    skillScanPaths: [],
    skillAutoMatch: true,
    skillFallbackMode: 'progressive',
    disabledSkillIds: [],
    maxToolRounds: null,
    toolTimeoutMs: 60_000,
    mcpIdleTimeoutMs: 600_000,
    approvalPolicy: 'readonly_auto_sensitive_confirm',
    subAgentConcurrency: 12,
    requestDebugEnabled: false,
    nativeTools: {
      readFile: true,
      writeFile: true,
      editFile: true,
      runCommand: true,
      skillRuntime: true,
      webSearch: true,
      webFetch: true,
      knowledgeSearch: true,
      automation: true,
      workingDirectory: '',
      workspaceRoots: [],
    },
    ...overrides,
  }
}

/**
 * 设置页 tab 组件测试用的最小 Settings。
 *
 * Settings 有 60+ 字段，逐个填只会让测试脆弱（加一个字段就要改所有 fixture）。
 * 这里只给被测 tab 真正读到的字段，其余按 Partial 断言掉 —— 组件只读自己那几个键，
 * 缺失字段若被误读会立刻 undefined 报错，反而比填假值更能暴露问题。
 */
export function makeSettings(overrides: Partial<SettingsData> = {}): SettingsData {
  return {
    hotkey: 'CommandOrControl+Shift+K',
    chatHotkey: 'CommandOrControl+Shift+J',
    closeChatHotkey: 'CommandOrControl+Shift+W',
    theme: 'system',
    themeColor: 'neutral',
    translucentSidebar: true,
    targetLang: 'auto',
    autoPaste: false,
    launchAtStartup: false,
    launchMinimizedToTray: false,
    keepChatWindowAlive: false,
    chatCompletionNotifications: false,
    translatorProviderId: 'p1',
    translatorModel: 'gpt-4o',
    chatProviderId: 'p1',
    chatModel: 'gpt-4o',
    retryEnabled: true,
    retryAttempts: 3,
    providers: [],
    defaultModels: {
      chat: { providerId: '', model: '' },
      vision: { providerId: '', model: '' },
      videoAnalysis: { providerId: '', model: '' },
      titleSummary: { providerId: '', model: '' },
      compression: { providerId: '', model: '' },
      imageGeneration: { providerId: '', model: '' },
      promptOptimize: { providerId: '', model: '' },
      advisor: { providerId: '', model: '' },
    },
    chatTools: makeChatToolsFixture(),
    screenshotTranslation: {
      enabled: true,
      hotkey: 'CommandOrControl+Shift+A',
      textHotkey: 'CommandOrControl+Shift+T',
      replaceHotkey: 'CommandOrControl+Shift+R',
      providerId: 'p1',
      model: 'gpt-4o',
    },
    screenshotAnnotate: {
      hotkey: 'CommandOrControl+Shift+S',
    },
    lens: {
      enabled: true,
      hotkey: 'CommandOrControl+Shift+G',
    },
    ...overrides,
  } as SettingsData
}

export function makeProvider(
  overrides: Omit<Partial<ModelProvider>, 'request'> & { request?: Partial<ProviderRequestConfig> } = {},
): ModelProvider {
  const request = { ...createProviderRequestDraft(), ...overrides.request }
  return {
    id: 'p1',
    name: 'OpenAI',
    baseUrl: 'https://api.openai.com/v1',
    apiKeys: ['sk-test'],
    availableModels: ['gpt-4o'],
    enabledModels: ['gpt-4o'],
    enabled: true,
    apiFormat: 'openai_chat',
    ...overrides,
    request,
  }
}
