use super::super::types::{
    ExternalMcpInjection, ModelProbeStrategy, PromptInputFormat, RuntimeAgentDef,
    RuntimeBuildOptions, RuntimeContext, SlashStrategy, StreamFormat,
};

const FALLBACK_MODELS: &[(&str, &str)] = &[
    ("default", "Default"),
    (
        "anthropic/claude-sonnet-4-5",
        "anthropic/claude-sonnet-4-5",
    ),
    ("openai/gpt-5", "openai/gpt-5"),
    ("google/gemini-2.5-pro", "google/gemini-2.5-pro"),
];

pub fn build_opencode_args(
    _ctx: &RuntimeContext,
    _options: &RuntimeBuildOptions,
    _prompt: Option<&str>,
) -> Vec<String> {
    // ACP launch: the model is set via `session/set_model` inside run_acp_session, not flags.
    vec!["acp".to_string()]
}

pub const OPENCODE_AGENT_DEF: RuntimeAgentDef = RuntimeAgentDef {
    id: "opencode",
    name: "OpenCode",
    bin: "opencode-cli",
    fallback_bins: &["opencode"],
    version_args: &["--version"],
    auth_probe_args: None,
    fallback_models: FALLBACK_MODELS,
    reasoning_options: &[],
    list_models_args: None,
    list_models_timeout_secs: Some(15),
    models_from_stderr: false,
    model_probe: Some(ModelProbeStrategy::Acp),
    model_probe_args: Some(&["acp"]),
    slash_strategy: SlashStrategy::Acp,
    env: &[],
    max_prompt_arg_bytes: None,
    prompt_via_stdin: false,
    prompt_input_format: PromptInputFormat::Text,
    stream_format: StreamFormat::AcpJsonRpc,
    json_event_parser: None,
    external_mcp_injection: Some(ExternalMcpInjection::AcpMerge),
    resumes_session_via_cli: false,
    build_args: build_opencode_args,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opencode_build_args_acp_mode() {
        let args = build_opencode_args(
            &RuntimeContext {
                cwd: None,
                extra_allowed_dirs: vec![],
                resume_session_id: None,
                new_session_id: None,
                include_partial_messages: false,
            },
            &RuntimeBuildOptions {
                model: Some("anthropic/claude-sonnet-4-5".to_string()),
                reasoning: None,
                sandbox: None,
            },
            None,
        );
        assert!(args.contains(&"acp".to_string()));
        assert!(!args.contains(&"run".to_string()));
    }
}
