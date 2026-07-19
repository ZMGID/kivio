# 本地 CLI 附件/图片打通

## Goal

让「外部本地 CLI」后端（`external_agents/`，如 grok/ACP、Claude Code、Codex）能接收用户在聊天里附带的**图片**（及可行范围内的文件附件），而不是像现在这样被静默丢弃。参照 Paseo（`getpaseo/paseo`）的做法：按各 CLI 的协议**原生注入图片块**，而非把路径塞进 prompt 文本。

## Background

- 现状 bug：`chat/commands/reply.rs` 外部分支只取最后一条 user 消息的 `content`（纯文本），`send.rs` 已算好的 `last_user_image_paths` 没有传给外部分支；`external_agents/run.rs` 的 `compose_external_prompt` 也只拼文本。→ 图片彻底不进 CLI。
- 内置 loop 走 `main_image_paths` + `build_chat_api_messages` 正常带图，仅外部 CLI 分支缺失。
- 调研见 `research/paseo-vs-kivio-research.md`。

## Requirements

- R1: 用户发送带图片附件的消息、且会话后端为外部 CLI 时，图片必须按该 CLI 的原生协议送达。
  - ACP（grok、通用 ACP）：`session/prompt` 数组追加 `{type:"image", data:<base64>, mimeType}`。
  - Claude（`ClaudeStreamJson`）：user message 注入 base64 image content block。**仅 mime ∈ {jpeg,png,gif,webp}**；超出白名单降级为路径提示（R2），不静默丢弃。
  - Codex（`CodexAppServer`）：图片落临时文件（内容 hash 命名），传原生 `{type:"localImage", path}` 输入项。
- R1b: 图片能力用 `RuntimeAgentDef.supports_native_image` + `image_mime_whitelist` 显式标记（镜像 Paseo ad-hoc gating 但集中化）。`false` → 走 R2 降级。
- R2（图片降级）: 不支持结构化图片块的协议（PiRpc、kimi 的 JsonEventStream）：图片降级为在 prompt 文本里列出绝对路径 + 加 allowed-dir，让 CLI 自读（次优，但比丢弃好）。
- R2b（非图片文件，Paseo 对等做法）: **所有**协议的非图片文件附件，统一渲染成一段文本块「文件名 / 绝对路径 / MIME / 大小」并加 allowed-dir，让 CLI 用自己的 read 工具读。这与 Paseo 的 `uploaded_file` 渲染（`prompt-attachments.ts:119`）一致——文件不 inline 内容，是一等做法而非妥协。
- R3: 复用已存在的附件磁盘文件（`<conversations_dir>/<id>_attachments/`），不重复保存；只在 Codex 需要「独立临时文件」时才另落盘。
- R4: 纯文本消息、无附件消息的行为完全不变（防回归）。
- R5: slash 命令（`/xxx`）路径不注入附件（与现有 passthrough 语义一致）。
- R6: 图片读取/编码失败要有可见错误或降级提示，不能静默吞掉导致模型答非所问。

## Non-Goals

- 不改模型发现（kivio 现状已与 Paseo 对齐）。
- 不引入 Paseo 的终端/PTY 直通机制。
- 不做 OpenCode（kivio 暂无该 CLI 后端）。
- 不做「CLI 输出图片」反向渲染。
- **不做以下 Paseo 有、但 kivio 无对应 UI 的附件类**（全量核对后明确排除，非遗漏）：forge/github PR·issue、内联 diff review、浏览器元素截图、PR 面板上下文、语音/音频（Paseo 也是 STT 转文字，音频从不发 agent）、workspace 创建首条 prompt。

## Acceptance Criteria

- [ ] AC1: grok(ACP) 会话发一张图 + 「这是什么」，CLI 能基于图片内容作答（截图复现场景）。
- [ ] AC2: Claude Code 会话发一张图，能基于图片作答。
- [ ] AC3: Codex 会话发一张图，临时文件被创建且 `localImage` path 正确传入，能基于图片作答。
- [ ] AC4: PiRpc/kimi 图片走降级：prompt 里出现图片绝对路径，allowed-dir 包含附件目录，CLI 可读到。
- [ ] AC4b: 任意会话发一个非图片文件（如 .pdf/.txt）：prompt 出现「文件名/路径/MIME/大小」文本块，allowed-dir 含附件目录，CLI 能读到该文件。
- [ ] AC5: 无附件的纯文本/ slash 命令消息行为逐字节不变（现有测试全绿）。
- [ ] AC6: 图片编码失败时给出明确错误文案，不静默。
- [ ] AC7: `cargo test` 相关模块新增单测通过（附件→content block 转换、降级路径 prompt 拼装）。

## Notes

- 优先级：grok(ACP) + Claude 是截图里出问题的两家，先打通收益最大；Codex 次之；降级路径兜底其余。
