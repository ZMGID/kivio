use super::super::types::{
    AgentErrorPolicy, AgentImportPolicy, AgentInstallSpec, AgentRunPolicy, CapabilitySupport,
    ContextWindowStrategy, CurrentConfigStrategy, ModelProbeStrategy, NativeProviderStrategy,
    PromptInputFormat, ProviderProfileStrategy, RewindStrategy, RuntimeAgentDef,
    RuntimeBuildOptions, RuntimeContext, SlashStrategy, StreamFormat, UsageFallbackStrategy,
};

pub(super) const fn acp_def(
    id: &'static str,
    name: &'static str,
    bin: &'static str,
    fallback_models: &'static [(&'static str, &'static str)],
    launch_args: &'static [&'static str],
    install: AgentInstallSpec,
    build_args: fn(&RuntimeContext, &RuntimeBuildOptions, Option<&str>) -> Vec<String>,
) -> RuntimeAgentDef {
    RuntimeAgentDef {
        id,
        name,
        bin,
        fallback_bins: &[],
        version_args: &["--version"],
        auth_probe_args: None,
        fallback_models,
        reasoning_options: &[],
        sandbox_options: &[],
        list_models_args: None,
        list_models_timeout_secs: Some(15),
        models_from_stderr: false,
        model_probe: Some(ModelProbeStrategy::Acp),
        model_probe_args: Some(launch_args),
        current_config: CurrentConfigStrategy::None,
        provider_profile: ProviderProfileStrategy::Environment,
        native_providers: NativeProviderStrategy::None,
        context_window: ContextWindowStrategy::Generic,
        usage_fallback: UsageFallbackStrategy::None,
        error_policy: AgentErrorPolicy::GENERIC,
        launch: super::super::types::AgentLaunchPolicy::DEFAULT,
        instructions_via_launch_flag: false,
        compact_prompt: None,
        install,
        import: AgentImportPolicy::NONE,
        run: AgentRunPolicy::STANDARD,
        slash_strategy: SlashStrategy::Acp,
        env: &[],
        max_prompt_arg_bytes: None,
        prompt_via_stdin: false,
        prompt_input_format: PromptInputFormat::Text,
        stream_format: StreamFormat::AcpJsonRpc,
        resumes_session_via_cli: false,
        steering: CapabilitySupport::Unavailable,
        follow_up: CapabilitySupport::Fallback,
        image_input: CapabilitySupport::Native,
        rewind: RewindStrategy::Replay,
        account: None,
        image_mime_whitelist: &[],
        build_args,
    }
}

// ACP launch: the model is set via `session/set_model` inside run_acp_session, not flags.
pub(super) fn build_acp_args(
    _c: &RuntimeContext,
    _o: &RuntimeBuildOptions,
    _p: Option<&str>,
) -> Vec<String> {
    vec!["acp".to_string()]
}

#[cfg(test)]
mod tests {
    use super::super::{
        gemini::GEMINI_AGENT_DEF, hermes::HERMES_AGENT_DEF, kimi::KIMI_AGENT_DEF,
        opencode::OPENCODE_AGENT_DEF,
    };
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
