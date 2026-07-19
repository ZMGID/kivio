# Design — 本地 CLI 检测缓存重构

## 1. 两层拆分

### 可用性层（cwd 无关，全局缓存）
- `DetectedAgent` 现含 `available/path/version/auth_status/models/reasoning_options/sandbox_options`。
- 新增 `detect_availability_single(def)` = binary lookup + version + auth（**去掉 model probe**）。models 回填：优先取任一已缓存 (agent, *) 的模型，否则 `fallback_models`。
  - 简化：回填直接用 `fallback_models`（列表不需要真实模型）；真实模型由模型层懒查覆盖。
- `detect_availability_all()` = 并发跑 `detect_availability_single`（不接 cwd）。
- 缓存：复用 `external_detected_agents_cache`，但 key 改为常量 `AVAILABILITY_CACHE_KEY = "__availability__"`。TTL 常量 `AVAILABILITY_CACHE_TTL = 600s`。

### 模型层（按 agent+cwd，懒查）
- 复用现有 `probe_models(def, path, cwd)` + `external_agent_models_cache`（key = `cache_key(agent,cwd)`，已存在）。
- 新命令 `chat_detect_external_agent_models(agent_id, conversation_id, force) -> { models, reasoningOptions }`：解析 cwd → 命中缓存直接返 → 否则 resolve_binary + probe_models → 缓存 → 返回。

## 2. 命令改动（`external_agents/commands.rs`）

```rust
chat_detect_external_agents(force_refresh, conversation_id):
  // conversation_id 现在仅用于设置页 force 全量；普通列表忽略 cwd
  if !force { if let Some(a) = get_cached(AVAILABILITY_CACHE_KEY, AVAILABILITY_CACHE_TTL) { return a } }
  let agents = single_flight_availability(|| detect_availability_all()).await;  // 见 §3
  set_cached(AVAILABILITY_CACHE_KEY, agents);
  return agents

chat_detect_external_agent_models(agent_id, conversation_id, force):  // 新增
  let cwd = resolve_detection_cwd(conversation_id);
  let key = cache_key(agent_id, cwd);
  if !force { if let Some(m) = get_cached_external_agent_models(key, MODELS_TTL) { return m } }
  let models = single_flight_models(key, || probe one agent).await;
  set_cached_external_agent_models(key, models);
  return models
```

设置页 force 全量列表（R7）：`chat_detect_external_agents(force=true)` 后，前端对每个 available agent 各调一次 `chat_detect_external_agent_models(force=true)`（或后端在 force 时顺带 warm，二选一——倾向前端触发，后端保持单一职责）。

## 3. single-flight（`state.rs`）

Rust 版：每个 key 一把 `tokio::sync::Mutex`，持锁期间探测，释放后并发者读缓存命中。
- 可用性：一把全局 `Mutex<()>`（`AppState.availability_probe_lock: tokio::sync::Mutex<()>`）。
- 模型：`AppState.model_probe_locks: std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>` 按 key 取锁。
- 模式：`let _g = lock.lock().await;` 后**先复查缓存**（可能前一个持锁者刚填），命中则返；否则探测。

## 4. 前端改动

- `RuntimePicker`（agent 列表）：`useEffect` 去掉 `setAgents([])`（保留上次，不闪）。仍调 `detectExternalAgents(false, conversationId)`——现在后端秒回缓存。
- `PermissionPicker`：同上（去 `setAgents([])`）。
- `ExternalModelSelector`（选中 agent 的模型下拉）：挂载/选中 agent 时，若该 agent `models` 为空则调 `chat_detect_external_agent_models(agentId, conversationId)` 填充；下拉打开时也可触发。保留上次。
- `api.ts`：新增 `detectExternalAgentModels(agentId, conversationId, force)`。
- 类型：`DetectedExternalAgent.models` 可能为空（列表阶段），ModelSelector 懒填——已有空态处理（"暂无可用模型"）。

## 5. 契约 / 兼容

- `chat_detect_external_agents` 返回结构不变（仍 `{success, agents, cached}`），只是 `models` 在列表阶段是 fallback/cached，不再是每次现探。
- 设置页测试 `ExternalAgentsSettings.test.tsx` mock 了 detect——检查是否需同步（force 路径行为）。
- cwd 仍由 `resolve_detection_cwd` 用于模型层；可用性层不再用 cwd。

## 6. Tradeoffs

- 用**长 TTL（600s）+ 手动刷新**替代 Paseo 的事件驱动无 TTL 失效——省去 PATH/settings 变更监听（YAGNI；新装 CLI 最多等 10min 或手动刷新，设置页有刷新按钮）。
- 模型回填用 fallback 而非"最近一次任意 cwd 的真实模型"——更简单，代价是列表阶段模型名不精确，但列表阶段本就不展示模型（只展示 agent），选中后懒查即精确。
- single-flight 用 per-key async mutex，非共享 future——Rust 里更省事，语义等价（并发者串行但第二个立即命中缓存）。
