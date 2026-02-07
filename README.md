# KeyLingo v2.0.5

<p align="center">
  <img src="public/icon.png" width="128" height="128" alt="KeyLingo Icon">
</p>

[English](#english) | [中文](#中文)

---

<a name="english"></a>
## 🇬🇧 English

**KeyLingo** is a lightweight translation and AI vision utility for **macOS** and **Windows**. It focuses on a **small package size** and **low runtime usage**, while providing instant translation, screenshot translation, and screenshot explanation through global shortcuts.

### ✨ Key Features

- **Global Hotkeys**
  - Translator: `Cmd/Ctrl+Alt+T`
  - Screenshot Translation: `Cmd/Ctrl+Shift+A`
  - Screenshot Explain: `Cmd/Ctrl+Shift+E`
- **OpenAI-Compatible Providers**: OpenAI / DeepSeek / SiliconFlow / custom compatible endpoints.
- **Multi-Provider Routing**: Separate providers/models for Translation, OCR, and Explain.
- **Screenshot Translation**: OCR + translation pipeline with optional direct-translate mode.
- **Screenshot Explain**: AI image understanding with follow-up Q&A and history.
- **LaTeX Math Adaptation**: Better readability for formula-heavy outputs.
- **Auto Paste**: Enter to paste translated content back to your active app.
- **Launch at Startup**: Optional startup toggle, default **off**.
- **Safer Settings UX**:
  - Saving settings does not auto-close the settings window.
  - Unsaved changes trigger a 3-action close dialog.
- **Provider Diagnostics**: One-click **Test Connection** + **Fetch Models**.
- **Permission Dashboard (macOS)**: Accessibility/Screen Recording status + deep-link to System Settings.

### 📦 Release Assets (v2.0.5)

- **macOS (Apple Silicon)**: `KeyLingo_2.0.5_aarch64.dmg`
- **Windows Installer (NSIS)**: `KeyLingo_2.0.5_x64-setup.exe`
- **Windows Installer (MSI)**: `KeyLingo_2.0.5_x64_en-US.msi`

### 📚 Detailed Changelog

#### v2.0.5 (2026-02-07)

- **Provider Workflow Upgrade**
  - Test connection and fetch model list now support current unsaved provider inputs (Base URL / API Key).
  - You no longer need to save once before selecting models or validating connectivity.
- **Windows Launch & Instance Behavior**
  - Added single-instance guard to prevent multiple app processes from repeated icon clicks.
  - Clicking app icon now focuses the existing instance and opens settings.
  - Manual launch opens settings by default on Windows.
  - Autostart launch uses dedicated argument to avoid forced settings popup.
- **Hotkey & Capture Stability**
  - Fixed a Windows issue where screenshot hotkey could fail after opening settings.
  - Improved hotkey conflict error messages with clearer actionable hints.
- **Release Engineering**
  - Added GitHub Actions release workflow for macOS + Windows.
  - Windows release now includes both NSIS `.exe` and MSI packages.

#### v2.0.3 (2026-02-06)

- **Cross-Platform / Windows Adaptation**
  - Added Windows-native screenshot workflow with dedicated capture overlay.
  - Region-capture flow is isolated by platform guards to avoid affecting macOS path.
  - Integrated Windows schema/capabilities generation for Tauri v2 packaging.
- **Screenshot Translation Enhancements**
  - Added screenshot direct-translation mode toggle.
  - Improved OCR-to-translation flow and result presentation logic.
- **Settings & System Integration**
  - Added `Launch at startup` switch in settings and backend apply/rollback logic.
  - Retained safer close behavior with unsaved-changes confirmation.
- **Stability Fixes**
  - Fixed capture close path not clearing pending/busy state.
  - Fixed screenshot result model label mismatch in direct-translate mode.
  - Stabilized transparent window rendering behavior on desktop.
- **Release / Distribution**
  - Published unified release with both macOS and Windows installers.

#### v2.0.2 (2026-02-06)

- **Settings Experience**
  - Saving settings keeps the page open for continuous editing.
  - Added unsaved-changes close confirmation (`Save & Close` / `Discard` / `Continue Editing`).
- **Provider Tooling**
  - Added per-provider connection test button in Models.
- **Permission Visibility (macOS)**
  - Added visual permission status panel.
  - Added direct jump to corresponding System Settings pages.
- **Output Quality & Product Direction**
  - Improved LaTeX math content adaptation.
  - Continued optimization for lightweight package/runtime footprint.

#### v2.0.1 (2026-02-05)

- Fixed visual border artifacts and shadow inconsistencies on transparent windows.

#### v2.0.0 (2026-02-05)

- Major architecture rewrite for better maintainability.
- Introduced multi-provider model system.
- Upgraded screenshot and settings workflows.

### 🚀 Installation

1. Download the latest installer from [Releases](https://github.com/ZMGID/keylingo/releases).
2. **macOS**: Open `.dmg`, drag `KeyLingo.app` into Applications.
3. **Windows**: Run `.exe` or `.msi` installer.
4. **Permissions**:
   - **macOS**: grant Accessibility + Screen Recording if prompted.
   - **Windows**: usually no extra permission prompts required.

### 🛠 Usage

#### Translation
1. Press `Cmd/Ctrl + Alt + T`
2. Input text
3. Press `Enter` to commit/paste

#### Screenshot Translation
1. Press `Cmd/Ctrl + Shift + A`
2. Select capture region
3. View OCR + translated result

#### Screenshot Explain
1. Press `Cmd/Ctrl + Shift + E`
2. Select capture region
3. Ask follow-up questions on image content

#### Settings
Click gear icon ⚙️ in translator panel:
- Providers/models/hotkeys/prompts
- Test provider connectivity + fetch models
- Launch at startup toggle
- Permission status (macOS)
- Unsaved-changes close guard

### 💻 Development

Built with [Tauri v2](https://v2.tauri.app/), [React](https://react.dev/), [Vite](https://vitejs.dev/) and [TailwindCSS v4](https://tailwindcss.com/).

```bash
# Install dependencies
npm install

# Dev (Tauri + UI)
npm run dev

# Dev (UI only)
npm run dev:ui

# Build app bundle
npm run build

# Build UI only
npm run build:ui

# Lint
npm run lint
```

---

<a name="中文"></a>
## 🇨🇳 中文

**KeyLingo** 是一款面向 **macOS** 与 **Windows** 的轻量化翻译与 AI 视觉工具。产品核心方向是 **小体积、低占用**，并通过全局快捷键提供即时文本翻译、截图翻译与截图讲解能力。

### ✨ 核心功能

- **全局快捷键**
  - 文本翻译：`Cmd/Ctrl+Alt+T`
  - 截图翻译：`Cmd/Ctrl+Shift+A`
  - 截图讲解：`Cmd/Ctrl+Shift+E`
- **OpenAI 兼容生态**：支持 OpenAI / DeepSeek / SiliconFlow 及兼容接口。
- **多 Provider 路由**：翻译、OCR、讲解可分别指定服务商与模型。
- **截图翻译增强**：支持 OCR+翻译流程，并可切换直译模式。
- **截图讲解**：支持图片理解、多轮追问与历史记录。
- **LaTeX 数学公式适配**：优化公式展示与可读性。
- **自动粘贴**：回车即可回填到当前应用。
- **开机启动**：可选开关，默认关闭。
- **设置页安全流程**：
  - 保存后不自动关闭设置页。
  - 有未保存更改时，关闭弹出三选确认。
- **供应商诊断**：模型页支持一键“测试连接”与“获取模型列表”。
- **权限状态面板（macOS）**：可视化查看辅助功能/屏幕录制授权并直达系统设置。

### 📦 安装包（v2.0.5）

- **macOS（Apple Silicon）**：`KeyLingo_2.0.5_aarch64.dmg`
- **Windows NSIS 安装包**：`KeyLingo_2.0.5_x64-setup.exe`
- **Windows MSI 安装包**：`KeyLingo_2.0.5_x64_en-US.msi`

### 📚 详细更新目录

#### v2.0.5（2026-02-07）

- **Provider 工作流升级**
  - 测试连接与获取模型列表支持直接使用“当前未保存”的 Base URL / API Key。
  - 新增 Provider 时无需先保存一次设置再配置模型。
- **Windows 启动与实例行为优化**
  - 增加单实例保护，连续点击应用图标不会重复拉起多个进程。
  - 再次点击应用图标会激活已运行实例并打开设置页。
  - Windows 手动启动默认进入设置页。
  - 开机自启通过专用启动参数区分，不会强制弹出设置页。
- **快捷键与截图稳定性修复**
  - 修复 Windows 下“打开设置后截图快捷键偶发失效”的问题。
  - 优化快捷键冲突报错文案，提示更清晰。
- **发布流程增强**
  - 新增 GitHub Actions 自动发布流程（macOS + Windows）。
  - Windows 发布同时产出 NSIS `.exe` 与 MSI 安装包。

#### v2.0.3（2026-02-06）

- **跨平台 / Windows 适配**
  - 新增 Windows 原生截图工作流与独立框选遮罩层。
  - 截图流程通过平台隔离（`cfg`）实现，不影响 macOS 逻辑。
  - 补齐 Windows 相关 schema/capability 生成链路。
- **截图翻译增强**
  - 新增截图直译模式开关。
  - 优化 OCR 到翻译的结果展示与状态流转。
- **设置与系统集成**
  - 新增“开机启动”开关及后端应用/回滚逻辑。
  - 继续保留未保存更改的关闭确认流程。
- **稳定性修复**
  - 修复截图窗口关闭后 busy/pending 状态未清理的问题。
  - 修复直译模式下模型来源标识不准确的问题。
  - 进一步稳定透明窗口在桌面端的显示效果。
- **发布与分发**
  - 同一版本统一发布 macOS + Windows 安装包。

#### v2.0.2（2026-02-06）

- **设置体验升级**
  - 保存设置后不再自动退出设置页。
  - 新增未保存更改的三选确认（保存并关闭 / 放弃 / 继续编辑）。
- **供应商工具链**
  - 模型配置页新增 Provider 连接测试。
- **权限可视化（macOS）**
  - 增加辅助功能/屏幕录制状态卡片。
  - 支持一键跳转对应系统设置页面。
- **输出质量与产品方向**
  - 增强 LaTeX 数学公式场景适配。
  - 持续优化体积与运行资源占用。

#### v2.0.1（2026-02-05）

- 修复透明窗口边框线段与阴影显示异常。

#### v2.0.0（2026-02-05）

- 完成核心架构重写，提升稳定性与可维护性。
- 引入多供应商模型配置体系。
- 升级截图与设置交互流程。

### 🚀 安装说明

1. 从 [Releases](https://github.com/ZMGID/keylingo/releases) 下载最新安装包。
2. **macOS**：打开 `.dmg`，将 `KeyLingo.app` 拖到“应用程序”。
3. **Windows**：运行 `.exe` 或 `.msi` 安装器。
4. **权限说明**：
   - **macOS**：如提示请授予辅助功能与屏幕录制权限。
   - **Windows**：通常无需额外授权。

### 🛠 使用指南

#### 文本翻译
1. 按 `Cmd/Ctrl + Alt + T`
2. 输入文本
3. 按 `Enter` 提交并自动粘贴

#### 截图翻译
1. 按 `Cmd/Ctrl + Shift + A`
2. 框选截图区域
3. 查看 OCR 与译文结果

#### 截图讲解
1. 按 `Cmd/Ctrl + Shift + E`
2. 框选截图区域
3. 继续追问图像内容

#### 设置
点击翻译面板右上角齿轮 ⚙️：
- 管理 Provider / 模型 / 快捷键 / Prompt
- 测试 Provider 连通性 + 获取模型
- 开机启动开关
- 权限状态（macOS）
- 未保存更改关闭确认

### 💻 开发构建

基于 [Tauri v2](https://v2.tauri.app/)、[React](https://react.dev/)、[Vite](https://vitejs.dev/) 与 [TailwindCSS v4](https://tailwindcss.com/)。

```bash
# 安装依赖
npm install

# 本地开发（Tauri + UI）
npm run dev

# 仅前端开发
npm run dev:ui

# 打包构建
npm run build

# 仅构建前端
npm run build:ui

# 代码检查
npm run lint
```
