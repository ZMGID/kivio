# KeyLingo v2.1.0

<p align="center">
  <img src="public/icon.png" width="128" height="128" alt="KeyLingo Icon">
</p>

[English](#english) | [中文](#中文)

---

<a name="english"></a>
## 🇬🇧 English

**KeyLingo** is a lightweight translation and AI vision utility for **macOS** and **Windows**. It focuses on a **small package size** and **low runtime usage**, while providing instant translation, screenshot translation, screenshot explanation, and a Claude-Desktop-style **Cowork** mode through global shortcuts.

### ✨ Key Features

- **Global Hotkeys**
  - Translator: `Cmd/Ctrl+Alt+T`
  - Screenshot Translation: `Cmd/Ctrl+Shift+A`
  - Cowork (capture + multi-turn ask): `Cmd/Ctrl+Shift+G`
- **Cowork Mode**: Hover-highlight an app window or drag a region; a single floating bar flies into the capture spot, with a streamed answer + multi-turn chat expanding underneath. Per-image history dropdown. Pure-text questions also work without a screenshot.
- **OpenAI-Compatible Providers**: OpenAI / DeepSeek / SiliconFlow / custom compatible endpoints.
- **Multi-Provider Routing**: Separate providers/models for Translation, OCR, and Cowork.
- **Screenshot Translation**: OCR + translation pipeline with optional direct-translate mode.
- **LaTeX Math Adaptation**: Better readability for formula-heavy outputs.
- **Auto Paste**: Enter to paste translated content back to your active app.
- **Launch at Startup**: Optional startup toggle, default **off**.
- **Redesigned Settings UI**: Sidebar navigation, card-based modules, brand-orange accent, custom scrollbar, and a dedicated Cowork settings tab.
- **Provider Diagnostics**: One-click **Test Connection** + **Fetch Models**.
- **Permission Dashboard (macOS)**: Accessibility/Screen Recording status + deep-link to System Settings.

### 📦 Release Assets (v2.1.0)

- **macOS (Apple Silicon)**: `KeyLingo_2.1.0_aarch64.dmg`
- **Windows Installer (NSIS)**: `KeyLingo_2.1.0_x64-setup.exe`
- **Windows Installer (MSI)**: `KeyLingo_2.1.0_x64_en-US.msi`

### 📚 Detailed Changelog

#### v2.1.0 (2026-04-28)

- **Cowork Mode (new feature)** — `Cmd/Ctrl+Shift+G`
  - Greyscale overlay with hover-highlight on app windows, click-to-capture or drag-region.
  - Single fixed-position prompt bar that flies into the capture area via CSS transition; the captured frame keeps an orange ring + app label as a visual hint.
  - Smart anchor positioning (below / right / left / above) so the dialog and answer area never get clipped by screen edges.
  - Streamed answer panel with copy / stop, Esc cancels stream then closes.
  - Pure-text quick ask without screenshot also supported from the same bottom bar.
  - Dedicated Cowork settings tab: enable, hotkey, provider/model, response language, streaming toggle, and custom system / question prompts (each falls back to Explain when blank).
- **Screenshot Explain reuses Cowork capture (macOS)**
  - Triggers the same select overlay (hover-highlight + drag-region) instead of `screencapture -i`.
  - The explain window is placed beside the captured area using the same smart anchor algorithm.
  - Streaming on/off toggle, copy / stop / regenerate buttons, auto-summary toggle, cancellable streams.
- **Settings UI Overhaul**
  - Sidebar layout replaces top tabs; each module renders as a card with brand-orange accent and a custom scrollbar.
  - New Cowork tab; Explain tab gains streaming, auto-summary, and full custom-prompt slots.
  - Fixed a hotkey-recording state bug where the recorded value could be overwritten on re-render.
- **Translator main window polish**
  - Frosted-glass surface, refined shadow / corner radius, full bilingual i18n.
- **Stability Fixes**
  - Fixed multiple settings panes appearing when opening Settings while a screenshot/explain window was active (event broadcast → targeted emit).
  - Fixed screenshot model not staying in sync with provider `enabledModels`; settings fallback paths hardened.
  - Fixed Explain window blanking, sizing flicker, loading animation glitches, and model-name display.
  - Fixed the launch white-screen and improved multi-monitor screenshot positioning.
  - Cowork captures now keep the webview visible end-to-end (no hide/show flash for window-id capture; shorter hide for region capture).
- **Release Engineering**
  - GitHub Actions release workflow now supports building from a custom ref via `workflow_dispatch`.

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

#### Cowork
1. Press `Cmd/Ctrl + Shift + G`
2. Hover an app window (one-click capture) or drag to crop, **or** type a question directly into the bottom bar without screenshotting.
3. The prompt bar flies into the captured area; the streamed answer expands beneath it.
4. Esc cancels the stream and closes; clicking another app auto-cancels the select state.

#### Settings
Click gear icon ⚙️ in translator panel:
- Sidebar tabs: General / Translate / Screenshot / Cowork / Models / About
- Manage providers, models, hotkeys, prompts
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

**KeyLingo** 是一款面向 **macOS** 与 **Windows** 的轻量化翻译与 AI 视觉工具。产品核心方向是 **小体积、低占用**，并通过全局快捷键提供即时文本翻译、截图翻译，以及对标 Claude Desktop 的 **Cowork** 快速提问 + 多轮对话模式。

### ✨ 核心功能

- **全局快捷键**
  - 文本翻译：`Cmd/Ctrl+Alt+T`
  - 截图翻译：`Cmd/Ctrl+Shift+A`
  - Cowork（截图 + 多轮提问）：`Cmd/Ctrl+Shift+G`
- **Cowork 模式**：hover 应用窗口高亮 / 拖动选区截图，对话栏 CSS 飞入选区附近，下方展开流式答案 + 多轮 chat。每张截图独立历史 dropdown。也支持纯文字直接提问。
- **OpenAI 兼容生态**：支持 OpenAI / DeepSeek / SiliconFlow 及兼容接口。
- **多 Provider 路由**：翻译、OCR、Cowork 可分别指定服务商与模型。
- **截图翻译增强**：支持 OCR+翻译流程，并可切换直译模式。
- **LaTeX 数学公式适配**：优化公式展示与可读性。
- **自动粘贴**：回车即可回填到当前应用。
- **开机启动**：可选开关，默认关闭。
- **设置 UI 重构**：侧栏导航 + 卡片式模块 + 品牌橙主色 + 自定义滚动条；专属 Cowork 设置 tab。
- **供应商诊断**：模型页支持一键“测试连接”与“获取模型列表”。
- **权限状态面板（macOS）**：可视化查看辅助功能/屏幕录制授权并直达系统设置。

### 📦 安装包（v2.1.0）

- **macOS（Apple Silicon）**：`KeyLingo_2.1.0_aarch64.dmg`
- **Windows NSIS 安装包**：`KeyLingo_2.1.0_x64-setup.exe`
- **Windows MSI 安装包**：`KeyLingo_2.1.0_x64_en-US.msi`

### 📚 详细更新目录

#### v2.1.0（2026-04-28）

- **Cowork 模式（新功能）** — `Cmd/Ctrl+Shift+G`
  - 屏幕变灰 + hover 应用窗口高亮（橙色描边 + 标签）+ 单击整窗截图 / 拖动选区。
  - 单一对话栏通过 CSS transition 平滑飞入选区附近；截图框保留橙色边框 + 应用名作为已截视觉提示。
  - 智能锚点定位（下 / 右 / 左 / 上 优先级），不被屏幕边缘遮挡。
  - 流式答案区下展开，支持复制 / 停止；Esc 流式时取消、否则关闭。
  - 底部对话栏支持纯文字直接提问（不截图也能用）。
  - 完整设置 tab：启用 / 热键 / 模型 / 响应语言 / 流式开关 / 自定义 system 与 question 提示词（留空时 fallback 到截图讲解）。
- **截图讲解复用 Cowork 截图（macOS）**
  - 不再调用 macOS 原生 `screencapture -i`，改走 Cowork 同款 select 态（hover 高亮 + 区域选择）。
  - 讲解窗口落位由智能锚点算法决定，紧贴选区不被裁剪。
  - 流式输出开关、复制 / 停止 / 重新生成、自动总结开关、可中断流式。
- **设置 UI 全面重做**
  - 侧栏导航代替顶部 tab，每个模块独立卡片，品牌橙主色 + 自定义滚动条。
  - 新增 Cowork tab；讲解 tab 加流式开关、自动总结、完整自定义提示词槽位。
  - 修复热键录制 state 在重渲染时被覆盖的 bug。
- **翻译器主窗口质感升级**
  - 磨砂玻璃 + 阴影 / 圆角细节 + 完整中英 i18n。
- **稳定性修复**
  - 修复打开设置时同时弹出多个设置面板（事件广播 → 定向 emit）。
  - 修复截图模型与 provider `enabledModels` 不同步、settings fallback 链路加固。
  - 修复讲解窗口空白、尺寸闪烁、loading 动画、模型名显示问题。
  - 修复启动白屏与多显示器截图位置。
  - Cowork 截图全程窗口不闪（整窗截图不再 hide webview，区域截图缩短 hide 时长）。
- **发布工具链**
  - GitHub Actions release workflow 支持通过 `workflow_dispatch` 指定任意 ref 构建。

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

#### Cowork
1. 按 `Cmd/Ctrl + Shift + G`
2. hover 应用窗口（单击截窗）或拖动框选；**也可不截图，直接在底部对话栏输入纯文字提问**。
3. 对话栏 CSS 飞到截图位置附近，下方展开流式答案。
4. Esc：流式中取消、否则关闭；点击其他应用自动收起 select 态。

#### 设置
点击翻译面板右上角齿轮 ⚙️：
- 侧栏分类：通用 / 翻译 / 截图 / Cowork / 模型 / 关于
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
