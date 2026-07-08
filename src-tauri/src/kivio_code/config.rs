//! Persisted kivio-code configuration (distinct from the shared Kivio
//! `Settings`). Stored as `<app_data>/kivio-code/config.json`, resolved via
//! [`settings_loader::app_data_dir`].
//!
//! Today this holds a single toggle: whether to read Claude-Code context files
//! (`CLAUDE.md` ancestors, `~/.claude/CLAUDE.md`, `<cwd>/.claude/CLAUDE.md`) for
//! cross-tool compatibility. Default is ON (like opencode reading legacy
//! `CLAUDE.md`). Missing or malformed config falls back to the default so the
//! CLI never fails to boot over a corrupt config file.

use serde::{Deserialize, Serialize};

use super::settings_loader;

/// kivio-code's own persisted config (not the shared Kivio `Settings`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KivioCodeConfig {
    /// Read Claude-Code context files (`CLAUDE.md` + `.claude/CLAUDE.md`) for
    /// compatibility. Default ON; the `/settings` command flips it.
    #[serde(default = "default_read_claude_dir")]
    pub read_claude_dir: bool,
    /// kivio-code-specific default provider id. When unset (None/empty), model
    /// resolution falls back to the shared `Settings` chat model. A CLI `--provider`
    /// flag still overrides this.
    #[serde(default)]
    pub default_provider_id: Option<String>,
    /// kivio-code-specific default model name (bare, no provider prefix). Paired with
    /// `default_provider_id`. Unset → fall back to the shared chat model. A CLI
    /// `--model` flag still overrides this.
    #[serde(default)]
    pub default_model: Option<String>,
    /// Tool approval policy for kivio-code runs:
    /// `"auto"` | `"readonly_auto_sensitive_confirm"` | `"always_confirm"`.
    /// Unset → the existing default (`"auto"`). A CLI `--no-approve` flag forces
    /// `"always_confirm"` regardless.
    #[serde(default)]
    pub approval_policy: Option<String>,
    /// Auto build→plan switching: when ON and in build mode, the model is offered
    /// an `enter_plan_mode` tool and prompted to call it first for complex /
    /// multi-step / multi-file tasks; the interactive layer then runs a read-only
    /// planning pass and pauses for the user to `proceed`. Default ON; `/autoplan
    /// off` (or the settings overlay) disables it, restoring purely manual
    /// Shift+Tab plan switching.
    #[serde(default = "default_auto_plan")]
    pub auto_plan: bool,
}

fn default_read_claude_dir() -> bool {
    true
}

fn default_auto_plan() -> bool {
    true
}

impl Default for KivioCodeConfig {
    fn default() -> Self {
        Self {
            read_claude_dir: default_read_claude_dir(),
            default_provider_id: None,
            default_model: None,
            approval_policy: None,
            auto_plan: default_auto_plan(),
        }
    }
}

/// Path to `<app_data>/kivio-code/config.json` (None when no home/app dir).
fn config_path() -> Option<std::path::PathBuf> {
    settings_loader::app_data_dir().map(|dir| dir.join("kivio-code").join("config.json"))
}

/// Load the persisted config. Missing/unreadable/malformed → [`KivioCodeConfig::default`]
/// (never panics, never errors).
pub fn load() -> KivioCodeConfig {
    let Some(path) = config_path() else {
        return KivioCodeConfig::default();
    };
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(_) => return KivioCodeConfig::default(),
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

/// Persist the config to `<app_data>/kivio-code/config.json`, creating the
/// directory if needed.
pub fn save(config: &KivioCodeConfig) -> Result<(), String> {
    let path = config_path().ok_or_else(|| "could not resolve app data directory".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
}

/// A project-level config patch (`<project>/.kivio/config.json`). EVERY field is
/// optional — including `read_claude_dir` (unlike [`KivioCodeConfig`] where it is a
/// plain bool) — so an absent key inherits the global value rather than silently
/// overriding it with a serde default. Project settings layer over the global config
/// per key (see [`load_merged`]).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct KivioCodeConfigPatch {
    pub read_claude_dir: Option<bool>,
    pub default_provider_id: Option<String>,
    pub default_model: Option<String>,
    pub approval_policy: Option<String>,
    pub auto_plan: Option<bool>,
}

/// Strictness rank for an approval policy (higher = stricter). Unknown / unset → 0
/// (`auto`, the loosest). Used to enforce the "project can only tighten" rule.
fn approval_rank(policy: Option<&str>) -> u8 {
    match policy {
        Some("always_confirm") => 2,
        Some("readonly_auto_sensitive_confirm") => 1,
        _ => 0, // "auto" or unset/unknown
    }
}

/// Canonical policy string for a rank.
fn approval_for_rank(rank: u8) -> &'static str {
    match rank {
        2 => "always_confirm",
        1 => "readonly_auto_sensitive_confirm",
        _ => "auto",
    }
}

/// Merge a project patch over the global config, per key. Project wins for model /
/// provider / read_claude_dir; `approval_policy` is **tighten-only** — the effective
/// value is the stricter of {global, project}, so a project can never loosen the
/// user's global approval policy (clone-and-run can't silently auto-approve tools).
/// Empty/whitespace string overrides are treated as unset.
pub fn merge_config(mut base: KivioCodeConfig, patch: KivioCodeConfigPatch) -> KivioCodeConfig {
    if let Some(read_claude_dir) = patch.read_claude_dir {
        base.read_claude_dir = read_claude_dir;
    }
    if let Some(auto_plan) = patch.auto_plan {
        base.auto_plan = auto_plan;
    }
    if let Some(provider) = patch
        .default_provider_id
        .filter(|value| !value.trim().is_empty())
    {
        base.default_provider_id = Some(provider);
    }
    if let Some(model) = patch.default_model.filter(|value| !value.trim().is_empty()) {
        base.default_model = Some(model);
    }
    if let Some(project_policy) = patch.approval_policy.filter(|p| !p.trim().is_empty()) {
        let effective = approval_rank(base.approval_policy.as_deref())
            .max(approval_rank(Some(&project_policy)));
        base.approval_policy = Some(approval_for_rank(effective).to_string());
    }
    base
}

/// Find the nearest project config patch by walking from `cwd` up to the filesystem
/// root, returning the first `.kivio/config.json` found (closest to `cwd` wins).
/// Missing/unreadable/malformed files are skipped (never fatal).
fn find_project_patch(cwd: &std::path::Path) -> Option<KivioCodeConfigPatch> {
    let mut dir: Option<&std::path::Path> = Some(cwd);
    while let Some(d) = dir {
        let path = d.join(".kivio").join("config.json");
        if path.is_file() {
            if let Ok(raw) = std::fs::read_to_string(&path) {
                if let Ok(patch) = serde_json::from_str::<KivioCodeConfigPatch>(&raw) {
                    return Some(patch);
                }
            }
        }
        dir = d.parent();
    }
    None
}

/// Load the effective config for a run rooted at `cwd`: the global config with the
/// nearest project `.kivio/config.json` (if any) merged over it per [`merge_config`].
/// When no project config exists this equals [`load`].
pub fn load_merged(cwd: &std::path::Path) -> KivioCodeConfig {
    let global = load();
    match find_project_patch(cwd) {
        Some(patch) => merge_config(global, patch),
        None => global,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_reads_claude_dir() {
        assert!(KivioCodeConfig::default().read_claude_dir);
    }

    #[test]
    fn missing_file_yields_default() {
        // load() reads the real app-data path; on a machine with no config file
        // it must return the default (read_claude_dir = true) rather than erroring.
        let cfg = load();
        // The value is whatever is persisted (default true if absent); the
        // contract is simply that it never panics and yields a valid struct.
        let _ = cfg.read_claude_dir;
    }

    #[test]
    fn deserialize_missing_field_defaults_true() {
        // An empty object (e.g. a forward/backward-compat config) must default the
        // toggle to true via serde default, and the new optional fields to None.
        let cfg: KivioCodeConfig = serde_json::from_str("{}").unwrap();
        assert!(cfg.read_claude_dir);
        assert_eq!(cfg.default_provider_id, None);
        assert_eq!(cfg.default_model, None);
        assert_eq!(cfg.approval_policy, None);
        assert!(cfg.auto_plan);
    }

    #[test]
    fn deserialize_legacy_config_keeps_new_fields_none() {
        // A pre-existing config written before the new fields existed must still load.
        let cfg: KivioCodeConfig = serde_json::from_str(r#"{"readClaudeDir": false}"#).unwrap();
        assert!(!cfg.read_claude_dir);
        assert_eq!(cfg.default_model, None);
        assert_eq!(cfg.approval_policy, None);
        // auto_plan is absent in this legacy config → defaults ON.
        assert!(cfg.auto_plan);
    }

    #[test]
    fn garbage_deserializes_to_default() {
        let cfg: KivioCodeConfig = serde_json::from_str("not json").unwrap_or_default();
        assert!(cfg.read_claude_dir);
    }

    #[test]
    fn roundtrip_serialize_deserialize() {
        let cfg = KivioCodeConfig {
            read_claude_dir: false,
            default_provider_id: Some("p1".to_string()),
            default_model: Some("gemma4:31b".to_string()),
            approval_policy: Some("always_confirm".to_string()),
            auto_plan: false,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: KivioCodeConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cfg);
        assert!(!back.read_claude_dir);
        assert_eq!(back.default_model.as_deref(), Some("gemma4:31b"));
    }

    fn patch(json: &str) -> KivioCodeConfigPatch {
        serde_json::from_str(json).expect("patch parses")
    }

    #[test]
    fn merge_project_overrides_model_and_provider() {
        let global = KivioCodeConfig {
            default_provider_id: Some("ds".to_string()),
            default_model: Some("deepseek-v4-flash".to_string()),
            ..KivioCodeConfig::default()
        };
        let merged = merge_config(global, patch(r#"{"defaultModel": "gpt-5.5"}"#));
        // model overridden by project; provider inherited from global.
        assert_eq!(merged.default_model.as_deref(), Some("gpt-5.5"));
        assert_eq!(merged.default_provider_id.as_deref(), Some("ds"));
    }

    #[test]
    fn merge_absent_field_inherits_global() {
        // read_claude_dir absent in the patch must NOT reset the global value to the
        // serde default — it inherits global (false here).
        let global = KivioCodeConfig {
            read_claude_dir: false,
            ..KivioCodeConfig::default()
        };
        let merged = merge_config(global, patch("{}"));
        assert!(!merged.read_claude_dir);
    }

    #[test]
    fn merge_read_claude_dir_overridable() {
        let global = KivioCodeConfig::default(); // true
        let merged = merge_config(global, patch(r#"{"readClaudeDir": false}"#));
        assert!(!merged.read_claude_dir);
    }

    #[test]
    fn auto_plan_defaults_on_and_is_overridable() {
        // Absent in config → ON.
        assert!(KivioCodeConfig::default().auto_plan);
        // Project patch may turn it off per key; absent inherits global.
        let global = KivioCodeConfig::default();
        assert!(!merge_config(global, patch(r#"{"autoPlan": false}"#)).auto_plan);
        let global = KivioCodeConfig {
            auto_plan: false,
            ..KivioCodeConfig::default()
        };
        assert!(!merge_config(global, patch("{}")).auto_plan);
    }

    #[test]
    fn merge_approval_policy_tightens_only() {
        // Project may tighten: global auto + project always_confirm → always_confirm.
        let global = KivioCodeConfig {
            approval_policy: Some("auto".to_string()),
            ..KivioCodeConfig::default()
        };
        let merged = merge_config(global, patch(r#"{"approvalPolicy": "always_confirm"}"#));
        assert_eq!(merged.approval_policy.as_deref(), Some("always_confirm"));

        // Project may NOT loosen: global always_confirm + project auto → stays strict.
        let global = KivioCodeConfig {
            approval_policy: Some("always_confirm".to_string()),
            ..KivioCodeConfig::default()
        };
        let merged = merge_config(global, patch(r#"{"approvalPolicy": "auto"}"#));
        assert_eq!(merged.approval_policy.as_deref(), Some("always_confirm"));

        // Unset global (= auto) + project sensitive → sensitive.
        let merged = merge_config(
            KivioCodeConfig::default(),
            patch(r#"{"approvalPolicy": "readonly_auto_sensitive_confirm"}"#),
        );
        assert_eq!(
            merged.approval_policy.as_deref(),
            Some("readonly_auto_sensitive_confirm")
        );
    }

    #[test]
    fn load_merged_picks_up_project_config_from_disk() {
        // Real filesystem path: a project dir's .kivio/config.json overrides the global
        // config per key, proving find_project_patch (walk) + parse + merge end-to-end.
        let dir = std::env::temp_dir().join(format!("kivio-cfg-{}", uuid::Uuid::new_v4()));
        let kivio = dir.join(".kivio");
        std::fs::create_dir_all(&kivio).expect("mkdir .kivio");
        std::fs::write(
            kivio.join("config.json"),
            r#"{"defaultModel": "project-only-model", "readClaudeDir": false}"#,
        )
        .expect("write project config");

        // Project values win regardless of whatever the machine's global config holds.
        let merged = load_merged(&dir);
        assert_eq!(merged.default_model.as_deref(), Some("project-only-model"));
        assert!(!merged.read_claude_dir);

        // A nested working directory still finds the ancestor .kivio/config.json.
        let sub = dir.join("a").join("b");
        std::fs::create_dir_all(&sub).expect("mkdir sub");
        assert_eq!(
            load_merged(&sub).default_model.as_deref(),
            Some("project-only-model")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
