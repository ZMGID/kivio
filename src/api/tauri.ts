// Tauri 前端与 Rust 后端的桥接模块
// 所有 invoke 调用和事件监听都集中在这里，作为前后端的统一接口层

import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getVersion } from '@tauri-apps/api/app'
import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window'

// ========== 类型定义 ==========

// Cowork 多轮对话消息类型（视觉模型）
// reasoning：推理模型（DeepSeek-R1 等）的思维链文本，仅本地展示，不回传后端
export type ExplainMessage = { role: 'user' | 'assistant'; content: string; reasoning?: string }

// Cowork 流式输出负载（事件名 cowork-stream）
// reasoningDelta：思维链增量（推理模型才会有）
export type CoworkStreamPayload = {
  imageId: string
  kind: 'answer'
  delta: string
  reasoningDelta?: string
  done?: boolean
  reason?: 'done' | 'cancelled' | 'error'
  full?: string
}

// 截图翻译流式负载（事件名 cowork-translate-stream）
// kind: 'original' = OCR 阶段；'translated' = 翻译阶段
export type CoworkTranslateStreamPayload = {
  imageId: string
  kind?: 'original' | 'translated'
  delta?: string
  done?: boolean
  success?: boolean
  error?: string | null
}

// Cowork 屏幕窗口元信息（macOS 实际数据；Windows 空数组）
export type CoworkWindowInfo = {
  id: number
  owner: string
  title: string
  x: number
  y: number
  width: number
  height: number
}

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
    /** 思考模式开关（默认 false）。OCR 模型 + 翻译模型都会注入对应字段 */
    thinkingEnabled?: boolean
    /** 流式输出开关（默认 true）。OCR + 翻译两步都用 SSE，token 逐字到达 */
    streamEnabled?: boolean
    prompt?: string
  }
  cowork: {
    enabled: boolean
    hotkey: string
    providerId?: string
    model?: string
    defaultLanguage?: string
    streamEnabled?: boolean
    /** 思考模式开关（默认 true）。false 时 body 注入各厂商关闭思考的字段并集 */
    thinkingEnabled?: boolean
    systemPrompt?: string
    questionPrompt?: string
    /** 消息排序：'asc' 老到新（默认），'desc' 新到老 */
    messageOrder?: 'asc' | 'desc'
  }
  settingsLanguage?: 'zh' | 'en'
}

// 默认提示词模板
export type DefaultPromptTemplates = {
  translationTemplate: string
  screenshotTranslationTemplate?: string
  coworkPrompts: {
    zh: { system: string; question: string }
    en: { system: string; question: string }
  }
}

// macOS 权限状态
export type PermissionStatus = {
  platform: 'macos' | 'other'
  accessibility: boolean
  screenRecording: boolean
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
  showWindow: async () => {
    const win = getCurrentWindow()
    await win.show()
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

  // 读取截图（cowork ready 态拉缩略图用）
  explainReadImage: (imageId: string) =>
    invoke<{ success: boolean; data?: string; error?: string }>('explain_read_image', { imageId }),

  // Cowork 模式
  onCoworkStream: (listener: (payload: CoworkStreamPayload) => void) =>
    on<CoworkStreamPayload>('cowork-stream', (payload) => listener(payload)),
  onCoworkTranslateStream: (listener: (payload: CoworkTranslateStreamPayload) => void) =>
    on<CoworkTranslateStreamPayload>('cowork-translate-stream', (payload) => listener(payload)),
  coworkRequest: () => invoke<void>('cowork_request'),
  coworkListWindows: () => invoke<CoworkWindowInfo[]>('cowork_list_windows'),
  coworkCaptureWindow: (windowId: number) =>
    invoke<{ success: boolean; imageId?: string; error?: string }>('cowork_capture_window', { windowId }),
  coworkCaptureRegion: (params: {
    absoluteX: number
    absoluteY: number
    x: number
    y: number
    width: number
    height: number
    scaleFactor: number
  }) => invoke<{ success: boolean; imageId?: string; error?: string }>('cowork_capture_region', params),
  coworkRequestTranslate: () => invoke<void>('cowork_request_translate'),
  coworkTranslate: (imageId: string) =>
    invoke<{ success: boolean; original?: string; translated?: string; error?: string }>(
      'cowork_translate', { imageId }
    ),
  coworkAsk: (imageId: string, messages: ExplainMessage[]) =>
    invoke<{ success: boolean; response?: string; error?: string }>('cowork_ask', { imageId, messages }),
  coworkCancelStream: () => invoke<void>('cowork_cancel_stream'),
  coworkClose: () => invoke<void>('cowork_close'),
}
