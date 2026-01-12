# KeyLingo

<p align="center">
  <img src="public/icon.png" width="128" height="128" alt="KeyLingo Icon">
</p>

[English](#english) | [中文](#chinese)

---

<a name="english"></a>
## 🇬🇧 KeyLingo - Smart Translation & AI Vision Tool

**KeyLingo** is a smart translation and AI vision utility designed for macOS. With global hotkeys, you can instantly translate text, analyze screenshots, and work across any application seamlessly.

### ✨ Key Features

*   **Global Hotkey**: Toggle the translation bar instantly from any app (Default: `Cmd+Option+T`).
*   **Screenshot Translation**: Capture any part of your screen and translate text from images using **OpenAI (Custom)**, GLM-4V, or System OCR (Default: `Cmd+Shift+A`).
*   **Screenshot Explanation (NEW)**: AI-powered screenshot analysis with conversational Q&A - explain code, designs, or any visual content (Default: `Cmd+Shift+E`).
*   **Minimalist Design**: Clean, distracting-free UI that floats over your windows. Supports **Light** and **Dark** modes (System sync).
*   **Smart Translation**:
    *   **Bing Translate**: Fast, unlimited, and free built-in translation.
    *   **AI Integration**: Support for **DeepSeek**, **Zhipu**, **Qwen**, and other OpenAI-compatible APIs for high-quality, context-aware translations.
*   **Performance Optimized**: Swift OCR binary caching, asynchronous I/O, and frontend lazy loading for ⚡️ lightning-fast response.
*   **Auto-Paste**: Press `Enter` to translate and automatically paste the result into your text editor, browser, or chat window.
*   **Menu Bar Integration**: Unobtrusive tray icon for quick access to settings and quitting.
*   **Target Language**: Supports Auto-detection, English, Chinese, Japanese, Korean, French, and German.

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

#### Screenshot Translation & Explanation
1.  **Translate**: Press `Command + Shift + A`. Select area, wait for OCR and translation.
2.  **Explain**: Press `Command + Shift + E`. Select area, and start a conversation with AI about the image content.

#### Settings
Hover over the top-right corner of the translation bar and click the **Gear Icon ⚙️**:
*   **Translation Source**: Switch between Bing (default) or OpenAI (DeepSeek/Zhipu).
*   **AI Configuration**: Enter your API Key, Base URL, and Model Name.
*   **Screenshot Translation**: Configure hotkeys and OCR sources (System OCR, OpenAI, or GLM-4V).
*   **Language**: Switch UI language (English/Chinese).

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

<a name="chinese"></a>
## 🇨🇳 KeyLingo - 智能翻译与 AI 视觉工具

**KeyLingo** 是一款专为 macOS 设计的智能翻译和 AI 视觉工具。通过全局快捷键，您可以随时翻译文本、分析截图，并无缝跨应用工作。

### ✨ 核心功能

*   **全局快捷键**：在任何应用中随时呼出翻译栏（默认：`Cmd+Option+T`）。
*   **截图翻译**：截取屏幕任意区域，使用 **OpenAI (自定义)**、GLM-4V 或系统 OCR 识别并翻译（默认：`Cmd+Shift+A`）。
*   **截图讲解 (NEW)**：AI 驱动的截图分析与多轮对话 - 解释代码、设计稿或任何视觉内容（默认：`Cmd+Shift+E`）。
*   **极简设计**：干净、无干扰的悬浮界面。支持 **亮色** 和 **暗色** 模式（跟随系统）。
*   **性能优化**：Swift OCR 二进制缓存、异步 I/O 及前端组件懒加载，响应如闪电般迅速 ⚡️。
*   **智能翻译**：
    *   **Bing 翻译**：内置快速、免费的必应翻译，无需配置。
    *   **AI 大模型**：支持配置 **DeepSeek (深度求索)**、**智谱清言**、**通义千问** 等兼容 OpenAI 格式的 API，提供更精准、更自然的翻译体验。
*   **自动上屏**：按下 `Enter` 键确认，翻译结果将自动输入到您当前的光标位置（如编辑器、浏览器、微信等）。
*   **菜单栏常驻**：顶部菜单栏图标，方便快速访问设置或退出应用，不占用 Dock 栏。
*   **多语言支持**：支持自动检测，以及中、英、日、韩、法、德互译。

### 🚀 安装说明

1.  在 [release](./release) 文件夹中找到最新的 `.dmg` 安装包。
2.  双击 `.dmg` 并将 **KeyLingo** 拖入 `应用程序 (Applications)` 文件夹。
3.  **权限授予**：首次运行时，系统会提示授予 **辅助功能 (Accessibility)** 权限。这是实现“自动粘贴”功能所必需的，请前往“系统设置 -> 隐私与安全性 -> 辅助功能”中勾选本应用。

### 🛠 使用指南

#### 主翻译功能
1.  **唤出**：按下 `Command + Option + T`（可在设置中修改）。
2.  **翻译**：直接输入文字，并在上方查看实时翻译结果。
3.  **确认/上屏**：按下 `Enter`。译文会自动复制并粘贴到您刚才工作的窗口中。

#### 截图翻译与讲解
1.  **截图翻译**：按下 `Command + Shift + A`. 框选区域，自动 OCR 并翻译。
2.  **截图讲解**：按下 `Command + Shift + E`. 框选区域，AI 将对图片内容进行解析，并支持后续提问。

#### 设置
将鼠标悬停在翻译栏右上角，点击出现的 **齿轮图标 ⚙️**：
*   **翻译源**：选择 Bing（默认）或 OpenAI（自定义 AI 模型）。
*   **AI 配置**：填写您的 API Key、Base URL 和模型名称。
*   **截图翻译**：配置快捷键及 OCR 识别源（系统 OCR、OpenAI 或 GLM-4V）。
*   **语言**：切换界面语言（中文/英文）。

---

## 📋 Version History / 版本历史

**v1.3.6** (2025-01-12)
- 🇬🇧 ✨ Added **Language Setting**: Switch app interface language between English and Chinese.
- 🇨🇳 ✨ 新增 **界面语言设置**：可在设置中自由切换中英文界面。

**v1.3.5** (2025-01-12)
- 🇬🇧 ✨ Added **OpenAI OCR** for screenshot translation (GPT-4o compatible).
- 🇬🇧 🚀 **Performance Optimization**: Swift OCR caching, async I/O, lazy loading.
- 🇨🇳 ✨ 新增 **OpenAI OCR** 支持：截图识别可使用 GPT-4o 等兼容模型。
- 🇨🇳 🚀 **性能极致优化**：OCR 缓存、异步 I/O、组件懒加载，启动速度大幅提升。

**v1.3.0** (2025-12-21)
- 🎉 **Project Renamed** to **KeyLingo** / 项目更名为 **KeyLingo**
- ✨ Added **Screenshot Explanation** / 新增截图讲解 (Cmd+Shift+E)
- ✨ Added **Conversation History** / 新增对话历史
- ✨ Added **Custom Prompts** / 新增自定义提示词

**v1.2.0** (2025-12-20)
- ✨ Added **System OCR** (offline, free) / 新增系统 OCR（离线免费）
- 🔧 UX Improvements / 体验优化

**v1.1.0** (2025-12-17)
- ✨ Added **Screenshot Translation** with GLM-4V / 新增截图翻译 (GLM-4V)

**v1.0.0** (Initial Release)
- 🚀 Global translation, Bing/AI support / 全局翻译，支持 Bing/AI
