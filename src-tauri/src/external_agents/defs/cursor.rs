use super::super::types::{
    ModelProbeStrategy, PromptInputFormat, RuntimeAgentDef, RuntimeBuildOptions, RuntimeContext,
    SlashStrategy, StreamFormat,
};

const FALLBACK_MODELS: &[(&str, &str)] = &[
    ("auto", "auto"),
    ("sonnet-4", "sonnet-4"),
    ("gpt-5", "gpt-5"),
];

pub fn build_cursor_args(
    _ctx: &RuntimeContext,
    _options: &RuntimeBuildOptions,
    _prompt: Option<&str>,
) -> Vec<String> {
    // ACP launch: the model is set via `session/set_model` inside run_acp_session, not flags.
    vec!["acp".to_string()]
}

pub const CURSOR_AGENT_DEF: RuntimeAgentDef = RuntimeAgentDef {
    id: "cursor-agent",
    name: "Cursor Agent",
    bin: "cursor-agent",
    fallback_bins: &[],
    version_args: &["--version"],
    auth_probe_args: Some(&["status"]),
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
    resumes_session_via_cli: false,
    build_args: build_cursor_args,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_build_args_acp_mode() {
        let args = build_cursor_args(
            &RuntimeContext {
                cwd: Some("/proj".to_string()),
                extra_allowed_dirs: vec![],
                resume_session_id: None,
                new_session_id: None,
                include_partial_messages: false,
            },
            &RuntimeBuildOptions {
                model: Some("auto".to_string()),
                reasoning: None,
                sandbox: None,
            },
            None,
        );
        assert_eq!(args, vec!["acp"]);
    }
}
