# KeyLingo v2.3.0

<p align="center">
  <img src="public/icon.png" width="128" height="128" alt="KeyLingo Icon">
</p>

[English](#english) | [中文](#中文)

---

<a name="english"></a>
## 🇬🇧 English

**KeyLingo** is a lightweight translation and AI vision utility for **macOS** and **Windows**. It focuses on a **small package size** and **low runtime usage**, while providing instant translation, screenshot translation, and a Claude-Desktop-style **Lens** mode through global shortcuts.

### ✨ Key Features

- **Global Hotkeys**
  - Translator: `Cmd/Ctrl+Alt+T`
  - Screenshot Translation: `Cmd/Ctrl+Shift+A`
  - Lens (capture + multi-turn ask): `Cmd/Ctrl+Shift+G`
- **Unified Capture Surface**: Both Screenshot Translation and Lens share the same SCK-powered capture overlay (hover an app window or drag a region) and the same floating result card flies into the captured spot.
- **Lens Mode**: Streamed answers with multi-turn chat, per-image history dropdown, optional reasoning chain (auto-collapsed once the answer arrives, with elapsed time + token estimate). Pure-text questions also work without a screenshot.
- **Screenshot Translation**: OCR + translation pipeline with token-by-token streaming, optional direct-translate mode, and a dedicated thinking-mode toggle (off by default for speed).
- **OpenAI-Compatible Providers**: OpenAI / DeepSeek / SiliconFlow / custom compatible endpoints.
- **Multi-Provider Routing**: Separate providers/models for Translation, OCR, and Lens.
- **LaTeX Math Adaptation**: Better readability for formula-heavy outputs.
- **Auto Paste**: Enter to paste translated content back to your active app.
- **Launch at Startup**: Optional startup toggle, default **off**.
- **Provider Diagnostics**: One-click **Test Connection** + **Fetch Models**.
- **Permission Dashboard (macOS)**: Accessibility/Screen Recording status + deep-link to System Settings.

### 📦 Release Assets (v2.3.0)

- **macOS (Apple Silicon)**: `KeyLingo_2.3.0_aarch64.dmg`
- **Windows Installer (NSIS)**: `KeyLingo_2.3.0_x64-setup.exe`
- **Windows Installer (MSI)**: `KeyLingo_2.3.0_x64_en-US.msi`

### 📚 Detailed Changelog

#### v2.3.0 (2026-04-29)

- **Renamed Cowork → Lens** across the codebase, settings tab, file names, Tauri commands (`lens_*`), event names (`lens-stream` / `lens-translate-stream`), CSS animations, capabilities whitelist, and UI strings. Existing `settings.json` migrates automatically via `#[serde(alias = "cowork")]`. Tab icon swapped from `Sparkles` to `Aperture` (semantic photographic-lens metaphor).
- **Screenshot translation: single-call combined prompt.** The vision model now reads the screenshot, outputs the translation, a `<<<ORIGINAL>>>` separator, then the original text — all in one API round-trip. Translation streams immediately (no longer blocks behind a finished OCR phase). UI reordered: translation on top, original below as small grey reference. New `stream_translate_combined` parses the separator across SSE chunks (UTF-8 safe, 13-byte tail buffer) so partial-marker boundaries don't bleed.
- **"Show original" toggle**: `screenshotTranslateMode` ("Translation Mode") reframed as `Show original` (default on). Backend field `direct_translate` unchanged; UI binding inverted, no migration needed.
- **Custom translate prompt now honored** in combined mode (was silently dropped via an unused `_template` arg).
- **Translator card shadow fix**: card filled the Tauri window edge-to-edge so `box-shadow` was clipped at every corner (no halo). Window 360×120 → 392×152, `.window-container` becomes a 16px transparent padding shell, `.window-frosted` drops to an inner div with a softer near-symmetric shadow (no more bottom-heavy halo). Border radius bumped 16 → 18px.
- **Translate result UI cleanup**: removed the "原文 / 译文" labels (font size + divider already convey hierarchy).
- **Cleanup**: dropped 8 dead i18n keys, removed `build_text_request_body` + `DEFAULT_OCR_PROMPT`, fixed 4 rustc lifetime warnings — `cargo check` now warning-free.

#### v2.2.0 (2026-04-29)

- **Screenshot Translation now reuses the Lens capture surface**
  - Pressing the screenshot-translation hotkey opens the same fullscreen overlay as Lens (hover-highlight or drag-region), captured via ScreenCaptureKit on macOS / xcap on Windows.
  - Result is a floating card next to the selection: original (small grey) on top, translation (main) below, both Markdown-rendered with selectable text.
  - Header badge shows live elapsed seconds + a rough token estimate; loading state uses shimmer placeholders instead of a spinner.
- **Streaming everywhere**
  - Lens answers parse `delta.reasoning_content` / `delta.reasoning` and surface the chain in a collapsible "Reasoning" block (auto-folded once the answer starts, header shows seconds + ~tokens).
  - Screenshot translation streams OCR and translation phases independently via `lens-translate-stream` events; tokens flow into the card as they arrive.
  - Pressing Esc during OCR now aborts before the translation request fires (used to translate the partial text silently).
- **Thinking-mode toggle**
  - Lens: default **on**. Off injects `thinking={type:"disabled"}` for compatible providers (DeepSeek / Kimi K2.6 official) and falls back to a no-think instruction in the system prompt plus a `/no_think` token for Qwen3 hybrid models.
  - Screenshot translation: default **off** (translation prioritises speed); same dual disable strategy when re-enabled by the user.
- **Lens message ordering**: New setting to flip chat order (asc / desc); descending pins the newest reply at the top, like Claude Desktop.
- **Capabilities fix**: Added `lens` to the Tauri `default` capability windows whitelist. Without it `event:listen` was silently rejected, which is why streaming and reasoning never reached the UI before this release.
- **Removed the standalone Screenshot Explain feature** — Lens supersedes it. The dedicated webview, nine `explain_*` commands, settings tab, history persistence, and ~700 lines of UI code are gone.
- **Bug fixes**
  - Stream-listener double-mount under React StrictMode no longer leaves a ghost listener (used to duplicate every character).
  - Translation second-stage failure now surfaces an error instead of leaving an empty card.
  - History restore cancels the running stream before swapping messages, so deltas can't bleed into the restored chat.
  - `Settings.tsx` default-object fallbacks now include `thinkingEnabled` / `streamEnabled` / `messageOrder`.
  - OCR request body aligned with Lens (image-before-text, temperature 0.2, explicit `max_tokens`, `stream:true`), fixing the K2.6 / strict-proxy "error sending request" failure caused by `temperature:0`.
- **Cleanup**: ~2120 net lines removed (dead capture overlay, screenshot result window, BusyGuard, anchor helpers, etc.).

#### v2.1.0 (2026-04-28)

- **Lens Mode (new feature)** — `Cmd/Ctrl+Shift+G`
  - Greyscale overlay with hover-highlight on app windows, click-to-capture or drag-region.
  - Single fixed-position prompt bar that flies into the capture area via CSS transition; the captured frame keeps an orange ring + app label as a visual hint.
  - Smart anchor positioning (below / right / left / above) so the dialog and answer area never get clipped by screen edges.
  - Streamed answer panel with copy / stop, Esc cancels stream then closes.
  - Pure-text quick ask without screenshot also supported from the same bottom bar.
  - Dedicated Lens settings tab: enable, hotkey, provider/model, response language, streaming toggle, and custom system / question prompts (each falls back to Explain when blank).
- **Screenshot Explain reuses Lens capture (macOS)**
  - Triggers the same select overlay (hover-highlight + drag-region) instead of `screencapture -i`.
  - The explain window is placed beside the captured area using the same smart anchor algorithm.
  - Streaming on/off toggle, copy / stop / regenerate buttons, auto-summary toggle, cancellable streams.
- **Settings UI Overhaul**
  - Sidebar layout replaces top tabs; each module renders as a card with brand-orange accent and a custom scrollbar.
  - New Lens tab; Explain tab gains streaming, auto-summary, and full custom-prompt slots.
  - Fixed a hotkey-recording state bug where the recorded value could be overwritten on re-render.
- **Translator main window polish**
  - Frosted-glass surface, refined shadow / corner radius, full bilingual i18n.
- **Stability Fixes**
  - Fixed multiple settings panes appearing when opening Settings while a screenshot/explain window was active (event broadcast → targeted emit).
  - Fixed screenshot model not staying in sync with provider `enabledModels`; settings fallback paths hardened.
  - Fixed Explain window blanking, sizing flicker, loading animation glitches, and model-name display.
  - Fixed the launch white-screen and improved multi-monitor screenshot positioning.
  - Lens captures now keep the webview visible end-to-end (no hide/show flash for window-id capture; shorter hide for region capture).
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

#### Lens
1. Press `Cmd/Ctrl + Shift + G`
2. Hover an app window (one-click capture) or drag to crop, **or** type a question directly into the bottom bar without screenshotting.
3. The prompt bar flies into the captured area; the streamed answer expands beneath it.
4. Esc cancels the stream and closes; clicking another app auto-cancels the select state.

#### Settings
Click gear icon ⚙️ in translator panel:
- Sidebar tabs: General / Translate / Screenshot / Lens / Models / About
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

**KeyLingo** 是一款面向 **macOS** 与 **Windows** 的轻量化翻译与 AI 视觉工具。产品核心方向是 **小体积、低占用**，并通过全局快捷键提供即时文本翻译、截图翻译，以及对标 Claude Desktop 的 **Lens** 快速提问 + 多轮对话模式。

### ✨ 核心功能

- **全局快捷键**
  - 文本翻译：`Cmd/Ctrl+Alt+T`
  - 截图翻译：`Cmd/Ctrl+Shift+A`
  - Lens（截图 + 多轮提问）：`Cmd/Ctrl+Shift+G`
- **统一截图 UI**：截图翻译和 Lens 共用同一套 SCK 截图选择层（hover 应用窗口或拖动选区），结果浮动卡同样飞入选区位置。
- **Lens 模式**：流式回答 + 多轮对话，每张截图独立历史 dropdown，可选思维链区块（正文出来后默认折叠，header 显示耗时 + token 估算）。也支持不截图直接纯文字提问。
- **截图翻译**：OCR + 翻译两步流式（token 逐字到达），可选直译模式，独立的"思考模式"开关（默认关闭追求速度）。
- **OpenAI 兼容生态**：支持 OpenAI / DeepSeek / SiliconFlow 及兼容接口。
- **多 Provider 路由**：翻译、OCR、Lens 可分别指定服务商与模型。
- **LaTeX 数学公式适配**：优化公式展示与可读性。
- **自动粘贴**：回车即可回填到当前应用。
- **开机启动**：可选开关，默认关闭。
- **供应商诊断**：模型页支持一键"测试连接"与"获取模型列表"。
- **权限状态面板（macOS）**：可视化查看辅助功能/屏幕录制授权并直达系统设置。

### 📦 安装包（v2.3.0）

- **macOS（Apple Silicon）**：`KeyLingo_2.3.0_aarch64.dmg`
- **Windows NSIS 安装包**：`KeyLingo_2.3.0_x64-setup.exe`
- **Windows MSI 安装包**：`KeyLingo_2.3.0_x64_en-US.msi`

### 📚 详细更新目录

#### v2.3.0（2026-04-29）

- **Cowork 重命名为 Lens**：覆盖代码、设置 tab、文件名、Tauri 命令（`lens_*`）、事件名（`lens-stream` / `lens-translate-stream`）、CSS 动画、capabilities 白名单和 UI 文案。旧 `settings.json` 通过 `#[serde(alias = "cowork")]` 自动迁移，不需要重配置。tab 图标 `Sparkles` → `Aperture`，更贴近"取景看屏"的语义。
- **截图翻译改为单次调用合并模式**：视觉模型一次性输出译文 + `<<<ORIGINAL>>>` 分隔符 + 原文，省一次 round-trip。译文先流出来（不再等 OCR 整段输出），UI 顺序也调整为译文在上、原文小字灰色作参考在下。新增 `stream_translate_combined` 在 SSE 分片边界做 UTF-8 安全的分隔符切分（13 字节 tail buffer 防止跨片误判）。
- **"显示原文" 开关**：`screenshotTranslateMode`（"翻译模式"）改名 "显示原文"，默认开。后端字段 `direct_translate` 不变，UI 绑定取反，无需迁移。
- **自定义翻译 prompt 在合并模式生效**（之前 `_template` 参数没人读，用户配的 prompt 被静默丢弃）。
- **翻译输入卡四角阴影修复**：之前卡片贴满 Tauri window 边缘，`box-shadow` 没空间向外渲染，四角看起来被硬切。窗口 360×120 → 392×152，`.window-container` 改为 16px 透明 padding 外壳，`.window-frosted` 移到内层 div，阴影换成两道近对称的柔和层（不再头重脚轻）。圆角 16 → 18px。
- **翻译结果卡 UI 简化**：去掉"原文 / 译文"标签（字号 + 分隔线已经能区分）。
- **代码清理**：删 8 个死 i18n key、`build_text_request_body` + `DEFAULT_OCR_PROMPT`、修 4 个 rustc lifetime warning，`cargo check` 现在零 warning。

#### v2.2.0（2026-04-29）

- **截图翻译复用 Lens 截图 UI**
  - 截图翻译热键现在打开 Lens 同款全屏选择层（hover 应用窗口或拖动选区），macOS 走 ScreenCaptureKit / Windows 走 xcap。
  - 截完结果是飞入选区旁的浮动卡片：上方原文（小灰）+ 下方译文（主色），都通过 ReactMarkdown 渲染，文字可选中复制。
  - Header 徽章实时显示耗时秒数 + token 估算；加载态用 shimmer 占位条代替单一的转圈 spinner。
- **全面流式**
  - Lens 后端解析 `delta.reasoning_content` / `delta.reasoning`，前端单独渲染可折叠的"思考过程"区块（正文出现后自动折叠，header 显示秒数 + ~tokens）。
  - 截图翻译两步分别流式：OCR 阶段 → 翻译阶段，事件 `lens-translate-stream` 携带 `{kind, delta, done}`，token 逐字到达卡片。
  - OCR 阶段按 Esc 现在会在翻译请求发出之前中断（之前会继续把残缺 OCR 翻译一遍）。
- **思考模式开关**
  - Lens：默认 **开**。关闭时注入 `thinking={type:"disabled"}`（DeepSeek / Kimi K2.6 官方支持），同时在 system prompt 末尾追加显式禁止指令 + user 消息后追加 `/no_think`（Qwen3 hybrid 识别）。
  - 截图翻译：默认 **关**（追求速度）；用户可在设置中开启，会同样应用上面的双层兜底。
- **Lens 消息顺序**：新增设置切换聊天排序（asc / desc），desc 把最新答案钉在顶部，类 Claude Desktop。
- **修复 capabilities**：`default.json` 的 windows 数组之前漏了 `lens`，导致 `event:listen` 静默失败 — 这是流式 + 思维链之前一直显示不出来的根因。
- **删除截图讲解整套功能** — Lens 完全替代之。专属 webview、9 个 `explain_*` 命令、设置 tab、历史持久化、~700 行 UI 代码全部移除。
- **BUG 修复**
  - React StrictMode 双挂导致流监听器残留幽灵 listener，每个字符重复（已通过 cancelled 旗标 + late-resolve dispose 修复）。
  - 翻译第二阶段失败之前会静默上报 success → 浮卡空白，现在会上报 error。
  - 历史项点击恢复时会先取消正在跑的流，避免 delta 灌到错误的对话。
  - `Settings.tsx` 默认对象 fallback 补齐 `thinkingEnabled` / `streamEnabled` / `messageOrder`。
  - OCR 请求体与 Lens 对齐（image-before-text、temperature 0.2、显式 max_tokens、stream:true），修复 K2.6 / 严格代理 "error sending request" 报错。
- **代码清理**：净删除约 2120 行死代码（旧的 capture overlay、screenshot result 窗口、BusyGuard、anchor 计算等）。

#### v2.1.0（2026-04-28）

- **Lens 模式（新功能）** — `Cmd/Ctrl+Shift+G`
  - 屏幕变灰 + hover 应用窗口高亮（橙色描边 + 标签）+ 单击整窗截图 / 拖动选区。
  - 单一对话栏通过 CSS transition 平滑飞入选区附近；截图框保留橙色边框 + 应用名作为已截视觉提示。
  - 智能锚点定位（下 / 右 / 左 / 上 优先级），不被屏幕边缘遮挡。
  - 流式答案区下展开，支持复制 / 停止；Esc 流式时取消、否则关闭。
  - 底部对话栏支持纯文字直接提问（不截图也能用）。
  - 完整设置 tab：启用 / 热键 / 模型 / 响应语言 / 流式开关 / 自定义 system 与 question 提示词（留空时 fallback 到截图讲解）。
- **截图讲解复用 Lens 截图（macOS）**
  - 不再调用 macOS 原生 `screencapture -i`，改走 Lens 同款 select 态（hover 高亮 + 区域选择）。
  - 讲解窗口落位由智能锚点算法决定，紧贴选区不被裁剪。
  - 流式输出开关、复制 / 停止 / 重新生成、自动总结开关、可中断流式。
- **设置 UI 全面重做**
  - 侧栏导航代替顶部 tab，每个模块独立卡片，品牌橙主色 + 自定义滚动条。
  - 新增 Lens tab；讲解 tab 加流式开关、自动总结、完整自定义提示词槽位。
  - 修复热键录制 state 在重渲染时被覆盖的 bug。
- **翻译器主窗口质感升级**
  - 磨砂玻璃 + 阴影 / 圆角细节 + 完整中英 i18n。
- **稳定性修复**
  - 修复打开设置时同时弹出多个设置面板（事件广播 → 定向 emit）。
  - 修复截图模型与 provider `enabledModels` 不同步、settings fallback 链路加固。
  - 修复讲解窗口空白、尺寸闪烁、loading 动画、模型名显示问题。
  - 修复启动白屏与多显示器截图位置。
  - Lens 截图全程窗口不闪（整窗截图不再 hide webview，区域截图缩短 hide 时长）。
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

#### Lens
1. 按 `Cmd/Ctrl + Shift + G`
2. hover 应用窗口（单击截窗）或拖动框选；**也可不截图，直接在底部对话栏输入纯文字提问**。
3. 对话栏 CSS 飞到截图位置附近，下方展开流式答案。
4. Esc：流式中取消、否则关闭；点击其他应用自动收起 select 态。

#### 设置
点击翻译面板右上角齿轮 ⚙️：
- 侧栏分类：通用 / 翻译 / 截图 / Lens / 模型 / 关于
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
