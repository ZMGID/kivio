use super::super::types::{
    PromptInputFormat, RuntimeAgentDef, RuntimeBuildOptions, RuntimeContext, StreamFormat,
};

const FALLBACK_MODELS: &[(&str, &str)] = &[
    ("default", "Default"),
    (
        "anthropic/claude-sonnet-4-5",
        "Claude Sonnet 4.5 (anthropic)",
    ),
    ("anthropic/claude-opus-4-5", "Claude Opus 4.5 (anthropic)"),
    ("openai/gpt-5", "GPT-5 (openai)"),
    ("openai/o4-mini", "o4-mini (openai)"),
    ("google/gemini-2.5-pro", "Gemini 2.5 Pro (google)"),
    ("google/gemini-2.5-flash", "Gemini 2.5 Flash (google)"),
];

const REASONING: &[(&str, &str)] = &[
    ("default", "Default"),
    ("off", "Off"),
    ("minimal", "Minimal"),
    ("low", "Low"),
    ("medium", "Medium"),
    ("high", "High"),
    ("xhigh", "XHigh"),
];

pub fn build_pi_args(
    ctx: &RuntimeContext,
    options: &RuntimeBuildOptions,
    _prompt: Option<&str>,
) -> Vec<String> {
    let mut args = vec!["--mode".to_string(), "rpc".to_string()];
    if let Some(model) = options.model.as_ref().filter(|m| *m != "default" && !m.is_empty()) {
        args.push("--model".to_string());
        args.push(model.clone());
    }
    if let Some(reasoning) = options
        .reasoning
        .as_ref()
        .filter(|r| *r != "default" && !r.is_empty())
    {
        args.push("--thinking".to_string());
        args.push(reasoning.clone());
    }
    for dir in &ctx.extra_allowed_dirs {
        if !dir.is_empty() {
            args.push("--append-system-prompt".to_string());
            args.push(dir.clone());
        }
    }
    args
}

pub const PI_AGENT_DEF: RuntimeAgentDef = RuntimeAgentDef {
    id: "pi",
    name: "Pi",
    bin: "pi",
    fallback_bins: &[],
    version_args: &["--version"],
    auth_probe_args: None,
    fallback_models: FALLBACK_MODELS,
    reasoning_options: REASONING,
    list_models_args: Some(&["--list-models"]),
    list_models_timeout_secs: Some(20),
    models_from_stderr: true,
    model_probe: None,
    model_probe_args: None,
    slash_strategy: super::super::types::SlashStrategy::None,
    env: &[],
    max_prompt_arg_bytes: None,
    prompt_via_stdin: true,
    prompt_input_format: PromptInputFormat::Text,
    stream_format: StreamFormat::PiRpc,
    json_event_parser: None,
    external_mcp_injection: None,
    resumes_session_via_cli: false,
    build_args: build_pi_args,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pi_build_args_rpc_mode_and_thinking() {
        let args = build_pi_args(
            &RuntimeContext {
                cwd: Some("/proj".to_string()),
                extra_allowed_dirs: vec!["/skills".to_string()],
                resume_session_id: None,
                new_session_id: None,
                include_partial_messages: false,
            },
            &RuntimeBuildOptions {
                model: Some("anthropic/claude-sonnet-4-5".to_string()),
                reasoning: Some("high".to_string()),
                sandbox: None,
            },
            None,
        );
        assert!(args.contains(&"rpc".to_string()));
        assert!(args.contains(&"--thinking".to_string()));
        assert!(args.contains(&"--append-system-prompt".to_string()));
    }
}
