# Design：Chat GUI 无头测试通道（文件监听 probe）

前置：读 prd.md + research/（external-send-channel / generation-entry-and-result / probe-hook-points）。

## 1. 总体架构（BACKEND-DIRECT）

```
automation ── 写 <app_data>/chat_probe/request.json ──▶ [debug watcher poll ~700ms]
                                                              │ mtime 变化触发、串行
                                                              ▼
                                        mint scratch 会话 (create_chat_conversation_internal)
                                                              ▼
                                        probe 生成（复用 GUI 工具装配 + run_agent_loop，
                                                     ProbeAgentHost 自动放行 + auto policy）
                                                              ▼
automation ◀── 读 <app_data>/chat_probe/result.json ◀── 序列化 AgentRunResult + 清理 scratch
```

全部 `#[cfg(debug_assertions)]`，release 编译掉。新增模块 `src-tauri/src/chat/probe.rs`（挂 `chat/mod.rs`，`#[cfg(debug_assertions)]`）。

## 2. 文件契约

- **request.json**：`{ "id"?: string, "prompt": string, "provider"?: string, "model"?: string, "skillId"?: string }`
- **result.json**（带 `id` 时另写 `result-<id>.json`）：
```json
{ "id": "...", "conversationId": "conv_...", "answer": "...",
  "toolCalls": [{ "name": "glob", "arguments": "{...}", "status": "Success" }],
  "streamOutcome": "completed", "error": null, "usage": {...}, "finishedAt": 1730000000 }
```
- `toolCalls` ← `AgentRunResult.tool_records`（`name`/`arguments`(JSON 串)/`status`）。

## 3. Watcher（`probe.rs` + lib.rs spawn）

- lib.rs `.setup` 内、MCP reaper 块之后，`#[cfg(debug_assertions)]` 加一个 `tauri::async_runtime::spawn`：
  ```rust
  let app_handle = app.handle().clone();
  tauri::async_runtime::spawn(async move { crate::chat::probe::run_probe_watcher(app_handle).await; });
  ```
- `run_probe_watcher(app)`：`tokio::time::interval(700ms)` 循环；每 tick：
  1. `let base = app.path().app_data_dir()?; let dir = base.join("chat_probe");` 建目录。
  2. stat `request.json`；mtime 未变 → skip（去抖）。
  3. 读+parse → `ProbeRequest`；**先重命名 `request.json`→`request.consumed`**（防重复/并发）。
  4. `handle_probe_request(&app, req)`（串行 await，watcher 循环内不并发）。
- 无 notify 依赖（Cargo 无，且不需要）；轮询足够（调试用途，延迟无所谓）。

## 4. 真实生成 + 无头放行（关键）

**忠实复用 GUI 工具装配**（避免 probe 与真实客户端漂移），仅替换 host + 放行策略：

- **首选方案（低漂移）**：给单模型生成路径 `complete_assistant_reply_inner`（commands.rs:1748）加一个 `mode: ReplyHostMode { Gui, Probe }` 参数（现有 GUI 调用传 `Gui`）：
  - Probe 分支：`effective_chat_tools.approval_policy = "auto"`（仿 fan-out commands.rs:1997）；预置 `state.chat_session_consent` 该 scratch 会话 id（放行 file/shell 会话 consent）；host 用 `ProbeAgentHost`。
  - host 传递：若 `run_agent_loop` 形参是 `&impl AgentHost`，用 `match mode` 两臂各自 `run_agent_loop(config, &concrete_host, &executor)`（调用点小重复），或改签名收 `&dyn AgentHost`——实现期二选一。
- **备选（若穿参过侵入）**：`probe.rs` 内 `run_probe_generation` 复用**抽出的**公共装配 helper（把 commands.rs:2012-2048 的工具装配抽成 `pub(crate) fn assemble_chat_tools(...)`，GUI 与 probe 共用，杜绝漂移）+ 自建 `AgentRunConfig` + `ProbeAgentHost`。侵入更小但需复制 config 组装。
- 决策：**优先首选方案**（`mode` 参数，复用最彻底）；仅当 host 泛型/借用摩擦过大再退备选。

### ProbeAgentHost（`probe.rs`，impl `AgentHost`）
- `emit_stream_delta/done/tool_record/compaction_status/persist_partial_assistant`：no-op（结果从 `AgentRunResult` 内联取，不靠事件）。
- `request_tool_approval` → `true`；`request_session_consent` → `true`；`request_user_response` → 立即返回 skip/cancel（`AskUserResponseResult` 的取消态，不阻塞）。
- `is_generation_active` → 依 `state` 标准判断（复用 chat 的 generation 机制，保证取消/超时能生效）；`wait_for_generation_inactive` → 依标准实现或立即返回。

### 超时兜底
- `handle_probe_request` 用 `tokio::time::timeout(120s, generation)` 包裹；超时 → 写 `result.json {error:"timeout", streamOutcome:"timeout"}`，并触发该会话 generation 取消，watcher 继续。

## 5. 会话生命周期
- `create_chat_conversation_internal(&app, &state, req.provider, req.model, None, None, None, None)` → scratch 会话（provider/model 省略走 `settings.effective_chat_model()`）；`save_conversation` 防御性落盘。
- 单模型路径（scratch 无 `reply_models`，天然不 fan-out；避开 image-gen 模型或接受无 tool_calls）。
- 跑完 `delete_conversation(&app, &conv.id)` 清理，不进用户会话列表。

## 6. 数据流（一次 probe）
写 request.json → watcher tick 命中 mtime → 重命名 consumed → mint+save scratch conv → probe 生成（auto 放行）→ 拿 `AgentRunResult`（content/tool_records/stream_outcome/usage）→ 写 result(-id).json → delete scratch conv。

## 7. 兼容性 / 回滚
- 纯新增 + debug 门控：release 不含，GUI 热路径仅多一个 `mode` 参数（默认 Gui，行为不变）。回滚 = 删 `probe.rs` + lib.rs spawn + 去掉 `mode` 参数。
- 不改全局 settings（auto policy 仅作用于 probe 的这次生成的 `effective_chat_tools` 局部副本）；不影响用户真实会话审批门。

## 8. 测试策略
- 单测（`probe.rs`）：`ProbeRequest`/`ProbeResult` 序列化往返；`ProbeAgentHost` 放行返回值（approval/consent=true、ask_user=skip、emit no-op 不 panic）；结果从 `AgentRunResult` 提取 `toolCalls` 的映射。
- 集成/手测（AC1/AC2）：本环境 GUI app（`npm run dev`）能起——写 request.json，观察 result.json 出现且 `toolCalls` 含预期工具。（cargo test 二进制 0xC0000139 环境限制沿用既有做法，纯函数用编译+复核/harness。）
- 前端不涉及（`npm run typecheck/lint` 仅确认无回归）。

## 9. 不做（范围外）
多请求并发、流式逐帧捕获、历史注入、GUI 可视化、release 暴露、单实例 CLI arg 触发（选了文件监听）。
