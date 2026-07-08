use super::super::types::{
    PromptInputFormat, RuntimeAgentDef, RuntimeBuildOptions, RuntimeContext, StreamFormat,
};

// No fallback models — codex's `debug models` output is the source of truth. If discovery
// fails, the dropdown stays empty (the CLI defaults to whichever model the user configured).
const FALLBACK_MODELS: &[(&str, &str)] = &[];

const REASONING: &[(&str, &str)] = &[
    ("default", "Default"),
    ("none", "None"),
    ("minimal", "Minimal"),
    ("low", "Low"),
    ("medium", "Medium"),
    ("high", "High"),
    ("xhigh", "XHigh"),
];

fn codex_needs_danger_full_access() -> bool {
    if std::env::var("KIVIO_CODEX_SANDBOX")
        .ok()
        .as_deref()
        .map(str::trim)
        == Some("danger-full-access")
    {
        return true;
    }
    if cfg!(target_os = "windows") {
        return true;
    }
    std::env::var("WSL_DISTRO_NAME")
        .ok()
        .is_some_and(|v| !v.trim().is_empty())
}

#[allow(dead_code)]
fn _codex_sandbox_hint() -> bool {
    codex_needs_danger_full_access()
}

pub fn build_codex_args(
    _ctx: &RuntimeContext,
    _options: &RuntimeBuildOptions,
    _prompt: Option<&str>,
) -> Vec<String> {
    // The app-server protocol negotiates cwd / model / sandbox / approval over JSON-RPC
    // (`Thread/start` + `Turn/start`), so no model / sandbox CLI flags are needed here.
    vec!["app-server".to_string()]
}

pub const CODEX_AGENT_DEF: RuntimeAgentDef = RuntimeAgentDef {
    id: "codex",
    name: "Codex CLI",
    bin: "codex",
    fallback_bins: &[],
    version_args: &["--version"],
    auth_probe_args: Some(&["login", "status"]),
    fallback_models: FALLBACK_MODELS,
    reasoning_options: REASONING,
    list_models_args: Some(&["debug", "models"]),
    list_models_timeout_secs: Some(30),
    models_from_stderr: false,
    model_probe: None,
    model_probe_args: None,
    slash_strategy: super::super::types::SlashStrategy::CodexAppServer,
    env: &[],
    max_prompt_arg_bytes: None,
    prompt_via_stdin: false,
    prompt_input_format: PromptInputFormat::Text,
    stream_format: StreamFormat::CodexAppServer,
    json_event_parser: None,
    resumes_session_via_cli: false,
    build_args: build_codex_args,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_build_args_uses_app_server() {
        let args = build_codex_args(
            &RuntimeContext {
                cwd: Some("/tmp/p".to_string()),
                extra_allowed_dirs: vec![],
                resume_session_id: None,
                new_session_id: None,
                include_partial_messages: false,
            },
            &RuntimeBuildOptions {
                model: Some("gpt-5".to_string()),
                reasoning: Some("high".to_string()),
                sandbox: None,
            },
            None,
        );
        assert_eq!(args, vec!["app-server".to_string()]);
    }
}
