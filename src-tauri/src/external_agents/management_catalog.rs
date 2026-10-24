//! Shared settings metadata; it does not claim support for a runtime protocol.
use super::{defs, types::{AgentInstallSpec, RuntimeAgentDef}};
#[derive(Clone, Copy)]
pub struct Definition {
    pub id: &'static str,
    pub name: &'static str,
    pub bin: &'static str,
    pub fallback_bins: &'static [&'static str],
    pub version_args: &'static [&'static str],
    pub env: &'static [(&'static str, &'static str)],
    pub install: AgentInstallSpec,
}
impl From<&RuntimeAgentDef> for Definition {
    fn from(def: &RuntimeAgentDef) -> Self {
        Self { id:def.id, name:def.name, bin:def.bin, fallback_bins:def.fallback_bins,
            version_args:def.version_args, env:def.env, install:def.install }
    }
}
pub fn get(id: &str) -> Option<Definition> {
    Some(match id {
        "claude" => CLAUDE,
        "dsh" => DSH,
        "codex" => (&defs::codex::CODEX_AGENT_DEF).into(),
        "grok" => (&defs::grok::GROK_AGENT_DEF).into(),
        "pi" => (&defs::pi::PI_AGENT_DEF).into(),
        "antigravity" => (&defs::antigravity::ANTIGRAVITY_AGENT_DEF).into(),
        "opencode" => (&defs::acp::OPENCODE_AGENT_DEF).into(),
        "gemini" => (&defs::acp::GEMINI_AGENT_DEF).into(),
        "kimi" => (&defs::acp::KIMI_AGENT_DEF).into(),
        "hermes" => (&defs::acp::HERMES_AGENT_DEF).into(),
        _ => return None,
    })
}
pub const CLAUDE: Definition = Definition {
    id: "claude",
    name: "Claude Code",
    bin: "claude",
    fallback_bins: &["openclaude"],
    version_args: &["--version"],
    env: &[("CLAUDE_CODE_ENABLE_TODO_TOOLS", "1")],
    install: super::types::AgentInstallSpec {
        npm_package: Some("@anthropic-ai/claude-code"),
        npm_install_args: &[],
        pypi_package: None,
        script_unix: Some("curl -fsSL https://claude.ai/install.sh | bash"),
        script_windows: Some("irm https://claude.ai/install.ps1 | iex"),
        update: super::types::UpdateStrategy::Command(&["update"]),
        latest_version: super::types::LatestVersionStrategy::Registry,
        docs: "https://code.claude.com/docs/en/setup",
        config_dir: Some(".claude"),
        config_dir_env: None,
        requires_pnpm: false,
        post_install: super::types::PostInstallStrategy::None,
    },
};

pub const DSH: Definition = Definition {
    id: "dsh",
    name: "DeepSeek Harness",
    bin: "dsh",
    fallback_bins: &[],
    version_args: &["--version"],
    env: &[("DSH_TELEMETRY_DISABLED", "1")],
    install: super::types::AgentInstallSpec {
        npm_package: Some("@deepseek-ai/dsh"),
        npm_install_args: &[],
        pypi_package: None,
        script_unix: None,
        script_windows: None,
        update: super::types::UpdateStrategy::ManagedPackage {
            package: "@deepseek-ai/dsh",
            brew_formula: "dsh",
        },
        latest_version: super::types::LatestVersionStrategy::Registry,
        docs: "https://github.com/deepseek-ai/deepseek-harness",
        config_dir: Some(".dsh"),
        config_dir_env: Some("DSH_HOME"),
        requires_pnpm: true,
        post_install: super::types::PostInstallStrategy::DshProfile,
    },
};
