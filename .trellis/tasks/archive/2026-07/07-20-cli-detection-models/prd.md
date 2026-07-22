# PRD — 检测缓存、模型探测与前端呈现

父任务：`07-20-external-cli-overhaul`。研究依据：父任务 `research/kivio-audit.md`（缺陷 3、N2、N4、B2、B4、D1/D2/D3、F2/F4、N5）与 `research/paseo-reference.md`（A 节、F.1、G 节缺陷 3 映射）。

## Goal

模型下拉展示真实模型；探测失败可见、可重试；每轮发消息零探测开销。

## Requirements

### R1 消除每轮回复的完整模型探测（N2，性能主因）
- `run.rs`（现 :88）回复前置检查改为轻量：优先读可用性缓存（`AVAILABILITY_CACHE_KEY`，600s），未命中只跑 `detect_availability_single`（或仅 `resolve_binary`），**绝不**触发 `probe_models`。
- 效果指标：第 2+ 轮从发送到 CLI 收到 prompt 的后端耗时 < 500ms。

### R2 模型探测容错（缺陷 3 + N4）
- `detect_acp_models`：非 JSON 行 `continue`（对齐同文件其他 reader），不 `?` 放弃；banner 免疫。
- 参照 `detect_acp_commands` 同时监听异步 `session/update` 推送的模型列表（N4）；`session/new` result 的 `availableModels` 与 `configOptions[category=model]` 双通道任一有值即用（现有 normalize_models 已双通道，补异步等待窗口）。
- `codex debug models` 显式超时上调（F4，5s → 15s）。

### R3 探测结果状态化（缺陷 3 呈现面 + D1）
- `chat_detect_external_agent_models` 返回值增加来源标记：`probed`（真实探测）/ `fallback`（降级静态表）+ 失败时的 `error` 摘要。仿 Paseo 四态但最小化：不引入完整状态机，只加字段（向后兼容，前端未升级时行为不变）。
- 探测失败**不写缓存**的现行为保留；增加负缓存短 TTL（如 30s）防止连续失败风暴（design 定）。

### R4 前端呈现（D1 + D2 + B2）
- `ExternalModelSelector`：加载态、`fallback` 降级角标（如"默认列表"）、失败可重试（force）；不再静默吞错。
- `RuntimePicker` popover 加刷新按钮（force 可用性检测），解决新装 CLI 10 分钟不可见。
- D3：`agent.models[0]` 隐式契约加注释即可，不改逻辑。

### R5 defs 数据修正（N5 + F2 + B4）
- pi：移除把 `extra_allowed_dirs` 塞进 `--append-system-prompt` 的错误用法；确认 pi 的目录授权 flag，无则不传目录。
- kimi：调研 CLI 是否支持列模型；支持则补 `list_models_args`，否则更新静态表并依赖 R3 的 fallback 标记。
- 各 def 的 fallback_models 按本机 CLI 当前版本校准一轮（audit F 节版本清单为基线）。
- B4：`detect_availability_all` 记录 join 失败日志。

## 非目标

- 不动会话运行时（acp.rs 的 session 驱动、run.rs 的错误处理——分属另两个子任务；本任务对 acp.rs 仅触碰 `detect_acp_models` 一个函数，对 run.rs 仅触碰回复前置检查一处，冲突面极小）。

## 依赖与顺序

- 前置：`07-20-cli-message-chain` 已归档（run.rs 有一处交集）。
- 可与 `07-20-cli-session-lifecycle` 并行；若同时改 run.rs 出现冲突，以 session-lifecycle 为准先合。

## Acceptance Criteria

- [x] 单测：banner 容错/异步推送合并/fallback 标记/负缓存 TTL 均有覆盖（commit 3487e05，--lib 1037 通过）。
- [ ] 【待用户真机验收】grok 登录态下模型下拉出现真实模型列表（非仅 Default）；断网/登出制造探测失败 → 下拉显示降级角标 + 重试按钮，重试可恢复。
- [ ] 【待用户真机验收】外部会话第 2+ 轮发送 → 后端耗时 < 500ms（debug 构建有计时日志）。
- [ ] 【待用户真机验收】新装 CLI 后 RuntimePicker 点刷新立即可见。
- [x] `cargo test --lib` 1037 + lint/typecheck/vitest(300) 全绿；RuntimePicker 新增竞态守卫（复检修复）。
