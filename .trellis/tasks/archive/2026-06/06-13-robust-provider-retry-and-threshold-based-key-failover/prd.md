# 健壮的 provider 重试 + 阈值化 key failover（稳定性 A）

## Goal
让任何**暂时性**错误不要直接停掉流程：自动重试约 5 次、间隔约 5s 再放弃；429 不在第一次就换 key，而是累计到阈值（且有备用 key）才换。目标：偶发错误/限流时整条 run 能扛过去，而不是一报错就死。

## 现状（已核实，api.rs）
- `send_with_failover`（230）按 key 列表轮换；内层 `send_with_retry_for_failover` 用 `is_retryable_status_for_failover = is_retryable_status && !is_failover_status` 重试 5xx/timeout，但**把 401/402/403/429 都排除**（交回外层立刻换 key）。
- `is_failover_error`/`is_failover_status`（215/219）= 401/402/403/429 —— **429 第一次就触发换 key**。
- `is_retryable_status`（154）= 429 | 5xx；`is_retryable_error`（160）= timeout|connect；退避 `retry_delay_ms`：base 500ms 指数→cap 10s，**优先用 Retry-After**。
- 次数 `effective_retry_attempts` = `settings.retry_attempts`(默认见 default_retry_attempts) if retry_enabled else 1。
- **缺陷**：①429 不退避重试、第一次就换 key；②默认重试次数/间隔偏小，暂时性错误容易直接失败 → planning_step 的 `?` 冒泡 → 整条 run 死。

## 设计（要做）
**错误分类与处理**：
- **401/402/403（坏/失效 key）** → 立即换 key（保留现有 immediate failover）。
- **429（限流）** → **先在当前 key 退避重试**（不第一次就换）；只有当**同一 key 连续 429 达到阈值**（如 2 次）**且存在未冷却的备用 key** 时才换 key（换后在新 key 上重新计数/重试）；无备用 key 则继续退避重试到总次数上限。
- **5xx / timeout / connect（暂时性）** → 当前 key 退避重试（不换 key，这不是 key 的问题）。
- **400 / 404 / 422 等确定性客户端错误** → **不重试，快速失败**（重试也永远失败）。
- 始终**优先尊重 Retry-After**。

**次数与间隔（默认，可配置）**：
- 默认重试次数 = **5**（现 default_retry_attempts 若小于则提到 5；retry_enabled 默认保持 true）。
- 间隔 ≈ **5s**（把 RETRY_BASE_DELAY 调到 ~5000ms；可保留温和退避但起步 5s、cap 合理如 30s；Retry-After 覆盖）。
- clamp 上限相应放宽（clamp_retry_attempts 容纳 5+）。

**实现要点**：
- 解耦"内层是否重试"与"外层是否换 key"：把 429 从"内层立即排除"挪成"内层可退避重试"；外层换 key 决策对 429 设阈值（同 key 连续 429 计数达标 + 有备用 key 才换）。401/403 仍立即换。
- 阈值可用"同一 key 上 429 重试已达 N 次"近似实现（N 即阈值），不必引入复杂时间窗计数器；若实现简单的窗口计数也可。
- 改动集中在 api.rs（is_failover_status/is_retryable_*、send_with_failover/send_with_retry* 流程、retry_delay/常量）+ settings.rs（默认值/clamp）。

## Acceptance Criteria
- [ ] 暂时性错误（5xx/timeout/connect/429）会自动重试 ~5 次、间隔 ~5s（Retry-After 优先）再放弃，不是一次就失败。
- [ ] 429 不在第一次换 key；同 key 重试达阈值且有备用 key 才换；无备用 key 时持续退避重试。
- [ ] 401/402/403 仍立即换 key；400/404/422 不重试快速失败。
- [ ] 重试覆盖所有 provider 调用路径（planning/synthesis/sub-agent/vision/translate 等都经 send_with_failover/send_with_retry），偶发错误不再让整条 run 直接停。
- [ ] cargo test 全绿（含新增：429 阈值换 key、429 同 key 退避、5xx 重试次数、400 不重试、Retry-After 优先 的单测）；cargo check 无新 warning。

## Out of Scope
- 后台执行（B）。本任务只做重试/failover 韧性。
- 不改 UI（除非需要把重试中状态显示，本期可不做）。

## Technical Notes
- api.rs:130-360（常量/parse_retry_after/is_retryable_*/is_failover_*/send_with_*）；state.rs pick_active_key/mark_key_failed/KEY_COOLDOWN(60s)。
- settings.rs:842-845/1815-1825（retry_enabled/retry_attempts 默认 + clamp）。
- 现有测试：api.rs 1716+（extract_status_code / is_failover_error 系列）——改 failover 分类后要同步更新。
