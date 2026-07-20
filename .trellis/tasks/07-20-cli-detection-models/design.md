# Design — 检测缓存、模型探测与前端呈现

## 1. 消除每轮探测（N2）

`run.rs`（现 :88-94）：

```rust
// 旧：let detected = detect_single_agent(def, &cwd).await;  // 必跑 probe_models，10-20s
// 新：
let available = state
    .get_cached_detected_agents(AVAILABILITY_CACHE_KEY, AVAILABILITY_CACHE_TTL)
    .map(|agents| agents.iter().any(|a| a.id == def.id && a.available));
let available = match available {
    Some(v) => v,
    None => resolve_binary(def).await.is_some(),  // 缓存未命中：只查二进制，不探版本/auth/模型
};
```

随后的 `resolve_binary`（:96）本就要跑，两处合一：直接以 `resolve_binary` 结果为准，缓存仅作快速通道。最终形态：**回复路径只调一次 `resolve_binary`**，不 probe 版本/auth/模型。auth 状态显示交给列表阶段（本就有）；会话侧 auth 失败由 session-lifecycle 任务的错误分类兜住。

## 2. 探测容错（缺陷 3 + N4 + F4）

- `detect_acp_models`（acp.rs:217）：`serde_json::from_str(...).ok()?` → `match ... Err(_) => continue`（三行改动，对齐同文件其他 reader）。
- N4 异步模型推送：在 `detect_acp_models` 的读循环中，`session/new` 响应到达后不立即退出，再等一个短窗口（1.5s）收 `session/update`（`available_commands_update` 同级的模型推送——参照 `detect_acp_commands` 的通知处理），期间收到模型列表则合并。`normalize_models` 双通道保持。
- F4：codex def `list_models_timeout_secs: None` → `Some(15)`。

## 3. 探测结果状态化（R3）

`chat_detect_external_agent_models` 返回值（commands.rs:104）增加字段：

```json
{ "success": true, "models": [...], "source": "probed" | "fallback", "probeError": "..."? , "cached": bool }
```

- `detect_agent_models`（detection.rs:65）返回类型改为携带来源：`(models, reasoning, source, probe_error)`——`probe_models` 返回 `Ok(models)` / `Err(reason)`（现在是 `Option`，改 `Result<Vec<_>, String>`，`None` 场景语义化为错误串）。
- 负缓存：探测失败时以 30s TTL 缓存 fallback 结果（state.rs 现有 `set_cached_external_agent_models` 加 TTL 变体，或存 `(models, source)` 元组）——防止用户反复打开下拉连续触发 15s 探测。force 刷新绕过。

## 4. 前端（R4）

- `api.ts::detectExternalAgentModels`：透传 `source`/`probeError`。
- `ExternalModelSelector`（RuntimePicker.tsx）：
  - 加载中：下拉 trigger 显示 spinner（现无加载态）。
  - `source === 'fallback'`：模型列表头部显示一行降级提示（"探测失败，显示默认列表"）+ 重试按钮（force=true 重新调用）；`.catch` 不再静默——置 error 态显示同一提示。
  - 真实列表（probed）无附加 UI。
- `RuntimePicker` popover 头部加刷新 IconButton（`detectExternalAgents(true)`），用 `src/components/Button.tsx` 的 `IconButton` xs 规格。
- i18n：文案走 chat 侧现有的中文硬编码风格（该组件现状即中文字面量，保持一致）。

## 5. defs 修正（R5）

- pi（defs/pi.rs:43-48）：删除 `--append-system-prompt <dir>` 循环。查 `pi --help`：若有 `--add-dir`/allowed-dir 等价 flag 则替换；无则彻底移除（附件目录路径已在 prompt 文本的附件说明块里，CLI 读文件靠自身权限模型）。
- kimi：跑 `kimi --help` 查列模型子命令；有 → 补 `list_models_args`；无 → 静态表更新为当前主流型号并依赖 fallback 标记。
- fallback_models 校准：以本机 CLI 实探结果（人工跑一次各 CLI 列模型）更新 defs 静态表；audit F 节版本为基线。
- B4：`detect_availability_all` 的 `if let Ok(agent)` 加 else 分支 `eprintln!` join error。

## 影响面与回滚

- 触碰：`run.rs`（一处前置检查）、`detection.rs`、`session/acp.rs`（仅 detect_acp_models 函数）、`commands.rs`、`state.rs`（负缓存 TTL）、`defs/pi.rs`/`defs/kimi.rs`（+校准的其他 defs）、前端 `RuntimePicker.tsx`/`api.ts`。
- 与 session-lifecycle 的 acp.rs 交集仅 `detect_acp_models` 一个函数（session 驱动不碰）；run.rs 交集仅回复前置一处。冲突时以 session-lifecycle 先合，本任务 rebase。
- 返回值加字段为向后兼容扩展；前端旧版本忽略新字段不受影响。
