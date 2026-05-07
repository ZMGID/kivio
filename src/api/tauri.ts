// Tauri 前端与 Rust 后端的桥接模块
// 所有 invoke 调用和事件监听都集中在这里，作为前后端的统一接口层

import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getVersion } from '@tauri-apps/api/app'
import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window'

// ========== 类型定义 ==========

// Lens 多轮对话消息类型（视觉模型）
// reasoning：推理模型（DeepSeek-R1 等）的思维链文本，仅本地展示，不回传后端
export type ExplainMessage = { role: 'user' | 'assistant'; content: string; reasoning?: string }

// Lens 流式输出负载（事件名 lens-stream）
// reasoningDelta：思维链增量（推理模型才会有）
export type LensStreamPayload = {
  imageId: string
  kind: 'answer'
  delta: string
  reasoningDelta?: string
  done?: boolean
  reason?: 'done' | 'cancelled' | 'error'
  full?: string
}

// 截图翻译流式负载（事件名 lens-translate-stream）
// kind: 'original' = OCR 阶段；'translated' = 翻译阶段
export type LensTranslateStreamPayload = {
  imageId: string
  kind?: 'original' | 'translated'
  delta?: string
  done?: boolean
  success?: boolean
  error?: string | null
}

// Lens 屏幕窗口元信息（macOS 实际数据；Windows 空数组）
export type LensWindowInfo = {
  id: number
  owner: string
  title: string
  x: number
  y: number
  width: number
  height: number
}

// AI 模型提供商配置
// apiKeys 支持多 key failover：第一个为主 key，其余为备用 key；
// 当某个 key 触发限流/配额/鉴权失败时后端会自动切下一个。
export type ModelProvider = {
  id: string
  name: string
  apiKeys: string[]
  baseUrl: string
  availableModels: string[]
  enabledModels: string[]
}

// 提供商连接测试输入（支持使用未保存的配置进行测试）
export type ProviderConnectionInput = {
  id?: string
  baseUrl: string
  apiKeys: string[]
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
    textHotkey: string
    providerId: string
    model: string
    directTranslate?: boolean
    /** 思考模式开关（默认 false）。OCR 模型 + 翻译模型都会注入对应字段 */
    thinkingEnabled?: boolean
    /** 流式输出开关（默认 true）。OCR + 翻译两步都用 SSE，token 逐字到达 */
    streamEnabled?: boolean
    /** 截图后是否保持全屏覆盖（默认 true）。false 时截图后窗口缩小为浮动 */
    keepFullscreenAfterCapture?: boolean
    /** 使用系统 OCR(macOS Apple Vision / Windows OCR) 做文字识别,然后让 provider 翻译纯文本(默认 false)。
     *  true 时 provider 可以是任意文字模型;false 时 provider 必须是多模态视觉模型。
     *  Apple Intelligence 作为 provider 时自动等同于 true(其 SDK 不支持图像)。
     *  从 vNext 起作 ocrMode 的降级镜像保留:System→true，其它→false。新代码应读 ocrMode。 */
    useSystemOcr?: boolean
    /** OCR 引擎选择(vNext+):
     *  - 'cloud_vision': 现有云端多模态 provider 一次完成 OCR+翻译
     *  - 'system': macOS Apple Vision / Windows.Media.Ocr 识别后交 provider 翻译
     *  - 'rapid_ocr': 本地 RapidOCR (PaddleOCR ONNX) 识别后交 provider 翻译。
     *    模型文件 + onnxruntime dylib 由用户在设置页面下载,安装包不带。
     *  缺省时由后端 sanitize_settings 按 useSystemOcr 自动迁移。 */
    ocrMode?: 'cloud_vision' | 'system' | 'rapid_ocr'
    prompt?: string
  }
  lens: {
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
    /** 截图后是否保持全屏覆盖（默认 true）。false 时截图后窗口缩小为浮动 */
    keepFullscreenAfterCapture?: boolean
  }
  settingsLanguage?: 'zh' | 'en'
  /** 启动时静默检查 GH Releases 是否有新版（默认 true） */
  autoCheckUpdate?: boolean
  /** 截图自动归档开关（默认 false） */
  imageArchiveEnabled?: boolean
  /** 自动归档目标目录路径 */
  imageArchivePath?: string
}

/** 更新检查结果（来自后端 GitHub Releases API 调用） */
export type UpdateInfo = {
  available: boolean
  /** 最新版本号（剥掉 v 前缀的 semver，如 "2.5.0"） */
  version?: string
  /** GitHub release tag (含 v 前缀，如 "v2.5.0") */
  tag?: string
  /** GH release 页面 URL，用于"去 GitHub 下载"按钮 */
  htmlUrl?: string
  /** Release notes / changelog (markdown) */
  body?: string
  publishedAt?: string
}

/** RapidOCR 离线 OCR 状态:检查 app data 目录里 dylib + det + rec + keys 4 个文件齐不齐 */
export type RapidOcrStatus = {
  /** 4 个必备文件全在才返回 true,有一个缺都返回 false */
  modelsAvailable: boolean
  /** app data 目录下的模型文件夹路径(用于 UI 展示) */
  modelDir?: string
}

/** RapidOCR 一键下载结果 */
export type RapidOcrInstallResult = {
  success: boolean
  /** 成功时是状态信息("RapidOCR 包下载完成"),失败时是错误片段 */
  message: string
}

// 默认提示词模板
export type DefaultPromptTemplates = {
  translationTemplate: string
  screenshotTranslationTemplate?: string
  lensPrompts: {
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

  // 读取截图（lens ready 态拉缩略图用）
  explainReadImage: (imageId: string) =>
    invoke<{ success: boolean; data?: string; error?: string }>('explain_read_image', { imageId }),

  // Lens 模式
  onLensStream: (listener: (payload: LensStreamPayload) => void) =>
    on<LensStreamPayload>('lens-stream', (payload) => listener(payload)),
  onLensTranslateStream: (listener: (payload: LensTranslateStreamPayload) => void) =>
    on<LensTranslateStreamPayload>('lens-translate-stream', (payload) => listener(payload)),
  lensRequest: () => invoke<void>('lens_request'),
  lensListWindows: () => invoke<LensWindowInfo[]>('lens_list_windows'),
  lensCaptureWindow: (windowId: number) =>
    invoke<{ success: boolean; imageId?: string; error?: string }>('lens_capture_window', { windowId }),
  lensCaptureRegion: (params: {
    absoluteX: number
    absoluteY: number
    x: number
    y: number
    width: number
    height: number
    scaleFactor: number
  }) => invoke<{ success: boolean; imageId?: string; error?: string }>('lens_capture_region', params),
  lensRegisterAnnotatedImage: (base64Png: string) =>
    invoke<{ success: boolean; imageId?: string; error?: string }>(
      'lens_register_annotated_image', { base64Png }
    ),
  lensRequestTranslate: () => invoke<void>('lens_request_translate'),
  lensRequestTranslateText: () => invoke<void>('lens_request_translate_text'),
  lensTranslate: (imageId: string) =>
    invoke<{ success: boolean; original?: string; translated?: string; error?: string }>(
      'lens_translate', { imageId }
    ),
  lensTranslateText: (text: string, requestId: string) =>
    invoke<{ success: boolean; original?: string; translated?: string; error?: string }>(
      'lens_translate_text', { text, requestId }
    ),
  lensAsk: (imageId: string, messages: ExplainMessage[]) =>
    invoke<{ success: boolean; response?: string; error?: string }>('lens_ask', { imageId, messages }),
  lensCancelStream: () => invoke<void>('lens_cancel_stream'),
  lensClose: () => invoke<void>('lens_close'),
  // 把当前活跃 image 拷贝到 lens-history 持久目录，让重启后历史能继续提问
  lensCommitImageToHistory: (imageId: string) =>
    invoke<void>('lens_commit_image_to_history', { imageId }),
  // 历史淘汰一条记录时调用，删除 lens-history 中对应 PNG 防止目录无限增长
  lensDeleteHistoryImage: (imageId: string) =>
    invoke<void>('lens_delete_history_image', { imageId }),
  lensSetFloating: (rect: { x?: number; y?: number; width: number; height: number }) =>
    invoke<void>('lens_set_floating', { rect }),

  // 取走 Rust 端在 lens_request_internal 中抓到的选中文本（take 一次清一次）
  takeLensSelection: () => invoke<string>('take_lens_selection'),

  // ========== 自动更新（仅检查 + 跳转，不做自动下载安装） ==========

  /** 调后端 GitHub Releases API 检查最新版本 */
  checkUpdate: () => invoke<UpdateInfo>('check_github_latest_release'),

  /** Apple Intelligence(端上 Foundation Models) 是否可用。仅 macOS 26+ Apple Silicon 且用户已开 Apple Intelligence 时返回 true */
  appleIntelligenceAvailable: () => invoke<boolean>('apple_intelligence_available'),

  /** 下载新版本安装包到 OS temp 目录，返回本地文件路径。下载进度通过 onUpdateDownloadProgress 派发 */
  downloadUpdate: (version: string) => invoke<string>('download_update_asset', { version }),

  /** 启动安装包并退出当前应用（macOS：cp 新 .app 到 /Applications + open；Windows：spawn NSIS installer） */
  installUpdate: (path: string) => invoke<void>('install_update_and_quit', { path }),

  /** 下载进度事件：每次百分比变化派发一次 */
  onUpdateDownloadProgress: (
    listener: (p: { percent: number; downloadedBytes: number; totalBytes: number }) => void,
  ) => on<{ percent: number; downloadedBytes: number; totalBytes: number }>(
    'update-download-progress',
    (payload) => listener(payload),
  ),

  /** 启动时若发现新版，后端 emit 此事件让 Settings UI 自动展示更新提示 */
  onUpdateAvailable: (listener: (info: UpdateInfo) => void) =>
    on<UpdateInfo>('update-available', (payload) => listener(payload)),

  // ========== RapidOCR 离线 OCR ==========

  /** 查询 RapidOCR 模型 + onnxruntime dylib 是否就绪(app data 里 4 个文件齐不齐) */
  rapidOcrStatus: () => invoke<RapidOcrStatus>('rapidocr_status'),

  /** 下载 RapidOCR 包(onnxruntime dylib + 模型 + 字典)到 app data 目录。
   *  阻塞到全部完成返回(~15-30s,共 ~30-50MB),前端转圈圈等。 */
  rapidOcrInstall: () => invoke<RapidOcrInstallResult>('rapidocr_install'),
}
