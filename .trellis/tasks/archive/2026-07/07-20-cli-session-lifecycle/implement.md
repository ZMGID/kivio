# Implement — 持久会话生命周期与错误自愈

前置：`07-20-cli-message-chain` 已归档。阅读：本任务 `prd.md`/`design.md`；父任务 `research/kivio-audit.md`（N1、缺陷 4、N3、A3-A6、C2/C3、B3）、`research/paseo-reference.md`（B.4、E 节）。

## 执行清单（按序）

### Step 1 — errors.rs 错误分类（R2 基础）
- [ ] 新建 `src-tauri/src/external_agents/errors.rs`：`ExternalAgentErrorKind` + `classify()` + per-agent 登录引导静态表。
- [ ] 单测：auth/timeout/exited/protocol 四类样例 + grok/claude 登录引导查表。
- 回滚点：独立 commit（未接线，零风险）。

### Step 2 — stderr tail（N1）
- [ ] `spawn.rs`：`drain_stderr` 改造/新增 `spawn_stderr_tail`（环形 8KB 上限）。
- [ ] `AcpSession::connect` / `CodexAppServerSession::connect`：take stderr 起 tail 任务，close/错误路径 join。
- [ ] 单测：写超 8KB 的 fake stderr，断言 tail 只留尾部且任务结束。
- 回滚点：commit。

### Step 3 — 错误出口接线 + 阶段前缀（R2 完成）
- [ ] acp.rs / codex_app_server.rs：handshake 各阶段错误加 `spawn:`/`initialize:`/`session-new:` 前缀；超时常量上调 30s。
- [ ] `run.rs` 错误出口改走 `classify`，气泡主文案 = user_message，detail 折叠段。
- [ ] 单测：run.rs 错误落地格式（含 detail 折叠、不含裸串）。
- 回滚点：commit。

### Step 4 — 自动重连（R3）
- [ ] `run_persistent_turn`：非 cancelled、非 Auth 失败 → 清理 + fresh 重连 + first_prompt 重发一次（bool 防循环）。
- [ ] C3 核实：resume Err 吞错分支补日志/注释（design §3 末）。
- [ ] 单测：fake control channel 模拟首轮 Err → 断言重连一次、Auth 类不重连。
- 回滚点：commit。

### Step 5 — 中途换模型（R4 / N3）
- [ ] `AcpSession`：current_model/current_reasoning 字段；actor 传递 model/reasoning；run_turn 轮前 set_model/set_config_option；NeedsReconnect 信号（reasoning 无 config 项时）。
- [ ] `run_persistent_turn`：处理 NeedsReconnect → Close + fresh connect。
- [ ] 单测：fake 双工流，断言模型变更轮前发出 set 请求；NeedsReconnect 触发重连序列。
- 回滚点：commit。

### Step 6 — 取消兜底 + 一次性路径（R5）
- [ ] A5：Cancel 后 10s 兜底 Close；A6：非持久错误路径先 kill 再 wait；A3：run_acp_session prompt 阶段 600s 墙钟；A4：不变式注释。
- [ ] 单测：Cancel 无响应升级 Close 的状态机测试。
- 回滚点：commit。

### Step 7 — 全量回归
- [ ] `cargo test --manifest-path src-tauri/Cargo.toml`
- [ ] `npm run lint && npm run typecheck && npm test`（前端零改动应直接绿）

## 审查门（主会话执行）

1. Diff review：UnifiedAgentEvent / chat-stream payload 零改动；errors.rs 文案审校。
2. 实测四项（prd Acceptance）：登出报错文案；中途切模型生效；kill 子进程自动重连；登录恢复无需重启。
3. `trellis-check` 复检 → 归档。

## 风险

- set_config_option 各 CLI 支持度不一：ack 等待是 best-effort，超时不报错——若 CLI 静默忽略，模型仍未换但 UI 认为换了。缓解：实测 grok/cursor 双 CLI；若发现静默忽略，该 agent 归入 NeedsReconnect 路径。
- 自动重连用 first_prompt（含 transcript）重发：依赖 message-chain 修复后 transcript 无重复，已满足。
