import { app, BrowserWindow, globalShortcut, screen, ipcMain, clipboard, shell, Tray, Menu, nativeImage } from 'electron'
import { fileURLToPath, pathToFileURL } from 'node:url'
import path from 'node:path'
import { exec, spawn } from 'node:child_process'
import { translate as bingTranslate } from 'bing-translate-api'
import Store from 'electron-store'
import OpenAI from 'openai'
import fs from 'node:fs'
import { randomUUID } from 'node:crypto'

const __dirname = path.dirname(fileURLToPath(import.meta.url))
process.env.APP_ROOT = path.join(__dirname, '..')

export const VITE_DEV_SERVER_URL = process.env['VITE_DEV_SERVER_URL']
export const MAIN_DIST = path.join(process.env.APP_ROOT, 'dist-electron')
export const RENDERER_DIST = path.join(process.env.APP_ROOT, 'dist')
process.env.VITE_PUBLIC = VITE_DEV_SERVER_URL ? path.join(process.env.APP_ROOT, 'public') : RENDERER_DIST
const ALLOWED_FILE_BASE = pathToFileURL(path.join(RENDERER_DIST, path.sep)).toString()

// Define Store Schema
type StoreType = {
  hotkey: string;
  theme: 'system' | 'light' | 'dark';
  targetLang: string; // 'auto' | 'zh' | 'en' ...
  source: 'bing' | 'openai' | 'custom';
  openai: {
    apiKey: string;
    baseURL: string;
    model: string;
  };
  screenshotTranslation: {
    enabled: boolean;
    hotkey: string;
    ocrSource: 'system' | 'glm' | 'openai';
    glmApiKey: string;
    openai?: {
      apiKey: string;
      baseURL: string;
      model: string;
    };
  };
  screenshotExplain: {
    enabled: boolean;
    hotkey: string;
    model: {
      provider: 'glm' | 'openai';
      apiKey: string;
      baseURL: string;
      modelName: string;
    };
    defaultLanguage: 'zh' | 'en';
    customPrompts?: {
      systemPrompt?: string;
      summaryPrompt?: string;
      questionPrompt?: string;
    };
  };
  explainHistory: Array<{
    id: string;
    timestamp: number;
    messages: Array<{ role: string; content: string }>;
  }>;
}

const store = new Store<StoreType>({
  defaults: {
    hotkey: 'Command+Option+T',
    theme: 'system',
    targetLang: 'auto',
    source: 'bing',
    openai: {
      apiKey: '',
      baseURL: 'https://api.deepseek.com/v1',
      model: 'deepseek-chat'
    },
    screenshotTranslation: {
      enabled: true,
      hotkey: 'Command+Shift+A',
      ocrSource: 'system',
      glmApiKey: '',
      openai: {
        apiKey: '',
        baseURL: 'https://api.openai.com/v1',
        model: 'gpt-4o'
      }
    },
    screenshotExplain: {
      enabled: true,
      hotkey: 'Command+Shift+E',
      model: {
        provider: 'glm',
        apiKey: '',
        baseURL: 'https://open.bigmodel.cn/api/paas/v4',
        modelName: 'glm-4v-flash'
      },
      defaultLanguage: 'zh'
    },
    explainHistory: []
  }
});

let win: BrowserWindow | null
let screenshotWin: BrowserWindow | null = null
let explainWin: BrowserWindow | null = null
let tray: Tray | null = null
let currentExplainImageId: string | null = null
const explainImages = new Map<string, { path: string; createdAt: number }>()

function isAllowedIpcSender(senderUrl: string): boolean {
  if (VITE_DEV_SERVER_URL) return senderUrl.startsWith(VITE_DEV_SERVER_URL)
  return senderUrl.startsWith(ALLOWED_FILE_BASE)
}

function sanitizeExternalUrl(rawUrl: unknown): string | null {
  if (typeof rawUrl !== 'string') return null
  let url: URL
  try {
    url = new URL(rawUrl)
  } catch {
    return null
  }
  if (url.protocol !== 'https:') return null
  return url.toString()
}

function hardenWindow(window: BrowserWindow) {
  window.webContents.setWindowOpenHandler(() => ({ action: 'deny' }))
  window.webContents.on('will-navigate', (event, url) => {
    if (!isAllowedIpcSender(url)) event.preventDefault()
  })
}

function sanitizeSettings(input: unknown): Partial<StoreType> {
  const defaults = store.store
  if (!input || typeof input !== 'object') return {}

  const raw = input as Record<string, unknown>
  const result: Partial<StoreType> = {}

  if (typeof raw.hotkey === 'string' && raw.hotkey.trim()) result.hotkey = raw.hotkey.trim()

  if (raw.theme === 'system' || raw.theme === 'light' || raw.theme === 'dark') result.theme = raw.theme

  if (typeof raw.targetLang === 'string' && raw.targetLang.trim()) result.targetLang = raw.targetLang.trim()

  if (raw.source === 'bing' || raw.source === 'openai' || raw.source === 'custom') result.source = raw.source

  if (raw.openai && typeof raw.openai === 'object') {
    const o = raw.openai as Record<string, unknown>
    result.openai = {
      apiKey: typeof o.apiKey === 'string' ? o.apiKey : defaults.openai.apiKey,
      baseURL: typeof o.baseURL === 'string' ? o.baseURL : defaults.openai.baseURL,
      model: typeof o.model === 'string' ? o.model : defaults.openai.model,
    }
  }

  if (raw.screenshotTranslation && typeof raw.screenshotTranslation === 'object') {
    const st = raw.screenshotTranslation as Record<string, unknown>
    const oa = st.openai && typeof st.openai === 'object' ? (st.openai as Record<string, unknown>) : {}

    result.screenshotTranslation = {
      enabled: typeof st.enabled === 'boolean' ? st.enabled : defaults.screenshotTranslation.enabled,
      hotkey: typeof st.hotkey === 'string' && st.hotkey.trim() ? st.hotkey.trim() : defaults.screenshotTranslation.hotkey,
      ocrSource: (st.ocrSource === 'system' || st.ocrSource === 'glm' || st.ocrSource === 'openai') ? st.ocrSource : defaults.screenshotTranslation.ocrSource,
      glmApiKey: typeof st.glmApiKey === 'string' ? st.glmApiKey : defaults.screenshotTranslation.glmApiKey,
      openai: {
        apiKey: typeof oa.apiKey === 'string' ? oa.apiKey : (defaults.screenshotTranslation.openai?.apiKey || ''),
        baseURL: typeof oa.baseURL === 'string' ? oa.baseURL : (defaults.screenshotTranslation.openai?.baseURL || 'https://api.openai.com/v1'),
        model: typeof oa.model === 'string' ? oa.model : (defaults.screenshotTranslation.openai?.model || 'gpt-4o'),
      }
    }
  }

  if (raw.screenshotExplain && typeof raw.screenshotExplain === 'object') {
    const se = raw.screenshotExplain as Record<string, unknown>
    const modelRaw = se.model && typeof se.model === 'object' ? (se.model as Record<string, unknown>) : {}
    const promptsRaw = se.customPrompts && typeof se.customPrompts === 'object' ? (se.customPrompts as Record<string, unknown>) : undefined

    result.screenshotExplain = {
      enabled: typeof se.enabled === 'boolean' ? se.enabled : defaults.screenshotExplain.enabled,
      hotkey: typeof se.hotkey === 'string' && se.hotkey.trim() ? se.hotkey.trim() : defaults.screenshotExplain.hotkey,
      defaultLanguage: se.defaultLanguage === 'zh' || se.defaultLanguage === 'en' ? se.defaultLanguage : defaults.screenshotExplain.defaultLanguage,
      model: {
        provider: modelRaw.provider === 'glm' || modelRaw.provider === 'openai' ? modelRaw.provider : defaults.screenshotExplain.model.provider,
        apiKey: typeof modelRaw.apiKey === 'string' ? modelRaw.apiKey : defaults.screenshotExplain.model.apiKey,
        baseURL: typeof modelRaw.baseURL === 'string' ? modelRaw.baseURL : defaults.screenshotExplain.model.baseURL,
        modelName: typeof modelRaw.modelName === 'string' ? modelRaw.modelName : defaults.screenshotExplain.model.modelName,
      },
      ...(promptsRaw
        ? {
          customPrompts: {
            systemPrompt: typeof promptsRaw.systemPrompt === 'string' ? promptsRaw.systemPrompt : undefined,
            summaryPrompt: typeof promptsRaw.summaryPrompt === 'string' ? promptsRaw.summaryPrompt : undefined,
            questionPrompt: typeof promptsRaw.questionPrompt === 'string' ? promptsRaw.questionPrompt : undefined,
          },
        }
        : {}),
    }
  }

  return result
}

function resolveExplainImagePath(imageId: string): string {
  const record = explainImages.get(imageId)
  if (!record) throw new Error('Image not found')

  const tempDir = app.getPath('temp')
  const normalized = path.normalize(record.path)
  const normalizedTemp = path.normalize(tempDir + path.sep)
  if (!normalized.startsWith(normalizedTemp)) throw new Error('Invalid image path')
  if (!fs.existsSync(normalized)) throw new Error('Image missing on disk')
  return normalized
}

function cleanupExplainImage(imageId: string | null) {
  if (!imageId) return
  const record = explainImages.get(imageId)
  if (record) {
    try {
      fs.unlinkSync(record.path)
    } catch {
      // Ignore cleanup errors
    }
    explainImages.delete(imageId)
  }
  if (currentExplainImageId === imageId) currentExplainImageId = null
}

function getOcrHelperPath(): string | null {
  const helperName = 'keylingo-ocr'

  const resourcesPath = (process as unknown as { resourcesPath?: string }).resourcesPath
  if (resourcesPath) {
    const packagedPath = path.join(resourcesPath, 'ocr', helperName)
    if (fs.existsSync(packagedPath)) return packagedPath
  }

  const devPath = path.join(process.env.APP_ROOT || process.cwd(), 'resources', 'ocr', helperName)
  if (fs.existsSync(devPath)) return devPath

  return null
}

function createWindow() {
  const iconPath = path.join(process.env.VITE_PUBLIC, 'icon.png');
  // fallback if png not found, use svg
  const icon = nativeImage.createFromPath(iconPath);

  try {
    if (app.dock) app.dock.hide(); // Hide from Dock for "Menu Bar App" feel
  } catch {
    // Ignore platforms without dock support
  }

  win = new BrowserWindow({
    width: 360,
    height: 120, // Initial compact size
    icon: icon,
    frame: false,
    transparent: true,
    alwaysOnTop: true,
    skipTaskbar: true,
    resizable: false,
    show: false,
    hasShadow: true,
    webPreferences: {
      preload: path.join(__dirname, 'preload.mjs'),
      nodeIntegration: false,
      contextIsolation: true,
    },
  })

  hardenWindow(win)
  win.setVisibleOnAllWorkspaces(true, { visibleOnFullScreen: true });

  if (VITE_DEV_SERVER_URL) {
    win.loadURL(VITE_DEV_SERVER_URL)
  } else {
    win.loadFile(path.join(RENDERER_DIST, 'index.html'))
  }
}

function registerHotkey() {
  globalShortcut.unregisterAll()
  const hotkey = store.get('hotkey');

  try {
    const ret = globalShortcut.register(hotkey, () => {
      console.log('Global Shortcut Triggered')
      if (!win) return

      if (win.isVisible() && !win.isFocused()) {
        win.focus();
        return;
      }

      if (win.isVisible()) {
        win.hide()
        app.hide()
      } else {
        const point = screen.getCursorScreenPoint()
        const display = screen.getDisplayNearestPoint(point)

        const x = Math.min(Math.max(point.x, display.bounds.x), display.bounds.x + display.bounds.width - 360)
        const y = Math.min(Math.max(point.y + 20, display.bounds.y), display.bounds.y + display.bounds.height - 120) // Use 120 or current height?

        win.setPosition(x, y)
        win.show()
        win.focus()
      }
    });

    if (!ret) {
      console.error('Registration failed for:', hotkey)
    } else {
      console.log('Registered hotkey:', hotkey)
    }

  } catch (e) {
    console.error('Invalid hotkey:', hotkey)
  }
}

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') {
    app.quit()
    win = null
  }
})

app.on('activate', () => {
  if (BrowserWindow.getAllWindows().length === 0) {
    createWindow()
  }
})

app.whenReady().then(() => {
  createWindow()
  registerHotkey()
  registerScreenshotHotkey()
  registerExplainHotkey()

  // Tray Setup
  const iconPath = path.join(process.env.VITE_PUBLIC, 'icon.png');
  const trayIcon = nativeImage.createFromPath(iconPath).resize({ width: 16, height: 16 });

  tray = new Tray(trayIcon)
  tray.setToolTip('Translation Utility')

  const contextMenu = Menu.buildFromTemplate([
    {
      label: 'Show Translator',
      click: () => {
        if (!win) return;
        win.show();
        win.focus();
      }
    },
    {
      label: 'Settings',
      click: () => {
        if (!win) return;
        win.show();
        win.focus();
        // Tell renderer to open settings
        win.webContents.send('open-settings');
      }
    },
    { type: 'separator' },
    {
      label: 'Quit', click: () => {
        app.quit();
      }
    }
  ])

  tray.setContextMenu(contextMenu)
})

app.on('will-quit', () => {
  globalShortcut.unregisterAll()
})

// --- IPC Handlers ---

ipcMain.on('close-window', () => {
  // If we are in settings mode (height is large), maybe just shrink?
  // Simpler: Just hide everything.
  win?.hide()
  app.hide()
})

ipcMain.on('close-screenshot-window', () => {
  screenshotWin?.close()
})


ipcMain.on('resize-window', (_event, width, height) => {
  win?.setSize(width, height);
})

ipcMain.on('commit-translation', (_event, text) => {
  clipboard.writeText(text)
  win?.hide()
  app.hide()

  setTimeout(() => {
    exec(`osascript -e 'tell application "System Events" to keystroke "v" using command down'`, (error) => {
      if (error) console.error('Paste failed:', error)
    })
  }, 150)
})

// Settings IPC
ipcMain.handle('get-settings', () => {
  return store.store;
})

ipcMain.handle('save-settings', (event, newSettings) => {
  if (!isAllowedIpcSender(event.senderFrame.url)) return false
  store.set(sanitizeSettings(newSettings));

  // Unregister all hotkeys first
  globalShortcut.unregisterAll();

  // Re-register all hotkeys with new settings
  registerHotkey();
  registerScreenshotHotkey();
  registerExplainHotkey();

  return true;
})

ipcMain.on('hide-window', () => {
  win?.hide();
});

ipcMain.on('open-external', (event, url) => {
  if (!isAllowedIpcSender(event.senderFrame.url)) return
  const safeUrl = sanitizeExternalUrl(url)
  if (!safeUrl) return
  shell.openExternal(safeUrl);
})

// Translation Logic
ipcMain.handle('get-app-version', () => {
  return app.getVersion();
})

ipcMain.handle('translate-text', async (event, text) => {
  if (!isAllowedIpcSender(event.senderFrame.url)) return "Blocked"
  const source = store.get('source');
  const targetLangPref = store.get('targetLang');
  const trimmed = text.trim();
  if (!trimmed) return "";

  // Determine Target Language
  let targetLang = 'en';
  const hasChinese = /[\u4e00-\u9fa5]/.test(trimmed);

  if (targetLangPref === 'auto') {
    targetLang = hasChinese ? 'en' : 'zh-Hans';
  } else {
    targetLang = targetLangPref;
    // Minor fix for bing language codes (zh-Hans vs zh)
    if (targetLang === 'zh') targetLang = 'zh-Hans';
  }

  // 1. Bing
  if (source === 'bing') {
    try {
      const res = await bingTranslate(trimmed, null, targetLang);
      return res?.translation || "Bing Error";
    } catch (e) {
      console.error("Bing Error:", e);
      return "Bing Fail";
    }
  }

  // 2. OpenAI / DeepSeek / Zhipu
  if (source === 'openai') {
    const config = store.get('openai');
    if (!config.apiKey) return "Missing API Key";

    try {
      const openai = new OpenAI({
        apiKey: config.apiKey,
        baseURL: config.baseURL,
        dangerouslyAllowBrowser: false
      });

      // Prompt Engineering
      let langName = targetLang;
      if (targetLang === 'zh-Hans') langName = 'Simplified Chinese';
      if (targetLang === 'en') langName = 'English';
      if (targetLang === 'ja') langName = 'Japanese';
      if (targetLang === 'ko') langName = 'Korean';

      const prompt = `Translate the following text to ${langName}. Only output the translation, no explanation. Text: "${trimmed}"`;

      const completion = await openai.chat.completions.create({
        messages: [{ role: "user", content: prompt }],
        model: config.model || "gpt-3.5-turbo",
      });

      return completion.choices[0]?.message?.content?.trim() || "AI Error";
    } catch (e: unknown) {
      console.error("AI API Error:", e);
      const message = e instanceof Error ? e.message : String(e)
      return `API Error: ${message}`;
    }
  }

  return "Unknown Source";
})

// ========== Screenshot Translation ==========

function createScreenshotWindow() {
  if (screenshotWin) return;

  const iconPath = path.join(process.env.VITE_PUBLIC, 'icon.png');
  const icon = nativeImage.createFromPath(iconPath);

  screenshotWin = new BrowserWindow({
    width: 500,
    height: 400,
    icon: icon,
    frame: false,
    transparent: false,
    alwaysOnTop: true,
    skipTaskbar: false,
    resizable: true,
    show: false,
    webPreferences: {
      preload: path.join(__dirname, 'preload.mjs'),
      nodeIntegration: false,
      contextIsolation: true,
    },
  });

  hardenWindow(screenshotWin)
  screenshotWin.setVisibleOnAllWorkspaces(true, { visibleOnFullScreen: true });

  if (VITE_DEV_SERVER_URL) {
    screenshotWin.loadURL(VITE_DEV_SERVER_URL + '?mode=screenshot')
  } else {
    screenshotWin.loadFile(path.join(RENDERER_DIST, 'index.html'), { hash: 'screenshot' })
  }

  screenshotWin.on('close', () => {
    screenshotWin = null;
  });
}

async function callGLM4V(imagePath: string, apiKey: string): Promise<string> {
  try {
    const imageBuffer = await fs.promises.readFile(imagePath);
    const base64Image = imageBuffer.toString('base64');

    const response = await fetch('https://open.bigmodel.cn/api/paas/v4/chat/completions', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Authorization': `Bearer ${apiKey}`
      },
      body: JSON.stringify({
        model: 'glm-4v-flash',
        messages: [{
          role: 'user',
          content: [
            {
              type: 'text',
              text: '请识别图片中的所有文字内容，按行返回。只输出文字，不要其他解释。'
            },
            {
              type: 'image_url',
              image_url: {
                url: `data:image/png;base64,${base64Image}`
              }
            }
          ]
        }]
      })
    });

    const data = await response.json();

    if (!response.ok) {
      throw new Error(data.error?.message || 'GLM API Error');
    }

    return data.choices[0]?.message?.content || '';
  } catch (error: unknown) {
    console.error('GLM-4V Error:', error);
    throw error;
  }
}

async function callSystemOCR(imagePath: string): Promise<string> {
  const helperPath = getOcrHelperPath()
  if (!helperPath) {
    throw new Error('系统 OCR 不可用：缺少 OCR helper。请在开发环境运行 `npm run build:ocr`，或在发布版中重新打包包含 helper。')
  }

  return await new Promise((resolve, reject) => {
    const child = spawn(helperPath, [imagePath], { stdio: ['ignore', 'pipe', 'pipe'] })

    const stdoutChunks: Buffer[] = []
    const stderrChunks: Buffer[] = []

    const timeout = setTimeout(() => {
      child.kill('SIGKILL')
      reject(new Error('系统OCR识别超时'))
    }, 15_000)

    child.stdout.on('data', (chunk: Buffer) => stdoutChunks.push(chunk))
    child.stderr.on('data', (chunk: Buffer) => stderrChunks.push(chunk))

    child.on('error', (err) => {
      clearTimeout(timeout)
      reject(err)
    })

    child.on('close', (code) => {
      clearTimeout(timeout)

      const stdout = Buffer.concat(stdoutChunks).toString('utf8').trim()
      const stderr = Buffer.concat(stderrChunks).toString('utf8').trim()

      if (code !== 0) {
        reject(new Error('系统OCR识别失败: ' + (stderr || `exit code ${code}`)))
        return
      }

      resolve(stdout || '未识别到文字')
    })
  })
}

async function callOpenAIOCR(imagePath: string, config: { apiKey: string, baseURL: string, model: string }): Promise<string> {
  try {
    const imageBuffer = await fs.promises.readFile(imagePath);
    const base64Image = imageBuffer.toString('base64');

    const openai = new OpenAI({
      apiKey: config.apiKey,
      baseURL: config.baseURL,
    });

    const response = await openai.chat.completions.create({
      model: config.model,
      messages: [
        {
          role: "user",
          content: [
            { type: "text", text: "Read all text in this image. Output only the text content, preserving original lines." },
            {
              type: "image_url",
              image_url: {
                url: `data:image/png;base64,${base64Image}`,
              },
            },
          ],
        },
      ],
    });

    return response.choices[0]?.message?.content || '';
  } catch (error: unknown) {
    console.error('OpenAI OCR Error:', error);
    throw error;
  }
}

async function translateTextHelper(text: string): Promise<string> {
  const trimmed = text.trim();
  if (!trimmed) return "";

  const source = store.get('source');
  const targetLang = store.get('targetLang');

  if (source === 'bing' || !source) {
    try {
      const hasChinese = /[\u4e00-\u9fa5]/.test(trimmed);
      const lang = targetLang === 'auto' ? (hasChinese ? 'en' : 'zh-Hans') : targetLang;

      const res = await bingTranslate(trimmed, null, lang);
      return res?.translation || trimmed;
    } catch (e) {
      console.error("Bing Error:", e);
      return trimmed;
    }
  }

  if (source === 'openai') {
    const config = store.get('openai');
    if (!config.apiKey) return trimmed;

    try {
      const openai = new OpenAI({
        apiKey: config.apiKey,
        baseURL: config.baseURL,
      });

      const prompt = `Translate the following text. Only output the translation:\n\n${trimmed}`;

      const completion = await openai.chat.completions.create({
        messages: [{ role: "user", content: prompt }],
        model: config.model || "deepseek-chat",
      });

      return completion.choices[0]?.message?.content?.trim() || trimmed;
    } catch (e: unknown) {
      console.error("AI Error:", e);
      return trimmed;
    }
  }

  return trimmed;
}

// ========== 工具函数 ==========

/**
 * 捕获屏幕截图并返回临时文件路径
 * 如果用户取消截图，返回 null
 */
async function captureScreenshot(prefix: string = 'screenshot'): Promise<string | null> {
  return new Promise((resolve) => {
    const tempPath = path.join(app.getPath('temp'), `${prefix}-${Date.now()}.png`);

    exec(`screencapture -i "${tempPath}"`, (error) => {
      if (error || !fs.existsSync(tempPath)) {
        resolve(null);
        return;
      }
      resolve(tempPath);
    });
  });
}

/**
 * 注册全局快捷键的通用辅助函数
 */
function registerGlobalHotkey(
  hotkey: string,
  name: string,
  callback: () => void
): boolean {
  try {
    globalShortcut.unregister(hotkey);
    const ret = globalShortcut.register(hotkey, callback);

    if (!ret) {
      console.error(`${name} hotkey registration failed:`, hotkey);
      return false;
    }

    console.log(`Registered ${name} hotkey:`, hotkey);
    return true;
  } catch (e) {
    console.error(`Invalid ${name} hotkey:`, hotkey, e);
    return false;
  }
}

function registerScreenshotHotkey() {
  const config = store.get('screenshotTranslation');
  if (!config.enabled) return;

  registerGlobalHotkey(config.hotkey, 'screenshot', async () => {
    console.log('Screenshot Hotkey Triggered');
    win?.hide();
    app.hide();

    // 短暂延迟确保窗口完全隐藏
    await new Promise(resolve => setTimeout(resolve, 300));

    const tempPath = await captureScreenshot('translation');
    app.show();

    if (!tempPath) {
      console.log('Screenshot cancelled');
      return;
    }

    console.log('Screenshot captured');

    const config = store.get('screenshotTranslation');
    const ocrSource = config.ocrSource || 'system';

    // 验证 API Key
    if (ocrSource === 'glm' && !config.glmApiKey) {
      console.error('GLM API Key not configured');
      return;
    }
    if (ocrSource === 'openai' && !config.openai?.apiKey) {
      console.error('OpenAI API Key not configured');
      return;
    }

    // 创建或显示截图窗口
    if (!screenshotWin) {
      createScreenshotWindow();
      await new Promise(resolve => setTimeout(resolve, 500));
    }

    screenshotWin?.webContents.send('screenshot-processing');
    screenshotWin?.show();
    screenshotWin?.focus();

    try {
      let recognizedText: string;

      // 根据配置选择 OCR 方法
      if (ocrSource === 'system') {
        console.log('Using system OCR (Vision framework)');
        recognizedText = await callSystemOCR(tempPath);
      } else if (ocrSource === 'openai' && config.openai) {
        console.log('Using OpenAI OCR');
        recognizedText = await callOpenAIOCR(tempPath, config.openai);
      } else {
        console.log('Using GLM-4V OCR');
        recognizedText = await callGLM4V(tempPath, config.glmApiKey);
      }

      console.log('Recognized:', recognizedText.substring(0, 100) + '...');

      const translatedText = await translateTextHelper(recognizedText);
      console.log('Translated:', translatedText.substring(0, 100) + '...');

      screenshotWin?.webContents.send('screenshot-result', {
        original: recognizedText,
        translated: translatedText
      });

    } catch (err: unknown) {
      console.error('Processing error:', err);
      const message = err instanceof Error ? err.message : String(err);
      screenshotWin?.webContents.send('screenshot-error', message);
    } finally {
      try {
        fs.unlinkSync(tempPath);
      } catch {
        // 忽略清理错误
      }
    }
  });
}

// ============================================
// Screenshot Explanation Functions
// ============================================

// Create Screenshot Explanation Window
function createExplainWindow(imageId: string) {
  if (explainWin) {
    explainWin.focus();
    return;
  }

  explainWin = new BrowserWindow({
    width: 700,
    height: 800,
    resizable: true,
    frame: true,
    titleBarStyle: 'hiddenInset',
    webPreferences: {
      preload: path.join(__dirname, 'preload.mjs'),
      contextIsolation: true,
      nodeIntegration: false
    }
  });

  explainWin.on('closed', () => {
    explainWin = null;
    cleanupExplainImage(currentExplainImageId)
  });

  hardenWindow(explainWin)

  if (process.env.VITE_DEV_SERVER_URL) {
    explainWin.loadURL(`${process.env.VITE_DEV_SERVER_URL}#explain?imageId=${encodeURIComponent(imageId)}`);
  } else {
    const distPath = process.env.DIST || path.join(__dirname, '../dist');
    explainWin.loadFile(path.join(distPath, 'index.html'), {
      hash: `explain?imageId=${encodeURIComponent(imageId)}`
    });
  }
}

// Call Vision API (GLM-4V or OpenAI compatible)
async function callVisionAPI(imageId: string, messages: Array<{ role: string, content: unknown }>, language: string): Promise<string> {
  const config = store.get('screenshotExplain');
  const { provider, apiKey, baseURL, modelName } = config.model;

  // Read and encode image
  const imagePath = resolveExplainImagePath(imageId)
  const imageBuffer = await fs.promises.readFile(imagePath);
  const imageBase64 = imageBuffer.toString('base64');

  // System prompt based on language (will be prepended to first user message)
  const defaultSystemPrompt = language === 'zh'
    ? '你是一个图片分析助手。请用自然流畅的语言回答，不要使用小标题、序号或分点列举。\n\n'
    : 'You are an image analysis assistant. Please respond naturally without headings, bullet points, or numbered lists.\n\n';

  const systemPrompt = config.customPrompts?.systemPrompt || defaultSystemPrompt;

  // Build API messages
  const apiMessages = messages.map((msg, index) => {
    if (msg.role === 'user' && index === 0) {
      // First user message includes the image
      // Prepend system prompt to first user message (GLM doesn't support system role)
      const userText = systemPrompt + msg.content;

      return {
        role: 'user',
        content: [
          {
            type: 'image_url',
            image_url: {
              url: `data:image/png;base64,${imageBase64}`
            }
          },
          {
            type: 'text',
            text: userText
          }
        ]
      };
    }
    return msg;
  });

  // Construct API URL
  const apiUrl = `${baseURL}/chat/completions`;

  // Make API call
  const requestBody: { model: string; messages: unknown; temperature: number; max_tokens?: number } = {
    model: modelName,
    messages: apiMessages,  // No system role for GLM
    temperature: 0.7
  };

  // Only add max_tokens for OpenAI, not GLM
  if (provider === 'openai') {
    requestBody.max_tokens = 2000;
  }

  const response = await fetch(apiUrl, {
    method: 'POST',
    headers: {
      'Authorization': `Bearer ${apiKey}`,
      'Content-Type': 'application/json'
    },
    body: JSON.stringify(requestBody)
  });

  if (!response.ok) {
    const errorText = await response.text();
    throw new Error(`Vision API Error: ${response.status} - ${errorText}`);
  }

  const data = await response.json();
  return data.choices[0].message.content;
}

// Get initial summary for an image
async function getInitialSummary(imageId: string, language: string): Promise<string> {
  const config = store.get('screenshotExplain');

  const defaultPrompt = language === 'zh'
    ? '你是一个图片分析助手。请简洁地总结这张图片的主要内容，不要使用小标题、序号或分点列举。\n\n要求：\n- 用1-3句话概括图片核心内容\n- 语言自然流畅，像在和朋友描述\n- 突出最重要的信息\n- 不要使用"图片显示..."这样的开头\n\n请用中文回复。'
    : 'You are an image analysis assistant. Please provide a concise summary of this image\'s main content without using headings, bullet points, or numbered lists.\n\nRequirements:\n- Summarize in 1-3 natural sentences\n- Write conversationally as if describing to a friend\n- Highlight the most important information\n- Don\'t start with "The image shows..."\n\nPlease respond in English.';

  const prompt = config.customPrompts?.summaryPrompt || defaultPrompt;

  const messages = [
    { role: 'user', content: prompt }
  ];

  return await callVisionAPI(imageId, messages, language);
}

// Register Screenshot Explanation Hotkey
function registerExplainHotkey() {
  const config = store.get('screenshotExplain');
  if (!config.enabled) return;

  registerGlobalHotkey(config.hotkey, 'explain', async () => {
    console.log('Explain Hotkey Triggered');
    win?.hide();

    const tempImagePath = await captureScreenshot('explain');
    if (!tempImagePath) {
      console.log('Screenshot cancelled');
      win?.show();
      return;
    }

    try {
      cleanupExplainImage(currentExplainImageId);
      const imageId = randomUUID();
      currentExplainImageId = imageId;
      explainImages.set(imageId, { path: tempImagePath, createdAt: Date.now() });
      createExplainWindow(imageId);
    } catch (err) {
      console.error('Error creating explain window:', err);
      cleanupExplainImage(currentExplainImageId);
      win?.show();
    }
  });
}

// IPC Handlers for Screenshot Explanation
ipcMain.handle('explain-get-initial-summary', async (event, imageId: string) => {
  if (!isAllowedIpcSender(event.senderFrame.url)) return { success: false, error: 'Blocked' };
  const language = store.get('screenshotExplain').defaultLanguage;
  try {
    const summary = await getInitialSummary(imageId, language);
    return { success: true, summary };
  } catch (error: unknown) {
    console.error('Error getting initial summary:', error);
    const message = error instanceof Error ? error.message : String(error)
    return { success: false, error: message };
  }
});

ipcMain.handle('explain-ask-question', async (event, imageId: string, messages: Array<{ role: string, content: string }>) => {
  if (!isAllowedIpcSender(event.senderFrame.url)) return { success: false, error: 'Blocked' };
  const language = store.get('screenshotExplain').defaultLanguage;
  try {
    // Build conversation with question prompt
    const questionPrompt = language === 'zh'
      ? `你是一个图片分析助手。用户正在询问关于这张图片的问题。\n\n要求：\n- 直接回答问题，不要使用小标题或分点列举\n- 语言自然、简洁\n- 基于图片内容回答\n- 如果问题与图片无关，礼貌地引导回到图片内容\n\n请用中文回复。`
      : `You are an image analysis assistant. The user is asking a question about this image.\n\nRequirements:\n- Answer directly without headings or bullet points\n- Be natural and concise\n- Base your answer on the image content\n- If the question is unrelated to the image, politely guide back\n\nPlease respond in English.`;

    const lastUserMessage = messages[messages.length - 1];
    const userQuestion = lastUserMessage.content;

    const apiMessages = messages.slice(0, -1).concat([
      { role: 'user', content: `${questionPrompt}\n\n用户问题：${userQuestion}` }
    ]);

    const response = await callVisionAPI(imageId, apiMessages, language);
    return { success: true, response };
  } catch (error: unknown) {
    console.error('Error asking question:', error);
    const message = error instanceof Error ? error.message : String(error)
    return { success: false, error: message };
  }
});

ipcMain.handle('explain-read-image', async (event, imageId: string) => {
  if (!isAllowedIpcSender(event.senderFrame.url)) return { success: false, error: 'Blocked' };
  try {
    const imagePath = resolveExplainImagePath(imageId)
    const imageBuffer = fs.readFileSync(imagePath);
    const base64 = imageBuffer.toString('base64');
    return { success: true, data: `data:image/png;base64,${base64}` };
  } catch (error: unknown) {
    console.error('Error reading image:', error);
    const message = error instanceof Error ? error.message : String(error)
    return { success: false, error: message };
  }
});

ipcMain.on('close-explain-window', (event) => {
  if (!isAllowedIpcSender(event.senderFrame.url)) return
  if (explainWin) {
    explainWin.close();
    explainWin = null;
  }
  cleanupExplainImage(currentExplainImageId)
});

// Save explanation to history (max 5 records)
ipcMain.handle('explain-save-history', async (event, messages: Array<{ role: string; content: string }>) => {
  if (!isAllowedIpcSender(event.senderFrame.url)) return { success: false, error: 'Blocked' };
  try {
    const history = store.get('explainHistory') || [];
    const newRecord = {
      id: Date.now().toString(),
      timestamp: Date.now(),
      messages  // Only save messages, no images
    };

    // Add to beginning and keep only last 5
    const updatedHistory = [newRecord, ...history].slice(0, 5);
    store.set('explainHistory', updatedHistory);

    console.log('History saved, total:', updatedHistory.length);
    return { success: true };
  } catch (error: unknown) {
    console.error('Error saving history:', error);
    const message = error instanceof Error ? error.message : String(error)
    return { success: false, error: message };
  }
});

// Get explanation history
ipcMain.handle('explain-get-history', async (event) => {
  if (!isAllowedIpcSender(event.senderFrame.url)) return { success: false, error: 'Blocked', history: [] };
  try {
    const history = store.get('explainHistory') || [];
    return { success: true, history };
  } catch (error: unknown) {
    console.error('Error getting history:', error);
    const message = error instanceof Error ? error.message : String(error)
    return { success: false, error: message, history: [] };
  }
});

// Load a specific history record
ipcMain.handle('explain-load-history', async (event, historyId: string) => {
  if (!isAllowedIpcSender(event.senderFrame.url)) return { success: false, error: 'Blocked' };
  try {
    const history = store.get('explainHistory') || [];
    const record = history.find(h => h.id === historyId);

    if (!record) {
      return { success: false, error: 'History not found' };
    }

    return { success: true, record };
  } catch (error: unknown) {
    console.error('Error loading history:', error);
    const message = error instanceof Error ? error.message : String(error)
    return { success: false, error: message };
  }
});

