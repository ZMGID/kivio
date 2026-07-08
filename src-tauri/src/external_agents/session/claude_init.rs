use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::external_agents::types::RuntimeModelOption;

/// `--model` aliases accepted by Claude Code, used to build a static model catalog with
/// labels + context windows (no per-alias process probe). The CLI validates the alias at
/// run time, so an unsupported alias simply fails that turn rather than the picker load.
const CLAUDE_MODEL_ALIASES: &[&str] = &[
    "opus",
    "sonnet",
    "sonnet[1m]",
    "opus[1m]",
    "haiku",
    "fable",
    "fable[1m]",
];

/// `env.*` keys in `~/.claude/settings.json` (and the matching process env vars) that
/// point Claude Code at a custom/third-party model. We surface these as extra `--model`
/// targets so a user's gateway/bedrock setup shows up in the picker. These are the
/// Claude CLI's own public env interface.
const CLAUDE_ENV_MODEL_KEYS: &[&str] = &[
    "ANTHROPIC_MODEL",
    "ANTHROPIC_SMALL_FAST_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
];

pub fn context_window_from_claude_resolved_model(resolved: &str) -> Option<u32> {
    let trimmed = resolved.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.to_ascii_lowercase().ends_with("[1m]") {
        return Some(1_000_000);
    }
    if trimmed.to_ascii_lowercase().contains("claude-") {
        return Some(200_000);
    }
    None
}

pub fn context_window_from_claude_model_alias(alias: &str) -> Option<u32> {
    let alias = alias.trim();
    if alias.is_empty() {
        return None;
    }
    if alias.to_ascii_lowercase().contains("[1m]") {
        return Some(1_000_000);
    }
    if CLAUDE_MODEL_ALIASES.contains(&alias) {
        return Some(200_000);
    }
    None
}

fn title_case_token(token: &str) -> String {
    let lower = token.to_ascii_lowercase();
    if lower.is_empty() {
        return lower;
    }
    let mut chars = lower.chars();
    let first = chars.next().unwrap().to_ascii_uppercase().to_string();
    first + chars.as_str()
}

/// Build Claude's model picker list. Entirely static — the CLI's `system/init` event used to
/// be probed to label a synthetic "Default" option, but that duplicated whichever alias the CLI
/// already resolves to (Opus / Fable / etc.) and made "Default" mean different things after the
/// user changed their gateway model. The current list is: built-in aliases + any custom model
/// ids the user configured through Claude Code's own env / settings.json interface.
pub async fn detect_claude_models(
    _resolved_bin: &Path,
    _cwd: &Path,
) -> Option<Vec<RuntimeModelOption>> {
    let mut out: Vec<RuntimeModelOption> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for &alias in CLAUDE_MODEL_ALIASES {
        if seen.insert(alias.to_string()) {
            out.push(catalog_model_option(alias));
        }
    }

    for model in claude_config_models() {
        if seen.insert(model.clone()) {
            out.push(RuntimeModelOption {
                context_window_tokens: context_window_from_claude_resolved_model(&model),
                label: model.clone(),
                id: model,
            });
        }
    }

    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Static catalog entry for a Claude `--model` alias — label + context window with no probe.
fn catalog_model_option(alias: &str) -> RuntimeModelOption {
    let is_1m = alias.to_ascii_lowercase().ends_with("[1m]");
    let base = alias
        .get(..alias.len().saturating_sub(if is_1m { 4 } else { 0 }))
        .unwrap_or(alias);
    let family = title_case_token(base);
    RuntimeModelOption {
        id: alias.to_string(),
        label: if is_1m {
            format!("{family} (1M context)")
        } else {
            family
        },
        context_window_tokens: context_window_from_claude_model_alias(alias),
    }
}

/// Config dir Claude Code reads: `$CLAUDE_CONFIG_DIR`, else `~/.claude`.
fn claude_config_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        let dir = dir.trim();
        if !dir.is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    directories::BaseDirs::new().map(|base| base.home_dir().join(".claude"))
}

/// Extra model ids the user configured for Claude Code via settings.json `env.*` and process
/// env vars (e.g. a gateway/bedrock model). Returns deduped, non-empty ids in discovery order.
fn claude_config_models() -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let push = |raw: &str, out: &mut Vec<String>, seen: &mut HashSet<String>| {
        let model = raw.trim();
        if !model.is_empty() && seen.insert(model.to_string()) {
            out.push(model.to_string());
        }
    };

    if let Some(text) =
        claude_config_dir().and_then(|dir| std::fs::read_to_string(dir.join("settings.json")).ok())
    {
        if let Ok(value) = serde_json::from_str::<Value>(&text) {
            if let Some(env) = value.get("env").and_then(|v| v.as_object()) {
                for key in CLAUDE_ENV_MODEL_KEYS {
                    if let Some(model) = env.get(*key).and_then(|v| v.as_str()) {
                        push(model, &mut out, &mut seen);
                    }
                }
            }
        }
    }

    for key in CLAUDE_ENV_MODEL_KEYS {
        if let Ok(model) = std::env::var(key) {
            push(&model, &mut out, &mut seen);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_window_from_resolved_model() {
        assert_eq!(
            context_window_from_claude_resolved_model("claude-opus-4-8[1m]"),
            Some(1_000_000)
        );
        assert_eq!(
            context_window_from_claude_resolved_model("claude-sonnet-4-6"),
            Some(200_000)
        );
    }

    #[test]
    fn context_window_from_alias() {
        assert_eq!(
            context_window_from_claude_model_alias("sonnet[1m]"),
            Some(1_000_000)
        );
        assert_eq!(
            context_window_from_claude_model_alias("sonnet"),
            Some(200_000)
        );
        assert_eq!(context_window_from_claude_model_alias(""), None);
    }

    #[test]
    fn catalog_options_have_labels_and_windows() {
        let opus = catalog_model_option("opus");
        assert_eq!(opus.id, "opus");
        assert_eq!(opus.label, "Opus");
        assert_eq!(opus.context_window_tokens, Some(200_000));

        let sonnet_1m = catalog_model_option("sonnet[1m]");
        assert_eq!(sonnet_1m.id, "sonnet[1m]");
        assert_eq!(sonnet_1m.label, "Sonnet (1M context)");
        assert_eq!(sonnet_1m.context_window_tokens, Some(1_000_000));
    }

    #[test]
    fn full_catalog_covers_every_alias_without_spawn() {
        for &alias in CLAUDE_MODEL_ALIASES {
            let option = catalog_model_option(alias);
            assert_eq!(option.id, alias);
            assert!(!option.label.is_empty());
            assert!(option.context_window_tokens.is_some());
        }
    }

    #[test]
    fn parse_context_window_label_still_works() {
        use crate::external_agents::context::parse_context_window_label;
        assert_eq!(parse_context_window_label("1m"), Some(1_000_000));
        assert_eq!(parse_context_window_label("200K"), Some(200_000));
    }

    #[tokio::test]
    async fn detect_claude_models_returns_full_catalog_without_default() {
        // The picker is purely static now — no CLI probe, no synthetic "default" entry.
        let cwd = std::env::temp_dir();
        let models = detect_claude_models(std::path::Path::new(""), &cwd)
            .await
            .expect("catalog is never empty");
        assert!(models.iter().all(|m| m.id != "default"));
        assert!(models.iter().any(|m| m.id == "opus"));
        assert!(models.iter().any(|m| m.id == "sonnet[1m]"));
    }
}
