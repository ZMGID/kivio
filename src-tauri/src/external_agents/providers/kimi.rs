use super::super::types::{
    AgentErrorPolicy, AgentImportPolicy, AgentInstallSpec, ContextWindowStrategy,
    CurrentConfigStrategy, LatestVersionStrategy, PostInstallStrategy, ProviderProfileStrategy,
    RuntimeAgentDef, UpdateStrategy, UsageFallbackStrategy,
};
use super::acp::{acp_def, build_acp_args};

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

// The legacy kimi-cli binary implements a different protocol; do not use it as an alias.
pub const KIMI_AGENT_DEF: RuntimeAgentDef = RuntimeAgentDef {
    current_config: CurrentConfigStrategy::Kimi,
    provider_profile: ProviderProfileStrategy::Kimi,
    context_window: ContextWindowStrategy::Kimi,
    usage_fallback: UsageFallbackStrategy::KimiWireLog,
    error_policy: AgentErrorPolicy::login("kimi"),
    import: AgentImportPolicy::acp(true),
    ..acp_def(
        "kimi",
        "Kimi CLI",
        "kimi",
        KIMI_MODELS,
        &["acp"],
        KIMI_INSTALL,
        build_acp_args,
    )
};
