# PRD — pi/kimi 原生会话接入与会话-CLI 绑定

父任务：`07-20-external-cli-overhaul` 的第 4 个子任务（用户 2026-07-20 补充的两点需求）。
**执行时机：前三个子任务（message-chain / session-lifecycle / detection-models）真机验收通过之后。**

## 背景（用户原话要点）

1. "pi 和 kimi 都是有会话机制的"——当前实现把两者当作无会话 CLI，每轮全量重发 transcript，是错的。
2. "不能发送历史记录……一个会话只绑定一个 CLI，不允许切换"——历史重放机制应整体废除，改为会话-CLI 强绑定 + CLI 原生 resume。

## 本机 CLI 能力核实（2026-07-20，已用 --help 验证）

- **pi**（v0.79.7）：有完整会话机制——`--session <path|id>`、`--session-id <id>`（**不存在则创建**，最适合 Kivio 生成固定 id 绑定会话）、`-c/--continue`、`-r/--resume`、`--fork`、`--session-dir`、`--no-session`。当前 def `resumes_session_via_cli: false` 是错的。
- **kimi**（v0.27.0）：有会话机制——`-S/--session [id]`（resume 指定会话）、`-c/--continue`（续接 cwd 上一会话）；且**支持 `kimi acp`**（ACP server over stdio）——可以直接把 kimi 迁到 ACP 持久会话家族，与 grok/cursor 同路径，比逐轮 `-p` + `-S` 更彻底。另有 `--add-dir`（目录授权，detection-models 任务 F1 时未接）。

## Requirements

### R1 pi 接入原生会话
- 通过 `--session-id <kivio 生成的固定 id>` 将 pi 会话与 Kivio conversation 绑定（首轮创建、后续轮自动续接）；`resumes_session_via_cli` 语义或 live-session 注册表二选一（design 定：pi 走 PiRpc 常驻进程的话优先 live registry；每轮 spawn 的话用 --session-id resume）。
- 接入后 pi 每轮只发最新一条用户消息，不再重放 transcript。

### R2 kimi 迁移到 ACP
- kimi def 改走 `kimi acp`（StreamFormat::AcpJsonRpc + ModelProbeStrategy::Acp + SlashStrategy::Acp），进入持久会话家族，自动获得：跨轮原生上下文、中途换模型、错误分类重连、stderr 排空——全部复用 session-lifecycle 任务已建好的基建。
- 顺带接 `--add-dir`（ACP 模式下确认等价机制；detection-models 任务 F1 的遗留）。
- 模型探测优先走 ACP（session/new 的 availableModels/configOptions），`provider list --json` 降为 fallback 探测或删除（design 定）。

### R3 历史重放机制整体废除 + 会话-CLI 强绑定
- 所有 9 个 CLI 均具备原生会话后，`compose_external_prompt` 的 `build_transcript` 历史重放路径删除（保留 instructions/skill/附件块）；任何轮次只发最新用户消息 + 必要的注入块。
- **会话-CLI 绑定**：conversation 一旦选定某个外部 CLI 并产生第一条消息，前端锁定 RuntimePicker（不可再切 CLI / 不可切回内置模型）；新 CLI = 新会话。后端拒绝 agent_id 与会话已绑定 id 不一致的发送（防御性校验）。
- 绑定的 UI 呈现（置灰 + tooltip"外部 CLI 会话不可切换"之类）design 阶段定稿。

### R4 回归约束
- message-chain 任务的 AcpTextAssembler、prompt 单测需随 transcript 删除同步调整（末条排除逻辑随 build_transcript 一起消失，测试改为断言"prompt 不含历史消息"）。
- fresh 重连场景（session-lifecycle 的 first_prompt 路径）改为依赖 CLI 原生 resume，不再靠重发全量历史兜底——resume 失败的降级行为 design 定（可提示"上下文已丢失"而非静默重放）。

## 非目标

- 不动内置 agent loop；不做跨 CLI 会话迁移（明确"新 CLI = 新会话"）。

## Acceptance Criteria

- [ ] pi 三轮对话：第 2/3 轮 CLI 进程收到的 prompt 只含最新消息；杀掉 Kivio 重开后继续对话，pi 记得前文（--session-id resume 生效）。
- [ ] kimi 走 ACP：多轮上下文由 kimi 自持；模型下拉出实时列表；中途换模型生效。
- [ ] 任意 CLI 会话（含 claude/grok）：第 2+ 轮 prompt 不含历史 transcript。
- [ ] 已绑定 CLI 的会话：前端无法切换 CLI/模型来源；后端对不一致 agent_id 的发送返回明确错误。
- [ ] `cargo test --lib` + 前端三命令全绿；受影响单测按 R4 调整。

## Notes

- 研究材料复用父任务 `research/paseo-reference.md` D 节（Paseo 正是"只发当前 prompt + 原生 session"架构，本任务把 Kivio 对齐过去）。
- 现有 external-agent-sessions 持久化（`session/mod.rs`）与 live registry 是现成地基，pi/kimi 接入主要是 defs + 少量 run.rs 分支调整。
