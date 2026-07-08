use std::time::Duration;

use crate::external_agents::registry::AGENT_DEFS;
use crate::external_agents::session::acp::detect_acp_models;
use crate::external_agents::session::claude_init::detect_claude_models;
use crate::external_agents::session::pi_rpc::parse_pi_models;
use crate::external_agents::types::{
    fallback_models_from_pairs, reasoning_options_from_pairs, DetectedAgent, ModelProbeStrategy,
    RuntimeAgentDef, RuntimeModelOption,
};
use crate::proc::NoConsoleWindow;

pub const EXTERNAL_AGENT_MODELS_CACHE_TTL: Duration = Duration::from_secs(300);

pub async fn detect_all_agents() -> Vec<DetectedAgent> {
    // Probe every CLI concurrently — each detection does a binary lookup + version + auth
    // (5s timeout) + model probe, so serial detection stacked those latencies. Order is
    // preserved by collecting the join handles in registry order.
    let handles: Vec<_> = AGENT_DEFS
        .iter()
        .map(|def| tokio::spawn(detect_single_agent(def)))
        .collect();
    let mut out = Vec::with_capacity(handles.len());
    for handle in handles {
        if let Ok(agent) = handle.await {
            out.push(agent);
        }
    }
    out
}

pub async fn detect_single_agent(def: &RuntimeAgentDef) -> DetectedAgent {
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
        probe_models(def, path.as_deref())
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
) -> Option<Vec<RuntimeModelOption>> {
    let bin = path?;

    if def.model_probe == Some(ModelProbeStrategy::Acp) {
        let args: Vec<&str> = def.model_probe_args?.iter().copied().collect();
        let timeout_secs = def.list_models_timeout_secs.unwrap_or(15);
        let cwd = std::env::temp_dir();
        return detect_acp_models(bin, &args, &cwd, timeout_secs).await;
    }

    if def.model_probe == Some(ModelProbeStrategy::ClaudeInit) {
        let timeout_secs = def.list_models_timeout_secs.unwrap_or(25);
        let cwd = std::env::temp_dir();
        return tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            detect_claude_models(bin, &cwd),
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

fn parse_models_list(agent_id: &str, stdout: &str) -> Option<Vec<RuntimeModelOption>> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() || trimmed.to_lowercase().contains("no models available") {
        return None;
    }
    // No seeded "Default" entry — return only the real models the CLI reports.
    let mut out: Vec<RuntimeModelOption> = Vec::new();
    match agent_id {
        "cursor-agent" => {
            for line in trimmed.lines().map(str::trim).filter(|l| !l.is_empty()) {
                if line.eq_ignore_ascii_case("available models")
                    || line.eq_ignore_ascii_case("models")
                {
                    continue;
                }
                let id = line.split_whitespace().next()?.to_string();
                if id == "default" {
                    continue;
                }
                out.push(RuntimeModelOption {
                    id: id.clone(),
                    label: id,
                    context_window_tokens: None,
                });
            }
        }
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
                            context_window_tokens: None,
                        });
                    }
                }
            }
        }
        "opencode" => {
            for line in trimmed.lines().map(str::trim).filter(|l| !l.is_empty()) {
                if line.contains('/') {
                    out.push(RuntimeModelOption {
                        id: line.to_string(),
                        label: line.to_string(),
                        context_window_tokens: None,
                    });
                }
            }
        }
        _ => {}
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
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
        let detected = detect_single_agent(def).await;
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
    fn parse_cursor_models_skips_header() {
        let models = parse_models_list(
            "cursor-agent",
            "Available models\nauto\nsonnet-4 - Sonnet 4",
        )
        .unwrap();
        assert!(models.iter().any(|m| m.id == "auto"));
        assert!(models.iter().any(|m| m.id == "sonnet-4"));
    }

    #[test]
    fn parse_opencode_line_models() {
        let models =
            parse_models_list("opencode", "anthropic/claude-sonnet-4-5\nopenai/gpt-5").unwrap();
        assert!(models.iter().any(|m| m.id == "anthropic/claude-sonnet-4-5"));
    }
}
