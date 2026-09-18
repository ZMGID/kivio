use crate::external_agents::defs::{acp, antigravity, claude, codex, dsh, grok, pi};
use crate::external_agents::types::RuntimeAgentDef;

pub const AGENT_DEFS: &[RuntimeAgentDef] = &[
    claude::CLAUDE_AGENT_DEF,
    codex::CODEX_AGENT_DEF,
    acp::CURSOR_AGENT_DEF,
    acp::OPENCODE_AGENT_DEF,
    acp::GEMINI_AGENT_DEF,
    acp::KIMI_AGENT_DEF,
    pi::PI_AGENT_DEF,
    acp::HERMES_AGENT_DEF,
    grok::GROK_AGENT_DEF,
    dsh::DSH_AGENT_DEF,
    antigravity::ANTIGRAVITY_AGENT_DEF,
];

pub fn get_agent_def(id: &str) -> Option<&'static RuntimeAgentDef> {
    AGENT_DEFS.iter().find(|def| def.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external_agents::types::{
        AdditionalDirsStrategy, ApprovalStrategy, CurrentConfigStrategy, HistorySourceStrategy,
        HistoryTitleStrategy, ModelSelectionStrategy, ProviderProfileStrategy, RegenerateStrategy,
        RetryStrategy, UpdateStrategy,
    };

    #[test]
    fn registry_has_eleven_agents() {
        assert_eq!(AGENT_DEFS.len(), 11);
        assert!(get_agent_def("antigravity").is_some());
        assert!(get_agent_def("claude").is_some());
        assert!(get_agent_def("opencode").is_some());
        assert!(get_agent_def("pi").is_some());
        assert!(get_agent_def("hermes").is_some());
        assert!(get_agent_def("grok").is_some());
        assert!(get_agent_def("dsh").is_some());
        assert!(get_agent_def("unknown").is_none());
    }

    #[test]
    fn agent_def_owns_install_detection_and_provider_profile_routing() {
        let cases = [
            (
                "antigravity",
                CurrentConfigStrategy::None,
                ProviderProfileStrategy::Environment,
            ),
            (
                "claude",
                CurrentConfigStrategy::Claude,
                ProviderProfileStrategy::Claude,
            ),
            (
                "codex",
                CurrentConfigStrategy::Codex,
                ProviderProfileStrategy::Codex,
            ),
            (
                "cursor-agent",
                CurrentConfigStrategy::None,
                ProviderProfileStrategy::Environment,
            ),
            (
                "opencode",
                CurrentConfigStrategy::None,
                ProviderProfileStrategy::OpenCode,
            ),
            (
                "gemini",
                CurrentConfigStrategy::None,
                ProviderProfileStrategy::Environment,
            ),
            (
                "kimi",
                CurrentConfigStrategy::Kimi,
                ProviderProfileStrategy::Kimi,
            ),
            ("pi", CurrentConfigStrategy::Pi, ProviderProfileStrategy::Pi),
            (
                "grok",
                CurrentConfigStrategy::None,
                ProviderProfileStrategy::Grok,
            ),
            (
                "dsh",
                CurrentConfigStrategy::None,
                ProviderProfileStrategy::Dsh,
            ),
            (
                "hermes",
                CurrentConfigStrategy::None,
                ProviderProfileStrategy::Environment,
            ),
        ];

        for (id, current_config, provider_profile) in cases {
            let def = get_agent_def(id).expect("existing agent must have a definition");
            assert_eq!(
                def.current_config, current_config,
                "detection route for {id}"
            );
            assert_eq!(
                def.provider_profile, provider_profile,
                "profile route for {id}"
            );
            assert!(!def.install.docs.is_empty(), "install metadata for {id}");
        }

        assert!(matches!(
            get_agent_def("kimi").unwrap().install.update,
            UpdateStrategy::KimiManaged { .. }
        ));
        assert!(matches!(
            get_agent_def("gemini").unwrap().install.update,
            UpdateStrategy::ManagedPackage { .. }
        ));
        assert!(matches!(
            get_agent_def("dsh").unwrap().install.update,
            UpdateStrategy::ManagedPackage { .. }
        ));
        assert!(get_agent_def("unknown").is_none());
    }

    #[test]
    fn agent_def_owns_import_and_run_routing() {
        for def in AGENT_DEFS {
            let expected_history = match def.id {
                "claude" => (
                    HistorySourceStrategy::ClaudeJsonl,
                    HistoryTitleStrategy::Claude,
                ),
                "codex" => (
                    HistorySourceStrategy::CodexRollout,
                    HistoryTitleStrategy::None,
                ),
                "grok" => (
                    HistorySourceStrategy::GrokDirectory,
                    HistoryTitleStrategy::Grok,
                ),
                _ => (HistorySourceStrategy::None, HistoryTitleStrategy::None),
            };
            assert_eq!(
                (def.import.history_source, def.import.history_title),
                expected_history,
                "import route for {}",
                def.id
            );

            let expected_run = match def.id {
                "antigravity" => (
                    AdditionalDirsStrategy::Effective,
                    ModelSelectionStrategy::Direct,
                    RegenerateStrategy::None,
                    ApprovalStrategy::Default,
                    RetryStrategy::NeverReplay,
                ),
                "claude" => (
                    AdditionalDirsStrategy::ConversationOnly,
                    ModelSelectionStrategy::ClaudeWire,
                    RegenerateStrategy::None,
                    ApprovalStrategy::Default,
                    RetryStrategy::RetryFresh,
                ),
                "pi" => (
                    AdditionalDirsStrategy::ConversationOnly,
                    ModelSelectionStrategy::Direct,
                    RegenerateStrategy::PiRpc,
                    ApprovalStrategy::Default,
                    RetryStrategy::RetryFresh,
                ),
                "grok" => (
                    AdditionalDirsStrategy::ConversationOnly,
                    ModelSelectionStrategy::Direct,
                    RegenerateStrategy::None,
                    ApprovalStrategy::GrokAlwaysApprove,
                    RetryStrategy::RetryFresh,
                ),
                _ => (
                    AdditionalDirsStrategy::ConversationOnly,
                    ModelSelectionStrategy::Direct,
                    RegenerateStrategy::None,
                    ApprovalStrategy::Default,
                    RetryStrategy::RetryFresh,
                ),
            };
            assert_eq!(
                (
                    def.run.additional_dirs,
                    def.run.model_selection,
                    def.run.regenerate,
                    def.run.approval,
                    def.run.retry,
                ),
                expected_run,
                "run route for {}",
                def.id
            );
        }
        assert!(get_agent_def("unknown").is_none());
    }

    /// 「运行中注入」是逐协议能力。Codex 使用 `turn/steer`，Pi 使用 RPC `steer`，
    /// dsh 使用 bridge 的 `session/steer` → `agent.steer()`；都必须等对端成功响应后
    /// 才确认前端队列并发出 `UserSteer`。
    #[test]
    fn codex_pi_and_dsh_claim_mid_turn_steering() {
        let steerable: Vec<&str> = AGENT_DEFS
            .iter()
            .filter(|def| def.supports_steering)
            .map(|def| def.id)
            .collect();
        assert_eq!(steerable, vec!["codex", "pi", "dsh"]);
    }

    /// 原生 follow-up（当前轮结束后自动开下一轮）目前只有 Pi RPC 与 dsh `session/prompt`。
    #[test]
    fn pi_and_dsh_claim_native_follow_up() {
        let follow_up: Vec<&str> = AGENT_DEFS
            .iter()
            .filter(|def| def.supports_follow_up)
            .map(|def| def.id)
            .collect();
        assert_eq!(follow_up, vec!["pi", "dsh"]);
    }

    #[test]
    fn sandbox_options_are_agent_capabilities() {
        let cases: &[(&str, &[(&str, &str)])] = &[
            (
                "claude",
                &[
                    ("plan", "计划 (只读)"),
                    ("default", "每次确认"),
                    ("acceptEdits", "接受编辑"),
                    ("auto", "自动"),
                    ("dontAsk", "不打扰 (只放行安全操作)"),
                    ("bypassPermissions", "完全 (默认)"),
                ],
            ),
            (
                "codex",
                &[
                    ("read-only", "只读"),
                    ("workspace-write", "工作区写 (默认)"),
                    ("danger-full-access", "完全"),
                ],
            ),
            (
                "dsh",
                &[
                    ("read-only", "只读"),
                    ("workspace-write", "工作区写 (默认)"),
                    ("danger-full-access", "完全"),
                ],
            ),
            (
                "grok",
                &[
                    ("ask", "工具请求时确认"),
                    ("strict", "严格沙箱"),
                    ("full", "完全放行 (默认)"),
                ],
            ),
            (
                "antigravity",
                &[
                    ("default", "遵循 CLI 配置"),
                    ("plan", "计划"),
                    ("accept-edits", "接受编辑"),
                    ("sandbox", "沙箱"),
                    ("always-proceed", "完全放行"),
                ],
            ),
        ];

        for (id, expected) in cases {
            let def = get_agent_def(id).expect("agent must be registered");
            assert_eq!(def.sandbox_options, *expected, "sandbox options for {id}");
        }

        for id in ["cursor-agent", "opencode", "gemini", "kimi", "pi", "hermes"] {
            let def = get_agent_def(id).expect("agent must be registered");
            assert!(
                def.sandbox_options.is_empty(),
                "{id} must not advertise a sandbox capability"
            );
        }
    }
}
