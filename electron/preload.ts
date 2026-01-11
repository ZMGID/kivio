import { ipcRenderer, contextBridge, type IpcRendererEvent } from 'electron'

type Unsubscribe = () => void

type ScreenshotResultPayload = { original: string; translated: string }
type ExplainMessage = { role: 'user' | 'assistant'; content: string }

type SettingsData = {
  hotkey: string
  theme: 'system' | 'light' | 'dark'
  targetLang: string
  source: 'bing' | 'openai' | 'custom'
  openai: {
    apiKey: string
    baseURL: string
    model: string
  }
  screenshotTranslation: {
    enabled: boolean
    hotkey: string
    ocrSource: 'system' | 'glm'
    glmApiKey: string
  }
  screenshotExplain: {
    enabled: boolean
    hotkey: string
    model: {
      provider: 'glm' | 'openai'
      apiKey: string
      baseURL: string
      modelName: string
    }
    defaultLanguage: 'zh' | 'en'
    customPrompts?: {
      systemPrompt?: string
      summaryPrompt?: string
      questionPrompt?: string
    }
  }
}

function on<TArgs extends unknown[]>(channel: string, listener: (event: IpcRendererEvent, ...args: TArgs) => void): Unsubscribe {
  const subscription = (event: IpcRendererEvent, ...args: unknown[]) => listener(event, ...(args as TArgs))
  ipcRenderer.on(channel, subscription)
  return () => ipcRenderer.removeListener(channel, subscription)
}

contextBridge.exposeInMainWorld('api', {
  getSettings: () => ipcRenderer.invoke('get-settings') as Promise<SettingsData>,
  saveSettings: (settings: Partial<SettingsData>) => ipcRenderer.invoke('save-settings', settings) as Promise<boolean>,
  getAppVersion: () => ipcRenderer.invoke('get-app-version') as Promise<string>,
  translateText: (text: string) => ipcRenderer.invoke('translate-text', text) as Promise<string>,

  resizeWindow: (width: number, height: number) => ipcRenderer.send('resize-window', width, height),
  commitTranslation: (text: string) => ipcRenderer.send('commit-translation', text),
  closeWindow: () => ipcRenderer.send('close-window'),
  hideWindow: () => ipcRenderer.send('hide-window'),
  closeScreenshotWindow: () => ipcRenderer.send('close-screenshot-window'),
  closeExplainWindow: () => ipcRenderer.send('close-explain-window'),

  openExternal: (url: string) => ipcRenderer.send('open-external', url),

  onOpenSettings: (listener: () => void) => on('open-settings', () => listener()),
  onMainProcessMessage: (listener: (message: string) => void) =>
    on<[string]>('main-process-message', (_event, message) => listener(message)),

  onScreenshotProcessing: (listener: () => void) => on('screenshot-processing', () => listener()),
  onScreenshotResult: (listener: (data: ScreenshotResultPayload) => void) =>
    on<[ScreenshotResultPayload]>('screenshot-result', (_event, data) => listener(data)),
  onScreenshotError: (listener: (errorMsg: string) => void) =>
    on<[string]>('screenshot-error', (_event, errorMsg) => listener(errorMsg)),

  explainReadImage: (imageId: string) => ipcRenderer.invoke('explain-read-image', imageId) as Promise<{ success: boolean; data?: string; error?: string }>,
  explainGetInitialSummary: (imageId: string) => ipcRenderer.invoke('explain-get-initial-summary', imageId) as Promise<{ success: boolean; summary?: string; error?: string }>,
  explainAskQuestion: (imageId: string, messages: ExplainMessage[]) =>
    ipcRenderer.invoke('explain-ask-question', imageId, messages) as Promise<{ success: boolean; response?: string; error?: string }>,
  explainGetHistory: () => ipcRenderer.invoke('explain-get-history') as Promise<{ success: boolean; history?: Array<{ id: string; timestamp: number; messages: ExplainMessage[] }>; error?: string }>,
  explainSaveHistory: (messages: ExplainMessage[]) => ipcRenderer.invoke('explain-save-history', messages) as Promise<{ success: boolean; error?: string }>,
  explainLoadHistory: (historyId: string) => ipcRenderer.invoke('explain-load-history', historyId) as Promise<{ success: boolean; record?: { id: string; timestamp: number; messages: ExplainMessage[] }; error?: string }>,
})
