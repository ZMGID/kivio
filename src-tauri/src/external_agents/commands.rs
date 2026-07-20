use tauri::AppHandle;

use crate::chat::storage::{load_conversation, save_conversation};
use crate::chat::types::AgentRuntimeConfig;
use crate::external_agents::detection::{
    detect_agent_models, detect_availability_all, AVAILABILITY_CACHE_KEY, AVAILABILITY_CACHE_TTL,
    EXTERNAL_AGENT_MODELS_CACHE_TTL, EXTERNAL_AGENT_MODELS_FALLBACK_TTL,
};
use crate::external_agents::registry::get_agent_def;
use crate::external_agents::slash::{cache_key, list_external_cli_slash_commands};
use crate::external_agents::types::{CachedAgentModels, ModelSource};
use crate::external_agents::workspace::resolve_detection_cwd;
use crate::state::AppState;

#[tauri::command]
pub async fn chat_detect_external_agents(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    force_refresh: Option<bool>,
    conversation_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let _ = (&app, &conversation_id); // 可用性与 cwd 无关；参数保留为兼容前端签名。
    let force = force_refresh.unwrap_or(false);
    if !force {
        if let Some(agents) =
            state.get_cached_detected_agents(AVAILABILITY_CACHE_KEY, AVAILABILITY_CACHE_TTL)
        {
            return Ok(serde_json::json!({
                "success": true,
                "agents": agents,
                "cached": true,
            }));
        }
    }

    // single-flight：并发调用只实跑一次；后到者持锁后复查缓存即命中。
    let _guard = state.availability_probe_lock.lock().await;
    if !force {
        if let Some(agents) =
            state.get_cached_detected_agents(AVAILABILITY_CACHE_KEY, AVAILABILITY_CACHE_TTL)
        {
            return Ok(serde_json::json!({
                "success": true,
                "agents": agents,
                "cached": true,
            }));
        }
    }
    let agents = detect_availability_all().await;
    state.set_cached_detected_agents(AVAILABILITY_CACHE_KEY.to_string(), agents.clone());
    Ok(serde_json::json!({
        "success": true,
        "agents": agents,
        "cached": false,
    }))
}

/// 懒查：只探一个指定 agent 的模型（cwd-scoped），single-flight + 缓存。前端在选中该 agent /
/// 打开其模型下拉时调用，避免列表阶段对所有 CLI 跑昂贵的模型探测（claude 达 25s）。
#[tauri::command]
pub async fn chat_detect_external_agent_models(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    agent_id: String,
    conversation_id: Option<String>,
    force: Option<bool>,
) -> Result<serde_json::Value, String> {
    let force = force.unwrap_or(false);
    let def = get_agent_def(&agent_id).ok_or_else(|| format!("未知外部 Agent: {agent_id}"))?;
    let cwd = resolve_detection_cwd(&app, conversation_id.as_deref())?;
    let cwd_key = cwd.to_string_lossy().into_owned();
    let key = cache_key(&agent_id, &cwd_key);

    if !force {
        if let Some(cached) = state.get_cached_external_agent_models(
            &key,
            EXTERNAL_AGENT_MODELS_CACHE_TTL,
            EXTERNAL_AGENT_MODELS_FALLBACK_TTL,
        ) {
            return Ok(cached_models_payload(def, &cached, true));
        }
    }

    let lock = state.model_probe_lock_for(&key);
    let _guard = lock.lock().await;
    if !force {
        if let Some(cached) = state.get_cached_external_agent_models(
            &key,
            EXTERNAL_AGENT_MODELS_CACHE_TTL,
            EXTERNAL_AGENT_MODELS_FALLBACK_TTL,
        ) {
            return Ok(cached_models_payload(def, &cached, true));
        }
    }
    let probe = detect_agent_models(def, &cwd).await;
    if !probe.models.is_empty() {
        // probed 长 TTL，fallback 短 TTL 负缓存——由 get 侧按 source 分别裁定过期。
        state.set_cached_external_agent_models(
            key,
            CachedAgentModels {
                models: probe.models.clone(),
                source: probe.source,
                current_model: probe.current_model.clone(),
                current_reasoning: probe.current_reasoning.clone(),
            },
        );
    }
    let mut payload = serde_json::json!({
        "success": true,
        "models": probe.models,
        "reasoningOptions": probe.reasoning_options,
        "source": probe.source.as_str(),
        "cached": false,
    });
    if let Some(model) = probe.current_model {
        payload["currentModel"] = serde_json::Value::String(model);
    }
    if let Some(reasoning) = probe.current_reasoning {
        payload["currentReasoning"] = serde_json::Value::String(reasoning);
    }
    if probe.source == ModelSource::Fallback {
        if let Some(err) = probe.probe_error {
            payload["probeError"] = serde_json::Value::String(err);
        }
    }
    Ok(payload)
}

/// 组装缓存命中的返回 JSON：模型 + reasoning 选项（从 def 静态表）+ 来源 + CLI 当前配置。
fn cached_models_payload(
    def: &crate::external_agents::types::RuntimeAgentDef,
    cached: &CachedAgentModels,
    cached_flag: bool,
) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "success": true,
        "models": cached.models,
        "reasoningOptions": crate::external_agents::types::reasoning_options_from_pairs(def.reasoning_options),
        "source": cached.source.as_str(),
        "cached": cached_flag,
    });
    if let Some(model) = &cached.current_model {
        payload["currentModel"] = serde_json::Value::String(model.clone());
    }
    if let Some(reasoning) = &cached.current_reasoning {
        payload["currentReasoning"] = serde_json::Value::String(reasoning.clone());
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
pub fn chat_set_agent_runtime(
    app: AppHandle,
    conversation_id: String,
    agent_runtime: AgentRuntimeConfig,
) -> Result<serde_json::Value, String> {
    let mut conversation = load_conversation(&app, &conversation_id)?;
    conversation.agent_runtime = agent_runtime;
    conversation.updated_at = chrono::Local::now().timestamp();
    save_conversation(&app, &conversation)?;
    Ok(serde_json::json!({
        "success": true,
        "conversation": conversation,
    }))
}
