# PRD — 持久会话生命周期与错误自愈

父任务：`07-20-external-cli-overhaul`。研究依据：父任务 `research/kivio-audit.md`（N1、缺陷 4、N3、A3/A4/A5/A6、C2/C3、B3）与 `research/paseo-reference.md`（B.4、E 节、G 节缺陷 4 映射）。

## Goal

持久会话（ACP / codex app-server）不挂死、错误分类可操作、中途切模型真实生效、取消可兜底。

## Requirements

### R1 stderr 排空（N1，阻断级）
- `AcpSession::connect` 与 `CodexAppServerSession::connect` 在 spawn 后 `stderr.take()` 起 drain 任务（复用/仿照 `spawn.rs::drain_stderr`），累积尾部若干 KB 供错误诊断使用；session close 时任务结束。
- drain 收集的 stderr 尾部要接入 R2 的错误报告（Paseo 做法：spawn/handshake 错误消息附 stderr）。

### R2 错误分类与呈现（缺陷 4 + B3）
- 在 `run.rs` 错误出口（现 :398 一带）引入错误分类：`auth`（含 "Authentication required"、401 语义 / auth 探测口径不一致时以会话侧为准）、`timeout`（handshake/turn 超时）、`exited`（进程退出，附退出码 + stderr 尾部）、`protocol`（RPC error 等其他）。
- 每类映射为可操作中文提示；auth 类附登录引导（per-agent 登录命令，如 `grok` 提示重新登录）；原始错误串保留为次级详情（可折叠/日志），不作为气泡主文案。
- handshake 阶段错误消息带阶段名（spawn / initialize / session-new），仿 Paseo 分阶段诊断。

### R3 握手健壮性与自动重连（缺陷 4）
- handshake 超时上调：initialize 与 session/new 各 ≥30s（Paseo 用 60s；结合桌面场景取 30-60s，design 定）；进程 early-exit（spawn error / stdout EOF）立即短路报错，不等超时。
- `run_persistent_turn`：连接/resume 失败或首轮 turn 失败（非 cancelled、非 auth 类）时，清理后自动 fresh 重连重试一次；重试仍失败才走 R2 呈现。
- C3：resume 失败静默降级 fresh 后，覆盖写新的 live handle（清除失效 native_id），消除下轮注定失败的 resume 尝试。

### R4 持久 ACP 会话中途换模型/推理（N3 + C2）
- `AcpSession::run_turn` 接收 `model`/`reasoning`；与会话当前值不同时轮前发 `session/set_config_option`/`session/set_model`（best-effort，失败记录）。
- reasoning 为启动 flag 的 agent（grok `--reasoning-effort`）：`session/set_config_option` 无对应项时，模型/推理变更触发重连（Close 旧会话 + fresh connect），保证 UI 所见即所得。
- codex 路径已每轮生效，仅补回归测试。

### R5 取消与等待兜底（A5 + A6 + A3 + A4）
- 持久路径：`SessionCommand::Cancel` 发出后启动兜底计时（如 10s），协议级取消无响应则升级 Close（强杀 + 清 registry/handle）。
- 一次性路径：错误 return 前统一 `start_kill()` 再 `wait()`（A6）；ACP 一次性 prompt 阶段加整体墙钟超时（A3，默认值 design 定，可按 def 覆盖）。
- A4：为 run_persistent_turn 事件收尾顺序加不变式注释（events 先于 done），或改为 events 通道关闭后读 done。

## 非目标

- 不动 prompt 组装/去重（前置任务已完成）；不动模型探测与检测缓存（`07-20-cli-detection-models`）；不做进程注册表对账（Paseo E.3，超出本轮范围，记入父任务 deferred）。

## 依赖与顺序

- **前置：`07-20-cli-message-chain` 已归档**（同文件 acp.rs/run.rs，串行避免冲突）。
- 与 `07-20-cli-detection-models` 文件交集小（detection.rs vs session/*），可并行。

## Acceptance Criteria

- [x] 单测：错误分类/模型变更/Cancel 兜底测试齐备（25+ 组，commit 4214956；401 改 token 边界匹配防误伤）。
- [ ] 【待用户真机验收】登出 grok 后发消息 → 气泡出现"未登录 + 登录引导"中文提示，无裸 RPC 串；重新登录后无需重启 App 恢复正常。
- [ ] 【待用户真机验收】持久 grok 会话中途切模型 → 下一轮回复的模型确实变化（或可观测到重连）；切 reasoning 档同理。
- [ ] 【待用户真机验收】kill 掉存活的 CLI 子进程后再发消息 → 自动重连成功，用户无感或仅一次轻提示。
- [x] `cargo test --lib` 1032 通过（复检含 clippy 基线对比，无新警告）。
