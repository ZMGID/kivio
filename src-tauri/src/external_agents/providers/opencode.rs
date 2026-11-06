use super::super::types::{
    AgentErrorPolicy, AgentImportPolicy, AgentInstallSpec, LatestVersionStrategy,
    ModelProbeStrategy, PostInstallStrategy, ProviderProfileStrategy, RuntimeAgentDef,
    UpdateStrategy,
};
use super::acp::{acp_def, build_acp_args};

const OPENCODE_MODELS: &[(&str, &str)] = &[
    ("default", "Default"),
    ("anthropic/claude-sonnet-4-5", "anthropic/claude-sonnet-4-5"),
    ("openai/gpt-5", "openai/gpt-5"),
    ("google/gemini-2.5-pro", "google/gemini-2.5-pro"),
];

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

// OpenCode 自报 category:"mode" 的 build / plan；经权限胶囊选，连接时发 session/set_config_option。
pub const OPENCODE_AGENT_DEF: RuntimeAgentDef = RuntimeAgentDef {
    fallback_bins: &["opencode"],
    sandbox_options: &[("build", "构建 (默认)"), ("plan", "计划 (只读)")],
    model_probe: Some(ModelProbeStrategy::OpenCodeThenAcp),
    provider_profile: ProviderProfileStrategy::OpenCode,
    error_policy: AgentErrorPolicy::login("opencode auth login"),
    compact_prompt: Some("/compact"),
    import: AgentImportPolicy::acp(true),
    ..acp_def(
        "opencode",
        "OpenCode",
        "opencode-cli",
        OPENCODE_MODELS,
        &["acp"],
        OPENCODE_INSTALL,
        build_acp_args,
    )
};
