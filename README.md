<p align="center">
  <img src="public/icon.png" width="120" height="120" alt="Kivio">
</p>

<h1 align="center">Kivio</h1>

<p align="center">
  <strong>Lightweight screen-level AI assistant for instant translation, screenshot OCR, and visual Q&A.</strong>
</p>

<p align="center">
  <a href="https://github.com/ZMGID/kivio/releases/latest"><img src="https://img.shields.io/github/v/release/ZMGID/kivio?style=flat-square&color=4f46e5&label=release" alt="Latest Release"></a>
  <img src="https://img.shields.io/badge/macOS-14%2B-success?style=flat-square" alt="macOS 14+">
  <img src="https://img.shields.io/badge/Windows-10%2F11-success?style=flat-square" alt="Windows 10/11">
  <img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="MIT">
</p>

<p align="center">
  <a href="https://github.com/ZMGID/kivio/releases/latest"><strong>Download</strong></a>
  &nbsp;·&nbsp;
  <a href="#中文">中文</a>
  &nbsp;·&nbsp;
  QQ Group: <strong>1104450740</strong>
</p>

---

## What it does

Kivio stays in the menu bar and appears only when you press a hotkey.

- 🌐 **Translate text anywhere** — type, translate, press Enter to paste back.
- 📸 **Translate screenshots or selected text** — capture a window/region, or highlight text and use the selected-text hotkey.
- 🔍 **Ask Lens about your screen** — capture visual context and chat with the model in a focused floating window.

Small bundle, low memory, no telemetry, and native system OCR where available.

### Screenshot translation

<p align="center">
  <img src="docs/screenshots/screenshot-translation.png" width="720" alt="Screenshot translation" onerror="this.style.display='none'">
</p>

Capture any window or region and get a compact translation card near your selection. Prefer text selection? Highlight text anywhere and press the selected-text hotkey; no screenshot needed.

### Lens — ask anything about your screen

<p align="center">
  <img src="docs/screenshots/lens-formula-extraction.gif" width="720" alt="Lens formula extraction" onerror="this.style.display='none'">
</p>

Capture a formula, a chart, or a wall of text, then ask follow-up questions with streamed answers and per-image history.

<p align="center">
  <img src="docs/screenshots/lens-optimize-text.gif" width="720" alt="Lens text optimization" onerror="this.style.display='none'">
  <br>
  <sub>Capture text and ask AI to optimize or translate it on the spot</sub>
</p>

### Settings

<p align="center">
  <img src="docs/screenshots/settings.png" width="480" alt="Settings" onerror="this.style.display='none'">
</p>

## Hotkeys

| Action | macOS | Windows |
|---|---|---|
| Translator | `⌘⌥T` | `Ctrl+Alt+T` |
| Screenshot translate | `⌘⇧A` | `Ctrl+Shift+A` |
| Selected text translate | `⌘⇧T` | `Ctrl+Shift+T` |
| Lens (capture & ask) | `⌘⇧G` | `Ctrl+Shift+G` |

All remappable in Settings.

## Quick start

1. **[Download the latest release](https://github.com/ZMGID/kivio/releases/latest)** (DMG for macOS, MSI / NSIS for Windows).
2. **Install and launch.** On macOS, grant Accessibility + Screen Recording when prompted (System Settings → Privacy & Security).
   > **macOS note:** If you see "Kivio.app is damaged and can't be opened", it's Gatekeeper blocking the unsigned app. Run this in Terminal and reopen:
   > ```bash
   > sudo xattr -rd com.apple.quarantine /Applications/Kivio.app
   > ```
3. **Add your API key** — Settings → Providers. Works with OpenAI, DeepSeek, SiliconFlow, Ollama Cloud, or any OpenAI-compatible endpoint.
4. **Hit a hotkey.** That's it.

## Why pick this

- **Short screen workflows.** Translate, OCR, and ask without switching context.
- **Real system OCR.** Apple Vision on macOS, `Windows.Media.Ocr` on Windows.
- **Bring your own model.** Pick different OpenAI-compatible providers for translation, screenshot translation, and Lens.
- **Multi-key failover.** Add backup keys and Kivio rotates when a key is rate-limited or out of quota.
- **Quiet and light.** No telemetry, small installer, low idle footprint.

## Settings

Open from the menu bar icon. The important bits:

- **Providers** — multi-provider, multi-key, with a one-click test-connection.
- **Per-feature routing** — translate / screenshot OCR / Lens each pick their own model.
- **Prompts** — every feature has an editable template with `{lang}` and `{text}` placeholders.
- **Streaming + reasoning** — togglable per feature; off by default for screenshot translate (speed wins).

## Upgrading from KeyLingo

If you ran v2.4.4 or earlier under the old name **KeyLingo**, your settings, API keys, and Lens history are migrated automatically the first time Kivio launches — nothing to do. The legacy `KeyLingo.app` in `/Applications` can be safely deleted; it won't be auto-removed because macOS treats the renamed bundle as a different app.

## Changelog

- **v2.5.8** — Restored the smooth per-frame fly animation in floating-mode Lens (the v2.5.7 snap-at-end approach reintroduced a pre-existing flicker bug). To keep cumulative jitter under control, `lens_set_floating` on Windows now uses a single atomic `SetWindowPos` call instead of separate position+size calls, halving the DWM/WebView2 resize coordination work per frame.
- **v2.5.7** — Fixed cumulative jitter in floating-mode Lens (the path used when "Keep fullscreen after capture" is OFF): the answer bar's fly to the screenshot used to call SetWindowPos every frame and degraded after many open/close cycles. Now the bar flies inside the still-fullscreen overlay and the window snaps to floating in a single step at the end.
- **v2.5.6** — Fixed Lens white flash on first show (Windows) and stutter when the answer bar flies into position on the second capture. Local OCR output now collapses blank-line padding before translation, producing tighter result cards.
- **v2.5.5** — Screenshot translation now uses native system OCR: Apple Vision on macOS and `Windows.Media.Ocr` on Windows. macOS OCR runs through a lightweight helper and no longer depends on Apple Intelligence.
- **v2.5.4** — Added selected-text screenshot translation: highlight text and press `⌘⇧T` / `Ctrl+Shift+T` to open the translation card directly.
- **v2.5.3** — Fixed Lens selection edge cases around the chat bar, history panel, and restored capture frames.
- **v2.5.2** — Added one-click tray access to Lens, source/render switching, empty-message screenshot analysis, and lazy Apple sidecar startup.
- **v2.5.1** — Added red-arrow screenshot annotations for Lens and improved streaming failover, send cancellation, and bundle cleanup.

See [GitHub Releases](https://github.com/ZMGID/kivio/releases) for the full history. Auto-update checks for new versions on launch and points you here.

## Development

Built with Tauri v2 + React 18 + TailwindCSS v4.

```bash
npm install
npm run dev   # full Tauri app (Rust backend + Vite UI)
```

PRs welcome. See `CLAUDE.md` and `AGENTS.md` for architecture notes.

## License

MIT © ZM

## Friends

- [LINUX DO](https://linux.do)
- QQ Group: 1104450740

---

<a name="中文"></a>

<h1 align="center">Kivio · 中文</h1>

<p align="center">
  <strong>轻量屏幕级 AI 助手：即时翻译、截图 OCR、视觉问答，全靠全局热键。</strong>
</p>

<p align="center">
  <a href="https://github.com/ZMGID/kivio/releases/latest"><strong>下载</strong></a>
  &nbsp;·&nbsp;
  <a href="#kivio">English</a>
  &nbsp;·&nbsp;
  QQ 群：<strong>1104450740</strong>
</p>

---

## 它能做什么

Kivio 常驻菜单栏，只在你按下热键时出现。

- 🌐 **随处翻译文本** —— 输入、翻译、回车粘回原应用
- 📸 **截图或选中文本翻译** —— 截窗口/选区，也能直接翻译已选文字
- 🔍 **用 Lens 问屏幕内容** —— 截取视觉上下文，在浮窗里和模型对话

安装包小、内存占用低、无遥测，并优先使用系统 OCR。

### 截图翻译

<p align="center">
  <img src="docs/screenshots/screenshot-translation.png" width="720" alt="截图翻译" onerror="this.style.display='none'">
</p>

截任意窗口或选区，译文卡片会出现在选区附近。只想翻译已选文字时，按选中文本热键即可，不需要截图。

### Lens — 对屏幕上的内容提问

<p align="center">
  <img src="docs/screenshots/lens-formula-extraction.gif" width="720" alt="Lens 公式提取" onerror="this.style.display='none'">
</p>

截下公式、图表或大段文字，继续追问。支持独立图片历史、流式回答和可选思考模式。

<p align="center">
  <img src="docs/screenshots/lens-optimize-text.gif" width="720" alt="Lens 文本优化" onerror="this.style.display='none'">
  <br>
  <sub>截取文字，让 AI 当场优化或翻译</sub>
</p>

### 设置面板

<p align="center">
  <img src="docs/screenshots/settings.png" width="480" alt="设置界面" onerror="this.style.display='none'">
</p>

## 热键

| 功能 | macOS | Windows |
|---|---|---|
| 翻译 | `⌘⌥T` | `Ctrl+Alt+T` |
| 截图翻译 | `⌘⇧A` | `Ctrl+Shift+A` |
| 选中文本翻译 | `⌘⇧T` | `Ctrl+Shift+T` |
| Lens（截图问答） | `⌘⇧G` | `Ctrl+Shift+G` |

热键全部可在设置中重绑。

## 快速上手

1. **[下载最新版](https://github.com/ZMGID/kivio/releases/latest)**（macOS 选 DMG，Windows 选 MSI 或 NSIS）
2. **安装并启动**。macOS 首次启动按提示授予辅助功能 + 屏幕录制权限（系统设置 → 隐私与安全性）
   > **macOS 提示：** 如果弹出「Kivio.app 已损坏，无法打开」，这是 Gatekeeper 拦截了未签名应用。在终端执行以下命令后重新打开即可：
   > ```bash
   > sudo xattr -rd com.apple.quarantine /Applications/Kivio.app
   > ```
3. **配置 API Key** —— 设置 → 服务商。支持 OpenAI、DeepSeek、SiliconFlow、Ollama Cloud，以及任何 OpenAI 兼容接口
4. **按热键**。就这样

## 为什么选它

- **屏幕流程短**。翻译、OCR、提问都不需要切走当前应用
- **真正系统 OCR**。macOS 用 Apple Vision，Windows 用 `Windows.Media.Ocr`
- **模型自己选**。翻译、截图翻译、Lens 可分别配置 OpenAI 兼容服务商
- **多 Key 容灾**。限流或额度耗尽时自动切到备用 Key
- **安静轻量**。无遥测，小安装包，低空闲占用

## 设置

从菜单栏图标打开。重点配置：

- **服务商** —— 多服务商、多 Key、一键测试连接
- **按功能分配** —— 翻译 / OCR / Lens 各自选自己的模型
- **提示词** —— 每个功能都有可编辑的模板，支持 `{lang}` 和 `{text}` 占位符
- **流式 + 思考模式** —— 按功能开关；截图翻译默认关闭思考（速度优先）

## 从 KeyLingo 升级

如果你之前用的是 v2.4.4 及更早的旧名 **KeyLingo**，第一次启动 Kivio 时会自动迁移设置、API Key 和 Lens 历史 —— 你不需要手动操作。`/Applications` 下旧的 `KeyLingo.app` 可以直接删掉；macOS 把改名后的应用视为不同 app，所以不会自动替换。

## 更新日志

- **v2.5.8** —— 恢复浮动模式 Lens 的逐帧平滑飞入动画（v2.5.7 的"末尾一次性 snap"方案重新引入了之前修过的"到位后闪一下"老毛病）。为防止累积抖动重现，Windows 上的 `lens_set_floating` 改成单次原子 `SetWindowPos`，把每帧的 DWM/WebView2 resize 协调工作砍一半。
- **v2.5.7** —— 修复浮动模式 Lens（关闭"截图后保持全屏覆盖"时走的那条路径）多次打开后悬浮栏飞入累积抖动的问题：以前 fly 期间每帧都调一次 SetWindowPos，多次开关 Lens 后 DWM 状态退化越发明显。改为 fly 期间窗口保持全屏、bar 在 webview 内 CSS 平滑飞入，fly 结束后一次性 snap 窗口到浮动尺寸。
- **v2.5.6** —— 修复 Windows 上首次打开 Lens 偶发的白屏闪烁，以及第二次截图时悬浮栏飞入抽搐的问题。本地 OCR 输出在送翻译前会先压缩多余空行，结果卡更紧凑。
- **v2.5.5** —— 截图翻译改用平台系统 OCR：macOS Apple Vision，Windows `Windows.Media.Ocr`。macOS OCR 独立 helper，不再依赖 Apple Intelligence。
- **v2.5.4** —— 新增选中文本翻译：选中文字后按 `⌘⇧T` / `Ctrl+Shift+T`，不用截图即可打开翻译卡片。
- **v2.5.3** —— 修复 Lens 选区、历史面板和恢复截图时的几个边界问题。
- **v2.5.2** —— 新增托盘左键打开 Lens、源码/渲染切换、空输入分析截图，并改为按需启动 Apple sidecar。
- **v2.5.1** —— Lens 支持红色箭头标注截图，并优化流式 failover、发送取消和安装包清理。

完整历史见 [GitHub Releases](https://github.com/ZMGID/kivio/releases)。应用启动时会自动检查更新，发现新版会指向这里。

## 开发

技术栈：Tauri v2 + React 18 + TailwindCSS v4。

```bash
npm install
npm run dev   # 启动完整 Tauri 应用（Rust 后端 + Vite UI）
```

欢迎 PR，架构说明见 `CLAUDE.md` 和 `AGENTS.md`。

## 许可证

MIT © ZM

## 友链

- [LINUX DO](https://linux.do)
- QQ 群：1104450740
