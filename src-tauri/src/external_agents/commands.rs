use tauri::{AppHandle, Emitter, Manager};

use crate::chat::types::AgentRuntimeConfig;
use crate::external_agents::detection::{
    detect_agent_models, native_provider_summaries, scan_availability, AVAILABILITY_CACHE_KEY,
    AVAILABILITY_CACHE_TTL, EXTERNAL_AGENT_MODELS_CACHE_TTL, EXTERNAL_AGENT_MODELS_FALLBACK_TTL,
};
use crate::external_agents::registry::{get_agent_def, AGENT_DEFS};
use crate::external_agents::slash::{cache_key, list_external_cli_slash_commands};
use crate::external_agents::types::{CachedAgentModels, DetectedAgent, ModelSource};
use crate::external_agents::workspace::resolve_detection_cwd;
use crate::state::AppState;

/// 上次探测到的可用性快照落盘位置。内存缓存只活一个进程，重启后第一次打开设置页 / 运行时
/// 选择器要干等一轮全量探测（9 个 CLI × version+auth 子进程）才有内容——快照就是为了让那一
/// 眼先有东西看。
fn availability_snapshot_path(state: &AppState) -> std::path::PathBuf {
    state.usage_dir.join("external-agent-availability.json")
}

fn load_availability_snapshot(state: &AppState) -> Option<Vec<DetectedAgent>> {
    let content = std::fs::read_to_string(availability_snapshot_path(state)).ok()?;
    serde_json::from_str(&content).ok()
}

fn save_availability_snapshot(state: &AppState, agents: &[DetectedAgent]) {
    let Ok(content) = serde_json::to_string(agents) else {
        return;
    };
    let path = availability_snapshot_path(state);
    if let Err(err) = crate::chat::storage::atomic_write(&path, &content, "外部 CLI 可用性快照")
    {
        eprintln!("[external-agent] 保存可用性快照失败: {err}");
    }
}

#[tauri::command]
pub async fn chat_detect_external_agents(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    force_refresh: Option<bool>,
    conversation_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let _ = &conversation_id; // 可用性与 cwd 无关；参数保留为兼容前端签名。
    let force = force_refresh.unwrap_or(false);
    if !force {
        if let Some(agents) = state
            .external_discovery()
            .get_cached_detected_agents(AVAILABILITY_CACHE_KEY, AVAILABILITY_CACHE_TTL)
        {
            return Ok(serde_json::json!({
                "success": true,
                "agents": stamp_availability(agents),
                "cached": true,
            }));
        }
        // 内存没有（刚启动 / TTL 过期）就先把上次的快照端上去，真探测丢到后台，
        // 探完通过 `external-agents-updated` 推给前端。用户看到的是「立刻有列表，
        // 几秒后自己刷新」，而不是空列表 + 手动点重新扫描。
        if let Some(agents) = load_availability_snapshot(&state) {
            spawn_availability_refresh(app);
            return Ok(serde_json::json!({
                "success": true,
                "agents": stamp_availability(agents),
                "cached": true,
            }));
        }
    }

    // single-flight：并发调用只实跑一次；后到者持锁后复查缓存即命中。
    let _guard = state
        .external_discovery()
        .acquire_availability_probe()
        .await;
    if force {
        crate::external_agents::wsl::invalidate_locator_cache();
        crate::external_agents::spawn::clear_probe_cache();
    } else if let Some(agents) = state
        .external_discovery()
        .get_cached_detected_agents(AVAILABILITY_CACHE_KEY, AVAILABILITY_CACHE_TTL)
    {
        return Ok(serde_json::json!({
            "success": true,
            "agents": stamp_availability(agents),
            "cached": true,
        }));
    }
    let agents = refresh_availability(&app, &state).await;
    Ok(serde_json::json!({
        "success": true,
        "agents": agents,
        "cached": false,
    }))
}

async fn refresh_availability(app: &AppHandle, state: &AppState) -> Vec<DetectedAgent> {
    let discovery = state.external_discovery();
    let previous = discovery
        .get_cached_detected_agents(AVAILABILITY_CACHE_KEY, AVAILABILITY_CACHE_TTL)
        .or_else(|| load_availability_snapshot(state))
        .unwrap_or_default();
    let scan = scan_availability(AGENT_DEFS, &previous).await;
    if scan.complete {
        discovery
            .set_cached_detected_agents(AVAILABILITY_CACHE_KEY.to_string(), scan.agents.clone());
        save_availability_snapshot(state, &scan.agents);
    }
    let agents = stamp_availability(scan.agents);
    let _ = app.emit(
        "external-agents-updated",
        serde_json::json!({ "agents": &agents }),
    );
    agents
}

/// 后台重探可用性并广播结果。single-flight 锁用 `try_lock`：已经有人在探就直接算了，
/// 那一轮探完自己会广播。
fn spawn_availability_refresh(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let Some(_guard) = state.external_discovery().try_acquire_availability_probe() else {
            return;
        };
        refresh_availability(&app, &state).await;
    });
}

/// 解析 Pi 实际使用的 agent 目录（认 `PI_CODING_AGENT_DIR`）。
#[tauri::command]
pub fn chat_external_cli_pi_agent_dir() -> Option<String> {
    crate::external_agents::provider_profile::pi_agent_dir()
        .map(|path| path.to_string_lossy().into_owned())
}

/// 供应商弹窗的「获取模型」：拿填好的 base_url + key 去中转站问模型列表。
/// 只作建议用，拉不到就报错文案，不影响手填。
#[tauri::command]
pub async fn chat_external_cli_fetch_relay_models(
    state: tauri::State<'_, AppState>,
    base_url: String,
    api_key: String,
) -> Result<Vec<String>, String> {
    crate::external_agents::relay_models::fetch(&state.http(), &base_url, &api_key).await
}

/// 扫描本机 cc-switch 的库，列出可导入的供应商。只读，不写任何东西。
#[tauri::command]
pub async fn chat_external_cli_scan_cc_switch() -> Result<serde_json::Value, String> {
    let scan = crate::external_agents::cc_switch::scan()?;
    serde_json::to_value(scan).map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn chat_external_cli_import_cc_switch(
    selections: Vec<super::cc_switch::ImportSelection>,
) -> Result<Vec<super::cc_switch::ImportedProvider>, String> {
    super::cc_switch::import_selected(&selections)
}

/// dsh 的供应商摘要只是读 `settings.yaml`，跟 binary/auth 探测无关，缓存命中也重读，
/// 避免官方 DeepSeek / 用户刚改的 llm-pi-ai 要等 10 分钟才出现在「所有供应商」。
fn stamp_availability(mut agents: Vec<DetectedAgent>) -> Vec<DetectedAgent> {
    for agent in agents.iter_mut() {
        agent.disabled = crate::external_agents::overrides::is_disabled(&agent.id);
        let def = get_agent_def(&agent.id);
        agent.capabilities = def.map(|def| def.capabilities()).unwrap_or_default();
        agent.native_providers = def.map(native_provider_summaries).unwrap_or_default();
    }
    agents
}

/// 用户手填的模型追加到探测结果后面；id 撞车时保留探测出来的那条。
fn merge_custom_models(
    agent_id: &str,
    models: &[crate::external_agents::types::RuntimeModelOption],
) -> Vec<crate::external_agents::types::RuntimeModelOption> {
    let mut merged = models.to_vec();
    for custom in crate::external_agents::overrides::custom_models(agent_id) {
        if merged.iter().any(|m| m.id == custom.id) {
            continue;
        }
        let label = if custom.label.is_empty() {
            custom.id.clone()
        } else {
            custom.label
        };
        merged.push(crate::external_agents::types::RuntimeModelOption {
            capabilities: None,
            id: custom.id,
            label,
            context_window_tokens: None,
        });
    }
    merged
}

fn model_snapshot_key(agent_id: &str, cwd_key: &str) -> String {
    use sha2::{Digest, Sha256};
    // Configurations may contain secrets; persist only their digest, never the configuration.
    let identity = serde_json::json!([agent_id, cwd_key, super::overrides::agent_config(agent_id)]);
    format!("{:x}", Sha256::digest(identity.to_string().as_bytes()))
}

/// Read a display snapshot without spawning a CLI; the same command owns the subsequent refresh.
#[tauri::command]
pub async fn chat_detect_external_agent_models(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    agent_id: String,
    conversation_id: Option<String>,
    force: Option<bool>,
    snapshot_only: Option<bool>,
) -> Result<serde_json::Value, String> {
    let def = get_agent_def(&agent_id).ok_or_else(|| format!("未知外部 Agent: {agent_id}"))?;
    let (models, cached, refreshing) = discover_agent_models(
        &app,
        &state,
        def,
        conversation_id.as_deref(),
        force.unwrap_or(false),
        snapshot_only.unwrap_or(false),
    )
    .await?;
    let mut payload = cached_models_payload(def, &models, cached);
    if let Some(refreshing) = refreshing {
        payload["refreshing"] = refreshing.into();
    }
    Ok(payload)
}

pub(super) async fn discover_agent_models(
    app: &AppHandle,
    state: &AppState,
    def: &crate::external_agents::types::RuntimeAgentDef,
    conversation_id: Option<&str>,
    force: bool,
    snapshot_only: bool,
) -> Result<(CachedAgentModels, bool, Option<bool>), String> {
    let started = std::time::Instant::now();
    let agent_id = def.id;
    let cwd = resolve_detection_cwd(app, conversation_id)?;
    let cwd_key = cwd.to_string_lossy().into_owned();
    let key = cache_key(agent_id, &cwd_key);
    let discovery = state.external_discovery();
    let generation = discovery.model_generation();
    let launch_revision = super::overrides::launch_revision(agent_id)
        .ok_or_else(|| "CLI 配置正在更新，请重试".to_string())?;
    let snapshot_key = model_snapshot_key(agent_id, &cwd_key);
    let snapshot_path = state.usage_dir.join("external-agent-models.json");

    if !force {
        if let Some(cached) = state.external_discovery().get_cached_external_agent_models(
            &key,
            EXTERNAL_AGENT_MODELS_CACHE_TTL,
            EXTERNAL_AGENT_MODELS_FALLBACK_TTL,
        ) {
            return Ok((cached, true, Some(false)));
        }
    }

    let previous = discovery.load_model_snapshot(&snapshot_path, &snapshot_key);
    if snapshot_only {
        let cached = previous.unwrap_or_else(|| CachedAgentModels {
            models: crate::external_agents::types::fallback_models_from_pairs(def.fallback_models),
            source: ModelSource::Fallback,
            reasoning_options: crate::external_agents::types::reasoning_options_from_pairs(
                def.reasoning_options,
            ),
            reasoning_by_model: Default::default(),
            current_model: None,
            current_reasoning: None,
            probe_error: None,
        });
        return Ok((cached, true, Some(true)));
    }

    let _guard = state.external_discovery().acquire_model_probe(&key).await;
    if discovery.model_generation() != generation
        || super::overrides::launch_revision(agent_id) != Some(launch_revision)
    {
        return Err("CLI 配置已变更，请重新获取模型".into());
    }
    if let Some(cached) = discovery.models_completed_since(&key, started) {
        return Ok((cached, true, None));
    }
    if !force {
        if let Some(cached) = state.external_discovery().get_cached_external_agent_models(
            &key,
            EXTERNAL_AGENT_MODELS_CACHE_TTL,
            EXTERNAL_AGENT_MODELS_FALLBACK_TTL,
        ) {
            return Ok((cached, true, None));
        }
    }
    let probe = detect_agent_models(def, &cwd).await;
    if super::overrides::launch_revision(agent_id) != Some(launch_revision) {
        return Err("CLI 配置已变更，请重新获取模型".into());
    }
    let mut cached = CachedAgentModels {
        models: probe.models,
        source: probe.source,
        reasoning_options: probe.reasoning_options,
        reasoning_by_model: probe.reasoning_by_model,
        current_model: probe.current_model,
        current_reasoning: probe.current_reasoning,
        probe_error: probe.probe_error,
    };
    if cached.source == ModelSource::Fallback {
        if let Some(previous) = previous {
            cached = CachedAgentModels {
                source: ModelSource::Fallback,
                probe_error: cached.probe_error,
                ..previous
            };
        }
    }
    if !discovery.set_models_if_current(key, cached.clone(), generation) {
        return Err("CLI 配置已变更，请重新获取模型".into());
    }
    if cached.source == ModelSource::Probed {
        discovery.save_model_snapshot(&snapshot_path, snapshot_key, cached.clone());
    }
    Ok((cached, false, None))
}

fn models_payload(
    models: &[crate::external_agents::types::RuntimeModelOption],
    reasoning_options: &[crate::external_agents::types::RuntimeModelOption],
    reasoning_by_model: &std::collections::HashMap<
        String,
        Vec<crate::external_agents::types::RuntimeModelOption>,
    >,
    source: ModelSource,
    current_model: Option<&str>,
    current_reasoning: Option<&str>,
    cached_flag: bool,
    probe_error: Option<&str>,
) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "success": true,
        "models": models,
        "reasoningOptions": reasoning_options,
        "reasoningByModel": reasoning_by_model,
        "source": source.as_str(),
        "cached": cached_flag,
    });
    if let Some(model) = current_model {
        payload["currentModel"] = serde_json::Value::String(model.to_string());
    }
    if let Some(reasoning) = current_reasoning {
        payload["currentReasoning"] = serde_json::Value::String(reasoning.to_string());
    }
    if source == ModelSource::Fallback {
        if let Some(err) = probe_error {
            payload["probeError"] = serde_json::Value::String(err.to_string());
        }
    }
    payload
}

/// 组装缓存命中的返回 JSON：模型 + 缓存的 reasoning 选项 + 来源 + CLI 当前配置。
/// 缓存里档位为空时回落 def 静态表（claude/codex/pi/grok 有表；ACP 系仍为空）。
fn cached_models_payload(
    def: &crate::external_agents::types::RuntimeAgentDef,
    cached: &CachedAgentModels,
    cached_flag: bool,
) -> serde_json::Value {
    let reasoning_options = if cached.reasoning_options.is_empty() {
        crate::external_agents::types::reasoning_options_from_pairs(def.reasoning_options)
    } else {
        cached.reasoning_options.clone()
    };
    let mut payload = models_payload(
        &merge_custom_models(def.id, &cached.models),
        &reasoning_options,
        &cached.reasoning_by_model,
        cached.source,
        cached.current_model.as_deref(),
        cached.current_reasoning.as_deref(),
        cached_flag,
        cached.probe_error.as_deref(),
    );
    // Select only from the native catalog, before inserting user-added picker entries.
    // A saved runtime and an explicit CLI configuration take precedence over first-use defaults.
    if cached.source == ModelSource::Probed && cached.current_model.is_none() {
        let mut available = cached.models.iter().filter(|model| model.id != "default");
        let preferred = match def.id {
            "codex" => ["gpt-6.2-astra", "gpt-6.2-sol"]
                .iter()
                .find_map(|id| {
                    available
                        .clone()
                        .find(|m| m.id.strip_prefix("openai.").unwrap_or(&m.id) == *id)
                })
                .or_else(|| available.next()), // model/list puts the server default first.
            "claude" => available.next(),
            _ => None,
        };
        if let Some(model) = preferred {
            payload["initialModel"] = model.id.clone().into();
        }
    }
    payload
}

#[tauri::command]
pub async fn chat_list_external_cli_slash_commands(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    agent_id: String,
    conversation_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let (supports, commands, message) =
        list_external_cli_slash_commands(&app, &state, &agent_id, conversation_id.as_deref())
            .await?;
    Ok(serde_json::json!({
        "success": true,
        "supportsSlashCommands": supports,
        "commands": commands,
        "message": message,
    }))
}

#[tauri::command]
pub async fn chat_set_agent_runtime(
    app: AppHandle,
    conversation_id: String,
    agent_runtime: AgentRuntimeConfig,
) -> Result<serde_json::Value, String> {
    let mut conversation = set_agent_runtime(&app, &conversation_id, agent_runtime).await?;
    crate::chat::commands::catalog::strip_transcripts_for_frontend(&mut conversation);
    Ok(serde_json::json!({
        "success": true,
        "conversation": conversation,
    }))
}

pub(crate) async fn set_agent_runtime(
    app: &AppHandle,
    conversation_id: &str,
    agent_runtime: AgentRuntimeConfig,
) -> Result<crate::chat::types::Conversation, String> {
    let persist_agent = agent_runtime.external_agent_id.clone();
    let persist_model = agent_runtime.external_model.clone();
    let persist_reasoning = agent_runtime.external_reasoning.clone();
    let state = app.state::<AppState>();
    let mut reservation = None;
    let mut goal_paused = false;
    let conversation = crate::chat::repository::repository(app)
        .mutate(app, conversation_id, |conversation| {
            use crate::chat::types::AgentProvider;
            if AgentProvider::selected(&conversation.agent_runtime, &conversation.provider_id)
                != AgentProvider::selected(&agent_runtime, &conversation.provider_id)
            {
                goal_paused = conversation
                    .goal_state
                    .as_ref()
                    .is_some_and(|goal| crate::chat::goal::is_running(goal.status));
                reservation = Some(
                    crate::chat::commands::ChatSendReservation::try_acquire(
                        state.inner(),
                        conversation_id,
                    )
                    .ok_or(crate::chat::commands::CHAT_REPLY_BUSY_ERROR)?,
                );
            }
            if agent_runtime.is_external() {
                crate::external_agents::context::reset_changed_external_meter(
                    &mut conversation.context_state,
                    agent_runtime
                        .external_agent_id
                        .as_deref()
                        .unwrap_or_default(),
                    agent_runtime.external_model.as_deref().unwrap_or_default(),
                );
            }
            conversation.agent_runtime = agent_runtime;
            Ok(())
        })
        .await
        .map_err(crate::chat::repository::repository_error)?;
    if goal_paused {
        crate::chat::goal::emit_goal_state(app, &conversation);
    }
    if let Some(agent_id) = persist_agent.as_deref() {
        if let Err(err) = crate::external_agents::provider_profile::persist_selected_model(
            agent_id,
            persist_model.as_deref(),
            persist_reasoning.as_deref(),
        ) {
            eprintln!("[external-agent] 模型落盘失败（{agent_id}）：{err}");
        }
    }
    Ok(conversation)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归：缓存命中必须带回探测到的 reasoning_options。
    /// kimi 等 ACP CLI 的 def.reasoning_options 是空的；若命中时只回落 def 表，
    /// effort 胶囊在第二次打开时消失（bac8f53 漏的那步）。
    #[test]
    fn cached_payload_preserves_probed_reasoning_options() {
        use crate::external_agents::registry::get_agent_def;
        use crate::external_agents::types::{CachedAgentModels, ModelSource, RuntimeModelOption};

        let def = get_agent_def("kimi").expect("kimi def");
        assert!(
            def.reasoning_options.is_empty(),
            "precondition: acp_def 静态档位表为空"
        );

        let cached = CachedAgentModels {
            models: vec![RuntimeModelOption {
                capabilities: None,
                id: "kimi-code/kimi-for-coding".into(),
                label: "K2.7 Coding".into(),
                context_window_tokens: None,
            }],
            source: ModelSource::Probed,
            reasoning_options: vec![
                RuntimeModelOption {
                    capabilities: None,
                    id: "low".into(),
                    label: "Low".into(),
                    context_window_tokens: None,
                },
                RuntimeModelOption {
                    capabilities: None,
                    id: "high".into(),
                    label: "High".into(),
                    context_window_tokens: None,
                },
                RuntimeModelOption {
                    capabilities: None,
                    id: "max".into(),
                    label: "Max".into(),
                    context_window_tokens: None,
                },
            ],
            reasoning_by_model: Default::default(),
            current_model: Some("kimi-code/kimi-for-coding".into()),
            current_reasoning: Some("high".into()),
            probe_error: None,
        };

        let payload = cached_models_payload(def, &cached, true);
        let opts = payload["reasoningOptions"]
            .as_array()
            .expect("reasoningOptions array");
        assert_eq!(opts.len(), 3, "缓存命中应保留探测档位，不能回落空 def 表");
        assert_eq!(opts[0]["id"], "low");
        assert_eq!(opts[2]["id"], "max");
        assert_eq!(payload["currentReasoning"], "high");
    }

    #[test]
    fn first_use_default_uses_the_native_catalog_and_defers_to_configured_models() {
        use crate::external_agents::types::fallback_models_from_pairs;
        let codex = get_agent_def("codex").unwrap();
        let mut cached = CachedAgentModels {
            models: fallback_models_from_pairs(&[
                ("server-default", ""),
                ("gpt-6-astra", ""),
                ("openai.gpt-6.2-astra", ""),
                ("gpt-6.2-sol", ""),
            ]),
            source: ModelSource::Probed,
            reasoning_options: vec![],
            reasoning_by_model: Default::default(),
            current_model: None,
            current_reasoning: None,
            probe_error: None,
        };
        for expected in ["openai.gpt-6.2-astra", "gpt-6.2-sol", "server-default"] {
            assert_eq!(
                cached_models_payload(codex, &cached, false)["initialModel"],
                expected
            );
            cached.models.retain(|m| m.id != expected);
        }
        let claude = get_agent_def("claude").unwrap();
        cached.models = fallback_models_from_pairs(claude.fallback_models);
        assert_eq!(
            cached_models_payload(claude, &cached, false)["initialModel"],
            "claude-opus-5-5"
        );
        cached.current_model = Some("custom-gateway".into());
        assert!(cached_models_payload(claude, &cached, false)["initialModel"].is_null());
        cached.current_model = None;
        cached.source = ModelSource::Fallback;
        assert!(cached_models_payload(claude, &cached, false)["initialModel"].is_null());
    }

    #[test]
    fn legacy_availability_snapshot_reports_current_capabilities() {
        let snapshot = serde_json::json!([{
            "id": "codex",
            "name": "Codex CLI",
            "available": true,
            "path": "codex",
            "version": "0.164.0",
            "models": [],
            "reasoningOptions": [],
            "authStatus": null,
            "supportsSteering": false,
            "supportsFollowUp": true
        }]);
        let agents = serde_json::from_value(snapshot).unwrap();
        let response = serde_json::to_value(super::stamp_availability(agents)).unwrap();
        assert_eq!(response[0]["version"], "0.164.0");
        assert_eq!(response[0]["capabilities"]["steering"], "native");
        assert_eq!(response[0]["capabilities"]["followUp"], "fallback");
        assert_eq!(response[0]["capabilities"]["account"], "native");
        assert!(response[0].get("supportsSteering").is_none());
    }
}

// ---------------------------------------------------------------------------------------------
// 从本地 CLI 导入对话
// ---------------------------------------------------------------------------------------------

/// 列出某个项目下可导入的原生会话（按项目工作目录过滤，已导入的带标记）。
#[tauri::command]
pub async fn chat_list_importable_cli_sessions(
    app: AppHandle,
    project_id: String,
) -> Result<serde_json::Value, String> {
    let sessions =
        crate::external_agents::import::list_importable_for_project(&app, &project_id).await?;
    Ok(serde_json::json!({ "success": true, "sessions": sessions }))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSessionRequest {
    pub agent_id: String,
    pub session_id: String,
}

/// 批量导入。**单条失败不影响其它条**——一次勾十条，不该因为其中一条的文件损坏就全军覆没。
#[tauri::command]
pub async fn chat_import_cli_sessions(
    app: AppHandle,
    project_id: String,
    items: Vec<ImportSessionRequest>,
) -> Result<serde_json::Value, String> {
    let mut imported = Vec::new();
    let mut failures = Vec::new();
    for item in items {
        match crate::external_agents::import::import_one_session(
            &app,
            &project_id,
            &item.agent_id,
            &item.session_id,
        )
        .await
        {
            Ok(conversation_id) => imported.push(serde_json::json!({
                "agentId": item.agent_id,
                "sessionId": item.session_id,
                "conversationId": conversation_id,
            })),
            Err(error) => failures.push(serde_json::json!({
                "agentId": item.agent_id,
                "sessionId": item.session_id,
                "error": error,
            })),
        }
    }
    Ok(serde_json::json!({
        "success": failures.is_empty(),
        "imported": imported,
        "failures": failures,
    }))
}

/// 这条导入的对话，在 CLI 那边是不是已经有新内容了（ADR-0002 的过期提示）。
#[tauri::command]
pub fn chat_imported_history_stale(app: AppHandle, conversation_id: String) -> bool {
    crate::external_agents::import::imported_history_is_stale(&app, &conversation_id)
}

/// 本地 CLI 对话绑定的原生会话 id（live handle，否则 Claude 的 `{id}.json`）。
#[tauri::command]
pub fn chat_external_native_session_id(app: AppHandle, conversation_id: String) -> Option<String> {
    crate::external_agents::session::bound_native_session_id(&app, &conversation_id)
}

#[tauri::command]
pub async fn chat_external_cli_account(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    agent_id: String,
    conversation_id: Option<String>,
    project_id: Option<String>,
    mode: Option<super::account::AccountSnapshotMode>,
) -> Result<Option<serde_json::Value>, String> {
    use super::{
        account::{self, AccountSnapshotMode, PersistedQuota},
        overrides::{launch_revision, LaunchIdentity},
        types::AccountProtocol,
    };
    let requested = std::time::Instant::now();
    let mode = mode.unwrap_or_default();
    let def = get_agent_def(&agent_id).ok_or("未知 CLI")?;
    let probe = def.account.ok_or("此 CLI 未提供账号额度查询")?;
    // These two reports read the CLI profile's account, independent of the chat workspace.
    let (conversation, project) = match probe.protocol {
        AccountProtocol::KimiWeb | AccountProtocol::AntigravityUsage => (None, None),
        _ => (conversation_id.as_deref(), project_id.as_deref()),
    };
    let unavailable = |reason: &str| {
        (mode != AccountSnapshotMode::Cached).then(|| account::unavailable_snapshot(def.id, reason))
    };
    let disk_path = if conversation.is_none() && project.is_none() {
        PersistedQuota::path(&state.usage_dir, def)
    } else {
        None
    };
    for _ in 0..2 {
        let cwd = match (conversation, project) {
            (None, Some(id)) => crate::chat::storage::find_project_by_id(&app, id)
                .and_then(super::workspace::project_cli_root),
            _ => resolve_detection_cwd(&app, conversation),
        };
        let cwd = match cwd {
            Ok(cwd) => cwd,
            Err(reason) => return Ok(unavailable(&reason)),
        };
        let scope = std::fs::canonicalize(&cwd).unwrap_or_else(|_| cwd.clone());
        if mode != AccountSnapshotMode::Refresh {
            if let Some(cached) = state.external_accounts.peek(def, &scope, mode, requested) {
                return Ok(Some(cached));
            }
            if mode == AccountSnapshotMode::Cached
                && !disk_path.as_ref().is_some_and(|path| path.is_file())
            {
                return Ok(None);
            }
        }
        let Some(revision) = launch_revision(def.id) else {
            continue;
        };
        let resolved = super::spawn::resolve_binary(def)
            .await
            .map(|bin| (LaunchIdentity::capture(def, &bin), bin));
        if launch_revision(def.id) != Some(revision) {
            continue;
        }
        let Some((identity, bin)) = resolved else {
            return Ok(unavailable("CLI 未安装或不在 PATH"));
        };
        let disk = PersistedQuota::for_cli(disk_path.clone(), def, &bin, &scope);
        // A mismatched launch or credential invalidates the entire acquisition.
        // Resolve again from scratch; a previous owner's report must never be published.
        let Ok((snapshot, updated)) = state
            .external_accounts
            .read(
                def.id,
                (identity, scope),
                mode,
                requested,
                |launch| launch.ensure_current(def, &bin),
                || async {
                    let _resource = super::session::live::ResourceLease::pending(def.id)?;
                    account::probe(def, probe, &bin, &cwd).await
                },
                disk.as_ref(),
            )
            .await
        else {
            continue;
        };
        if updated {
            let _ = app.emit("external-cli-account-updated", def.id);
        }
        if snapshot.is_some() || mode == AccountSnapshotMode::Cached {
            return Ok(snapshot);
        }
    }
    Ok(unavailable("CLI 账号或启动配置持续变更，请重新刷新额度"))
}

#[derive(serde::Serialize)]
pub struct CodexResetResult {
    outcome: super::account::CodexResetOutcome,
    refreshed: bool,
}

#[tauri::command]
pub(crate) async fn chat_external_cli_redeem_codex_reset_credit(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    target: super::account::CodexResetTarget,
    available_count: u64,
) -> Result<CodexResetResult, String> {
    let def = get_agent_def("codex").ok_or("Codex CLI 不可用")?;
    let probe = def.account.ok_or("Codex CLI 未提供额度查询")?;
    let cwd = resolve_detection_cwd(&app, None)?;
    let bin = super::spawn::resolve_binary(def)
        .await
        .ok_or("Codex CLI 未安装或不在 PATH")?;
    let outcome = state
        .external_accounts
        .redeem_codex_reset_credit(def, probe, &bin, &cwd, target, available_count)
        .await?;
    let refreshed = chat_external_cli_account(
        app,
        state,
        def.id.into(),
        None,
        None,
        Some(super::account::AccountSnapshotMode::Refresh),
    )
    .await
    .ok()
    .flatten()
    .is_some_and(|value| value["refreshError"].is_null() && value["unavailable"].is_null());
    Ok(CodexResetResult { outcome, refreshed })
}
