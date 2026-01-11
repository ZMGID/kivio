/// <reference types="vite-plugin-electron/electron-env" />

declare namespace NodeJS {
  interface ProcessEnv {
    /**
     * The built directory structure
     *
     * ```tree
     * ├─┬─┬ dist
     * │ │ └── index.html
     * │ │
     * │ ├─┬ dist-electron
     * │ │ ├── main.js
     * │ │ └── preload.js
     * │
     * ```
     */
    APP_ROOT: string
    /** /dist/ or /public/ */
    VITE_PUBLIC: string
  }
}

// Used in Renderer process, expose in `preload.ts`
interface Window {
  api: {
    getSettings: () => Promise<{
      hotkey: string
      theme: 'system' | 'light' | 'dark'
      targetLang: string
      source: 'bing' | 'openai' | 'custom'
      openai: { apiKey: string; baseURL: string; model: string }
      screenshotTranslation: { enabled: boolean; hotkey: string; ocrSource: 'system' | 'glm'; glmApiKey: string }
      screenshotExplain: {
        enabled: boolean
        hotkey: string
        model: { provider: 'glm' | 'openai'; apiKey: string; baseURL: string; modelName: string }
        defaultLanguage: 'zh' | 'en'
        customPrompts?: { systemPrompt?: string; summaryPrompt?: string; questionPrompt?: string }
      }
      explainHistory?: Array<{ id: string; timestamp: number; messages: Array<{ role: string; content: string }> }>
    }>

    saveSettings: (settings: unknown) => Promise<boolean>
    getAppVersion: () => Promise<string>
    translateText: (text: string) => Promise<string>

    resizeWindow: (width: number, height: number) => void
    commitTranslation: (text: string) => void
    closeWindow: () => void
    hideWindow: () => void
    closeScreenshotWindow: () => void
    closeExplainWindow: () => void

    openExternal: (url: string) => void

    onOpenSettings: (listener: () => void) => () => void
    onMainProcessMessage: (listener: (message: string) => void) => () => void

    onScreenshotProcessing: (listener: () => void) => () => void
    onScreenshotResult: (listener: (data: { original: string; translated: string }) => void) => () => void
    onScreenshotError: (listener: (errorMsg: string) => void) => () => void

    explainReadImage: (imageId: string) => Promise<{ success: boolean; data?: string; error?: string }>
    explainGetInitialSummary: (imageId: string) => Promise<{ success: boolean; summary?: string; error?: string }>
    explainAskQuestion: (imageId: string, messages: Array<{ role: 'user' | 'assistant'; content: string }>) => Promise<{ success: boolean; response?: string; error?: string }>
    explainGetHistory: () => Promise<{ success: boolean; history?: Array<{ id: string; timestamp: number; messages: Array<{ role: 'user' | 'assistant'; content: string }> }>; error?: string }>
    explainSaveHistory: (messages: Array<{ role: 'user' | 'assistant'; content: string }>) => Promise<{ success: boolean; error?: string }>
    explainLoadHistory: (historyId: string) => Promise<{ success: boolean; record?: { id: string; timestamp: number; messages: Array<{ role: 'user' | 'assistant'; content: string }> }; error?: string }>
  }
}
