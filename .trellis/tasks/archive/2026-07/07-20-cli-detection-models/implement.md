# Implement — 检测缓存、模型探测与前端呈现

前置：`07-20-cli-message-chain` 已归档。可与 `07-20-cli-session-lifecycle` 并行（冲突时对方先合、本任务 rebase）。阅读：本任务 `prd.md`/`design.md`；父任务 `research/kivio-audit.md`（缺陷 3、N2、N4、B2/B4、D1-D3、F2/F4、N5）、`research/paseo-reference.md`（A 节、F.1）。

## 执行清单（按序）

### Step 1 — N2：回复路径去探测
- [ ] `run.rs` 回复前置检查改为仅 `resolve_binary`（design §1），删除 `detect_single_agent` 调用。
- [ ] 后端计时日志（debug 级）：发送到 spawn/turn 开始的耗时，供验收测量。
- [ ] 验证：`cargo test`；实测第 2+ 轮耗时 < 500ms。
- 回滚点：commit。

### Step 2 — 缺陷 3 + N4 + F4：探测容错
- [ ] `acp.rs::detect_acp_models`：非 JSON 行 continue；session/new 后加 1.5s 通知收集窗口合并异步模型推送。
- [ ] `defs/codex.rs`：`list_models_timeout_secs: Some(15)`。
- [ ] 单测：banner 首行 + JSON 后续仍出模型；异步 update 推送被合并。
- 回滚点：commit。

### Step 3 — R3：探测结果状态化
- [ ] `detection.rs`：`probe_models` 改 `Result<Vec<_>, String>`；`detect_agent_models` 返回携带 `source`/`probe_error`。
- [ ] `commands.rs`：返回 JSON 加 `source`/`probeError` 字段。
- [ ] `state.rs`：fallback 结果 30s 负缓存（force 绕过）。
- [ ] 单测：fallback 时 source 标记正确；负缓存 TTL 生效。
- 回滚点：commit。

### Step 4 — R4：前端呈现
- [ ] `src/api/tauri.ts` 或 `src/chat/api.ts`（以现有位置为准）：透传 source/probeError。
- [ ] `RuntimePicker.tsx`：ExternalModelSelector 加载态 + fallback 降级行 + 重试；popover 刷新 IconButton（xs）。
- [ ] Vitest：fallback 降级行渲染 + 重试触发 force 调用（mock api）。
- [ ] 验证：`npm run lint && npm run typecheck && npm test`。
- 回滚点：commit。

### Step 5 — R5：defs 修正
- [ ] 本机跑 `pi --help` / `kimi --help` 确认 flag（不发起 AI 会话）。
- [ ] `defs/pi.rs`：移除 `--append-system-prompt` 目录注入（design §5）。
- [ ] `defs/kimi.rs`：补 `list_models_args` 或更新静态表。
- [ ] 其余 defs fallback_models 按实探校准；`detection.rs` B4 join 错误日志。
- [ ] 更新/新增受影响的 defs 单测（如 pi build_args 断言不含目录注入）。
- 回滚点：commit。

### Step 6 — 全量回归
- [ ] `cargo test --manifest-path src-tauri/Cargo.toml` + `npm run lint && npm run typecheck && npm test`

## 审查门（主会话执行）

1. Diff review：返回值扩展向后兼容；UI 契约不破坏；IconButton 规范（CLAUDE.md 按钮约定）。
2. 实测四项（prd Acceptance）：grok 真实模型列表；探测失败降级角标+重试；第 2+ 轮 < 500ms；刷新按钮即时生效。
3. `trellis-check` 复检 → 归档。

## 风险

- 1.5s 通知窗口拉长首次模型探测——可接受（懒查、有缓存、仅首次）；若实测某 CLI 在 session/new 前就发模型推送则窗口提前结束。
- probe_models 改 Result 触碰 detect_all_agents/detect_single_agent 既有调用方——保持行为（Err → fallback），仅签名传播。
