use super::super::types::{
    JsonEventParser, PromptInputFormat, RuntimeAgentDef, RuntimeBuildOptions, RuntimeContext,
    StreamFormat,
};

const FALLBACK_MODELS: &[(&str, &str)] = &[
    ("default", "Default"),
    ("kimi-k2-turbo-preview", "kimi-k2-turbo-preview"),
    ("moonshot-v1-8k", "moonshot-v1-8k"),
    ("moonshot-v1-32k", "moonshot-v1-32k"),
];

pub fn build_kimi_args(
    _ctx: &RuntimeContext,
    options: &RuntimeBuildOptions,
    prompt: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "-p".to_string(),
        prompt.unwrap_or("").to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
    ];
    if let Some(model) = options
        .model
        .as_ref()
        .filter(|m| *m != "default" && !m.is_empty())
    {
        args.push("--model".to_string());
        args.push(model.clone());
    }
    args
}

pub const KIMI_AGENT_DEF: RuntimeAgentDef = RuntimeAgentDef {
    id: "kimi",
    name: "Kimi CLI",
    bin: "kimi",
    fallback_bins: &[],
    version_args: &["--version"],
    auth_probe_args: None,
    fallback_models: FALLBACK_MODELS,
    reasoning_options: &[],
    list_models_args: None,
    list_models_timeout_secs: None,
    models_from_stderr: false,
    model_probe: None,
    model_probe_args: None,
    slash_strategy: super::super::types::SlashStrategy::None,
    env: &[],
    max_prompt_arg_bytes: Some(30_000),
    prompt_via_stdin: false,
    prompt_input_format: PromptInputFormat::Text,
    stream_format: StreamFormat::JsonEventStream,
    json_event_parser: Some(JsonEventParser::Kimi),
    resumes_session_via_cli: false,
    build_args: build_kimi_args,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kimi_build_args_puts_prompt_in_argv() {
        let args = build_kimi_args(
            &RuntimeContext {
                extra_allowed_dirs: vec![],
                resume_session_id: None,
                new_session_id: None,
                include_partial_messages: false,
            },
            &RuntimeBuildOptions {
                model: Some("kimi-k2-turbo-preview".to_string()),
                reasoning: None,
                sandbox: None,
            },
            Some("hello world"),
        );
        assert_eq!(args[0], "-p");
        assert_eq!(args[1], "hello world");
        assert!(args.contains(&"--model".to_string()));
    }
}
