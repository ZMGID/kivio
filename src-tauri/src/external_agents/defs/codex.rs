//! Codex CLI external agent: `codex app-server` JSON-RPC (stdio).
//!
//! Handshake / turn / steer base schema was verified against 0.148.0 and launch/API deltas
//! re-checked through 0.155.0 (`thread/start` still
//! takes the kebab `sandbox` string; `turn/start` uses `sandboxPolicy` only as a last-resort
//! override). 0.149 rejects the obsolete `permissionProfile` field — Kivio never sent it.
//! 0.152 adds `clock` items and `openai/elicitation` form requests; Kivio maps supported form
//! schemas to its structured question card and safely declines unsupported URL/oversized forms.
//!
//! Approval: workspace-write / read-only send `approvalPolicy: "on-request"` and route
//! command/file/permissions RPCs through the existing tool-approval card. The 「完全」档
//! (`danger-full-access`) keeps `never` and auto-allows.
//!
//! Workspace / execution-environment grants need `capabilities.experimentalApi` plus
//! `runtimeWorkspaceRoots` on `thread/start` (and extra attachment dirs on `turn/start`).
//! Echoing `item/permissions/requestApproval` without those roots leaves `:workspace_roots`
//! empty — native Codex TUI sets them, which is why "同意一路下去" works there and not here.

use super::super::types::{
    PromptInputFormat, RuntimeAgentDef, RuntimeBuildOptions, RuntimeContext, StreamFormat,
};

/// 探测彻底失败时的静态兜底 — 与 desktop-cc-gui `generatedModelCatalog.json` 四档一致。
const FALLBACK_MODELS: &[(&str, &str)] = &[
    ("default", "Default"),
    ("gpt-5.6-sol", "gpt-5.6-sol"),
    ("gpt-5.6-terra", "gpt-5.6-terra"),
    ("gpt-5.6-luna", "gpt-5.6-luna"),
    ("gpt-5.5", "gpt-5.5"),
];

const REASONING: &[(&str, &str)] = &[
    ("default", "Default"),
    ("low", "Low"),
    ("medium", "Medium"),
    ("high", "High"),
    ("xhigh", "XHigh"),
    ("max", "Max"),
    ("ultra", "Ultra"),
];

pub fn build_codex_args(
    _ctx: &RuntimeContext,
    _options: &RuntimeBuildOptions,
    _prompt: Option<&str>,
) -> Vec<String> {
    // The app-server protocol negotiates cwd / model / sandbox / approval over JSON-RPC
    // (`Thread/start` + `Turn/start`), so no model / sandbox CLI flags are needed here.
    vec!["app-server".to_string()]
}

pub const CODEX_AGENT_DEF: RuntimeAgentDef = RuntimeAgentDef {
    id: "codex",
    name: "Codex CLI",
    bin: "codex",
    fallback_bins: &[],
    version_args: &["--version"],
    auth_probe_args: Some(&["login", "status"]),
    fallback_models: FALLBACK_MODELS,
    reasoning_options: REASONING,
    sandbox_options: &[
        ("read-only", "只读"),
        ("workspace-write", "工作区写 (默认)"),
        ("danger-full-access", "完全"),
    ],
    list_models_args: Some(&["debug", "models"]),
    // `codex debug models` cold-start can exceed 5s（首次要拉配置/鉴权）；给 15s 免误判失败（F4）。
    list_models_timeout_secs: Some(20),
    models_from_stderr: false,
    model_probe: Some(super::super::types::ModelProbeStrategy::CodexAppServer),
    model_probe_args: None,
    current_config: super::super::types::CurrentConfigStrategy::Codex,
    provider_profile: super::super::types::ProviderProfileStrategy::Codex,
    native_providers: super::super::types::NativeProviderStrategy::None,
    context_window: super::super::types::ContextWindowStrategy::Generic,
    usage_fallback: super::super::types::UsageFallbackStrategy::None,
    error_policy: super::super::types::AgentErrorPolicy::login_with_detail(
        "codex login",
        super::super::types::AgentErrorDetailStrategy::CodexAppServer,
    ),
    launch: super::super::types::AgentLaunchPolicy::WSL_SHARED_CODEX_HOME,
    instructions_via_launch_flag: false,
    compact_prompt: Some("/compact"),
    install: super::super::types::AgentInstallSpec {
        npm_package: Some("@openai/codex"),
        npm_install_args: &[],
        pypi_package: None,
        script_unix: None,
        script_windows: None,
        update: super::super::types::UpdateStrategy::Command(&["update"]),
        latest_version: super::super::types::LatestVersionStrategy::Registry,
        docs: "https://developers.openai.com/codex/cli/",
        config_dir: Some(".codex"),
        config_dir_env: None,
        requires_pnpm: false,
        post_install: super::super::types::PostInstallStrategy::None,
    },
    import: super::super::types::AgentImportPolicy {
        discovery: super::super::types::ImportDiscoveryStrategy::FileHistory,
        history_source: super::super::types::HistorySourceStrategy::CodexRollout,
        history_title: super::super::types::HistoryTitleStrategy::None,
    },
    run: super::super::types::AgentRunPolicy::STANDARD,
    slash_strategy: super::super::types::SlashStrategy::CodexAppServer,
    env: &[],
    max_prompt_arg_bytes: None,
    prompt_via_stdin: false,
    prompt_input_format: PromptInputFormat::Text,
    stream_format: StreamFormat::CodexAppServer,
    resumes_session_via_cli: false,
    supports_native_image: true,
    supports_steering: true,
    supports_follow_up: false,
    image_mime_whitelist: &[],
    build_args: build_codex_args,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_build_args_uses_app_server() {
        let args = build_codex_args(
            &RuntimeContext {
                extra_allowed_dirs: vec![],
                resume_session_id: None,
                new_session_id: None,
                include_partial_messages: false,
            },
            &RuntimeBuildOptions {
                model: Some("gpt-5".to_string()),
                reasoning: Some("high".to_string()),
                sandbox: None,
            },
            None,
        );
        assert_eq!(args, vec!["app-server".to_string()]);
    }
}
