<p align="center">
  <img src="public/icon.png" width="120" height="120" alt="Kivio">
</p>

<h1 align="center">Kivio</h1>

<p align="center">
  <strong>A lightweight desktop AI client and screen-level agent: chat, tools, translation, OCR, and visual Q&A in one small app.</strong>
</p>

<p align="center">
  <a href="https://github.com/ZMGID/kivio/releases/latest"><img src="https://img.shields.io/github/v/release/ZMGID/kivio?style=flat-square&color=4f46e5&label=release" alt="Latest Release"></a>
  <img src="https://img.shields.io/badge/macOS-14%2B-success?style=flat-square" alt="macOS 14+">
  <img src="https://img.shields.io/badge/Windows-10%2F11-success?style=flat-square" alt="Windows 10/11">
  <img src="https://img.shields.io/badge/license-GPLv3-blue?style=flat-square" alt="GPL-3.0">
</p>

<p align="center">
  <a href="https://github.com/ZMGID/kivio/releases/latest"><strong>Download</strong></a>
  &nbsp;·&nbsp;
  <a href="#screenshots">Screenshots</a>
  &nbsp;·&nbsp;
  <a href="#changelog">Changelog</a>
  &nbsp;·&nbsp;
  <a href="#中文">中文</a>
  &nbsp;·&nbsp;
  QQ Group: <strong>1104450740</strong>
</p>

---

## What Kivio Is

Kivio is a compact desktop AI client with a built-in agent runtime. It stays in the tray or menu bar, opens when you need it, and keeps a much smaller footprint than a browser-based AI workspace.

- **Kivio Agent** — chat, projects, assistants, memory, file/image attachments, MCP, Skills, and native tools (file ops, shell, web fetch/search, Pyodide `run_python`).
- **Lens** — screenshot-based visual Q&A, OCR, formula extraction, text polishing, and optional web-aware answers.
- **Fast translation** — typed text, selected text, windows, and arbitrary screen regions.
- **Bring your own models** — OpenAI-compatible providers and Anthropic Messages, with per-feature model routing and multi-key failover.
- **Document-ready** — PDF / Word / Excel workflows run on a bundled offline Pyodide sandbox, no first-run CDN download required.
- **Private by default** — no telemetry; API keys and conversation data stay on your machine.

<a name="screenshots"></a>

## Screenshots

### Kivio Agent

<p align="center">
  <img src="docs/screenshots/chat-agent.png" width="840" alt="Kivio Agent chat workspace">
</p>

Start conversations, group work into projects, pick assistants, attach files, and call tools.

### Lens Visual Q&A

<p align="center">
  <img src="docs/screenshots/lens-formula-extraction.gif" width="760" alt="Lens formula extraction">
</p>

Capture a formula, chart, table, UI, or wall of text and ask follow-up questions with streamed answers and per-image history.

<p align="center">
  <img src="docs/screenshots/lens-optimize-text.gif" width="760" alt="Lens text optimization">
  <br>
  <sub>Capture text and ask AI to translate, rewrite, summarize, or polish it in place.</sub>
</p>

### Screenshot Translation

<p align="center">
  <img src="docs/screenshots/screenshot-translation.png" width="760" alt="Screenshot translation">
</p>

Capture a window or region and get a compact translation card near the selection.

### Settings

<p align="center">
  <img src="docs/screenshots/settings.png" width="560" alt="Kivio settings">
</p>

Configure providers, per-feature models, prompts, MCP servers, Skills, web search, memory, and tool approvals from one place.

## Hotkeys

| Action | macOS | Windows |
|---|---|---|
| Translator | `Command+Option+T` | `Ctrl+Alt+T` |
| Screenshot translate | `Command+Shift+A` | `Ctrl+Shift+A` |
| Selected text translate | `Command+Shift+T` | `Ctrl+Shift+T` |
| Lens capture & ask | `Command+Shift+G` | `Ctrl+Shift+G` |

All hotkeys are remappable in Settings. Kivio Agent can also be opened from the tray/menu bar.

## Quick Start

1. **[Download the latest release](https://github.com/ZMGID/kivio/releases/latest)** — macOS: Apple Silicon DMG; Windows: NSIS `.exe`.
2. **Install and launch.** macOS needs Accessibility and Screen Recording permissions for global hotkeys, paste-back, and capture. If macOS says `Kivio.app is damaged`, clear quarantine:
   ```bash
   sudo xattr -rd com.apple.quarantine /Applications/Kivio.app
   ```
3. **Add a provider** in Settings → Model Providers (OpenAI-compatible, Anthropic Messages, DeepSeek, SiliconFlow, Ollama Cloud, …).
4. **Pick your workflow** — open Kivio Agent for chat/tools/documents, or press a hotkey for translation or Lens.

<a name="changelog"></a>

## Changelog

- **v2.7.1** — Adds **Kivio Code**, a new terminal coding agent (Rust CLI/TUI, build/plan modes, context compaction, Pi-style tools, MCP/Skills); integrates external CLI agents (Claude Code, codex, pi, hermes) into Chat; rebuilds the assistants system with a dedicated Skill page; redesigns the chat title bar and polishes chat motion; and adds a Lens "Continue in AI client" handoff that syncs the full conversation. Apple Silicon DMG built locally; Windows NSIS via GitHub Actions.
- **v2.7.0** — Agentic platform release: a multi-agent / sub-agent runtime with Orchestrate mode, persistent MCP connections, a Skills system, long-term memory with `memory_search`, in-loop context compaction, and richer native tools. The Chat UI adds a mode pill, generated-file cards, and instant stop; macOS floating windows now appear over native fullscreen Spaces; and providers gain threshold-based multi-key failover plus a frontend test suite + CI.
- **v2.6.9** — Simplified native file tools to `write_file` + `edit_file` with windowed reads for large files; improved sidebar/search/reasoning display and `run_command` handling; stabilized the Windows chat frame; fixed Lens flashing the previous screenshot on handoff.
- **v2.6.8** — Added theme color presets, project workspace filesystem, better Agent planning/todo flows, Mermaid/timeline rendering, and usage stats; slimmer bundled Python sandbox; macOS built locally while Actions publishes the Windows NSIS `.exe`.
- **v2.6.7** — Restored the native-feeling Windows chat window frame, simplified Assistant Center so toolbar controls no longer collide with Windows window controls, and refreshed release packaging (local macOS DMG, Actions for Windows).

See [GitHub Releases](https://github.com/ZMGID/kivio/releases) for the full history. Kivio checks for updates on launch.

## Development

Built with Tauri v2, Rust, React 18, TypeScript, Vite, and TailwindCSS v4.

```bash
npm install
npm run dev      # run the app
npm run lint
npm run typecheck
cargo test --manifest-path src-tauri/Cargo.toml
```

Releases follow [docs/RELEASE_PACKAGING.md](docs/RELEASE_PACKAGING.md): the macOS Apple Silicon DMG is built locally and GitHub Actions publishes the Windows NSIS installer.

## License

GPL-3.0-or-later © ZM. See [LICENSE](LICENSE).

## Friends

- [LINUX DO](https://linux.do)
- QQ Group: 1104450740

---

<a name="中文"></a>

<h1 align="center">Kivio · 中文</h1>

<p align="center">
  <strong>轻量桌面 AI 客户端与屏幕级 Agent：聊天、工具、翻译、OCR、视觉问答，放进一个小应用。</strong>
</p>

<p align="center">
  <a href="https://github.com/ZMGID/kivio/releases/latest"><strong>下载</strong></a>
  &nbsp;·&nbsp;
  <a href="#截图">截图</a>
  &nbsp;·&nbsp;
  <a href="#更新日志">更新日志</a>
  &nbsp;·&nbsp;
  <a href="#kivio">English</a>
  &nbsp;·&nbsp;
  QQ 群：<strong>1104450740</strong>
</p>

---

## Kivio 是什么

Kivio 是一个小体积桌面 AI 客户端，内置 Agent 运行时。常驻托盘或菜单栏，需要时再出现，比浏览器里的 AI 工作台更轻。

- **Kivio Agent** —— 聊天、项目、助手、记忆、文件/图片附件、MCP、Skills、本地工具（文件操作、终端、网页抓取/搜索、Pyodide `run_python`）。
- **Lens** —— 基于截图的视觉问答、OCR、公式提取、文本优化，可选联网来源回答。
- **快速翻译** —— 输入文本、选中文本、窗口、任意屏幕区域。
- **自带模型选择权** —— 支持 OpenAI 兼容接口与 Anthropic Messages，可按功能分别路由模型，多 Key 故障转移。
- **文档可直接用** —— PDF / Word / Excel 工作流跑在随包的离线 Pyodide 沙箱，不依赖首次运行从 CDN 下载。
- **默认安静** —— 无遥测；API Key 与对话数据留在本机。

<a name="截图"></a>

## 截图

### Kivio Agent

<p align="center">
  <img src="docs/screenshots/chat-agent.png" width="840" alt="Kivio Agent 聊天工作区">
</p>

开对话、建项目、选助手、加附件、跑工具。

### Lens 视觉问答

<p align="center">
  <img src="docs/screenshots/lens-formula-extraction.gif" width="760" alt="Lens 公式提取">
</p>

截取公式、图表、表格、界面或大段文字后继续追问，支持流式回答与图片历史。

<p align="center">
  <img src="docs/screenshots/lens-optimize-text.gif" width="760" alt="Lens 文本优化">
  <br>
  <sub>截取文字，让 AI 当场翻译、改写、总结或润色。</sub>
</p>

### 截图翻译

<p align="center">
  <img src="docs/screenshots/screenshot-translation.png" width="760" alt="截图翻译">
</p>

截窗口或选区，译文卡片出现在选区附近。

### 设置

<p align="center">
  <img src="docs/screenshots/settings.png" width="560" alt="Kivio 设置">
</p>

服务商、功能模型、提示词、MCP、Skill、联网搜索、记忆、工具审批，集中管理。

## 热键

| 功能 | macOS | Windows |
|---|---|---|
| 翻译 | `Command+Option+T` | `Ctrl+Alt+T` |
| 截图翻译 | `Command+Shift+A` | `Ctrl+Shift+A` |
| 选中文本翻译 | `Command+Shift+T` | `Ctrl+Shift+T` |
| Lens 截图问答 | `Command+Shift+G` | `Ctrl+Shift+G` |

热键都可在设置里重绑。Kivio Agent 也可从托盘/菜单栏打开。

## 快速开始

1. **[下载最新版](https://github.com/ZMGID/kivio/releases/latest)** —— macOS：Apple Silicon DMG；Windows：NSIS `.exe`。
2. **安装并启动。** macOS 需授予辅助功能和屏幕录制权限（全局热键、粘回、截图）。若提示 `Kivio.app 已损坏`，清除隔离属性：
   ```bash
   sudo xattr -rd com.apple.quarantine /Applications/Kivio.app
   ```
3. **在设置 → 模型供应商里添加服务商**（OpenAI 兼容、Anthropic Messages、DeepSeek、SiliconFlow、Ollama Cloud 等）。
4. **选择工作流** —— 打开 Kivio Agent 做聊天/工具/文档，或按热键做翻译与 Lens。

<a name="更新日志"></a>

## 更新日志

- **v2.7.1** —— 新增全新终端编码 agent **Kivio Code**（Rust CLI/TUI、build/plan 模式、上下文压缩、Pi 风格工具、MCP/Skills）；Chat 接入外部 CLI agent（Claude Code、codex、pi、hermes）；重建助手系统并新增独立技能页；重做聊天顶栏并打磨动效；Lens 新增"在 AI 客户端继续"交接、同步完整对话。macOS DMG 本地构建，Windows NSIS 走 GitHub Actions。
- **v2.7.0** —— Agentic 平台版本：多代理/子代理运行时与 Orchestrate 模式、MCP 持久连接、Skills 系统、带 `memory_search` 的长期记忆、循环内上下文压缩、更强原生工具；Chat UI 新增模式 pill、生成文件卡片、即时停止；macOS 浮窗可浮现在原生全屏 Space 上方；并加入阈值化多 key 故障转移与前端测试 + CI。
- **v2.6.9** —— 原生文件工具精简为 `write_file` + `edit_file`，大文件支持窗口分段读取；优化侧边栏/搜索/推理块展示与 `run_command` 处理；稳定 Windows Chat 窗口边框；修复 Lens 交接后闪上次截图。
- **v2.6.8** —— 新增主题色预设、项目工作区文件系统、更好的 Agent 计划/待办流程、Mermaid/时间线渲染与用量统计；精简随包 Python 沙箱；macOS 本机构建，Actions 只发 Windows NSIS `.exe`。
- **v2.6.7** —— 恢复 Windows Chat 窗口接近原生的圆角与边框，精简助手中心避免与窗口按钮重叠，并刷新发布流程（macOS 本地 DMG，Windows 走 Actions）。

完整历史见 [GitHub Releases](https://github.com/ZMGID/kivio/releases)。Kivio 启动时会检查更新。

## 开发

技术栈：Tauri v2、Rust、React 18、TypeScript、Vite、TailwindCSS v4。

```bash
npm install
npm run dev      # 运行应用
npm run lint
npm run typecheck
cargo test --manifest-path src-tauri/Cargo.toml
```

发布流程见 [docs/RELEASE_PACKAGING.md](docs/RELEASE_PACKAGING.md)：macOS Apple Silicon DMG 本机构建，GitHub Actions 发布 Windows NSIS 安装包。

## 许可证

GPL-3.0-or-later © ZM。见 [LICENSE](LICENSE)。

## 友链

- [LINUX DO](https://linux.do)
- QQ 群：1104450740
