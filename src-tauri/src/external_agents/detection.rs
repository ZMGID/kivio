use std::path::Path;
use std::time::Duration;

use crate::external_agents::registry::AGENT_DEFS;
use crate::external_agents::session::acp::detect_acp_models;
use crate::external_agents::session::claude_init::detect_claude_models;
use crate::external_agents::session::pi_rpc::parse_pi_models;
use crate::external_agents::types::{
    default_model_option, fallback_models_from_pairs, reasoning_options_from_pairs, DetectedAgent,
    ModelProbeStrategy, RuntimeAgentDef, RuntimeModelOption,
};
use crate::proc::NoConsoleWindow;

pub const EXTERNAL_AGENT_MODELS_CACHE_TTL: Duration = Duration::from_secs(300);

/// 可用性缓存与 cwd 无关（binary/version/auth 都不随目录变），用全局常量 key + 长 TTL。
/// 换会话直接命中，不再重测。手动刷新（force）绕过。
pub const AVAILABILITY_CACHE_KEY: &str = "__availability__";
pub const AVAILABILITY_CACHE_TTL: Duration = Duration::from_secs(600);

/// 只探可用性（binary + version + auth），**不跑昂贵的模型探测**（claude 达 25s）。
/// models 回填 `fallback_models`——列表阶段不展示真实模型，选中后由模型层懒查覆盖。
pub async fn detect_availability_single(def: &RuntimeAgentDef) -> DetectedAgent {
    let path = super::spawn::resolve_binary(def).await;
    let available = path.is_some();
    let version = if available {
        probe_version(def, path.as_deref()).await
    } else {
        None
    };
    let auth_status = if available {
        probe_auth(def, path.as_deref()).await
    } else {
        Some("unavailable".to_string())
    };
    DetectedAgent {
        id: def.id.to_string(),
        name: def.name.to_string(),
        available,
        path: path.map(|p| p.to_string_lossy().into_owned()),
        version,
        models: fallback_models_from_pairs(def.fallback_models),
        reasoning_options: reasoning_options_from_pairs(def.reasoning_options),
        sandbox_options: sandbox_options_for(def.id),
        auth_status,
    }
}

/// 并发探测所有 CLI 的可用性（cwd 无关）。
pub async fn detect_availability_all() -> Vec<DetectedAgent> {
    let handles: Vec<_> = AGENT_DEFS
        .iter()
        .map(|def| tokio::spawn(async move { detect_availability_single(def).await }))
        .collect();
    let mut out = Vec::with_capacity(handles.len());
    for handle in handles {
        if let Ok(agent) = handle.await {
            out.push(agent);
        }
    }
    out
}

/// 只探单个 agent 的模型（cwd-scoped），供懒查命令用。返回 (models, reasoning_options)。
pub async fn detect_agent_models(
    def: &RuntimeAgentDef,
    cwd: &Path,
) -> (Vec<RuntimeModelOption>, Vec<RuntimeModelOption>) {
    let path = super::spawn::resolve_binary(def).await;
    let models = if path.is_some() {
        probe_models(def, path.as_deref(), cwd)
            .await
            .unwrap_or_else(|| fallback_models_from_pairs(def.fallback_models))
    } else {
        fallback_models_from_pairs(def.fallback_models)
    };
    (models, reasoning_options_from_pairs(def.reasoning_options))
}

pub async fn detect_all_agents(cwd: &Path) -> Vec<DetectedAgent> {
    // ponytail: 全量含模型的一次性检测。普通列表已改走 detect_availability_all + 懒查模型；
    // 保留此函数供潜在的"诊断/一键全量"用途，无调用方时不产生警告（pub 视为对外 API）。
    let handles: Vec<_> = AGENT_DEFS
        .iter()
        .map(|def| {
            let cwd = cwd.to_path_buf();
            tokio::spawn(async move { detect_single_agent(def, &cwd).await })
        })
        .collect();
    let mut out = Vec::with_capacity(handles.len());
    for handle in handles {
        if let Ok(agent) = handle.await {
            out.push(agent);
        }
    }
    out
}

pub async fn detect_single_agent(def: &RuntimeAgentDef, cwd: &Path) -> DetectedAgent {
    let path = super::spawn::resolve_binary(def).await;
    let available = path.is_some();
    let version = if available {
        probe_version(def, path.as_deref()).await
    } else {
        None
    };
    let auth_status = if available {
        probe_auth(def, path.as_deref()).await
    } else {
        Some("unavailable".to_string())
    };
    let models = if available {
        probe_models(def, path.as_deref(), cwd)
            .await
            .unwrap_or_else(|| fallback_models_from_pairs(def.fallback_models))
    } else {
        fallback_models_from_pairs(def.fallback_models)
    };

    DetectedAgent {
        id: def.id.to_string(),
        name: def.name.to_string(),
        available,
        path: path.map(|p| p.to_string_lossy().into_owned()),
        version,
        models,
        reasoning_options: reasoning_options_from_pairs(def.reasoning_options),
        sandbox_options: sandbox_options_for(def.id),
        auth_status,
    }
}

/// Sandbox/permission levels offered per agent. Ids are the agent's native flag values so
/// `build_args` can pass them straight through (claude `--permission-mode`, codex `--sandbox`).
/// Agents without a meaningful sandbox flag return an empty list (no capsule shown).
pub fn sandbox_options_for(agent_id: &str) -> Vec<RuntimeModelOption> {
    let pairs: &[(&str, &str)] = match agent_id {
        "claude" => &[
            ("plan", "计划 (只读)"),
            ("acceptEdits", "接受编辑"),
            ("bypassPermissions", "完全 (默认)"),
        ],
        "codex" => &[
            ("read-only", "只读"),
            ("workspace-write", "工作区写 (默认)"),
            ("danger-full-access", "完全"),
        ],
        _ => &[],
    };
    pairs
        .iter()
        .map(|(id, label)| RuntimeModelOption {
            id: (*id).to_string(),
            label: (*label).to_string(),
            context_window_tokens: None,
        })
        .collect()
}

async fn probe_version(def: &RuntimeAgentDef, path: Option<&std::path::Path>) -> Option<String> {
    let bin = path?;
    let output = tokio::process::Command::new(bin)
        .args(def.version_args)
        .no_console_window()
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().next()?.trim();
    if line.is_empty() {
        None
    } else {
        Some(line.to_string())
    }
}

async fn probe_auth(def: &RuntimeAgentDef, path: Option<&std::path::Path>) -> Option<String> {
    let args = def.auth_probe_args?;
    let bin = path?;
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::process::Command::new(bin)
            .args(args)
            .no_console_window()
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if output.status.success() {
        Some("ok".to_string())
    } else {
        Some("auth_required".to_string())
    }
}

async fn probe_models(
    def: &RuntimeAgentDef,
    path: Option<&std::path::Path>,
    cwd: &Path,
) -> Option<Vec<RuntimeModelOption>> {
    let bin = path?;

    // OpenCode's native command is the source of truth for merged global/project JSONC config.
    // Older versions without `models` fall through to ACP, then the static definition fallback.
    if def.id == "opencode" {
        let timeout_secs = def.list_models_timeout_secs.unwrap_or(15);
        if let Some(models) = probe_opencode_models(bin, cwd, timeout_secs).await {
            return Some(models);
        }
    }

    if def.model_probe == Some(ModelProbeStrategy::Acp) {
        let args: Vec<&str> = def.model_probe_args?.iter().copied().collect();
        let timeout_secs = def.list_models_timeout_secs.unwrap_or(15);
        return detect_acp_models(bin, &args, cwd, timeout_secs).await;
    }

    if def.model_probe == Some(ModelProbeStrategy::ClaudeInit) {
        let timeout_secs = def.list_models_timeout_secs.unwrap_or(25);
        return tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            detect_claude_models(bin, cwd),
        )
        .await
        .ok()
        .flatten();
    }

    let args = def.list_models_args?;
    let timeout_secs = def.list_models_timeout_secs.unwrap_or(5);
    let output = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        tokio::process::Command::new(bin)
            .args(args)
            .current_dir(cwd)
            .no_console_window()
            .output(),
    )
    .await
    .ok()?
    .ok()?;

    // Pi prints its model table to stdout (the `models_from_stderr` name is historical — older
    // builds used stderr). Prefer whichever stream actually has content, then parse the table.
    if def.models_from_stderr {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let text = if !stdout.trim().is_empty() {
            stdout
        } else {
            stderr
        };
        return parse_pi_models(text.as_ref());
    }

    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    parse_models_list(def.id, text.as_ref())
}

async fn probe_opencode_models(
    bin: &Path,
    cwd: &Path,
    timeout_secs: u64,
) -> Option<Vec<RuntimeModelOption>> {
    let output = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        tokio::process::Command::new(bin)
            .arg("models")
            .current_dir(cwd)
            .no_console_window()
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_opencode_models(String::from_utf8_lossy(&output.stdout).as_ref())
}

fn parse_opencode_models(stdout: &str) -> Option<Vec<RuntimeModelOption>> {
    let mut out = vec![default_model_option()];
    let mut seen = std::collections::HashSet::from(["default".to_string()]);
    for line in stdout.lines() {
        let id = line.trim();
        if id.is_empty() || id.chars().any(char::is_whitespace) {
            continue;
        }
        let Some((provider, model)) = id.split_once('/') else {
            continue;
        };
        if provider.is_empty() || model.is_empty() || !seen.insert(id.to_string()) {
            continue;
        }
        out.push(RuntimeModelOption {
            id: id.to_string(),
            label: id.to_string(),
            context_window_tokens: None,
        });
    }
    (out.len() > 1).then_some(out)
}

fn parse_models_list(agent_id: &str, stdout: &str) -> Option<Vec<RuntimeModelOption>> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() || trimmed.to_lowercase().contains("no models available") {
        return None;
    }
    let mut out = vec![default_model_option()];
    match agent_id {
        "codex" => {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
                if let Some(models) = value.get("models").and_then(|v| v.as_array()) {
                    for entry in models {
                        let id = entry
                            .get("slug")
                            .or_else(|| entry.get("id"))
                            .and_then(|v| v.as_str())?;
                        out.push(RuntimeModelOption {
                            id: id.to_string(),
                            label: id.to_string(),
                            // codex reports the real window per model (e.g. 272000); without
                            // it the context gauge falls back to the generic 200K estimate.
                            context_window_tokens: entry
                                .get("context_window")
                                .and_then(|v| v.as_u64())
                                .map(|v| v as u32),
                        });
                    }
                }
            }
            // "Default" = codex picks its own default (the first listed model), so give the
            // synthetic entry that model's window instead of leaving it unknown.
            if out.len() > 1 {
                out[0].context_window_tokens = out[1].context_window_tokens;
            }
        }
        _ => {}
    }
    if out.len() > 1 {
        Some(out)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires live pi CLI on PATH"]
    async fn live_pi_models_from_config_not_fallback() {
        use crate::external_agents::registry::get_agent_def;
        let def = get_agent_def("pi").expect("pi def");
        let detected = detect_single_agent(def, &std::env::temp_dir()).await;
        assert!(detected.available, "pi should be on PATH");
        for m in &detected.models {
            eprintln!("  {} -> {}", m.id, m.label);
        }
        // Real discovered models, not the bogus generic fallback.
        assert!(
            detected.models.iter().any(|m| m.id.contains('/')
                && !m.id.starts_with("anthropic/")
                && !m.id.starts_with("openai/")),
            "expected user-configured pi models, got: {:?}",
            detected.models.iter().map(|m| &m.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn parse_codex_json_models() {
        let models = parse_models_list(
            "codex",
            r#"{"models":[{"slug":"gpt-5.3-codex","context_window":272000},{"slug":"o3"}]}"#,
        )
        .unwrap();
        assert!(models.iter().any(|m| m.id == "gpt-5.3-codex"));
        assert!(models.iter().any(|m| m.id == "o3"));
        // Real window is carried through, and "Default" inherits the first model's window.
        let sol = models.iter().find(|m| m.id == "gpt-5.3-codex").unwrap();
        assert_eq!(sol.context_window_tokens, Some(272000));
        assert_eq!(models[0].id, "default");
        assert_eq!(models[0].context_window_tokens, Some(272000));
        let o3 = models.iter().find(|m| m.id == "o3").unwrap();
        assert_eq!(o3.context_window_tokens, None);
    }

    #[test]
    fn parse_opencode_models_accepts_custom_providers_and_variants() {
        let models =
            parse_opencode_models("custom/minimax-m2.7\ncustom/deep/model-v1\nopenai/gpt-5\n")
                .unwrap();
        assert!(models.iter().any(|model| model.id == "custom/minimax-m2.7"));
        assert!(models
            .iter()
            .any(|model| model.id == "custom/deep/model-v1"));
        assert!(models.iter().any(|model| model.id == "openai/gpt-5"));
    }

    #[test]
    fn parse_opencode_models_ignores_invalid_and_duplicate_lines() {
        let models = parse_opencode_models(
            "custom/model-a\ncustom/model-a\ninvalid\n/providerless\nprovider/\nlog line here\n",
        )
        .unwrap();
        assert_eq!(
            models
                .iter()
                .filter(|model| model.id == "custom/model-a")
                .count(),
            1
        );
        assert_eq!(models.len(), 2, "default plus one valid custom model");
    }

    #[test]
    fn parse_opencode_models_returns_none_without_valid_models() {
        assert!(parse_opencode_models("").is_none());
        assert!(
            parse_opencode_models("invalid\n/providerless\nprovider/\nlog line here\n").is_none()
        );
    }
}
