# KeyLingo v2.0.1

<p align="center">
  <img src="public/icon.png" width="128" height="128" alt="KeyLingo Icon">
</p>

[English](#english) | [中文](#中文)

---

<a name="english"></a>
## 🇬🇧 English

**KeyLingo** is a smart translation and AI vision utility designed for **macOS** and **Windows**. It enables instant text translation, screenshots translation, and AI-powered image analysis across any application with global hotkeys.

### ✨ Key Features

*   **Global Hotkey**: Toggle translation bar instantly (Default: `Cmd/Ctrl+Alt+T`).
*   **AI Translation Engine**: Support for **OpenAI-compatible** APIs (OpenAI, DeepSeek, Agri, etc.) for high-quality, context-aware translations.
*   **Screenshot Translation**: Capture any screen area and translate text instantly using OCR models (Default: `Cmd/Ctrl+Shift+A`).
*   **Screenshot Explanation**: AI-powered analysis of screenshots with multi-turn conversation support - explain code, UI designs, or data (Default: `Cmd/Ctrl+Shift+E`).
*   **Multi-Provider Support**: 
    *   Configure different providers for Translation, OCR, and Explanation.
    *   Support for custom Base URLs and Models.
*   **Cross-Platform**: Fully optimized for **macOS** and **Windows**.
    *   **macOS**: Native `screencapture` integration.
    *   **Windows**: Native `ms-screenclip` integration.
*   **Auto-Paste**: Automatically paste translation results into your active application (Editor, Browser, IDE).
*   **Minimalist UI**: Clean, floating interface with Light/Dark mode support (System sync).
*   **History**: Automatically saves recent screenshot explanation sessions.

### 📋 Version History

**v2.0.1** (2025-02-05)
- 🐛 **Fixes**: Resolved UI border artifacts on transparent windows.

**v2.0.0** (2025-02-05)
- 🚀 **Major Rewrite**: Refactored core architecture for better stability and maintainability.
- ✨ **Multi-Provider System**: Configure independent API providers for different tasks (Translation/OCR/Explain).
- 🖼️ **Enhanced Windows Support**: Optimized screenshot capture workflow for Windows.
- 🎨 **UI Overhaul**: Redesigned Settings interface for better usability.
- 🔧 **Performance**: Improved binary handling and asynchronous operations.

### 🚀 Installation

1.  Download the latest installer from [Releases](https://github.com/ZMGID/keylingo/releases).
2.  **macOS**: Drag `KeyLingo.app` to Applications.
3.  **Windows**: Run the `.msi` or `.exe` installer.
4.  **Permissions**:
    *   **macOS**: Grant **Accessibility** permission (for Auto-Paste) and **Screen Recording** permission (for Screenshots).
    *   **Windows**: No special permissions required usually.

### 🛠 Usage

#### Translation
1.  **Activate**: Press `Cmd/Ctrl + Alt + T`.
2.  **Type**: Enter text to translate.
3.  **Commit**: Press `Enter` to copy & paste result automatically.

#### Screenshot Translation
1.  **Activate**: Press `Cmd/Ctrl + Shift + A`.
2.  **Select**: Click and drag to capture area.
3.  **Result**: View OCR text and translation side-by-side.

#### Screenshot Explanation
1.  **Activate**: Press `Cmd/Ctrl + Shift + E`.
2.  **Select**: Capture area to analyze.
3.  **Chat**: Ask questions about the image content.

#### Settings
Click the gear icon ⚙️ in the translation bar to configure:
*   **Providers**: Add OpenAI, DeepSeek, or other compatible API keys.
*   **Hotkeys**: Customize global shortcuts.
*   **Prompts**: Custom system prompts for AI analysis.

### 💻 Development

Built with [Tauri v2](https://v2.tauri.app/), [React](https://react.dev/), [Vite](https://vitejs.dev/) & [TailwindCSS v4](https://tailwindcss.com/).

```bash
# Install dependencies
npm install

# Dev
npm run dev

# Build
npm run build
```

---

<a name="中文"></a>
## 🇨🇳 中文

**KeyLingo** 是一款适用于 **macOS** 和 **Windows** 的智能翻译与 AI 视觉工具。通过全局快捷键，您可以随时进行文本翻译、截图翻译以及基于 AI 的屏幕内容分析。

### ✨ 核心功能

*   **全局快捷键**：一键呼出翻译栏（默认：`Cmd/Ctrl+Alt+T`）。
*   **AI 翻译引擎**：支持所有 **OpenAI 兼容** 接口（如 DeepSeek、OpenAI、SiliconFlow 等），提供精准的上下文翻译。
*   **截图翻译**：截取屏幕任意区域，立即识别并翻译文字（默认：`Cmd/Ctrl+Shift+A`）。
*   **截图讲解**：AI 深度分析截图内容，支持多轮对话——无论是代码解释、UI 分析还是数据提取（默认：`Cmd/Ctrl+Shift+E`）。
*   **多供应商管理**：
    *   分别为 翻译、OCR、讲解 配置不同的模型服务商。
    *   支持自定义 Base URL 和 模型名称。
*   **双平台完美支持**：
    *   **macOS**: 原生 `screencapture` 集成。
    *   **Windows**: 原生 `ms-screenclip` 集成。
*   **自动上屏**：按下回车，自动将译文粘贴到您当前工作的软件中。
*   **极简设计**：无干扰的悬浮界面，完美适配深色/浅色模式。
*   **历史记录**：自动保存最近的截图分析会话方便回顾。

### 📋 版本历史

**v2.0.1** (2025-02-05)
- 🐛 **修复**: 修复了 macOS 下透明背景的边框渲染问题。

**v2.0.0** (2025-02-05)
- 🚀 **架构重写**: 全面重构底层代码，大幅提升稳定性与可维护性。
- ✨ **多供应商系统**: 支持为翻译、OCR、讲解功能分别配置不同的 API 服务商。
- 🖼️ **Windows 优化**: 重新设计 Windows 下的截图交互流程。
- 🎨 **界面升级**: 全新的设置面板设计，交互更直观。
- 🔧 **性能提升**: 优化异步任务处理与资源占用。

### 🚀 安装说明

1.  从 [Releases](https://github.com/ZMGID/keylingo/releases) 下载最新安装包。
2.  **macOS**: 将 `KeyLingo.app` 拖入“应用程序”文件夹。
3.  **Windows**: 运行 `.msi` 或 `.exe` 安装程序。
4.  **权限说明**:
    *   **macOS**: 首次运行需授予 **辅助功能**（用于模拟按键粘贴）和 **屏幕录制**（用于截图）权限。

### 🛠 使用指南

#### 文本翻译
1.  **呼出**: `Cmd/Ctrl + Alt + T`
2.  **输入**: 输入或粘贴文本
3.  **确认**: 按 `Enter` 自动复制并粘贴到上一个窗口

#### 截图翻译
1.  **呼出**: `Cmd/Ctrl + Shift + A`
2.  **截图**: 框选屏幕区域
3.  **查看**: 自动显示识别原文与译文

#### 截图讲解
1.  **呼出**: `Cmd/Ctrl + Shift + E`
2.  **截图**: 框选需要分析的内容
3.  **对话**: AI 会自动总结内容，您可以继续提问

#### 设置
点击翻译栏右上角的齿轮图标 ⚙️：
*   **模型配置**: 添加 DeepSeek、OpenAI 等 API Key。
*   **快捷键**: 自定义所有功能的触发热键。
*   **Prompt**: 自定义 AI 的系统提示词和风格。

### 💻 开发构建

基于 [Tauri v2](https://v2.tauri.app/), [React](https://react.dev/), [Vite](https://vitejs.dev/) 和 [TailwindCSS v4](https://tailwindcss.com/) 构建。

```bash
# 安装依赖
npm install

# 本地开发
npm run dev

# 打包构建
npm run build
```
