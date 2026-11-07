use super::super::types::{
    AgentErrorPolicy, AgentInstallSpec, LatestVersionStrategy, PostInstallStrategy,
    RuntimeAgentDef, RuntimeBuildOptions, RuntimeContext, UpdateStrategy,
};
use super::acp::acp_def;

fn build_gemini_args(
    _c: &RuntimeContext,
    _o: &RuntimeBuildOptions,
    _p: Option<&str>,
) -> Vec<String> {
    vec!["--acp".to_string()]
}

const GEMINI_MODELS: &[(&str, &str)] = &[
    ("default", "Default"),
    ("gemini-3-pro-preview", "gemini-3-pro-preview"),
    ("gemini-3-flash-preview", "gemini-3-flash-preview"),
    ("gemini-2.5-pro", "gemini-2.5-pro"),
    ("gemini-2.5-flash", "gemini-2.5-flash"),
    ("gemini-2.5-flash-lite", "gemini-2.5-flash-lite"),
];

const GEMINI_ENV: &[(&str, &str)] = &[("GEMINI_CLI_TRUST_WORKSPACE", "true")];

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

pub const GEMINI_AGENT_DEF: RuntimeAgentDef = RuntimeAgentDef {
    env: GEMINI_ENV,
    error_policy: AgentErrorPolicy::login("gemini"),
    ..acp_def(
        "gemini",
        "Gemini CLI",
        "gemini",
        GEMINI_MODELS,
        &["--acp"],
        GEMINI_INSTALL,
        build_gemini_args,
    )
};
