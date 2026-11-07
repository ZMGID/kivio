use super::super::types::{
    AgentErrorPolicy, AgentInstallSpec, LatestVersionStrategy, PostInstallStrategy,
    RuntimeAgentDef, RuntimeBuildOptions, RuntimeContext, UpdateStrategy,
};
use super::acp::acp_def;

fn build_hermes_args(
    _c: &RuntimeContext,
    _o: &RuntimeBuildOptions,
    _p: Option<&str>,
) -> Vec<String> {
    vec!["acp".to_string(), "--accept-hooks".to_string()]
}

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

pub const HERMES_AGENT_DEF: RuntimeAgentDef = RuntimeAgentDef {
    error_policy: AgentErrorPolicy::login("hermes"),
    ..acp_def(
        "hermes",
        "Hermes",
        "hermes",
        HERMES_MODELS,
        &["acp", "--accept-hooks"],
        HERMES_INSTALL,
        build_hermes_args,
    )
};
