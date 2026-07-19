# Implement — 本地 CLI 检测缓存重构

按「先后端拆分（收益核心）→ 前端懒查/去闪 → 验证」。每步 `cargo check`。

## S1. 可用性层拆分（detection.rs）
- [ ] 新增 `detect_availability_single(def)`：binary + version + auth，**不跑 probe_models**；models = `fallback_models_from_pairs(def.fallback_models)`。
- [ ] 新增 `detect_availability_all()`：并发跑 single（不接 cwd，复用现有 spawn 并发模式）。
- [ ] 常量：`AVAILABILITY_CACHE_KEY`、`AVAILABILITY_CACHE_TTL = 600s`、`MODELS_CACHE_TTL`（复用/新增）。
- 验证：`cargo check`

## S2. single-flight（state.rs）
- [ ] `AppState` 加 `availability_probe_lock: tokio::sync::Mutex<()>` + `model_probe_locks: std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>`（`base()` 初始化）。
- [ ] helper `model_probe_lock_for(key) -> Arc<Mutex<()>>`。
- 验证：`cargo check`

## S3. 命令改造（commands.rs）
- [ ] `chat_detect_external_agents`：走全局 `AVAILABILITY_CACHE_KEY`；miss 时持 `availability_probe_lock` → 复查缓存 → `detect_availability_all` → 存。移除逐 agent 模型缓存回填（模型层负责）。
- [ ] 新命令 `chat_detect_external_agent_models(app,state,agent_id,conversation_id,force)`：resolve cwd → key → 缓存命中返 → 持 per-key 锁复查 → resolve_binary + probe_models → 存 → 返 `{models, reasoningOptions}`。
- [ ] `lib.rs` invoke_handler 注册新命令。
- 验证：`cargo check`

## S4. 前端 api + 类型（api.ts）
- [ ] 新增 `detectExternalAgentModels(agentId, conversationId?, force?)` → invoke `chat_detect_external_agent_models`。
- 验证：`npm run typecheck`

## S5. 前端组件去闪 + 懒查
- [ ] `RuntimePicker` & `PermissionPicker`：`useEffect` 删 `setAgents([])`（保留上次）。
- [ ] `ExternalModelSelector`：选中 agent 且 `agent.models` 空时，调 `detectExternalAgentModels` 填充该 agent 的 models/reasoning（本地 state 覆盖）；保留上次不清空。
- [ ] 设置页 `ExternalAgentsSettings`：手动刷新时 force 全量可用性 + 对每个 available agent 触发一次模型探测（R7）。检查 `ExternalAgentsSettings.test.tsx` 是否需更新 mock。
- 验证：`npm run typecheck && npm run lint && npm test -- ExternalAgentsSettings`

## S6. 单测
- [ ] detection：`detect_availability_single` 不触发模型探测（可用 def 无 list_models 断言，或结构断言 models==fallback）。
- [ ] 缓存：全局 key 命中跨"不同 conversation_id"（模拟）。
- [ ] single-flight：两并发调只实跑一次（用计数器/mock）。
- 验证：`cargo test external_agents::detection`

## S7. 收尾
- [ ] `cargo test`（对齐 baseline）、`npm run lint && npm run typecheck && npm test`。
- [ ] 真机：连切多会话不再"检测中"闪、不再卡；选中 CLI 打开模型下拉才查模型。
- [ ] 逐条核 AC1–AC7；spec 更新；commit（`perf(external-agents): ...` 或 `refactor`）。

## 验证命令
- `cargo check --manifest-path src-tauri/Cargo.toml --lib`
- `cargo test --manifest-path src-tauri/Cargo.toml external_agents::detection`
- `npm run typecheck && npm run lint`

## 回滚点
- S1–S3 后端独立可编译；前端 S4–S5 独立。可用性/模型两层解耦，任一步出错回退上一步 commit；最坏回到现状"换会话重测"。
