//! `kivio-code` — headless terminal coding agent built on Kivio's existing
//! `chat::agent::run_agent_loop`. This module is the library core; the binary
//! (`src/bin/kivio-code.rs`) is a thin entry that parses args and calls
//! [`run_print`].
//!
//! Phase 1 scope: `-p "<prompt>"` runs ONE full agent turn (planning → tools →
//! synthesis) through the unmodified agent loop, streaming the answer to stdout.
//! No TUI, no run_python, no sessions yet.

pub mod executor;
pub mod host;
pub mod settings_loader;

use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::chat::agent::{run_agent_loop, AgentRunConfig, AgentRunEntry};
use crate::mcp::types::{
    native_edit_file_tool, native_glob_files_tool, native_list_dir_tool, native_read_file_tool,
    native_run_command_tool, native_search_files_tool, native_web_fetch_tool, native_write_file_tool,
    ChatToolDefinition,
};
use crate::settings::{ModelProvider, Settings};
use crate::skills::SkillRegistry;
use crate::state::AppState;

use executor::CliToolExecutor;
use host::CliAgentHost;
pub use settings_loader::{load_settings_from_disk, load_settings_from_path};

/// Options for a single print-mode run, parsed from CLI args by the bin.
#[derive(Debug, Clone)]
pub struct PrintOptions {
    /// The task prompt (already resolved: `-p`, positional, or stdin).
    pub prompt: String,
    /// `providerId:model` or just `providerId` override; empty = settings default.
    pub model: Option<String>,
    /// Provider id override; empty = settings default / from `model`.
    pub provider: Option<String>,
    /// Working directory the agent operates in (tools rooted here).
    pub cwd: PathBuf,
    /// Deny sensitive (write/edit/bash) tools, leaving only read-only tools.
    pub no_approve: bool,
    /// Stream reasoning to stderr.
    pub verbose: bool,
}

/// The core file/shell/web tools exposed to the model in print mode. Mirrors the
/// PI 7-tool coding set (read/write/edit/ls/find/grep/bash) plus web_fetch.
pub fn core_tool_definitions() -> Vec<ChatToolDefinition> {
    vec![
        native_read_file_tool(),
        native_list_dir_tool(),
        native_search_files_tool(),
        native_glob_files_tool(),
        native_write_file_tool(),
        native_edit_file_tool(),
        native_run_command_tool(),
        native_web_fetch_tool(),
    ]
}

/// Read the prompt from stdin when it is piped (not a TTY) and non-empty.
pub fn read_stdin_prompt() -> Option<String> {
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        return None;
    }
    let mut buffer = String::new();
    if std::io::stdin().read_to_string(&mut buffer).is_ok() {
        let trimmed = buffer.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    None
}

/// Resolve the (provider, model) pair to run, honoring `--provider` / `--model`
/// overrides over the settings chat default. `--model` may be `id:model`.
pub fn resolve_provider_model(
    settings: &Settings,
    provider_override: Option<&str>,
    model_override: Option<&str>,
) -> Result<(ModelProvider, String), String> {
    let (default_provider_id, default_model) = settings.effective_chat_model();

    // `--model providerId:model` splits into both; a bare value is the model.
    let (model_provider_id, model_name) = match model_override {
        Some(raw) if raw.contains(':') => {
            let (pid, model) = raw.split_once(':').unwrap();
            (Some(pid.to_string()), Some(model.to_string()))
        }
        Some(raw) => (None, Some(raw.to_string())),
        None => (None, None),
    };

    let provider_id = provider_override
        .map(str::to_string)
        .or(model_provider_id)
        .filter(|id| !id.trim().is_empty())
        .unwrap_or(default_provider_id);

    let model = model_name
        .filter(|m| !m.trim().is_empty())
        .unwrap_or(default_model);

    if provider_id.trim().is_empty() {
        return Err(
            "No chat provider configured. Set a default chat model in the Kivio app, or pass --provider/--model.".to_string(),
        );
    }

    let provider = settings
        .get_provider(&provider_id)
        .cloned()
        .ok_or_else(|| format!("Provider '{provider_id}' not found in settings."))?;

    if model.trim().is_empty() {
        return Err(format!(
            "No model configured for provider '{provider_id}'. Pass --model <name>."
        ));
    }

    Ok((provider, model))
}

/// A lean coding-agent system prompt for the terminal. Reuse of the chat
/// system-prompt builder is possible but it pulls in skills/memory/plan/todo
/// scaffolding the print-mode CLI does not use; this MVP prompt keeps the
/// contract tight (cwd + date + tool guidance).
pub fn build_system_prompt(cwd: &std::path::Path) -> String {
    let now = chrono::Local::now();
    format!(
        "You are kivio-code, an expert terminal coding assistant operating inside the user's project.\n\
\n\
You have these tools: read (read_file), write (write_file), edit (edit_file), ls (list_dir), find (glob_files), grep (search_files), bash (run_command), and web_fetch.\n\
\n\
Guidelines:\n\
- Use the tools to inspect and modify files instead of guessing. Prefer read_file over cat, grep over manual scanning.\n\
- Make edits with edit_file using exact, unique text matches; use write_file only for new files or full rewrites.\n\
- Run commands with bash (run_command) when you need to build, test, or inspect the environment.\n\
- Be concise. Show file paths clearly. Do only what the task requires.\n\
- When the task is complete, give a short final answer summarizing what you did.\n\
\n\
Current date: {date}\n\
Current working directory: {cwd}",
        date = now.format("%Y-%m-%d"),
        cwd = cwd.display()
    )
}

/// Build the headless `AppState` for a CLI run from loaded settings.
pub fn build_app_state(settings: Settings) -> Arc<AppState> {
    let usage_dir = settings_loader::app_data_dir()
        .map(|dir| dir.join("usage"))
        .unwrap_or_else(|| std::env::temp_dir().join("kivio-code-usage"));
    Arc::new(AppState::new_headless(settings, usage_dir))
}

/// Run one print-mode agent turn end to end. Returns the final assistant text.
///
/// `state` must outlive the loop; callers pass an owned `Arc<AppState>` that the
/// borrow here is tied to. The loop streams the answer to stdout via
/// [`CliAgentHost`]; this returns the accumulated content for the exit-code
/// decision (empty answer with no tools is treated as a failure by the bin).
pub async fn run_print(options: PrintOptions, state: &AppState) -> Result<String, String> {
    let settings = state.settings_read().clone();
    let (provider, model) = resolve_provider_model(
        &settings,
        options.provider.as_deref(),
        options.model.as_deref(),
    )?;

    let system_prompt = build_system_prompt(&options.cwd);
    let runtime_messages: Vec<Value> = vec![
        json!({ "role": "system", "content": system_prompt }),
        json!({ "role": "user", "content": options.prompt }),
    ];

    // Approval policy: print mode is non-interactive. Default consent-gated flow
    // (host grants session consent). With --no-approve, switch to per-call
    // confirmation so the host can deny sensitive (write/edit/bash) tools.
    let mut effective_chat_tools = settings.chat_tools.clone();
    if options.no_approve {
        effective_chat_tools.approval_policy = "always_confirm".to_string();
    } else {
        effective_chat_tools.approval_policy = "auto".to_string();
    }

    let cwd_root = options.cwd.to_string_lossy().into_owned();
    let host = CliAgentHost::new(options.verbose, !options.no_approve);
    let executor = CliToolExecutor::new(
        vec![cwd_root],
        state.http.clone(),
        effective_chat_tools.tool_timeout_ms,
    );

    let generation = host.generation();
    let language = if settings.chat.default_language.trim().is_empty() {
        "en".to_string()
    } else {
        settings.chat.default_language.clone()
    };
    let max_output_tokens = settings.chat.max_output_tokens;
    let retry_attempts = if settings.retry_enabled {
        settings.retry_attempts as usize
    } else {
        1
    };
    let thinking_enabled = settings.chat.thinking_enabled;
    let stream_enabled = settings.chat.stream_enabled;

    let config = AgentRunConfig {
        entry: AgentRunEntry::Send,
        state,
        conversation_id: "kivio-code".to_string(),
        tool_conversation_id: "kivio-code".to_string(),
        depth: 0,
        run_id: "kivio-code-run".to_string(),
        message_id: "kivio-code-msg".to_string(),
        generation,
        provider,
        model,
        runtime_messages,
        tools: core_tool_definitions(),
        blocked_tool_calls: Vec::new(),
        settings: settings.clone(),
        effective_chat_tools,
        language,
        has_image: false,
        thinking_enabled,
        stream_enabled,
        max_output_tokens,
        retry_attempts,
        skill_registry: SkillRegistry::default(),
        active_skill_id: None,
        active_skill_detail: None,
        assistant_snapshot: None,
        custom_system_prompt: String::new(),
        provider_tools_fallback_system_prompt: build_system_prompt(&options.cwd),
    };

    let result = run_agent_loop(config, &host, &executor).await?;
    Ok(result.content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ModelProvider;

    fn provider(id: &str) -> ModelProvider {
        ModelProvider {
            id: id.to_string(),
            name: id.to_string(),
            api_keys: vec!["sk-x".to_string()],
            api_key_legacy: None,
            base_url: "https://example.com/v1".to_string(),
            available_models: vec!["m1".to_string()],
            enabled_models: vec!["m1".to_string()],
            supports_tools: true,
            enabled: true,
            api_format: "openai_chat".to_string(),
            model_overrides: Default::default(),
        }
    }

    #[test]
    fn core_tools_are_the_expected_eight() {
        let names: Vec<String> = core_tool_definitions()
            .iter()
            .map(|t| t.name.clone())
            .collect();
        assert_eq!(names.len(), 8);
        for expected in [
            "read", "ls", "grep", "find", "write", "edit", "bash", "web_fetch",
        ] {
            assert!(
                names.iter().any(|n| n == expected),
                "missing core tool {expected}; got {names:?}"
            );
        }
    }

    #[test]
    fn resolve_provider_model_uses_settings_default() {
        let mut settings = Settings::default();
        settings.providers = vec![provider("chat")];
        settings.default_models.chat.provider_id = "chat".to_string();
        settings.default_models.chat.model = "m1".to_string();

        let (resolved, model) = resolve_provider_model(&settings, None, None).expect("resolves");
        assert_eq!(resolved.id, "chat");
        assert_eq!(model, "m1");
    }

    #[test]
    fn resolve_provider_model_honors_colon_override() {
        let mut settings = Settings::default();
        settings.providers = vec![provider("chat"), provider("other")];

        let (resolved, model) =
            resolve_provider_model(&settings, None, Some("other:m1")).expect("resolves");
        assert_eq!(resolved.id, "other");
        assert_eq!(model, "m1");
    }

    #[test]
    fn resolve_provider_model_errors_on_missing_provider() {
        let settings = Settings::default();
        let err = resolve_provider_model(&settings, Some("nope"), Some("m1")).unwrap_err();
        assert!(err.contains("not found") || err.contains("No chat provider"));
    }

    #[test]
    fn system_prompt_includes_cwd_and_date() {
        let prompt = build_system_prompt(std::path::Path::new("/tmp/project"));
        assert!(prompt.contains("/tmp/project"));
        assert!(prompt.contains("Current working directory"));
        assert!(prompt.contains("kivio-code"));
    }
}
