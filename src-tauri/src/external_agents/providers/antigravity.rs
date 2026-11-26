use super::super::types::{AccountProbe, AccountProtocol, CapabilitySupport, RewindStrategy};
use super::super::types::{
    PromptInputFormat, RuntimeAgentDef, RuntimeBuildOptions, RuntimeContext, SlashStrategy,
    StreamFormat,
};

pub(crate) const MISSING_RESUME_TARGET: &str = "Antigravity native conversation not found";

pub fn build_antigravity_args(
    ctx: &RuntimeContext,
    options: &RuntimeBuildOptions,
    _prompt: Option<&str>,
) -> Vec<String> {
    let mut args = vec!["acp".into()];
    append_launch_settings(&mut args, ctx, options);
    args
}

fn append_launch_settings(
    args: &mut Vec<String>,
    ctx: &RuntimeContext,
    options: &RuntimeBuildOptions,
) {
    match options.sandbox.as_deref() {
        Some("sandbox") => args.push("--sandbox".into()),
        Some("always-proceed") => args.push("--dangerously-skip-permissions".into()),
        _ => {}
    }
    for dir in &ctx.extra_allowed_dirs {
        args.extend(["--add-dir".into(), dir.clone()]);
    }
}

pub(crate) fn effective_reasoning<'a>(
    model: Option<&str>,
    reasoning: Option<&'a str>,
) -> Option<&'a str> {
    let model_has_effort = model.is_some_and(|model| {
        model.trim().rsplit_once('-').is_some_and(|(_, suffix)| {
            matches!(
                suffix.to_ascii_lowercase().as_str(),
                "low" | "medium" | "high" | "xhigh" | "max"
            )
        })
    });
    reasoning
        .map(str::trim)
        .filter(|value| !model_has_effort && !value.is_empty() && *value != "default")
}

pub(crate) fn antigravity_report_settings(
    ctx: &RuntimeContext,
    options: &RuntimeBuildOptions,
) -> Vec<String> {
    let mut args = Vec::new();
    let model = options
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "default");
    if let Some(model) = model {
        args.extend(["--model".into(), model.into()]);
    }
    if let Some(reasoning) = effective_reasoning(model, options.reasoning.as_deref()) {
        args.extend(["--effort".into(), reasoning.into()]);
    }
    if let Some(mode @ ("plan" | "accept-edits")) = options.sandbox.as_deref() {
        args.extend(["--mode".into(), mode.into()]);
    }
    append_launch_settings(&mut args, ctx, options);
    args
}

pub const ANTIGRAVITY_AGENT_DEF: RuntimeAgentDef = RuntimeAgentDef {
    id: "antigravity",
    name: "Antigravity CLI",
    bin: "agy",
    fallback_bins: &[],
    version_args: &["--version"],
    auth_probe_args: None,
    fallback_models: &[],
    reasoning_options: &[
        ("default", "Default"),
        ("low", "Low"),
        ("medium", "Medium"),
        ("high", "High"),
        ("xhigh", "Extra high"),
        ("max", "Max"),
    ],
    sandbox_options: &[
        ("default", "遵循 CLI 配置"),
        ("plan", "计划"),
        ("accept-edits", "接受编辑"),
        ("sandbox", "沙箱"),
        ("always-proceed", "完全放行"),
    ],
    list_models_args: Some(&["models"]),
    // Fetches the remote catalog; the desktop's first probe can exceed 30s.
    list_models_timeout_secs: Some(60),
    models_from_stderr: false,
    model_probe: None,
    model_probe_args: None,
    current_config: super::super::types::CurrentConfigStrategy::None,
    provider_profile: super::super::types::ProviderProfileStrategy::Environment,
    native_providers: super::super::types::NativeProviderStrategy::None,
    context_window: super::super::types::ContextWindowStrategy::Generic,
    usage_fallback: super::super::types::UsageFallbackStrategy::None,
    error_policy: super::super::types::AgentErrorPolicy::login("agy"),
    launch: super::super::types::AgentLaunchPolicy::DEFAULT,
    instructions_via_launch_flag: false,
    compact_prompt: None,
    install: super::super::types::AgentInstallSpec {
        npm_package: None,
        npm_install_args: &[],
        pypi_package: None,
        script_unix: Some("curl -fsSL https://antigravity.google/cli/install.sh | bash"),
        script_windows: Some("irm https://antigravity.google/cli/install.ps1 | iex"),
        update: super::super::types::UpdateStrategy::Command(&["update"]),
        latest_version: super::super::types::LatestVersionStrategy::AntigravityRelease,
        docs: "https://antigravity.google/docs/cli/install/",
        config_dir: Some(".gemini/antigravity-cli"),
        config_dir_env: None,
        requires_pnpm: false,
        post_install: super::super::types::PostInstallStrategy::None,
    },
    import: super::super::types::AgentImportPolicy {
        discovery: super::super::types::ImportDiscoveryStrategy::Antigravity,
        ..super::super::types::AgentImportPolicy::NONE
    },
    run: super::super::types::AgentRunPolicy {
        additional_dirs: super::super::types::AdditionalDirsStrategy::Effective,
        retry: super::super::types::RetryStrategy::NeverReplay,
        ..super::super::types::AgentRunPolicy::STANDARD
    },
    slash_strategy: SlashStrategy::Antigravity,
    env: &[],
    max_prompt_arg_bytes: None,
    prompt_via_stdin: true,
    prompt_input_format: PromptInputFormat::Text,
    stream_format: StreamFormat::AntigravityAcp,
    resumes_session_via_cli: true,
    image_input: CapabilitySupport::Fallback,
    steering: CapabilitySupport::Unavailable,
    follow_up: CapabilitySupport::Fallback,
    rewind: RewindStrategy::Replay,
    account: Some(AccountProbe {
        args: &[
            "-p",
            "/usage",
            "--output-format",
            "json",
            "--print-timeout",
            "20s",
        ],
        protocol: AccountProtocol::AntigravityUsage,
    }),
    image_mime_whitelist: &[],
    build_args: build_antigravity_args,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_flags_preserve_paths_and_never_invent_session_ids_or_permissions() {
        let ctx = RuntimeContext {
            extra_allowed_dirs: vec![r"C:\my project".into()],
            resume_session_id: Some("old".into()),
            new_session_id: Some("new".into()),
            include_partial_messages: true,
        };
        let mut options = RuntimeBuildOptions {
            model: Some("my-model".into()),
            reasoning: Some("high".into()),
            sandbox: None,
        };
        let args = build_antigravity_args(&ctx, &options, Some("must go to stdin"));
        assert_eq!(args, ["acp", "--add-dir", r"C:\my project"]);
        let report = antigravity_report_settings(&ctx, &options);
        assert!(report.windows(2).any(|w| w == ["--model", "my-model"]));
        assert!(report.windows(2).any(|w| w == ["--effort", "high"]));
        options.sandbox = Some("always-proceed".into());
        assert!(build_antigravity_args(&ctx, &options, None)
            .contains(&"--dangerously-skip-permissions".into()));
    }

    #[test]
    fn catalog_effort_variants_ignore_stale_reasoning() {
        let ctx = RuntimeContext {
            extra_allowed_dirs: vec![],
            resume_session_id: None,
            new_session_id: None,
            include_partial_messages: false,
        };
        for model in [
            "gemini-3.8-flash-low",
            "gemini-3.8-flash-high",
            "gpt-oss-120b-medium",
            "custom-xhigh",
            "custom-max",
        ] {
            let options = RuntimeBuildOptions {
                model: Some(model.into()),
                reasoning: Some("high".into()),
                sandbox: None,
            };
            assert_eq!(
                effective_reasoning(Some(model), options.reasoning.as_deref()),
                None
            );
            let args = antigravity_report_settings(&ctx, &options);
            assert!(args.windows(2).any(|w| w == ["--model", model]));
            assert!(!args.iter().any(|v| v == "--effort"));
        }
    }
}
