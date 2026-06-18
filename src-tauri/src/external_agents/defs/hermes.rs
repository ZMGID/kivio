use super::super::types::{
    ExternalMcpInjection, ModelProbeStrategy, PromptInputFormat, RuntimeAgentDef,
    RuntimeBuildOptions, RuntimeContext, StreamFormat,
};

const FALLBACK_MODELS: &[(&str, &str)] = &[
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

pub fn build_hermes_args(
    _ctx: &RuntimeContext,
    _options: &RuntimeBuildOptions,
    _prompt: Option<&str>,
) -> Vec<String> {
    vec!["acp".to_string(), "--accept-hooks".to_string()]
}

pub const HERMES_AGENT_DEF: RuntimeAgentDef = RuntimeAgentDef {
    id: "hermes",
    name: "Hermes",
    bin: "hermes",
    fallback_bins: &[],
    version_args: &["--version"],
    auth_probe_args: None,
    fallback_models: FALLBACK_MODELS,
    reasoning_options: &[],
    list_models_args: None,
    list_models_timeout_secs: Some(15),
    models_from_stderr: false,
    model_probe: Some(ModelProbeStrategy::Acp),
    model_probe_args: Some(&["acp", "--accept-hooks"]),
    slash_strategy: super::super::types::SlashStrategy::None,
    env: &[],
    max_prompt_arg_bytes: None,
    prompt_via_stdin: false,
    prompt_input_format: PromptInputFormat::Text,
    stream_format: StreamFormat::AcpJsonRpc,
    json_event_parser: None,
    external_mcp_injection: Some(ExternalMcpInjection::AcpMerge),
    resumes_session_via_cli: false,
    build_args: build_hermes_args,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hermes_build_args_acp_mode() {
        let args = build_hermes_args(
            &RuntimeContext {
                cwd: None,
                extra_allowed_dirs: vec![],
                resume_session_id: None,
                new_session_id: None,
                include_partial_messages: false,
            },
            &RuntimeBuildOptions {
                model: None,
                reasoning: None,
                sandbox: None,
            },
            None,
        );
        assert_eq!(args, vec!["acp", "--accept-hooks"]);
    }
}
