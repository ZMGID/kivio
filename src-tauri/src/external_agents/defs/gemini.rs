use super::super::types::{
    ExternalMcpInjection, ModelProbeStrategy, PromptInputFormat, RuntimeAgentDef,
    RuntimeBuildOptions, RuntimeContext, SlashStrategy, StreamFormat,
};

const GEMINI_ENV: &[(&str, &str)] = &[("GEMINI_CLI_TRUST_WORKSPACE", "true")];

const FALLBACK_MODELS: &[(&str, &str)] = &[
    ("default", "Default"),
    ("gemini-3-pro-preview", "gemini-3-pro-preview"),
    ("gemini-3-flash-preview", "gemini-3-flash-preview"),
    ("gemini-2.5-pro", "gemini-2.5-pro"),
    ("gemini-2.5-flash", "gemini-2.5-flash"),
    ("gemini-2.5-flash-lite", "gemini-2.5-flash-lite"),
];

pub fn build_gemini_args(
    _ctx: &RuntimeContext,
    _options: &RuntimeBuildOptions,
    _prompt: Option<&str>,
) -> Vec<String> {
    // ACP launch: the model is set via `session/set_model` inside run_acp_session, not flags.
    vec!["--experimental-acp".to_string()]
}

pub const GEMINI_AGENT_DEF: RuntimeAgentDef = RuntimeAgentDef {
    id: "gemini",
    name: "Gemini CLI",
    bin: "gemini",
    fallback_bins: &[],
    version_args: &["--version"],
    auth_probe_args: None,
    fallback_models: FALLBACK_MODELS,
    reasoning_options: &[],
    list_models_args: None,
    list_models_timeout_secs: Some(15),
    models_from_stderr: false,
    model_probe: Some(ModelProbeStrategy::Acp),
    model_probe_args: Some(&["--experimental-acp"]),
    slash_strategy: SlashStrategy::Acp,
    env: GEMINI_ENV,
    max_prompt_arg_bytes: None,
    prompt_via_stdin: false,
    prompt_input_format: PromptInputFormat::Text,
    stream_format: StreamFormat::AcpJsonRpc,
    json_event_parser: None,
    external_mcp_injection: Some(ExternalMcpInjection::AcpMerge),
    resumes_session_via_cli: false,
    build_args: build_gemini_args,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gemini_build_args_acp_mode() {
        let args = build_gemini_args(
            &RuntimeContext {
                cwd: None,
                extra_allowed_dirs: vec![],
                resume_session_id: None,
                new_session_id: None,
                include_partial_messages: false,
            },
            &RuntimeBuildOptions {
                model: Some("gemini-2.5-pro".to_string()),
                reasoning: None,
                sandbox: None,
            },
            None,
        );
        assert_eq!(args, vec!["--experimental-acp"]);
    }
}
