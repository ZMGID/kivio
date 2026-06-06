<p align="center">
  <img src="public/icon.png" width="120" height="120" alt="Kivio">
</p>

<h1 align="center">Kivio</h1>

<p align="center">
  <strong>A lightweight desktop AI client and screen-level agent for chat, tools, translation, OCR, and visual Q&A.</strong>
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

Kivio started as a small screen translation tool. It is now a compact desktop AI client with a built-in agent runtime:

- **Kivio Agent** for long-form chat, projects, assistants, memory, file attachments, MCP, Skills, and native tools.
- **Lens** for screenshot-based visual Q&A, OCR, formula extraction, text polishing, and web-aware answers.
- **Fast translation** for typed text, selected text, windows, and arbitrary screen regions.
- **Bring your own models** through OpenAI-compatible providers, Anthropic Messages, Apple local models, and provider-specific routing.

It stays quiet in the tray or menu bar, opens when you need it, and keeps the native footprint much smaller than a browser-based AI workspace.

## Highlights

- **Desktop agent workspace** — chat with models, organize projects, switch assistants, attach files and images, and keep useful memory.
- **Tool calling that feels local** — use built-in file tools, shell execution, web fetch/search, Pyodide `run_python`, MCP servers, and bundled Skills.
- **Document-ready Python sandbox** — PDF, Word, and Excel workflows can use packaged Pyodide plus common data/document libraries without depending on first-run CDN downloads.
- **Screen-first workflows** — translate selected text, OCR a screenshot, or ask Lens about a formula, chart, error dialog, code snippet, or UI.
- **Model routing** — choose separate models for chat, translation, OCR, Lens, vision pre-analysis, title generation, context compression, and image generation.
- **Provider resilience** — multi-provider, multi-key failover, retry controls, model metadata, and tool-capability defaults.
- **Private by default** — no telemetry. Your API keys and local conversation data stay on your machine.

<a name="screenshots"></a>

## Screenshots

### Kivio Agent

<p align="center">
  <img src="docs/screenshots/chat-agent.png" width="840" alt="Kivio Agent chat workspace">
</p>

Use Kivio as a normal desktop AI client: start conversations, group work into projects, pick assistants, attach files, call tools, and let the agent reason across local context.

### Lens Visual Q&A

<p align="center">
  <img src="docs/screenshots/lens-formula-extraction.gif" width="760" alt="Lens formula extraction">
</p>

Capture a formula, chart, table, UI, or wall of text, then ask follow-up questions with streamed answers and per-image history. Lens can optionally search the web and show sources inline.

<p align="center">
  <img src="docs/screenshots/lens-optimize-text.gif" width="760" alt="Lens text optimization">
  <br>
  <sub>Capture text and ask AI to translate, rewrite, summarize, or polish it in place.</sub>
</p>

### Screenshot Translation

<p align="center">
  <img src="docs/screenshots/screenshot-translation.png" width="760" alt="Screenshot translation">
</p>

Capture a window or region and get a compact translation card near the selection. If the text is already selectable, use the selected-text hotkey and skip the screenshot step.

### Settings

<p align="center">
  <img src="docs/screenshots/settings.png" width="560" alt="Kivio settings">
</p>

Configure providers, feature-specific models, prompts, MCP servers, Skills, web search, memory, tool approvals, and the Mixer from one place.

## Hotkeys

| Action | macOS | Windows |
|---|---|---|
| Translator | `Command+Option+T` | `Ctrl+Alt+T` |
| Screenshot translate | `Command+Shift+A` | `Ctrl+Shift+A` |
| Selected text translate | `Command+Shift+T` | `Ctrl+Shift+T` |
| Lens capture & ask | `Command+Shift+G` | `Ctrl+Shift+G` |

All translation and Lens hotkeys are remappable in Settings. Kivio Agent can also be opened from the tray/menu bar.

## Quick Start

1. **[Download the latest release](https://github.com/ZMGID/kivio/releases/latest)**.
   - macOS: choose the Apple Silicon DMG.
   - Windows: choose the MSI or NSIS `.exe` installer.
2. **Install and launch.**
   - macOS needs Accessibility and Screen Recording permissions for global hotkeys, paste-back, and screenshot capture.
   - If macOS says `Kivio.app is damaged and can't be opened`, remove the quarantine attribute and reopen:
     ```bash
     sudo xattr -rd com.apple.quarantine /Applications/Kivio.app
     ```
3. **Add a provider** in Settings -> Model Providers. Kivio works with OpenAI-compatible endpoints, Anthropic Messages, DeepSeek, SiliconFlow, Ollama Cloud, and other compatible services.
4. **Pick your workflow.**
   - Open Kivio Agent for chat, tools, documents, and projects.
   - Press a hotkey for translation or Lens.

## Kivio Agent

Kivio Agent is the main desktop AI workspace:

- **Conversations and projects** — keep chats grouped by project, use the sidebar for recent work, and switch models per conversation.
- **Assistant Center** — create reusable assistants with prompts, tool presets, and task-specific behavior.
- **Memory** — maintain local memory layers that can be injected into chat context when enabled.
- **Attachments** — send images and readable local files; image chat can automatically route through vision-capable models.
- **Tools** — enable built-in native tools, MCP servers, Skill workflows, web search/fetch, and Pyodide Python.
- **Tool approvals** — keep sensitive actions such as file writes or shell commands behind confirmation while allowing safe reads and analysis to run smoothly.
- **Streaming and reasoning** — view progressive responses, reasoning blocks, tool cards, and failures without losing the conversation.

## Lens And Translation

Lens remains the fastest path from screen content to an answer:

- Screenshot a region/window and ask questions about what you see.
- Translate screenshots with native OCR: Apple Vision on macOS and `Windows.Media.Ocr` on Windows.
- Translate selected text directly with a separate hotkey.
- Use optional Tavily or Exa search when the answer needs current facts or external context.
- Keep screenshot history and follow-up context for visual conversations.

## Model And Tool Settings

Important settings:

- **Model Providers** — provider list, API keys, enabled models, metadata, and connection testing.
- **Default Models** — separate defaults for chat, translation, screenshot translation, Lens, vision, title summary, compression, and image generation.
- **Mixer** — route side tasks to smaller or specialized models while keeping the main chat model unchanged.
- **Tools & Extensions** — native tools, MCP servers, Skills, web search, Python sandbox, tool limits, and approval policy.
- **Memory** — enable or edit local memory layers used by Kivio Agent.
- **Prompts** — tune feature prompts and the base Chat system prompt.

## Upgrading From KeyLingo

If you used v2.4.4 or earlier under the old **KeyLingo** name, Kivio migrates your settings, API keys, and Lens history on first launch. The old `KeyLingo.app` in `/Applications` can be deleted manually because macOS treats the renamed bundle as a separate app.

<a name="changelog"></a>

## Changelog

- **v2.6.7** — General polish release: restored the native-feeling Windows chat window frame with rounded corners and border, simplified Assistant Center so toolbar controls no longer collide with Windows window controls, and refreshed release packaging so macOS Apple Silicon DMG is built locally while GitHub Actions publishes Windows MSI/NSIS installers.
- **v2.6.6** — Major Kivio Agent refresh: added local memory, expanded Assistant Center behavior, improved projects/sidebar polish, image/file attachment handling, image preview/viewer support, and a stronger Agent runtime for tool planning and image-generation side tasks. Document workflows now ship with bundled Pyodide, common data/document packages, and looser readable-file mounting for PDF/Word/Excel analysis. Provider tool support is assumed by default.
- **v2.6.5** — Packaged the first full Kivio Agent wave: Chat client polish, MCP/Skill/native tool integration, bundled `pdf`/`docx`/`xlsx` Skills, document workflow improvements, Mixer auxiliary model routing, better tool/error display, and more stable Windows/macOS chat window chrome.
- **v2.6.3** — Lens stability release: Esc close behavior is more reliable, screenshot follow-up context no longer repeats the Lens prompt on every turn, answer panels have better scroll room, and Settings hotkey editing handles Esc/Enter/save/clear flows more cleanly.
- **v2.6.2** — Lens gained optional web search with Tavily and Exa, inline source blocks, and search-aware answers for current or ambiguous screen content. Settings opening and provider/model configuration were also smoothed out.

See [GitHub Releases](https://github.com/ZMGID/kivio/releases) for the full history. Kivio checks for updates on launch and points you to the latest release.

## Development

Built with Tauri v2, Rust, React 18, TypeScript, Vite, and TailwindCSS v4.

```bash
npm install
npm run dev
```

Useful commands:

```bash
npm run lint
npm run typecheck
cargo test --manifest-path src-tauri/Cargo.toml
```

### Release Packaging Requirements

- If `pdf`, `docx`, and `xlsx` Skills are bundled, their Python/Pyodide runtime must be bundled too.
- Installers must include Pyodide core files, `python_stdlib.zip`, `pyodide-lock.json`, and local wheels for common packages such as `numpy`, `pandas`, `matplotlib`, `scipy`, `sympy`, `scikit-learn`, `statsmodels`, `pillow`, `seaborn`, `openpyxl`, `xlrd`, `et_xmlfile`, and `micropip`.
- `run_python` must prefer packaged local Pyodide resources. CDN loading is only a fallback.
- Before publishing, inspect the final DMG / MSI / NSIS artifacts and verify both bundled Skills and the Python/Pyodide runtime resources are present.
- Current release flow builds the macOS Apple Silicon DMG locally and lets GitHub Actions publish the Windows MSI / NSIS installers.
- Follow [docs/RELEASE_PACKAGING.md](docs/RELEASE_PACKAGING.md) instead of releasing from memory.

## License

MIT © ZM

## Friends

- [LINUX DO](https://linux.do)
- QQ Group: 1104450740

---

<a name="中文"></a>

<h1 align="center">Kivio · 中文</h1>

<p align="center">
  <strong>轻量桌面 AI 客户端与屏幕级 Agent：聊天、工具、翻译、OCR、视觉问答，一起放进一个小应用。</strong>
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

Kivio 最早是一个轻量屏幕翻译工具。现在它已经演进成一个小体积桌面 AI 客户端，内置 Agent 运行时：

- **Kivio Agent**：长对话、项目、助手、记忆、文件附件、MCP、Skill、本地工具。
- **Lens**：基于截图的视觉问答、OCR、公式提取、文本优化、联网来源回答。
- **快速翻译**：输入文本、选中文本、窗口截图、屏幕区域都可以翻译。
- **自带模型选择权**：支持 OpenAI 兼容接口、Anthropic Messages、Apple 本地模型，以及按功能路由模型。

它常驻托盘或菜单栏，需要时再出现；比浏览器里的 AI 工作台更轻，也更贴近桌面操作。

## 主要能力

- **桌面 Agent 工作区** —— 对话、项目、助手、文件/图片附件、本地记忆。
- **本地感很强的工具调用** —— 内置文件工具、终端执行、网页抓取/搜索、Pyodide `run_python`、MCP 服务和内置 Skills。
- **文档分析可直接用** —— PDF、Word、Excel 工作流随包带 Pyodide 和常用数据/文档库，不依赖首次运行时临时从 CDN 下载。
- **屏幕优先** —— 选中文本翻译、截图 OCR、Lens 问公式/图表/报错/代码/UI。
- **模型路由** —— Chat、翻译、OCR、Lens、视觉预分析、标题总结、上下文压缩、图片生成都可以分别选模型。
- **供应商容灾** —— 多服务商、多 Key、失败重试、模型元数据、工具能力默认开启。
- **默认安静** —— 无遥测。API Key 和本地对话数据留在你的机器上。

<a name="截图"></a>

## 截图

### Kivio Agent

<p align="center">
  <img src="docs/screenshots/chat-agent.png" width="840" alt="Kivio Agent 聊天工作区">
</p>

把 Kivio 当成正常桌面 AI 客户端使用：开对话、建项目、选助手、加附件、跑工具，让 Agent 带着本地上下文完成任务。

### Lens 视觉问答

<p align="center">
  <img src="docs/screenshots/lens-formula-extraction.gif" width="760" alt="Lens 公式提取">
</p>

截取公式、图表、表格、界面或大段文字后继续追问。Lens 支持流式回答、图片历史，也可以按需联网搜索并在回答里展示来源。

<p align="center">
  <img src="docs/screenshots/lens-optimize-text.gif" width="760" alt="Lens 文本优化">
  <br>
  <sub>截取文字，让 AI 当场翻译、改写、总结或润色。</sub>
</p>

### 截图翻译

<p align="center">
  <img src="docs/screenshots/screenshot-translation.png" width="760" alt="截图翻译">
</p>

截窗口或选区，译文卡片会出现在选区附近。如果文字本身可选，直接用选中文本热键即可，不必截图。

### 设置

<p align="center">
  <img src="docs/screenshots/settings.png" width="560" alt="Kivio 设置">
</p>

服务商、功能模型、提示词、MCP、Skill、联网搜索、记忆、工具审批、Mixer 都在设置里集中管理。

## 热键

| 功能 | macOS | Windows |
|---|---|---|
| 翻译 | `Command+Option+T` | `Ctrl+Alt+T` |
| 截图翻译 | `Command+Shift+A` | `Ctrl+Shift+A` |
| 选中文本翻译 | `Command+Shift+T` | `Ctrl+Shift+T` |
| Lens 截图问答 | `Command+Shift+G` | `Ctrl+Shift+G` |

翻译和 Lens 热键都可在设置里重绑。Kivio Agent 也可以从托盘/菜单栏打开。

## 快速开始

1. **[下载最新版](https://github.com/ZMGID/kivio/releases/latest)**。
   - macOS：选择 Apple Silicon DMG。
   - Windows：选择 MSI 或 NSIS `.exe` 安装包。
2. **安装并启动**。
   - macOS 需要授予辅助功能和屏幕录制权限，用于全局热键、粘回原应用和截图捕获。
   - 如果 macOS 提示 `Kivio.app 已损坏，无法打开`，执行下面命令后重新打开：
     ```bash
     sudo xattr -rd com.apple.quarantine /Applications/Kivio.app
     ```
3. **在设置 -> 模型供应商里添加服务商**。支持 OpenAI 兼容接口、Anthropic Messages、DeepSeek、SiliconFlow、Ollama Cloud 等。
4. **选择你的工作流**。
   - 打开 Kivio Agent 做聊天、工具、文档和项目。
   - 按热键做翻译或 Lens 截图问答。

## Kivio Agent

Kivio Agent 是主要的桌面 AI 工作区：

- **对话和项目** —— 用项目组织聊天，侧边栏管理最近工作，每个对话可以切换模型。
- **助手中心** —— 创建可复用助手，配置提示词、工具预设和任务行为。
- **记忆** —— 本地维护记忆层，开启后注入 Chat 上下文。
- **附件** —— 发送图片和可读取的本地文件；图片对话可自动走视觉模型。
- **工具** —— 启用内置工具、MCP 服务、Skill 工作流、联网搜索/抓取、Pyodide Python。
- **工具审批** —— 写文件、改文件、运行命令等敏感动作保留确认；读取和分析类任务可以更顺畅地执行。
- **流式和思考** —— 渐进显示回答、思考块、工具卡片和错误信息，不丢上下文。

## Lens 与翻译

Lens 仍然是从屏幕内容到答案的最快路径：

- 截取屏幕区域或窗口后，对看到的内容直接提问。
- 截图翻译使用系统 OCR：macOS 是 Apple Vision，Windows 是 `Windows.Media.Ocr`。
- 选中文本可直接翻译，不用截图。
- 遇到需要实时信息或外部上下文的问题，可选 Tavily / Exa 联网搜索。
- 保留截图历史和追问上下文，适合视觉对话。

## 模型与工具设置

重点设置项：

- **模型供应商** —— 服务商列表、API Key、启用模型、模型元数据、连接测试。
- **默认模型** —— Chat、翻译、截图翻译、Lens、视觉、标题总结、上下文压缩、图片生成都可单独设置。
- **Mixer** —— 把副任务交给更小或更专用的模型，主对话模型保持不变。
- **工具与扩展** —— 内置工具、MCP、Skill、联网搜索、Python 沙箱、工具轮次和审批策略。
- **记忆** —— 开启或编辑 Kivio Agent 使用的本地记忆层。
- **提示词** —— 调整各功能提示词和 Chat 基础系统提示词。

## 从 KeyLingo 升级

如果你之前用的是 v2.4.4 或更早的 **KeyLingo**，Kivio 首次启动会自动迁移设置、API Key 和 Lens 历史。`/Applications` 下旧的 `KeyLingo.app` 可以手动删除，因为 macOS 会把改名后的应用当成另一个 app。

<a name="更新日志"></a>

## 更新日志

- **v2.6.7** —— 通用体验优化：恢复 Windows Chat 窗口接近原生应用的圆角、描边和边界效果；精简助手中心顶部工具栏，避免搜索、创建等控件和 Windows 右上角窗口按钮重叠；同步发布流程，macOS Apple Silicon DMG 改为本机构建上传，GitHub Actions 只发布 Windows MSI / NSIS 安装包。
- **v2.6.6** —— Kivio Agent 大更新：新增本地记忆，扩展助手中心能力，优化项目/侧边栏体验，改进图片与文件附件、图片查看器，以及面向工具规划和图片生成副任务的 Agent 运行时。文档工作流随包带 Pyodide、常用数据/文档库，并放宽可读取本地文件挂载，PDF / Word / Excel 分析更稳。模型供应商默认支持工具调用。
- **v2.6.5** —— 打包第一波完整 Kivio Agent 能力：Chat 客户端体验、MCP / Skill / 内置工具、内置 `pdf` / `docx` / `xlsx` Skills、文档工作流、Mixer 辅助模型路由、工具错误展示，以及更稳定的 Windows / macOS Chat 窗口外观。
- **v2.6.3** —— Lens 稳定性版本：Esc 关闭更可靠，截图追问不再每轮重复注入 Lens 提问提示词，回答区域滚动空间更合理，设置页热键编辑的 Esc / Enter / 保存 / 清空流程更稳。
- **v2.6.2** —— Lens 新增 Tavily / Exa 联网搜索、内联来源块和带来源的回答，适合需要实时信息或外部上下文的屏幕问题；同时优化设置窗口打开和服务商/模型配置体验。

完整历史见 [GitHub Releases](https://github.com/ZMGID/kivio/releases)。Kivio 启动时会检查更新，并指向最新版本。

## 开发

技术栈：Tauri v2、Rust、React 18、TypeScript、Vite、TailwindCSS v4。

```bash
npm install
npm run dev
```

常用检查：

```bash
npm run lint
npm run typecheck
cargo test --manifest-path src-tauri/Cargo.toml
```

### 发布打包要求

- 如果内置 `pdf`、`docx`、`xlsx` Skills，必须同时内置它们依赖的 Python / Pyodide 运行时。
- 安装包必须包含 Pyodide 核心文件、`python_stdlib.zip`、`pyodide-lock.json`，以及 `numpy`、`pandas`、`matplotlib`、`scipy`、`sympy`、`scikit-learn`、`statsmodels`、`pillow`、`seaborn`、`openpyxl`、`xlrd`、`et_xmlfile`、`micropip` 等本地 wheels。
- `run_python` 必须优先使用随包 Pyodide 资源；CDN 只能作为兜底。
- 发布前必须检查最终 DMG / MSI / NSIS，确认 Skills 和 Python / Pyodide 运行时资源都在安装包里。
- 当前发布流程是在本机构建并上传 macOS Apple Silicon DMG，GitHub Actions 只发布 Windows MSI / NSIS 安装包。
- 具体流程见 [docs/RELEASE_PACKAGING.md](docs/RELEASE_PACKAGING.md)，不要凭记忆发版。

## 许可证

MIT © ZM

## 友链

- [LINUX DO](https://linux.do)
- QQ 群：1104450740
