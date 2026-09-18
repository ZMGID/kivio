//! Shared definition for the ACP-family external agents (cursor / gemini / opencode / hermes / kimi).
//!
//! All launch over the Agent Client Protocol (`StreamFormat::AcpJsonRpc`), probe models via
//! `ModelProbeStrategy::Acp`, discover slash commands via `SlashStrategy::Acp`, and build a
//! constant launch-arg vec. They differ only in id / name / binary / auth-probe / fallback models
//! / launch args / env — so a single [`acp_def`] const constructor + data rows replaces several
//! near-identical struct literals. `kimi` joins this family via `kimi acp` (its own ACP server),
//! inheriting persistent sessions, native resume, mid-turn model switch, and error-classified
//! reconnect instead of the old per-turn `-p` + `stream-json` spawn.

use super::super::types::{
    AgentImportPolicy, AgentInstallSpec, AgentRunPolicy, ContextWindowStrategy,
    CurrentConfigStrategy, ImportDiscoveryStrategy, LatestVersionStrategy, ModelProbeStrategy,
    NativeProviderStrategy, PostInstallStrategy, PromptInputFormat, ProviderProfileStrategy,
    RuntimeAgentDef, RuntimeBuildOptions, RuntimeContext, SlashStrategy, StreamFormat,
    UpdateStrategy, UsageFallbackStrategy,
};

const fn acp_def(
    id: &'static str,
    name: &'static str,
    bin: &'static str,
    fallback_bins: &'static [&'static str],
    auth_probe_args: Option<&'static [&'static str]>,
    fallback_models: &'static [(&'static str, &'static str)],
    launch_args: &'static [&'static str],
    env: &'static [(&'static str, &'static str)],
    model_probe: ModelProbeStrategy,
    current_config: CurrentConfigStrategy,
    provider_profile: ProviderProfileStrategy,
    context_window: ContextWindowStrategy,
    usage_fallback: UsageFallbackStrategy,
    error_policy: super::super::types::AgentErrorPolicy,
    compact_prompt: Option<&'static str>,
    import: AgentImportPolicy,
    install: AgentInstallSpec,
    build_args: fn(&RuntimeContext, &RuntimeBuildOptions, Option<&str>) -> Vec<String>,
) -> RuntimeAgentDef {
    RuntimeAgentDef {
        id,
        name,
        bin,
        fallback_bins,
        version_args: &["--version"],
        auth_probe_args,
        fallback_models,
        reasoning_options: &[],
        sandbox_options: &[],
        list_models_args: None,
        list_models_timeout_secs: Some(15),
        models_from_stderr: false,
        model_probe: Some(model_probe),
        model_probe_args: Some(launch_args),
        current_config,
        provider_profile,
        native_providers: NativeProviderStrategy::None,
        context_window,
        usage_fallback,
        error_policy,
        launch: super::super::types::AgentLaunchPolicy::DEFAULT,
        instructions_via_launch_flag: false,
        compact_prompt,
        install,
        import,
        run: AgentRunPolicy::STANDARD,
        slash_strategy: SlashStrategy::Acp,
        env,
        max_prompt_arg_bytes: None,
        prompt_via_stdin: false,
        prompt_input_format: PromptInputFormat::Text,
        stream_format: StreamFormat::AcpJsonRpc,
        resumes_session_via_cli: false,
        supports_native_image: true,
        supports_steering: false,
        supports_follow_up: false,
        image_mime_whitelist: &[],
        build_args,
    }
}

// ACP launch: the model is set via `session/set_model` inside run_acp_session, not flags.
fn build_acp_args(_c: &RuntimeContext, _o: &RuntimeBuildOptions, _p: Option<&str>) -> Vec<String> {
    vec!["acp".to_string()]
}

fn build_gemini_args(
    _c: &RuntimeContext,
    _o: &RuntimeBuildOptions,
    _p: Option<&str>,
) -> Vec<String> {
    vec!["--acp".to_string()]
}

fn build_hermes_args(
    _c: &RuntimeContext,
    _o: &RuntimeBuildOptions,
    _p: Option<&str>,
) -> Vec<String> {
    vec!["acp".to_string(), "--accept-hooks".to_string()]
}

const ACP_IMPORT_POLICY: AgentImportPolicy = AgentImportPolicy {
    discovery: ImportDiscoveryStrategy::Acp,
    history_source: super::super::types::HistorySourceStrategy::None,
    history_title: super::super::types::HistoryTitleStrategy::None,
};

const CURSOR_MODELS: &[(&str, &str)] = &[
    ("default", "Default"),
    ("auto", "auto"),
    ("sonnet-4", "sonnet-4"),
    ("gpt-5", "gpt-5"),
];

const GEMINI_MODELS: &[(&str, &str)] = &[
    ("default", "Default"),
    ("gemini-3-pro-preview", "gemini-3-pro-preview"),
    ("gemini-3-flash-preview", "gemini-3-flash-preview"),
    ("gemini-2.5-pro", "gemini-2.5-pro"),
    ("gemini-2.5-flash", "gemini-2.5-flash"),
    ("gemini-2.5-flash-lite", "gemini-2.5-flash-lite"),
];

const OPENCODE_MODELS: &[(&str, &str)] = &[
    ("default", "Default"),
    ("anthropic/claude-sonnet-4-5", "anthropic/claude-sonnet-4-5"),
    ("openai/gpt-5", "openai/gpt-5"),
    ("google/gemini-2.5-pro", "google/gemini-2.5-pro"),
];

const HERMES_MODELS: &[(&str, &str)] = &[
    ("default", "Default"),
    ("grok-4.3", "grok-4.3 (xAI · default)"),
    ("grok-4.20-reasoning", "grok-4.20-reasoning (xAI · deep)"),
    (
        "grok-4.20-0309-non-reasoning",
        "grok-4.20-non-reasoning (xAI · fast)",
    ),
    (
        "grok-4.20-multi-agent-0309",
        "grok-4.20-multi-agent (xAI · orchestration)",
    ),
    ("openai-codex:gpt-5.5", "gpt-5.5 (openai-codex:gpt-5.5)"),
    ("openai-codex:gpt-5.4", "gpt-5.4 (openai-codex:gpt-5.4)"),
    (
        "openai-codex:gpt-5.4-mini",
        "gpt-5.4-mini (openai-codex:gpt-5.4-mini)",
    ),
];

const GEMINI_ENV: &[(&str, &str)] = &[("GEMINI_CLI_TRUST_WORKSPACE", "true")];

const CURSOR_INSTALL: AgentInstallSpec = AgentInstallSpec {
    npm_package: None,
    npm_install_args: &[],
    pypi_package: None,
    script_unix: Some("curl https://cursor.com/install -fsS | bash"),
    script_windows: Some("irm 'https://cursor.com/install?win32=true' | iex"),
    update: UpdateStrategy::Command(&["update"]),
    latest_version: LatestVersionStrategy::Registry,
    docs: "https://cursor.com/docs/cli",
    config_dir: Some(".cursor"),
    config_dir_env: None,
    requires_pnpm: false,
    post_install: PostInstallStrategy::None,
};

const GEMINI_INSTALL: AgentInstallSpec = AgentInstallSpec {
    npm_package: Some("@google/gemini-cli"),
    npm_install_args: &[],
    pypi_package: None,
    script_unix: None,
    script_windows: None,
    update: UpdateStrategy::ManagedPackage {
        package: "@google/gemini-cli",
        brew_formula: "gemini-cli",
    },
    latest_version: LatestVersionStrategy::Registry,
    docs: "https://www.geminicli.com/docs/get-started/installation",
    config_dir: Some(".gemini"),
    config_dir_env: None,
    requires_pnpm: false,
    post_install: PostInstallStrategy::None,
};

const OPENCODE_INSTALL: AgentInstallSpec = AgentInstallSpec {
    npm_package: Some("opencode-ai"),
    npm_install_args: &[],
    pypi_package: None,
    script_unix: Some("curl -fsSL https://opencode.ai/install | bash"),
    script_windows: None,
    update: UpdateStrategy::Command(&["upgrade"]),
    latest_version: LatestVersionStrategy::Registry,
    docs: "https://opencode.ai/docs/",
    config_dir: Some(".config/opencode"),
    config_dir_env: None,
    requires_pnpm: false,
    post_install: PostInstallStrategy::None,
};

const HERMES_INSTALL: AgentInstallSpec = AgentInstallSpec {
    npm_package: None,
    npm_install_args: &[],
    pypi_package: None,
    script_unix: Some("curl -fsSL https://hermes-agent.nousresearch.com/install.sh | bash"),
    script_windows: Some("iex (irm https://hermes-agent.nousresearch.com/install.ps1)"),
    update: UpdateStrategy::Command(&["update"]),
    latest_version: LatestVersionStrategy::HermesRelease,
    docs: "https://hermes-agent.nousresearch.com/docs/getting-started/installation",
    config_dir: Some(".hermes"),
    config_dir_env: None,
    requires_pnpm: false,
    post_install: PostInstallStrategy::None,
};

const KIMI_INSTALL: AgentInstallSpec = AgentInstallSpec {
    npm_package: Some("@moonshot-ai/kimi-code"),
    npm_install_args: &[],
    pypi_package: None,
    script_unix: Some("curl -fsSL https://code.kimi.com/kimi-code/install.sh | bash"),
    script_windows: Some("irm https://code.kimi.com/kimi-code/install.ps1 | iex"),
    update: UpdateStrategy::KimiManaged {
        package: "@moonshot-ai/kimi-code",
        brew_formula: "kimi-code",
    },
    latest_version: LatestVersionStrategy::Registry,
    docs: "https://moonshotai.github.io/kimi-code/en/guides/getting-started.html",
    config_dir: Some(".kimi-code"),
    config_dir_env: None,
    requires_pnpm: false,
    post_install: PostInstallStrategy::None,
};

// Kimi Code models used only when the ACP session/new probe reports none (offline / not logged in).
// The real catalog now comes from the ACP `availableModels` / `configOptions` probe like the other
// ACP agents — this table is the fallback the picker labels as "默认列表".
const KIMI_MODELS: &[(&str, &str)] = &[
    ("default", "Default"),
    ("kimi-code/k3", "K3 (kimi-code/k3)"),
    (
        "kimi-code/kimi-for-coding",
        "K2.7 Coding (kimi-code/kimi-for-coding)",
    ),
];

pub const CURSOR_AGENT_DEF: RuntimeAgentDef = acp_def(
    "cursor-agent",
    "Cursor Agent",
    "cursor-agent",
    &[],
    Some(&["status"]),
    CURSOR_MODELS,
    &["acp"],
    &[],
    ModelProbeStrategy::Acp,
    CurrentConfigStrategy::None,
    ProviderProfileStrategy::Environment,
    ContextWindowStrategy::Generic,
    UsageFallbackStrategy::None,
    super::super::types::AgentErrorPolicy::login("cursor-agent login"),
    None,
    ACP_IMPORT_POLICY,
    CURSOR_INSTALL,
    build_acp_args,
);

pub const GEMINI_AGENT_DEF: RuntimeAgentDef = acp_def(
    "gemini",
    "Gemini CLI",
    "gemini",
    &[],
    None,
    GEMINI_MODELS,
    &["--acp"],
    GEMINI_ENV,
    ModelProbeStrategy::Acp,
    CurrentConfigStrategy::None,
    ProviderProfileStrategy::Environment,
    ContextWindowStrategy::Generic,
    UsageFallbackStrategy::None,
    super::super::types::AgentErrorPolicy::login("gemini"),
    None,
    AgentImportPolicy::NONE,
    GEMINI_INSTALL,
    build_gemini_args,
);

pub const OPENCODE_AGENT_DEF: RuntimeAgentDef = acp_def(
    "opencode",
    "OpenCode",
    "opencode-cli",
    &["opencode"],
    None,
    OPENCODE_MODELS,
    &["acp"],
    &[],
    ModelProbeStrategy::OpenCodeThenAcp,
    CurrentConfigStrategy::None,
    ProviderProfileStrategy::OpenCode,
    ContextWindowStrategy::Generic,
    UsageFallbackStrategy::None,
    super::super::types::AgentErrorPolicy::login("opencode auth login"),
    Some("/compact"),
    ACP_IMPORT_POLICY,
    OPENCODE_INSTALL,
    build_acp_args,
);

pub const HERMES_AGENT_DEF: RuntimeAgentDef = acp_def(
    "hermes",
    "Hermes",
    "hermes",
    &[],
    None,
    HERMES_MODELS,
    &["acp", "--accept-hooks"],
    &[],
    ModelProbeStrategy::Acp,
    CurrentConfigStrategy::None,
    ProviderProfileStrategy::Environment,
    ContextWindowStrategy::Generic,
    UsageFallbackStrategy::None,
    super::super::types::AgentErrorPolicy::login("hermes"),
    None,
    AgentImportPolicy::NONE,
    HERMES_INSTALL,
    build_hermes_args,
);

pub const KIMI_AGENT_DEF: RuntimeAgentDef = acp_def(
    "kimi",
    "Kimi CLI",
    "kimi",
    // 刻意不加 `kimi-cli` 别名：那是已停止维护的旧 Python 版（版本号 1.4x），协议与模型列表
    // 都和现在的 TypeScript 版 Kimi Code（0.x，命令名就是 `kimi`）不同。回退过去会静默跑错
    // 目标，不如老实报「未安装」。
    &[],
    None,
    KIMI_MODELS,
    &["acp"],
    &[],
    ModelProbeStrategy::Acp,
    CurrentConfigStrategy::Kimi,
    ProviderProfileStrategy::Kimi,
    ContextWindowStrategy::Kimi,
    UsageFallbackStrategy::KimiWireLog,
    super::super::types::AgentErrorPolicy::login("kimi"),
    None,
    ACP_IMPORT_POLICY,
    KIMI_INSTALL,
    build_acp_args,
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acp_defs_build_expected_launch_args() {
        let ctx = RuntimeContext {
            extra_allowed_dirs: vec![],
            resume_session_id: None,
            new_session_id: None,
            include_partial_messages: false,
        };
        let opts = RuntimeBuildOptions {
            model: None,
            reasoning: None,
            sandbox: None,
        };
        let cases: &[(&RuntimeAgentDef, &[&str])] = &[
            (&CURSOR_AGENT_DEF, &["acp"]),
            (&GEMINI_AGENT_DEF, &["--acp"]),
            (&OPENCODE_AGENT_DEF, &["acp"]),
            (&HERMES_AGENT_DEF, &["acp", "--accept-hooks"]),
            (&KIMI_AGENT_DEF, &["acp"]),
        ];
        for (def, expected) in cases {
            let args = (def.build_args)(&ctx, &opts, None);
            let expected: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
            assert_eq!(args, expected, "launch args for {}", def.id);
            assert!(matches!(
                def.model_probe,
                Some(ModelProbeStrategy::Acp | ModelProbeStrategy::OpenCodeThenAcp)
            ));
            assert!(matches!(def.stream_format, StreamFormat::AcpJsonRpc));
        }
    }
}
