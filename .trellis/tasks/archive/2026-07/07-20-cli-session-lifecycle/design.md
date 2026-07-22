# Design — 持久会话生命周期与错误自愈

前提：`07-20-cli-message-chain` 已归档（acp.rs 已有 AcpTextAssembler，本任务改动叠在其上）。

## 1. stderr 排空（N1）

`AcpSession::connect` / `CodexAppServerSession::connect` spawn 后：

```rust
let stderr_tail = spawn_stderr_tail(child.stderr.take()); // JoinHandle<String>，环形保留尾部 8KB
```

新增 `spawn::spawn_stderr_tail`（改造现有 `drain_stderr`：加尾部环形缓冲上限，避免无限增长——现 drain_stderr 全量累积，长活会话不可用）。`AcpSession`/`CodexAppServerSession` 持有该 handle；`close()` 与错误路径 join 取尾部文本供 R2 使用。

## 2. 错误分类（缺陷 4 + B3）

新模块 `external_agents/errors.rs`：

```rust
pub enum ExternalAgentErrorKind { Auth, Timeout, Exited, Protocol }
pub struct ClassifiedError { kind, user_message: String, detail: String }
pub fn classify(raw: &str, exit_code: Option<i32>, stderr_tail: &str, agent_id: &str) -> ClassifiedError
```

- 分类规则：`Authentication required`/`401`/`unauthorized`（不区分大小写）→ Auth；`timeout`/`timed out` → Timeout；`exited`/`EOF`+exit_code → Exited；其余 → Protocol。
- `user_message`：中文可操作文案；Auth 类按 agent_id 查登录引导表（grok→`grok` 重新登录、claude→`claude /login`、codex→`codex login`、cursor→`cursor-agent login` 等，defs 各自提供 `login_hint` 字段或 errors.rs 内静态表——取静态表，不动 RuntimeAgentDef 结构）。
- `detail`：原始错误 + exit_code + stderr 尾部（截断 2000 字符），拼在气泡内折叠段（markdown `<details>` 由前端 ChatMarkdown 已支持，无前端改动）。
- 接入点：`run.rs` 错误出口统一走 `classify`；handshake 错误消息在 acp/codex 侧加阶段前缀（`spawn:`/`initialize:`/`session-new:`）供 classify 与 detail 使用。

## 3. 握手健壮性 + 自动重连（缺陷 4 / C3）

- 超时：initialize 30s、session/new 30s（常量集中在 acp.rs / codex_app_server.rs 顶部；Paseo 用 60s，桌面场景 30s 起步，可后调）。
- early-exit 短路：`acp_read_until_id` 的 `Ok(Ok(None))` 分支已短路（"exited during handshake"），保留；spawn 失败本就立即 Err。补：handshake 错误时 join stderr tail 进错误文本。
- 自动重连：`run_persistent_turn` 中 turn 结果为 Err 且非 `cancelled`、且 classify ≠ Auth 时：清 session/handle → `connect_persistent_session(fresh)` → 用 `first_prompt` 重发一次。仅重试一次（bool flag）。Auth 类不重试（注定失败且可能触发登录风暴）。
- C3：`connect_persistent_session` 内 resume `Err` 落 fresh 后，返回值已带 `resumed=false`，调用方 `save_live_handle` 本就覆盖写——审计所指"未清"发生在 resume 内部 Ok(session) 但 native_id 失效的情形之外，即 `if let Ok` 吞错分支：改为 resume Err 时记日志并继续 fresh（覆盖 handle 由现有 save 达成）。核实后如确无残留路径，仅补注释。

## 4. 持久 ACP 中途换模型（N3）

- `AcpSession` 增加字段 `current_model: Option<String>`、`current_reasoning: Option<String>`（connect 时初始化）。
- `run_turn` 签名不变（model/reasoning 已在 `SessionCommand::RunTurn` 里，actor 侧现被 `..` 丢弃）：actor 匹配处取出，传入 `run_turn(prompt, model, reasoning, ...)`。
- `run_turn` 轮前：`model != current_model` → 发 `session/set_config_option`（有 config id）或 `session/set_model`，等 ack（10s best-effort），更新 `current_model`。
- reasoning：ACP 无标准 reasoning 选项；若 connect 时 configOptions 含 `thought_level`/`reasoning` 类目则 set_config_option，否则向调用方返回 `NeedsReconnect` 信号 → `run_persistent_turn` 收到后 Close + fresh connect（新 build_args 带新 flag）。grok 即走此路径。
- 变更检测在 actor 内（拥有 session 状态），`run_persistent_turn` 只处理 NeedsReconnect。

## 5. 取消兜底与一次性路径清理（A5/A6/A3/A4）

- A5：`run_persistent_turn` `cancel_sent` 后记录时刻，10s 无 done → `control.send(Close)` + remove/clear，返回 "cancelled"。
- A6：非持久路径 `read_result` Err 分支先 `spawned.child.start_kill()` 再 `wait()`。
- A3：`run_acp_session` prompt 阶段加墙钟（默认 600s——外部 CLI 长任务合法，只防真挂死；常量可调）。
- A4：actor `done.send` 前必已 send 完 events 的不变式，加注释于 `spawn_acp_session_actor` 与 `run_persistent_turn` 收尾处。

## 影响面与回滚

- 触碰：`session/acp.rs`、`session/codex_app_server.rs`、`session/mod.rs`（handle 清理）、`run.rs`（错误出口 + 重连）、`spawn.rs`（stderr tail）、新 `errors.rs`。前端零改动。
- 每个 R 独立 commit，可单独 revert；R2（errors.rs）先行，R3/R4 依赖它。
