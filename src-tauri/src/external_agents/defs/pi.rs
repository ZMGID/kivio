use super::super::types::{
    PromptInputFormat, RuntimeAgentDef, RuntimeBuildOptions, RuntimeContext, StreamFormat,
};

const FALLBACK_MODELS: &[(&str, &str)] = &[
    // Pi's real models are user-configured and discovered via `pi --list-models`; if that fails
    // we only offer Default rather than inventing provider models the user never set up.
    ("default", "Default"),
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
    _ctx: &RuntimeContext,
    options: &RuntimeBuildOptions,
    _prompt: Option<&str>,
) -> Vec<String> {
    let mut args = vec!["--mode".to_string(), "rpc".to_string()];
    if let Some(model) = options
        .model
        .as_ref()
        .filter(|m| *m != "default" && !m.is_empty())
    {
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
    // 注意：pi 无「授权目录」flag（`--help` 无 --add-dir/allowed-dir 等价项，只有 --approve 信任
    // 项目本地文件）。此前把 extra_allowed_dirs 塞进 `--append-system-prompt` 是误用——该 flag
    // 是「向系统提示追加文本/文件内容」，会把目录路径当提示词写进去，既不授权也污染上下文。
    // 附件目录路径已在 prompt 文本的附件说明块里给出，pi 靠自身文件权限模型读取，无需此处注入。
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
    slash_strategy: super::super::types::SlashStrategy::PiRpc,
    env: &[],
    max_prompt_arg_bytes: None,
    prompt_via_stdin: true,
    prompt_input_format: PromptInputFormat::Text,
    stream_format: StreamFormat::PiRpc,
    json_event_parser: None,
    resumes_session_via_cli: false,
    supports_native_image: false,
    image_mime_whitelist: &[],
    build_args: build_pi_args,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pi_build_args_rpc_mode_and_thinking() {
        let args = build_pi_args(
            &RuntimeContext {
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
        // extra_allowed_dirs 不再被塞进 --append-system-prompt（pi 无授权目录 flag）。
        assert!(!args.contains(&"--append-system-prompt".to_string()));
        assert!(!args.contains(&"/skills".to_string()));
    }
}
