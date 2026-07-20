use super::super::types::{
    JsonEventParser, PromptInputFormat, RuntimeAgentDef, RuntimeBuildOptions, RuntimeContext,
    StreamFormat,
};

const FALLBACK_MODELS: &[(&str, &str)] = &[
    // Kimi Code 的真实模型来自 `kimi provider list --json`（下方 list_models_args）；探测失败时
    // 只给默认 + 当前 managed:kimi-code 主力型号（oauth 登录即得），前端会标注为「默认列表」。
    ("default", "Default"),
    ("kimi-code/k3", "K3 (kimi-code/k3)"),
    ("kimi-code/kimi-for-coding", "K2.7 Coding (kimi-code/kimi-for-coding)"),
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
    // `kimi provider list --json` 输出 providers/models 配置 JSON；模型键即 --model 别名
    // （如 kimi-code/k3）。解析见 detection.rs::parse_models_list 的 "kimi" 分支。
    list_models_args: Some(&["provider", "list", "--json"]),
    list_models_timeout_secs: Some(10),
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
    supports_native_image: false,
    image_mime_whitelist: &[],
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
