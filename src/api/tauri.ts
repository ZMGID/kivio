import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getVersion } from '@tauri-apps/api/app'
import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window'

export type ExplainMessage = { role: 'user' | 'assistant'; content: string }

export type ScreenshotResultPayload = { original: string; translated: string }

export type ExplainStreamPayload = { imageId: string; kind: 'summary' | 'answer'; delta: string }

export type ModelProvider = {
  id: string
  name: string
  apiKey: string
  baseUrl: string
  availableModels: string[]
  enabledModels: string[]
}

export type Settings = {
  hotkey: string
  theme: 'system' | 'light' | 'dark'
  targetLang: string
  source: string
  autoPaste: boolean
  launchAtStartup: boolean
  translatorProviderId: string
  translatorModel: string
  translatorPrompt?: string
  providers: ModelProvider[]
  retryEnabled: boolean
  retryAttempts: number
  screenshotTranslation: {
    enabled: boolean
    hotkey: string
    providerId: string
    model: string
    directTranslate?: boolean
    prompt?: string
  }
  screenshotExplain: {
    enabled: boolean
    hotkey: string
    providerId: string
    model: string
    defaultLanguage: 'zh' | 'en'
    streamEnabled?: boolean
    customPrompts?: {
      systemPrompt?: string
      summaryPrompt?: string
      questionPrompt?: string
    }
  }
  explainHistory: Array<{
    id: string
    timestamp: number
    messages: ExplainMessage[]
  }>
  settingsLanguage?: 'zh' | 'en'
}

export type DefaultPromptTemplates = {
  translationTemplate: string
  screenshotTranslationTemplate?: string
  explainPrompts: {
    zh: { system: string; summary: string; question: string }
    en: { system: string; summary: string; question: string }
  }
}

export type PermissionStatus = {
  platform: 'macos' | 'other'
  accessibility: boolean
  screenRecording: boolean
}

export type CaptureCommitPayload = {
  absoluteX: number
  absoluteY: number
  x: number
  y: number
  width: number
  height: number
  scaleFactor: number
}

type Unlisten = () => void

async function on<T>(event: string, handler: (payload: T) => void): Promise<Unlisten> {
  const unlisten = await listen<T>(event, (event) => handler(event.payload))
  return () => {
    unlisten()
  }
}

export const api = {
  getSettings: () => invoke<Settings>('get_settings'),
  getDefaultPromptTemplates: () => invoke<DefaultPromptTemplates>('get_default_prompt_templates'),
  saveSettings: (settings: Settings) => invoke<void>('save_settings', { settings }),
  fetchModels: (providerId: string) => invoke<string[]>('fetch_models', { providerId }),
  testProviderConnection: (providerId: string) =>
    invoke<{ success: boolean; error?: string }>('test_provider_connection', { providerId }),
  getPermissionStatus: () => invoke<PermissionStatus>('get_permission_status'),
  openPermissionSettings: (kind: 'accessibility' | 'screen-recording') =>
    invoke<void>('open_permission_settings', { kind }),
  captureRequest: (mode: 'translate' | 'explain') =>
    invoke<void>('capture_request', { mode }),
  captureCommit: (payload: CaptureCommitPayload) =>
    invoke<void>('capture_commit', payload),
  captureCancel: () => invoke<void>('capture_cancel'),
  getAppVersion: () => getVersion(),
  translateText: (text: string) => invoke<string>('translate_text', { text }),
  commitTranslation: (text: string) => invoke<void>('commit_translation', { text }),
  openExternal: (url: string) => invoke<void>('open_external', { url }),

  resizeWindow: async (width: number, height: number) => {
    const win = getCurrentWindow()
    await win.setSize(new LogicalSize(width, height))
  },
  hideWindow: async () => {
    const win = getCurrentWindow()
    await win.hide()
  },
  closeWindow: async () => {
    const win = getCurrentWindow()
    await win.hide()
  },
  closeScreenshotWindow: async () => {
    const win = getCurrentWindow()
    await win.hide()
  },
  closeExplainWindow: async () => {
    const win = getCurrentWindow()
    await win.hide()
  },
  startDragging: async () => {
    const win = getCurrentWindow()
    await win.startDragging()
  },

  onOpenSettings: (listener: () => void) => on('open-settings', () => listener()),
  onScreenshotProcessing: (listener: () => void) => on('screenshot-processing', () => listener()),
  onScreenshotResult: (listener: (data: ScreenshotResultPayload) => void) =>
    on<ScreenshotResultPayload>('screenshot-result', (data) => listener(data)),
  onScreenshotError: (listener: (errorMsg: string) => void) => on<string>('screenshot-error', (msg) => listener(msg)),
  onExplainStream: (listener: (payload: ExplainStreamPayload) => void) =>
    on<ExplainStreamPayload>('explain-stream', (payload) => listener(payload)),

  explainReadImage: (imageId: string) =>
    invoke<{ success: boolean; data?: string; error?: string }>('explain_read_image', { imageId }),
  explainGetInitialSummary: (imageId: string) =>
    invoke<{ success: boolean; summary?: string; error?: string }>('explain_get_initial_summary', { imageId }),
  explainAskQuestion: (imageId: string, messages: ExplainMessage[]) =>
    invoke<{ success: boolean; response?: string; error?: string }>('explain_ask_question', { imageId, messages }),
  explainGetHistory: () =>
    invoke<{ success: boolean; history?: Array<{ id: string; timestamp: number; messages: ExplainMessage[] }>; error?: string }>(
      'explain_get_history',
    ),
  explainSaveHistory: (messages: ExplainMessage[]) =>
    invoke<{ success: boolean; error?: string }>('explain_save_history', { messages }),
  explainLoadHistory: (historyId: string) =>
    invoke<{ success: boolean; record?: { id: string; timestamp: number; messages: ExplainMessage[] }; error?: string }>(
      'explain_load_history',
      { historyId },
    ),
  explainCloseCurrent: () => invoke<void>('explain_close_current'),
}
