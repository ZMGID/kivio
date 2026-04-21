// Tauri 前端与 Rust 后端的桥接模块
// 所有 invoke 调用和事件监听都集中在这里，作为前后端的统一接口层

import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getVersion } from '@tauri-apps/api/app'
import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window'

// ========== 类型定义 ==========

// 截图解释对话消息类型
export type ExplainMessage = { role: 'user' | 'assistant'; content: string }

// 截图翻译结果负载
export type ScreenshotResultPayload = { original: string; translated: string }

// 截图解释流式输出负载
export type ExplainStreamPayload = { imageId: string; kind: 'summary' | 'answer'; delta: string }

// AI 模型提供商配置
export type ModelProvider = {
  id: string
  name: string
  apiKey: string
  baseUrl: string
  availableModels: string[]
  enabledModels: string[]
}

// 提供商连接测试输入（支持使用未保存的配置进行测试）
export type ProviderConnectionInput = {
  id?: string
  baseUrl: string
  apiKey: string
}

// 应用设置数据结构
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

// 默认提示词模板
export type DefaultPromptTemplates = {
  translationTemplate: string
  screenshotTranslationTemplate?: string
  explainPrompts: {
    zh: { system: string; summary: string; question: string }
    en: { system: string; summary: string; question: string }
  }
}

// macOS 权限状态
export type PermissionStatus = {
  platform: 'macos' | 'other'
  accessibility: boolean
  screenRecording: boolean
}

// 截图区域提交数据
export type CaptureCommitPayload = {
  absoluteX: number
  absoluteY: number
  x: number
  y: number
  width: number
  height: number
  scaleFactor: number
}

// 事件取消监听函数类型
type Unlisten = () => void

/**
 * 通用的 Tauri 事件监听包装器
 * @param event 事件名称
 * @param handler 事件处理函数
 * @returns 取消监听的函数
 */
async function on<T>(event: string, handler: (payload: T) => void): Promise<Unlisten> {
  const unlisten = await listen<T>(event, (event) => handler(event.payload))
  return () => {
    unlisten()
  }
}

// ========== API 导出 ==========

export const api = {
  // 设置相关
  getSettings: () => invoke<Settings>('get_settings'),
  getDefaultPromptTemplates: () => invoke<DefaultPromptTemplates>('get_default_prompt_templates'),
  saveSettings: (settings: Settings) => invoke<void>('save_settings', { settings }),

  // 提供商相关
  fetchModels: (providerId: string, provider?: ProviderConnectionInput) =>
    invoke<string[]>('fetch_models', { providerId, provider }),
  testProviderConnection: (providerId: string, provider?: ProviderConnectionInput) =>
    invoke<{ success: boolean; error?: string }>('test_provider_connection', { providerId, provider }),

  // 权限相关（macOS）
  getPermissionStatus: () => invoke<PermissionStatus>('get_permission_status'),
  openPermissionSettings: (kind: 'accessibility' | 'screen-recording') =>
    invoke<void>('open_permission_settings', { kind }),

  // 截图捕获相关
  captureRequest: (mode: 'translate' | 'explain') =>
    invoke<void>('capture_request', { mode }),
  captureCommit: (payload: CaptureCommitPayload) =>
    invoke<void>('capture_commit', payload),
  captureCancel: () => invoke<void>('capture_cancel'),

  // 应用信息
  getAppVersion: () => getVersion(),

  // 文本翻译
  translateText: (text: string) => invoke<string>('translate_text', { text }),
  commitTranslation: (text: string) => invoke<void>('commit_translation', { text }),

  // 外部链接
  openExternal: (url: string) => invoke<void>('open_external', { url }),

  // 窗口控制
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
  setAlwaysOnTop: async (alwaysOnTop: boolean) => {
    const win = getCurrentWindow()
    await win.setAlwaysOnTop(alwaysOnTop)
  },

  // 事件监听
  onOpenSettings: (listener: () => void) => on('open-settings', () => listener()),
  onScreenshotProcessing: (listener: () => void) => on('screenshot-processing', () => listener()),
  onScreenshotResult: (listener: (data: ScreenshotResultPayload) => void) =>
    on<ScreenshotResultPayload>('screenshot-result', (data) => listener(data)),
  onScreenshotError: (listener: (errorMsg: string) => void) => on<string>('screenshot-error', (msg) => listener(msg)),
  onExplainStream: (listener: (payload: ExplainStreamPayload) => void) =>
    on<ExplainStreamPayload>('explain-stream', (payload) => listener(payload)),

  // 截图解释相关
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
