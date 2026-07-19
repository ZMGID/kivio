# Paseo 调研 → kivio external_agents 借鉴笔记

调研对象：`getpaseo/paseo`（`/Users/zmair/ZM database/cankao/paseo`，commit c9bcfa7）
目的：搞清 Paseo 怎么连本地 CLI、怎么传附件/图片、怎么获取模型，用于修 kivio「本地 CLI 无法发图片附件」的 bug。

---

## 0. 一句话结论

- **连接**：Paseo 有两套。① 终端直通（node-pty 跑 CLI 的 TUI，只做 running/idle 活动状态检测）；② **结构化客户端栈**（子进程 + stdio 管道 + 各家协议）—— 后者才是和 kivio `external_agents/` 对应的。
- **图片**：Paseo 按**协议原生图片块**传，不塞进 prompt 文本。Claude=base64 image block；ACP=原生 image block；Codex=落临时文件传 localImage path；OpenCode=data URL file part。
- **模型**：`fetchCatalog` 每家 CLI 自己实现（Codex 查 `model/list`、ACP 读 `availableModels`、OpenCode 查 `provider.list`、Claude 硬编码 manifest + 读 settings）。**kivio 已经做到同等水平**，不是缺口。

---

## 1. Paseo 连接本地 CLI（结构化栈，= kivio 对应物）

- 位置：`packages/server/src/server/agent/`，接口 `AgentClient`（`agent-sdk-types.ts:686`）。
- 每家一个 client：`ClaudeAgentClient` / `CodexAppServerAgentClient` / `CursorACPAgentClient` / `GenericACPAgentClient` / `PiRpcAgentClient` / `OpenCodeAgent`（`provider-registry.ts:32-43`）。
- 子进程 spawn：`jsonl-rpc-process.ts:62`，`stdio: ["pipe","pipe","pipe"]`（不是 PTY）。
- 统一方法：`startTurn` / `interrupt` / `respondToPermission` / `setModel` / `fetchCatalog`（`agent-sdk-types.ts:358-373,698`）。
- 检测安装：`findExecutable` + `probeExecutable`（跑 `<bin> --version`，2s 超时，`executable-resolution.ts:60-142`）。

> 另一套「终端直通」`packages/server/src/terminal/` 是给人用的 xterm+node-pty，`agent-hooks/{claude,codex,opencode}` 只写 CLI 自己的 config 让它回调 `paseo hooks` 上报 running/idle/needs-input。**与附件无关**。

## 2. 图片/附件端到端（这是重点）

UI 粘贴/拖拽 → 存 IndexedDB/文件 → 发送时转 base64 `{data,mimeType}` → 协议 `images[]`（`ImageAttachmentSchema` `protocol/messages.ts:989`）→ 服务端 `buildAgentPrompt(text, images, attachments)`（`agent/prompt-attachments.ts:7-38`）生成**协议无关的 content blocks**（每图 `{type:"image",data,mimeType}`）→ 各 provider adapter 转原生格式：

| Agent | 原生传递 | 落盘 | kivio 对应协议 |
|---|---|---|---|
| Claude | `{type:"image",source:{type:"base64",media_type,data}}` 进 SDKUserMessage（stream-json）（`providers/claude/agent.ts:3048`） | 否 | `ClaudeStreamJson` |
| ACP（cursor/generic/grok…） | ACP 原生 `{type:"image",data,mimeType}`（`providers/acp-agent.ts:2969`） | 否 | `AcpJsonRpc` |
| Codex app-server | 先 `materializeProviderImage` 落 `$TMPDIR/paseo-attachments-*/<sha256>.ext`(0600)，再传 `{type:"localImage",path}`（`codex-app-server-agent.ts:2954`；`provider-image-output.ts:40-100`） | **是（唯一）** | `CodexAppServer` |
| OpenCode | `data:` URL 的 `FilePartInput{type:"file",mime,filename,url}`（`opencode-agent.ts:904`） | 否 | (kivio 无) |

**没有任何一家把 `@path` 或 base64 塞进 prompt 文本，也没有往 PTY 里 paste escape 序列。**

## 3. 模型发现（kivio 已对齐，非缺口）

- 接口 `fetchCatalog(): ProviderCatalog{models,modes,defaultModeId}`（`agent-sdk-types.ts:668,698`）。`AgentModelDefinition{provider,id,label,contextWindowMaxTokens,thinkingOptions,isDefault}`。
- 每家来源不同：Claude=硬编码 manifest + 读 `~/.claude/settings.json`（`claude/model-manifest.ts:29`,`models.ts:34`）；Codex=`model/list` RPC（`codex-app-server-agent.ts:6399`）；OpenCode=`provider.list` 且按 auth 过滤（`opencode-agent.ts:1601`）；ACP=probe `newSession` 读 `availableModels`（`acp-agent.ts:851`）；pi/omp=`--model` argv。
- 选择：`setModel()` 各协议 RPC / 启动 argv。
- **kivio 现状**：`detection.rs` 已有 `detect_acp_models` / `detect_claude_models` / `parse_pi_models` / `probe_opencode_models` + `fallback_models`，与 Paseo 一一对应。✅

---

## 4. kivio 的缺口 + 落地方案

**现状 bug**：`chat/commands/reply.rs:88-113` 外部分支只取 `m.content`（纯文本），丢掉 attachments；`external_agents/run.rs` 的 `compose_external_prompt` 也只拼文本。图片彻底不进 CLI。

**kivio 已有的现成件**（能直接接 Paseo 的做法）：
- `session/acp.rs::run_acp_session` —— 可注入 ACP `{type:"image",...}` 块
- `session/codex_app_server.rs::run_codex_app_server_session` —— 可落临时文件传 localImage path
- `ClaudeStreamJson` 路径 —— 可注入 base64 image block
- 模型发现已完备

**推荐方案（镜像 Paseo，按协议原生注入，不走 prompt 文本）**：
1. `reply.rs` 外部分支：取最后一条 user 消息的 image attachments，解析绝对路径，连同 `latest_user_message` 传下去（`send.rs` 已算好 `last_user_image_paths`，只是没往外部分支传）。
2. `run_external_cli_reply` 增加 `image_paths: &[PathBuf]` 参数，透传给各 session 函数。
3. 各协议 session 函数把图片转成自己的原生块：
   - ACP（grok/generic）：读文件→base64→`{type:"image",data,mimeType}` content block
   - Codex：落临时文件（可复用 sha256 命名）→`{type:"localImage",path}`
   - Claude stream-json：base64 image block 进 user message
   - PiRpc / JsonEventStream(kimi)：看协议是否支持；不支持则降级为「路径注入 prompt + 加 allowed-dir」兜底
4. 非图片文件附件：Paseo 走 `AgentAttachment` 渲染成文本；kivio 可先只做图片（覆盖用户诉求），文件附件用路径注入兜底。

**优先级**：grok(ACP) + claude 是截图里出问题的两家，先打通这两条即最大收益。

---

## 5. 全量核对（第二轮，两个 agent 深挖）

### Paseo 输入入口全表（10 类）
| 入口 | schema | kivio |
|---|---|---|
| 粘贴/拖拽/选择器 图片 | `images[]` base64 | ✅ |
| 拖拽/选择器 任意文件 | `uploaded_file`(path) | ✅ 文件附件 |
| 粘贴非图片文件 | 忽略 | — |
| @-mention | 往 text 插带引号相对路径，非附件、不读文件，agent 自解析 | 无（印证给路径即可） |
| 浏览器元素截图 | 双份 images[]+text | ❌ 无浏览器面板 |
| PR/Issue 选择器 + URL 自动附加 | forge_*/github_* | ❌ |
| 内联 diff 评审 | review | ❌ |
| PR 面板上下文 | text(无 contextKind) | ❌ |
| 会话 fork→历史 | text+contextKind:chat_history，提到 prompt 最前 | 转录机制不同 |
| 语音/听写 | 服务端 STT→文字，音频不发 agent | ❌ 不在范围 |
| 建 agent/workspace 首条 prompt | 完整 images[]+attachments[] | 无对应 |

### 消费矩阵结论
- 只有 image 能变原生块，且只有 image 按 provider 分化。
- 7 种 AgentAttachment 除 text 外，**所有 provider 一律 `renderPromptAttachmentAsText` 拍平成文本**（claude/acp/codex/opencode/pi/omp 逐一确认，无例外）。
- `uploaded_file` 处处文本（连 OpenCode 原生文件通道也只对图片用）。
- 图片 gating ad-hoc：Claude mime 白名单{jpeg,png,gif,webp}否则**静默丢**；Pi/OMP 逐模型 `model.input?.includes("image")` 否则落临时文件+`[Image available at: {path}]`；ACP/Codex 不判断直接塞。无统一 supportsImages capability。
- uploaded_file 文本模板：`Uploaded file:{name}\nPath:{path}\nMIME:{mime}\nSize:{size} bytes`。
- Codex 临时文件 `$TMPDIR/paseo-attachments-*`(0700)/`<sha256>.<ext>`(0600)，**不清理**，靠 OS 回收。

### kivio 采纳的修正
1. Claude mime 白名单，超出**降级为路径提示**（不学 Paseo 静默丢）。
2. `RuntimeAgentDef` 加 `supports_native_image` + `image_mime_whitelist` 显式标记。
3. 文件=路径+元信息+allowed-dir（Paseo 对等，一等做法）。
4. kivio 有 `cleanup_orphan_temp_files`，Codex 临时文件纳入 GC（比 Paseo 稳）。

### 明确排除（Paseo 有、kivio 无对应 UI，非遗漏）
forge/github PR·issue、review diff、浏览器元素、PR 面板上下文、语音音频、workspace 创建首条 prompt。
