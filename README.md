# KeyLingo

<p align="center">
  <img src="public/icon.png" width="128" height="128" alt="KeyLingo Icon">
</p>

[English](#english) | [中文](#中文)

---

<a name="english"></a>
## 🇬🇧 English

**KeyLingo** is a smart translation and AI vision utility designed for macOS. With global hotkeys, you can instantly translate text, analyze screenshots, and work across any application seamlessly.

### ✨ Key Features

*   **Global Hotkey**: Toggle the translation bar instantly from any app (Default: `Cmd+Option+T`).
*   **Screenshot Translation**: Capture any part of your screen and translate text from images using GLM-4V or System OCR (Default: `Cmd+Shift+A`).
*   **Screenshot Explanation (NEW in v1.3.0)**: AI-powered screenshot analysis with conversational Q&A - explain code, designs, or any visual content (Default: `Cmd+Shift+E`).
*   **Minimalist Design**: Clean, distracting-free UI that floats over your windows. Supports **Light** and **Dark** modes (System sync).
*   **Smart Translation**:
    *   **Bing Translate**: Fast, unlimited, and free built-in translation.
    *   **AI Integration**: Support for **DeepSeek**, **Zhipu**, **Qwen**, and other OpenAI-compatible APIs for high-quality, context-aware translations.
*   **Auto-Paste**: Press `Enter` to translate and automatically paste the result into your text editor, browser, or chat window.
*   **Menu Bar Integration**: Unobtrusive tray icon for quick access to settings and quitting.
*   **Target Language**: Supports Auto-detection, English, Chinese, Japanese, Korean, French, and German.

### 📋 Version History

**v1.3.0** (2025-12-21)
- 🎉 **Project Renamed** to **KeyLingo**
- ✨ Added **Screenshot Explanation** feature (Cmd+Shift+E) - AI analysis of screenshots with multi-turn conversation
- ✨ Added **Conversation History** - Save and review last 5 screenshot explanations
- ✨ Added **Custom Prompts** - Customize AI system and summary prompts
- 🐛 Fixed tray menu settings bug - window now hides properly after closing settings

**v1.2.0** (2025-12-20)
- ✨ Added **System OCR** option for screenshot translation (offline, free using macOS Vision framework)
- 🔧 Improved input field UX - auto-scroll to show cursor, better text visibility
- 🔧 Fixed settings save bug - changes now apply immediately without restart
- 📝 Enhanced translation result display with auto-scroll to latest content

**v1.1.0** (2025-12-17)
- ✨ Added **Screenshot Translation** feature with GLM-4V OCR
- 🎨 New screenshot result UI with copy functionality

**v1.0.0** (Initial Release)
- 🚀 Global translation with customizable hotkey
- 🤖 Bing Translate and AI model (DeepSeek/Zhipu) support
- 🎨 Light/Dark mode with system sync

### 🚀 Installation

1.  Download the latest `.dmg` from the [Releases](./release) folder.
2.  Open the `.dmg` and drag **KeyLingo** to your `Applications` folder.
3.  **Permissions**: On first launch, you must grant **Accessibility Permissions** to allow the app to simulate keystrokes (for the Auto-Paste feature).

### 🛠 Usage

#### Main Translation
1.  **Activate**: Press `Command + Option + T` (configurable).
2.  **Translate**: Type your text. The translation updates in real-time (with debounce).
3.  **Commit**: Press `Enter`. The translated text is copied to your clipboard and pasted into the previous active app.
    *   *Tip: Press `Esc` to close the window without pasting.*

#### Screenshot Translation (NEW)
1.  **Activate**: Press `Command + Shift + A` (configurable).
2.  **Capture**: Select the screen area containing text you want to translate.
3.  **Wait**: The app will recognize text using your selected OCR engine and translate it automatically.
    *   **System OCR** (default): Uses macOS Vision framework - offline and free
    *   **GLM-4V**: Online AI model - higher accuracy, requires API key
4.  **View Results**: See both original recognized text and translation in a popup window.
    *   *Tip: Click the copy button to copy the translation to clipboard.*

#### Settings
Hover over the top-right corner of the translation bar and click the **Gear Icon ⚙️**:
*   **Translation Source**: Switch between Bing (default) or OpenAI (DeepSeek/Zhipu).
*   **AI Configuration**: Enter your API Key, Base URL, and Model Name.
*   **Screenshot Translation**: 
    *   Enable/disable screenshot translation
    *   Configure hotkey
    *   Choose **OCR Source**: System OCR (offline, free) or GLM-4V (online, requires API key from [bigmodel.cn](https://bigmodel.cn/console/apikey))
*   **Shortcut**: Click the input box and type your desired global hotkey.

### 💻 Development

Built with [Electron](https://www.electronjs.org/), [React](https://react.dev/), [Vite](https://vitejs.dev/), and [TailwindCSS](https://tailwindcss.com/).

```bash
# Install dependencies
npm install

# Build macOS System OCR helper (requires Xcode Command Line Tools)
npm run build:ocr

# Start development server
npm run dev

# Build for macOS
npm run build
```

---

<a name="中文"></a>
## 🇨🇳 中文

**KeyLingo** 是一款专为 macOS 设计的智能翻译和 AI 视觉工具。通过全局快捷键，您可以随时翻译文本、分析截图，并无缝跨应用工作。

### ✨ 核心功能

*   **全局快捷键**：在任何应用中随时呼出翻译栏（默认：`Cmd+Option+T`）。
*   **截图翻译（v1.1.0 新功能）**：截取屏幕任意区域，使用 GLM-4V 识别图片中的文字并翻译（默认：`Cmd+Shift+A`）。
*   **极简设计**：干净、无干扰的悬浮界面。支持 **亮色** 和 **暗色** 模式（跟随系统）。
*   **智能翻译**：
    *   **Bing 翻译**：内置快速、免费的必应翻译，无需配置。
    *   **AI 大模型**：支持配置 **DeepSeek (深度求索)**、**智谱清言**、**通义千问** 等兼容 OpenAI 格式的 API，提供更精准、更自然的翻译体验。
*   **自动上屏**：按下 `Enter` 键确认，翻译结果将自动输入到您当前的光标位置（如编辑器、浏览器、微信等）。
*   **菜单栏常驻**：顶部菜单栏图标，方便快速访问设置或退出应用，不占用 Dock 栏。
*   **多语言支持**：支持自动检测，以及中、英、日、韩、法、德互译。

### 📋 版本历史

**v1.2.0** (2025-12-20)
- ✨ 新增**系统 OCR** 选项用于截图翻译（离线、免费，使用 macOS Vision 框架）
- 🔧 优化输入框体验 - 自动滚动显示光标，文字可见性更好
- 🔧 修复设置保存问题 - 设置更改立即生效，无需重启
- 📝 增强翻译结果显示，自动滚动到最新内容

**v1.1.0** (2025-12-17)
- ✨ 新增**截图翻译**功能，支持 GLM-4V OCR
- 🎨 全新截图结果界面，支持复制功能

**v1.0.0** (首次发布)
- 🚀 全局翻译，支持自定义快捷键
- 🤖 Bing 翻译和 AI 模型（DeepSeek/智谱）支持
- 🎨 亮色/暗色主题，跟随系统

### 🚀 安装说明

1.  在 [release](./release) 文件夹中找到最新的 `.dmg` 安装包。
2.  双击 `.dmg` 并将 **Fanyi Translator** 拖入 `应用程序 (Applications)` 文件夹。
3.  **权限授予**：首次运行时，系统会提示授予 **辅助功能 (Accessibility)** 权限。这是实现“自动粘贴”功能所必需的，请前往“系统设置 -> 隐私与安全性 -> 辅助功能”中勾选本应用。

### 🛠 使用指南

#### 主翻译功能
1.  **唤出**：按下 `Command + Option + T`（可在设置中修改）。
2.  **翻译**：直接输入文字，并在上方查看实时翻译结果。
3.  **确认/上屏**：按下 `Enter`。译文会自动复制并粘贴到您刚才工作的窗口中。
    *   *提示：按 `Esc` 可直接关闭窗口而不进行任何操作。*

#### 截图翻译（新功能）
1.  **唤出**：按下 `Command + Shift + A`（可在设置中修改）。
2.  **截图**：选择屏幕中包含要翻译文字的区域。
3.  **等待**：应用会使用您选择的 OCR 引擎自动识别文字并翻译。
    *   **系统 OCR**（默认）：使用 macOS Vision 框架 - 离线且免费
    *   **GLM-4V**：在线 AI 模型 - 精度更高，需要 API Key
4.  **查看结果**：在弹出窗口中查看识别的原文和翻译结果。
    *   *提示：点击复制按钮可将译文复制到剪贴板。*

#### 设置
将鼠标悬停在翻译栏右上角，点击出现的 **齿轮图标 ⚙️**：
*   **翻译源**：选择 Bing（默认）或 OpenAI（自定义 AI 模型）。
*   **AI 配置**：填写您的 API Key、Base URL 和模型名称（如 `deepseek-chat`）。
*   **截图翻译**：
    *   启用/禁用截图翻译功能
    *   配置快捷键
    *   选择 **OCR 识别源**：系统 OCR（离线，免费）或 GLM-4V（在线，需要从 [bigmodel.cn](https://bigmodel.cn/console/apikey) 获取免费 API Key）
*   **快捷键**：在输入框中按下您习惯的组合键即可修改。

### 💻 开发构建

本项目基于 [Electron](https://www.electronjs.org/), [React](https://react.dev/), [Vite](https://vitejs.dev/) 和 [TailwindCSS](https://tailwindcss.com/) 构建。

```bash
# 安装依赖
npm install

# 构建 macOS 系统 OCR 辅助程序（需要安装 Xcode Command Line Tools）
npm run build:ocr

# 启动开发环境
npm run dev

# 打包 macOS 应用 (.dmg)
npm run build
```
