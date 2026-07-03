# Chat GUI 无头测试通道（文件监听 probe）

## Goal

让自动化（Claude Code / CI）能**真实驱动 GUI 客户端的 chat agent** 并捕获结果，免去手动测试。以文件监听方式：写一个请求 JSON → 运行中的 app 走**与聊天窗口完全相同的生成路径**（`chat_send_message` → `run_agent_loop` + 全量工具集）→ 把结果（回答 + 实际调用的工具列表）写成结果 JSON。用于验证工具改动（如改名后模型是否还能调对）、回归等。

**明确不用 kivio-code**（它工具集/路径与真实客户端差别大，不可信）。

## 决策（已与用户确认，2026-07-03）

- **触发机制 A：文件监听**（轮询 `<app_data>/chat_probe/request.json`），非单实例 CLI arg——解耦、稳、自动化最省事，且无需新依赖（复用 tokio interval，仿 lib.rs MCP reaper）。
- **BACKEND-DIRECT**：watcher 直接后端跑生成并内联拿结果，不绕前端队列（前端队列 Lens 通道不回传结果，捕获更差）。
- **仅调试构建**：整个 probe 用 `#[cfg(debug_assertions)]` 编译门控，release 不含。

## Requirements

### R1：请求/结果文件契约
- 请求：`<app_data>/chat_probe/request.json` = `{ id?, prompt, provider?, model?, skillId? }`（`provider`/`model` 省略则用 `settings.effective_chat_model()`）。
- 结果：`<app_data>/chat_probe/result.json`（若 request 带 `id` 也写 `result-<id>.json`）= `{ id?, conversationId, answer, toolCalls:[{name, arguments, status}], streamOutcome, error?, usage?, finishedAt }`。
- `toolCalls` 来自本轮 assistant 消息的 `tool_records`（`name`/`arguments`(JSON 串)/`status`）。

### R2：文件监听 watcher（debug 专用）
- 在 `lib.rs .setup` 里 `tauri::async_runtime::spawn` 一个 `tokio::time::interval`（~500ms–1s）轮询 request.json，仿 MCP reaper 模板，`app_handle.try_state::<AppState>()` 取状态。
- 去抖：按 mtime 变化触发；消费后重命名/删除 request.json（`request.consumed`）避免重复执行。
- **串行**：一次只处理一个请求，避免并发生成 + 结果文件歧义。

### R3：真实生成 + 无头自动放行
- 复用 GUI 生成路径：mint scratch 会话（`create_chat_conversation_internal`，`None` provider/model 走默认）→ 跑与 `chat_send_message` 相同的生成逻辑（单模型路径，全量工具集）。
- **无头放行**（关键，否则挂起）：probe 运行必须自动通过所有交互门——审批（`approval_policy` 视为 `auto`）、会话级 file/shell consent、`ask_user`（返回 skip/取消，不阻塞）。用**独立 ProbeAgentHost**（审批/consent 返 true、ask_user 返 skip、emit 可 no-op）或等效开关实现。
- **超时兜底**：生成超时（如 120s）也要写出 result.json（`error: "timeout"`），watcher 不永久卡死。

### R4：结果捕获 + 会话留存（观察调试）
- 生成完成后内联取 `AgentRunResult`（或 stripped conversation 的末条 assistant 消息）序列化进 result.json，含 `stream_outcome`、`error`。
- **会话与项目保留、不隔离**（用户要求 2026-07-03）：probe 会话绑到**固定复用**的「Chat Probe」项目（根=`cwd`），标题 `🔬 <prompt>`，**留在会话列表**供在 GUI 里点开观察完整轨迹。复用同一项目避免列表污染。

## Acceptance Criteria

- [ ] AC1：app（debug）运行时，写 `chat_probe/request.json {prompt}` → 数秒内出现 `result.json`，含 `answer` + `toolCalls`。
- [ ] AC2：`toolCalls` 反映模型真实调用的工具名（用一个强制用工具的 prompt 验证，如"用工具 glob 查找 *.rs 并 read 某目录"→ 出现 `glob`/`read`）。
- [ ] AC3：走的是 GUI 真实路径——全量工具集（native+skill+mcp+todo+ask_user+agent），单模型路径。
- [ ] AC4：无头放行生效——即使模型调用 write/run_command/ask_user 也不挂起（自动放行或 skip），超时有兜底。
- [ ] AC5：release 构建**不含** probe（`#[cfg(debug_assertions)]` 编译掉）；probe 会话保留在「Chat Probe」项目下供观察（不隔离/不删除）。
- [ ] AC6：`cargo check --lib --tests` 干净；`npm run typecheck/lint` 绿；probe 逻辑有单测（请求解析、结果序列化、ProbeAgentHost 放行）。
- [ ] AC7：README/文档记录用法（写 request.json 的 schema + 读 result.json）。

## Notes

- 范围外：多请求并发；流式增量捕获（只记最终聚合结果 + tool_records）；跨会话历史注入（scratch 单轮即可）；GUI 侧可视化。
- 安全：调试专用、默认不存在于 release；probe 自动放行仅限 probe 的 scratch 会话，不改全局 settings、不影响用户真实会话的审批门。
- 复用点：`create_chat_conversation_internal`(commands.rs:320)、`chat_send_message`/`complete_assistant_reply_inner` 的工具装配(commands.rs:2012-2048)、`ChatAgentHost`(4848) 作为 ProbeAgentHost 的对照、`AgentRunResult`(chat/agent/types.rs:85)、`ToolCallRecord`(chat/types.rs:202)、lib.rs MCP reaper spawn 模板(338-361)、`app.path().app_data_dir()`。
- 研究详情见 `research/external-send-channel.md`、`research/generation-entry-and-result.md`、`research/probe-hook-points.md`。
