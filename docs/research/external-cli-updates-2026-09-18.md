# 外部 CLI 最新版本与 Kivio 适配复核（2026-09-18）

## 2026-09-19 补齐结果

审计中发现的产品侧缺口已逐项补齐并验证。这里的“补齐”指 Kivio 的安装/更新判断、启动参数、协议适配、错误处理和迁移逻辑已经覆盖当前生产版本；为避免消耗账户额度，本轮没有用真实 prompt 做付费模型回合，因此不能把它写成所有账号、供应商和模型组合的生成质量认证。

| CLI | 补齐状态 | 本轮验证 |
| --- | --- | --- |
| Claude Code 2.1.276 | 当前启动参数仍有效，更新源码验证基线 | 本机 `--help` 解析 Kivio 使用的 stream-json、partial、subagent、session 参数 |
| Codex CLI 0.155.0 | app-server 的 MCP form elicitation 已接到 Kivio 问题卡，unsupported shape 仍 fail-closed | 本机 `codex app-server --help`；Rust codec/session tests |
| Cursor Agent | 既有阻塞 ACP 扩展保持兼容 | 本机未安装，无法做 build 级真机握手；共享 ACP 回归通过 |
| OpenCode 1.18.31 | 无需宿主协议改动 | `opencode-ai@1.18.31 acp --help` 通过 |
| Gemini CLI 0.60.0 | 正式 `--acp` 路径仍有效 | `@google/gemini-cli@0.60.0 --help` 明确列出 `--acp` |
| Kimi Code 2.0.1 | 新 Kimi Code 路径无需迁移改动 | `@moonshot-ai/kimi-code@2.0.1 acp --help` 通过 |
| Pi 0.85.1 | 已对齐 | external_agents 全量回归通过 |
| Hermes 0.21.3 | 最新版查询改用 GitHub native release 名称，不再读取滞后的 PyPI；日历 tag 不会误当 CLI semver | 本机 `hermes acp --check` 通过；版本解析测试通过 |
| Grok Build 1.0.34 | 会话/探针加 `--no-auto-update`；新增 strict sandbox；MCP form elicitation 可交互 | 本机 1.0.34 strict ACP 无模型握手通过 |
| dsh 0.1.5-rc.2 | profile SDK 与 core 精确同版并自动迁移；适配新版 base storage、user-question waterfall、preset Host 依赖；关闭路径有界 | Kivio 私有 profile 已迁移到两个 `0.1.5-rc.2` SDK；真实 initialize/session-open/close 无模型握手通过 |
| Antigravity 1.2.6 | report 显式限时；解析 `AGY_ERROR`；退出码 3 给出结构化错误；remote-control 保持 terminal-only | 本机 `agy --print-timeout 3s -p /help` 通过；错误 fixture 测试通过 |

额外验证结果：`external_agents` Rust 套件 **733 passed、0 failed、45 ignored**；前端 AskUser 工具 **3 passed**；TypeScript `tsc --noEmit` 通过。ignored 项主要是会产生模型费用、要求特定登录态或要求未安装 Cursor 的真机测试。

## 补齐前审计结论（2026-09-18）

截至 2026-09-18，Kivio 注册了 11 个外部 CLI：Claude Code、Codex CLI、Cursor Agent、OpenCode、Gemini CLI、Kimi Code、Pi、Hermes、Grok Build、DeepSeek Harness（dsh）和 Antigravity CLI。Kivio **没有锁定这些 CLI 的运行版本**：它探测用户机器上的 `--version`，安装/更新时使用官方脚本或 npm `@latest`。因此下表的“Kivio 基线”表示代码、测试或既有复核中最后一次有明确证据的适配版本，并非依赖锁。

不能笼统地说“全部适配到最新版”。目前没有证据表明 11 个 CLI 的最新稳定版都存在破坏性不兼容，但有 3 个真实、可定位的缺口：

1. **Hermes 更新版本来源错误（P1）**：Kivio 用 PyPI `hermes-agent` 查询最新版，但官方原生安装器/GitHub 已到 0.21.3，PyPI 仍是 0.19.0；会误报更新状态。
2. **dsh 核心与 Kivio profile 桥依赖可能错代（P0/P1）**：dsh 已到 0.1.5-rc.2，Session V3 期间明确有兼容性破坏；Kivio profile 安装两个不带版本的 SDK 包，而且只检查目录是否存在。npm 的 `latest` 仍分别落在旧 0.0.1 RC，已有 profile 也不会自动刷新，不能据现有测试声称已适配 0.1.5。
3. **Grok 的新版运行约束尚未接入（P1/P2）**：1.0.14 起支持 `--sandbox strict`，Kivio 仍只有“询问/完全放行”；官方还建议 headless/ACP 使用 `--no-auto-update`，Kivio 的 Grok 会话和模型探针没有传该参数。

另外，Claude、Codex、Cursor、Kimi、Gemini、Hermes、Grok、Antigravity 都应在最新稳定版做一次真机 smoke。这里的“需 smoke”不是已确认的破坏，而是仓库最后明确验证版本落后，不能用单元协议样本替代最新版端到端证据。Pi 是本轮最明确的“已跟上”；OpenCode 1.18.31 是上游 ACP 恢复修复，通常只需升级并验证。

> **Kimi 特别澄清**：Kivio 当前早已使用新的 TypeScript/Node **Kimi Code**：npm 包 `@moonshot-ai/kimi-code`、命令 `kimi acp`、配置目录 `~/.kimi-code`。本文所说的 2.0.1 复测，是从 Kimi Code 0.39.x 到 2.0.x 的协议/持久化回归验证，**不是**旧 Python `kimi-cli` 到新产品的迁移缺口。

## 版本与判断总表

| CLI | Kivio 最后明确适配证据 | 2026-09-18 最新稳定/生产通道 | 版本差距 | 判断 |
| --- | --- | --- | --- | --- |
| Claude Code | 2.1.238 参数；2.1.263 复核 | **2.1.276** | 13 个 patch（按 2.1.263 起算） | 无已知 wire break；需回归 stream-json、后台子代理和 MCP 启动 |
| Codex CLI | 0.148 schema；0.153.4 问题卡适配 | **0.155.0** | 0.154–0.155 | 已避开被移除的 `codex mcp-server`；需 app-server schema/live smoke |
| Cursor Agent | 2026-09-02 已接阻塞 ACP 扩展 | 安装器 build **2026.09.15-d2fe57e** | 无官方可比 semver | 文档仍与现实现匹配；缺 build 级协议日志，不能宣称全量验证 |
| OpenCode | ACP v1；上轮到 1.18.25 | **1.18.31** | 6 个 patch | 最新版主要是 ACP resume/fork 状态恢复修复；升级受益，无已知宿主改动 |
| Gemini CLI | 2026-09-02 改为 `--acp` | **0.60.0** | 0.58–0.60 | 主要是安全/受限模式收紧；无已知 ACP wire break，需权限和 MCP 配置 smoke |
| Kimi Code | 0.39.1 ACP 回归基线 | **2.0.1** | 0.40+ 与 2.0 大版本 | 新 Kimi 已接入；核心 ACP 能力相符，但 2.0 尚无仓库真机证据 |
| Pi | 0.85.1（9/8 适配，9/15 原生 fork） | **0.85.1** | 无 | **已跟上** |
| Hermes | 0.21.0 级别复核 | **0.21.3**（GitHub/native） | 3 个 patch | ACP 无已知 break；Kivio 的 PyPI 最新版查询源已失真 |
| Grok Build | 1.0.13 启动参数/权限 | **1.0.34** | 21 个 patch | Happy path 无已知 break；缺 strict sandbox、`--no-auto-update` 和交互式 MCP 补充输入 |
| DeepSeek Harness | 0.1.0-rc.6/rc.8 SDK bridge | **0.1.5-rc.2**（npm `latest`/`next`） | 多个 RC；尚无 final stable | 高风险未证实适配；profile 依赖存在可复现的版本错代风险 |
| Antigravity CLI | 1.1.26 NDJSON/斜杠目录 | **1.2.6** | 1.1.27–1.2.6 | 持久流 happy path 未见明确破坏；错误/退出码及 report 行为需补测 |

预发布版本没有当作“最新稳定”：Codex 0.156 alpha、Gemini 0.61 preview/0.62 nightly、Grok 1.0.36 alpha、dsh 0.1.6-alpha.2 均只列入观察范围。dsh 例外是它目前没有 final stable，所以以 npm `latest`/`next` 的 0.1.5-rc.2 作为生产通道基准。

## 逐项核查

### Claude Code 2.1.276

近期与宿主最相关的变化是：2.1.273 修复子代理转后台后 SDK/stream-json 消息；2.1.274 增加 `CLAUDE_CODE_MCP_STARTUP_WAIT_MS` 并改善 stream-json/MCP 启动；2.1.275 修复 `--forward-subagent-text` 在 `context: fork` 下的行为；2.1.276 修复 2.1.275 对代理/网关造成的 400 回归。Kivio 已使用 `--forward-subagent-text`，没有看到必须新增的 wire 字段，但仓库基线仍落后，建议验证一轮普通对话、后台子代理、MCP 慢启动和自定义网关。

官方来源：[2.1.273](https://github.com/anthropics/claude-code/releases/tag/v2.1.273)、[2.1.274](https://github.com/anthropics/claude-code/releases/tag/v2.1.274)、[2.1.275](https://github.com/anthropics/claude-code/releases/tag/v2.1.275)、[2.1.276](https://github.com/anthropics/claude-code/releases/tag/v2.1.276)、[CHANGELOG](https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md)。

### Codex CLI 0.155.0

0.154 引入异步 inline questions、worktree/daemon 相关能力，并移除已弃用的 `codex mcp-server`；0.155 增加 user-verification 相关 app-server API，并修复 daemon 重启恢复。Kivio 使用的是 `codex app-server`，不受 `mcp-server` 移除影响；9 月 8 日已经接住 `agentMessage.questions`。当前通用 elicitation 路径会安全拒绝未呈现的请求，因此新版 user-verification 不会把会话挂死，但也尚未作为用户可操作界面暴露。目前没有确认的核心破坏，但代码注释的最后 schema 验证仍是 0.148，应该对 0.155 导出的 app-server schema 做 diff，并跑 new/resume/question/tool/permission smoke。

官方来源：[0.154.0](https://github.com/openai/codex/releases/tag/rust-v0.154.0)、[0.155.0](https://github.com/openai/codex/releases/tag/rust-v0.155.0)。

### Cursor Agent（2026.09.15 build）

Kivio 在 9 月 2 日已经处理 `cursor/ask_question`、`cursor/create_plan` 以及相关通知；当前官方 ACP 文档仍列这些阻塞方法、`session/request_permission`、session load 等能力，与 Kivio 的实现方向一致。问题在于 Cursor 安装器只给 build 标识，公开 changelog 没有逐 build 的 ACP wire diff，因此只能判定“没有发现文档层面的新缺口”，不能判定“最新 build 已完整验证”。

官方来源：[安装器](https://cursor.com/install)、[ACP 文档](https://cursor.com/docs/cli/acp)、[changelog](https://cursor.com/changelog)。

### OpenCode 1.18.31

1.18.31 修复 ACP resume/fork 后 model、effort、mode 和 reasoning chunk 边界恢复。Kivio 已用 ACP v1 的 session load/model/mode，这属于上游修复，正常不需要 Kivio 改协议；应把本机从 1.18.30 升到 1.18.31 后验证恢复会话。不要为此切到实验 ACP v2。

官方来源：[1.18.30](https://github.com/anomalyco/opencode/releases/tag/v1.18.30)、[1.18.31](https://github.com/anomalyco/opencode/releases/tag/v1.18.31)。

### Gemini CLI 0.60.0

0.59–0.60 的重点是 OAuth issuer 校验、restricted mode fail-closed/MCP 过滤、环境变量修改同意、路径/符号链接/NTFS 边界和 sandbox 目录隔离。Kivio 已在 9 月 2 日从 `--experimental-acp` 切为正式 `--acp`；官方说明未显示新的 ACP wire break。风险主要是新版更严格后，旧配置或权限流会从“宽松通过”变为明确失败，应做 MCP、文件权限和 sandbox smoke。

官方来源：[0.59.0](https://github.com/google-gemini/gemini-cli/releases/tag/v0.59.0)、[0.60.0](https://github.com/google-gemini/gemini-cli/releases/tag/v0.60.0)、[官方 changelog 索引](https://github.com/google-gemini/gemini-cli/blob/main/docs/changelogs/index.md)。

### Kimi Code 2.0.1

Kivio 的实现已经明确指向新 Kimi Code：[`installer.rs`](../../src-tauri/src/external_agents/installer.rs) 使用 `@moonshot-ai/kimi-code` 和 `.kimi-code`，[`defs/acp.rs`](../../src-tauri/src/external_agents/defs/acp.rs) 通过 `kimi acp` 启动，不会把旧 Python `kimi-cli` 当 fallback。这部分**已经适配，不是待迁移事项**。

2.0.1 官方 ACP 文档仍提供 core/session、`session/set_model`、terminal/fs、load/resume/list/close/delete/fork；反向 `elicitation/create` 在客户端不声明表单能力时会回落到 `session/request_permission`。Kivio 当前不声明 elicitation form，且共享 ACP 宿主能处理 permission，因此没有明显的新 wire 断点。Kivio 用量读取仍依赖 `~/.kimi-code/session_index.jsonl` 定位 `wire.jsonl` 的 `usage.record`；2.0.1 源码仍保留旧 index 作为 hint，未发现已删除的证据，但 0.40+ 引擎/索引改造跨度很大，必须在 2.0.1 验证 new/load/resume、模型/思考切换、权限、图片和用量条。

官方来源：[Kimi Code 2.0.1](https://github.com/MoonshotAI/kimi-code/releases/tag/%40moonshot-ai/kimi-code%402.0.1)、[Kimi ACP reference](https://github.com/MoonshotAI/kimi-code/blob/main/docs/en/reference/kimi-acp.md)、[Kimi Code changelog](https://github.com/MoonshotAI/kimi-code/blob/main/apps/kimi-code/CHANGELOG.md)。

### Pi 0.85.1

上游最新仍是 0.85.1。Kivio 9 月 8 日适配了该版本的 thinking map 与 `clear_queue`，9 月 15 日又加入原生 fork。版本与适配证据均对齐，是本轮唯一可直接判为“已跟上”的 CLI。

官方来源：[Pi 0.85.1](https://github.com/earendil-works/pi/releases/tag/v0.85.1)、[CHANGELOG](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/CHANGELOG.md)。

### Hermes 0.21.3

0.21.2/0.21.3 包含敏感信息脱敏和 state DB handle 修复；官方 ACP 用法仍是 `hermes acp`，Kivio 的 `hermes acp --accept-hooks` 没有发现已废弃证据。真实问题是版本元数据：[`installer.rs`](../../src-tauri/src/external_agents/installer.rs) 用 PyPI 包 `hermes-agent` 查询最新版本，但安装/更新走 Nous 官方脚本或 `hermes update`。截至本次核查，PyPI 返回 0.19.0，而官方 GitHub/native release 是 0.21.3，因此 Kivio 的“已安装/最新”比较可能错误。应改为与原生发行一致的官方版本源，并补 0.21.3 ACP smoke。

官方来源：[v2026.9.11](https://github.com/NousResearch/hermes-agent/releases/tag/v2026.9.11)、[v2026.9.14（0.21.3）](https://github.com/NousResearch/hermes-agent/releases/tag/v2026.9.14)、[ACP/programmatic integration](https://github.com/NousResearch/hermes-agent/blob/main/website/docs/developer-guide/programmatic-integration.md)、[PyPI 项目](https://pypi.org/project/hermes-agent/)。

### Grok Build 1.0.34

1.0.14 新增 `--sandbox strict`；1.0.17 起 MCP 工具可在调用中请求补充输入；1.0.33 改进 MCP structured JSON result 和真实 cancellation；1.0.34 推进 memory GA。Kivio 的启动参数最后按 1.0.13 核实，目前只用是否传 `--always-approve` 表达“询问/完全放行”，因此无法暴露 strict sandbox。共享 ACP 宿主会取消未知阻塞请求，所以交互式 MCP 补充输入不会无限挂起，但用户也无法完成这类工具调用。另一个更直接的可靠性缺口是官方 headless 文档建议传 `--no-auto-update`，而 Kivio 的会话与 ACP 模型探针都未传，受管会话可能被后台更新干扰。现有 ACP happy path 没有已知 wire break，但应先补这些能力再以 1.0.34 做取消、structured MCP result、elicitation 和权限回归。

官方来源：[Grok Build changelog](https://x.ai/build/changelog)、[CLI reference](https://docs.x.ai/build/cli/reference)、[Headless scripting](https://docs.x.ai/build/cli/headless-scripting)、[Permissions](https://docs.x.ai/build/features/permissions)、[官方源码](https://github.com/xai-org/grok-build)。

### DeepSeek Harness 0.1.5-rc.2

dsh 仍处 developer preview，0.1.5-rc.1 的 Session V3 与插件/API 更新明确包含兼容性破坏；rc.2 主要是 UI feedback。Kivio 不直接读取 dsh session storage，而是运行自带 bridge 并复用官方 SDK JSON-RPC 包，这降低了 Session V3 存储迁移的直接影响，但没有消除 API 风险。

当前 [`dsh_profile.rs`](../../src-tauri/src/external_agents/dsh_profile.rs) 安装 `@deepseek-ai/dsh-sdk-jsonrpc-server` 和 `@deepseek-ai/dsh-sdk-protocol` 时没有钉同一版本，只检查两个 `node_modules` 目录存在；截至核查，这两个包的 npm `latest` 仍分别是 0.0.1-rc.5 和 0.0.1-rc.1，而 `next` 才是 0.1.5-rc.2。已有 profile 在 dsh 核心升级后也不会重装。因此可能出现 0.1.5 core + 0.0.1 bridge/protocol 的实际组合。这是本轮风险最高的适配缺口：应把 core 与 SDK 包统一到同一受支持版本、记录 profile schema/version 并做自动迁移，然后跑工具、思考、用量、续聊、图片、命令和中断全链路测试。0.1.6-alpha.2 不应进入生产适配声明。

官方来源：[dsh 0.1.5-rc.1](https://github.com/deepseek-ai/deepseek-harness/releases/tag/dsh-v0.1.5-rc.1)、[dsh 0.1.5-rc.2](https://github.com/deepseek-ai/deepseek-harness/releases/tag/dsh-v0.1.5-rc.2)、[官方仓库](https://github.com/deepseek-ai/deepseek-harness)、[SDK JSON-RPC server npm](https://www.npmjs.com/package/@deepseek-ai/dsh-sdk-jsonrpc-server)、[SDK protocol npm](https://www.npmjs.com/package/@deepseek-ai/dsh-sdk-protocol)。

### Antigravity CLI 1.2.6

1.1.26 之后增加 remote-control 能力；1.1.28 调整 print timeout/部分输出；1.2.6 把 headless 默认超时改为无限，并稳定 `AGY_ERROR` stderr JSON 与退出码语义。Kivio 的正常对话是持久 `stream-json`，并已设置 `AGY_CLI_DISABLE_AUTO_UPDATE=true`，所以普通 happy path 没有明确破坏。受影响面主要是独立 `-p` report、错误解释和退出码测试：Kivio 还有自己的 60 秒 report 超时，且当前更偏向把非零退出统一当字符串错误。建议更新 1.2.x 错误 fixture/测试；内置 slash 目录虽仍标注“1.1.26 验证”，但 Kivio 有意只暴露安全 report，remote-control 不应仅因上游存在就自动加入。

官方来源：[Antigravity 1.2.6](https://github.com/google-antigravity/antigravity-cli/releases/tag/1.2.6)、[releases](https://github.com/google-antigravity/antigravity-cli/releases)、[产品 changelog](https://www.antigravity.google/changelog)、[Headless 文档](https://www.antigravity.google/docs/cli/headless/)。

## 本机版本快照（仅用于验证环境）

以下是本次机器上 `--version` 的快照，不是 Kivio 的锁定版本：Claude 2.1.276、Codex 0.155.0、Cursor 未安装、OpenCode 1.18.30、Gemini 0.25.2、Kimi Code 0.37.0、Pi 0.85.1、Hermes 0.21.3、Grok 1.0.13、dsh 0.1.5-rc.2、Antigravity 1.2.6。尤其 Gemini、Kimi、Grok 的本机安装太旧，不能用来证明最新版本兼容；Cursor 则无法本机 smoke。

## 2026-09-18 审计阶段验证

- `cargo test --manifest-path src-tauri/Cargo.toml external_agents --lib`：**727 passed、0 failed、43 ignored**。这证明现有协议 fixtures 和内部回归均通过，不等同于最新 CLI 的付费模型端到端测试。
- 以最新版临时包检查启动面：Gemini 0.60.0 的 `--acp`、Kimi Code 2.0.1 的 `kimi acp`、OpenCode 1.18.31 的 `opencode acp` 均仍有效。
- 本机检查：Claude 2.1.276 接受 Kivio 使用的 effort/thinking/subagent 参数；Codex 0.155.0 的 `app-server`/models 调试入口存在；Hermes 0.21.3 的 `hermes acp --check` 通过；Antigravity 1.2.6 接受当前 stream-json、effort、mode、sandbox、conversation 等启动参数。
- Grok 1.0.13、dsh 0.1.5-rc.2 的当前启动入口仍能解析；Cursor 未安装。为避免消耗账户额度，本轮没有发送真实模型回合，因此上表仍把相关项目标为“需 live smoke”。

## 原建议处理顺序（已于 2026-09-19 执行）

1. **P0/P1：dsh** — 把 core、JSON-RPC server、protocol 固定为同一条受支持 release，给 profile 增加版本迁移，再对 0.1.5-rc.2 跑完整 live suite。
2. **P1：Hermes** — 修正最新版查询源，使其与官方 native installer/release 一致；验证 0.21.3 ACP。
3. **P1：Grok** — 对受管会话和探针加 `--no-auto-update`；产品若允许严格沙箱，映射 `--sandbox strict`；以 1.0.34 跑取消/MCP/权限测试。
4. **P1：Kimi** — 不做旧 Python 迁移；直接安装 Kimi Code 2.0.1，重点回归 ACP session、permission、图片和 `session_index`/usage。
5. **P2：Antigravity、Claude、Codex、Gemini、Cursor、OpenCode、Hermes** — 按上文最小矩阵做最新版 live smoke，更新源码中的“last verified”注释和 fixtures；没有捕获 wire 差异时不制造兼容层。

## 仓库证据与方法

- 代理名单来自 [`registry.rs`](../../src-tauri/src/external_agents/registry.rs)，核查基准为提交 `44c60d28097422add2451b7b1a8131471c586292`。
- 与 9 月 1 日的[上一轮全量调研](./external-cli-catchup-2026-09.md)和 9 月 8 日的[已验证修复记录](./external-cli-verified-fixes-2026-09-08.md)逐项比较；相关实现提交包括 [`7dd590d3`](https://github.com/ZMGID/kivio/commit/7dd590d3)、[`1eb8572f`](https://github.com/ZMGID/kivio/commit/1eb8572f)、[`c89198ae`](https://github.com/ZMGID/kivio/commit/c89198ae) 和 [`defcc4cb`](https://github.com/ZMGID/kivio/commit/defcc4cb)。
- 最新版以官方 GitHub release/changelog、官方产品文档、npm/PyPI registry 元数据为准；忽略 nightly/preview/alpha，除非上游没有 final stable（dsh）。
- 2026-09-18 的审计阶段是只读研究；2026-09-19 已按本文顶部“补齐结果”实施产品代码修复和无模型真机验证。仍未运行会产生模型费用的完整对话，所以“协议与启动面已适配”不等于所有账号/模型组合都完成付费端到端认证。
