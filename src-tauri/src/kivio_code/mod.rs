//! `kivio-code` — headless terminal coding agent built on Kivio's existing
//! `chat::agent::run_agent_loop`. This module is the library core; the binary
//! (`src/bin/kivio-code.rs`) is a thin entry that parses args and calls
//! [`run_print`].
//!
//! Phase 1 scope: `-p "<prompt>"` runs ONE full agent turn (planning → tools →
//! synthesis) through the unmodified agent loop, streaming the answer to stdout.
//! No TUI, no run_python, no sessions yet.

pub mod config;
pub mod errors;
pub mod executor;
pub mod host;
pub mod interactive;
pub mod mcp_setup;
pub mod project_context;
pub mod session;
pub mod settings_loader;
pub mod skill_setup;
pub mod tui;

use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::chat::agent::{run_agent_loop, AgentRunConfig, AgentRunEntry};
use crate::mcp::types::{
    native_edit_file_tool, native_glob_files_tool, native_list_dir_tool, native_read_file_tool,
    native_run_command_tool, native_search_files_tool, native_web_fetch_tool,
    native_web_search_tool, native_write_file_tool, ChatToolDefinition,
};
use crate::settings::{ModelProvider, Settings, WebSearchProvider};
use crate::skills::SkillRegistry;
use crate::state::AppState;

use executor::CliToolExecutor;
use host::CliAgentHost;
pub use settings_loader::{load_settings_from_disk, load_settings_from_path};

/// Plan-mode system note appended to a turn's message clone (never to the stored
/// `runtime_messages`) when the interactive App is in [`AgentMode::Plan`]. It tells the
/// model it is in read-only research/planning mode: research by reading/searching, do NOT
/// mutate files or run commands or claim to have made changes, and produce a concise,
/// numbered, executable plan that is implemented only after switching to build mode.
///
/// (Tooling already enforces read-only via [`into_config`]'s `plan_mode` filter; this note
/// is the behavioral half so the model proposes a plan instead of trying to act.)
///
/// [`AgentMode::Plan`]: crate::kivio_code::interactive::AgentMode
/// [`into_config`]: TurnAssembly::into_config
pub const PLAN_SYSTEM_NOTE: &str = "You are in PLAN mode (read-only). Research the project by reading and searching files; you CANNOT modify files, run commands, or claim to have made any changes. Produce a concise, numbered, executable plan, and state that implementation happens after the user switches to build mode.";

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

/// Whether web search is usable: the configured Lens web-search provider has a
/// non-empty API key. Mirrors the GUI's gate (`mcp::registry::web_search_configured`)
/// so the `web_search` tool is advertised to the model only when it would actually
/// work (a key is set), never advertised-but-broken.
pub fn web_search_configured(settings: &Settings) -> bool {
    match settings.lens.web_search.provider {
        WebSearchProvider::Tavily => !settings.lens.web_search.tavily_api_key.trim().is_empty(),
        WebSearchProvider::Exa => !settings.lens.web_search.exa_api_key.trim().is_empty(),
    }
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
/// system-prompt builder is possible but it pulls in memory/plan/todo
/// scaffolding the print-mode CLI does not use; this MVP prompt keeps the
/// contract tight (cwd + date + tool guidance + skill catalog).
///
/// `skill_registry` is the same registry that drives the per-turn skill tool
/// definitions (built in [`TurnAssembly::resolve`]). When it is non-empty we
/// append the GUI's skill catalog (`crate::skills::format_catalog`) so the model
/// is actually told which skills exist and when to `skill_activate` them — without
/// it the activation tools are advertised but the model is blind to the catalog.
pub fn build_system_prompt(cwd: &std::path::Path, skill_registry: &SkillRegistry) -> String {
    let now = chrono::Local::now();
    // Auto-load the project's own instruction files (.kivio/, root AGENTS.md /
    // KIVIO.md, optionally CLAUDE.md + .claude/CLAUDE.md, and global
    // <app_data>/agents/AGENTS.md) and splice them in after the base guidance but
    // before the date/cwd footer. The `read_claude_dir` toggle (persisted in
    // kivio-code's own config) gates the Claude-Code compatibility files; reading
    // the config per turn is cheap. Empty when nothing relevant is found.
    let read_claude = config::load().read_claude_dir;
    let project_context = project_context::load_project_context(cwd, read_claude);
    let project_block = if project_context.is_empty() {
        String::new()
    } else {
        format!("\n{project_context}\n")
    };
    // Skill catalog: only when the registry actually holds skills. The registry
    // is already filtered to enabled skills in `build_skill_registry`, so the
    // enabled-filter closure is always-true; `active_skill_id` = None (nothing
    // pre-activated); `tools_available` = true (the CLI runs tool-capable models
    // and advertises the skill_activate trio). Placed after the project context,
    // before the date/cwd footer, in a clearly delimited section.
    let skill_block = {
        let catalog = crate::skills::format_catalog(skill_registry, None, true, |_| true);
        if catalog.is_empty() {
            String::new()
        } else {
            format!("\n<skills>\n{catalog}\n</skills>\n")
        }
    };
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
{project_block}\
{skill_block}\
\n\
Current date: {date}\n\
Current working directory: {cwd}",
        project_block = project_block,
        skill_block = skill_block,
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

/// Shared, host-agnostic assembly for one agent turn. Both print mode
/// ([`run_print`]) and interactive mode build their [`AgentRunConfig`] from this,
/// so the provider/model resolution, system prompt, tool set, and the many
/// per-run knobs derived from `Settings` live in exactly one place.
///
/// What this deliberately does *not* hold: the `host`, the `executor`, the
/// `runtime_messages`, the cancel `generation`, and the per-turn `message_id` —
/// those differ between print (one shot, stdout host) and interactive (streaming
/// host, multi-turn message log) and are supplied at [`into_config`] time.
///
/// [`into_config`]: TurnAssembly::into_config
pub struct TurnAssembly {
    pub provider: ModelProvider,
    pub model: String,
    pub system_prompt: String,
    pub effective_chat_tools: crate::settings::ChatToolsConfig,
    pub language: String,
    pub thinking_enabled: bool,
    pub stream_enabled: bool,
    pub max_output_tokens: u32,
    pub retry_attempts: usize,
    pub settings: Settings,
    /// Tool definitions contributed by connected MCP servers. Collected
    /// asynchronously at startup (MCP connection is async) via
    /// [`mcp_setup::collect_mcp_tools`] and set with [`set_mcp_tools`]; empty
    /// until then. Merged into the per-turn tool set in [`into_config`].
    ///
    /// [`set_mcp_tools`]: TurnAssembly::set_mcp_tools
    /// [`into_config`]: TurnAssembly::into_config
    pub mcp_tools: Vec<ChatToolDefinition>,
    /// Skills discovered for this run (user dir + built-ins), built at
    /// construction via [`skill_setup::build_skill_registry`]. Passed to the
    /// loop and used to derive skill tool definitions in [`into_config`].
    ///
    /// [`into_config`]: TurnAssembly::into_config
    pub skill_registry: SkillRegistry,
}

impl TurnAssembly {
    /// Resolve the provider/model and derive every settings-driven knob for a
    /// run rooted at `cwd`. `approve_sensitive` mirrors print mode's
    /// `!no_approve`: when false the approval policy is `always_confirm` so the
    /// host can deny sensitive (write/edit/bash) tools; when true it is `auto`.
    pub fn resolve(
        settings: &Settings,
        provider_override: Option<&str>,
        model_override: Option<&str>,
        cwd: &std::path::Path,
        approve_sensitive: bool,
    ) -> Result<Self, String> {
        let (provider, model) =
            resolve_provider_model(settings, provider_override, model_override)?;

        let mut effective_chat_tools = settings.chat_tools.clone();
        effective_chat_tools.approval_policy = if approve_sensitive {
            "auto".to_string()
        } else {
            "always_confirm".to_string()
        };

        let language = if settings.chat.default_language.trim().is_empty() {
            "en".to_string()
        } else {
            settings.chat.default_language.clone()
        };
        let retry_attempts = if settings.retry_enabled {
            settings.retry_attempts as usize
        } else {
            1
        };

        // Skills are discovered synchronously here so they are available the
        // moment the assembly exists. Build the registry BEFORE the system prompt
        // so the same registry instance drives BOTH the prompt's skill catalog
        // and `into_config`'s skill tool definitions (kept consistent).
        let skill_registry = skill_setup::build_skill_registry(settings, cwd);
        let system_prompt = build_system_prompt(cwd, &skill_registry);

        Ok(Self {
            provider,
            model,
            system_prompt,
            effective_chat_tools,
            language,
            thinking_enabled: settings.chat.thinking_enabled,
            stream_enabled: settings.chat.stream_enabled,
            max_output_tokens: settings.chat.max_output_tokens,
            retry_attempts,
            settings: settings.clone(),
            // MCP tools are collected asynchronously at startup (see
            // `set_mcp_tools`); start empty.
            mcp_tools: Vec::new(),
            skill_registry,
        })
    }

    /// The `providerId:model` **resolution** value: used by session headers,
    /// `/model` selection, and `split_model_label` to RESOLVE the provider back
    /// from settings. This MUST stay id-based — do not switch it to the name.
    pub fn model_label(&self) -> String {
        format!("{}:{}", self.provider.id, self.model)
    }

    /// The human-readable **display** label for footers / welcome / notices:
    /// `<Provider Name> · <model>`. Falls back to the provider id only when the
    /// name is blank. This is purely cosmetic and is never parsed back into a
    /// provider — selection/resolution always go through [`model_label`].
    ///
    /// [`model_label`]: TurnAssembly::model_label
    pub fn model_label_display(&self) -> String {
        provider_model_display(&self.provider.name, &self.provider.id, &self.model)
    }

    /// Install the MCP tool definitions collected by
    /// [`mcp_setup::collect_mcp_tools`]. Called once at startup after the (async)
    /// MCP connection completes; they are then merged into every turn's tool set
    /// by [`into_config`].
    ///
    /// [`into_config`]: TurnAssembly::into_config
    pub fn set_mcp_tools(&mut self, tools: Vec<ChatToolDefinition>) {
        self.mcp_tools = tools;
    }

    /// Turn this assembly into a ready-to-run [`AgentRunConfig`]. `runtime_messages`
    /// is the full conversation context (system + prior turns + new user message);
    /// `message_id`/`run_id` identify this turn; `generation` is the cancel token
    /// the host's `is_generation_active` polls against.
    ///
    /// `plan_mode` gates the tool set: when true, the assembled tools are filtered to
    /// only [`ChatToolDefinition::is_read_only_tool`] entries (drops write/edit/bash;
    /// keeps read/ls/grep/find/web_fetch + read-only skill tools), so a plan-mode turn
    /// cannot mutate the workspace. Print mode passes `false` (plan mode is interactive
    /// only); the plan-mode SYSTEM note is injected by the caller into the per-turn
    /// `runtime_messages` clone (see [`PLAN_SYSTEM_NOTE`]).
    #[allow(clippy::too_many_arguments)]
    pub fn into_config<'a>(
        &'a self,
        state: &'a AppState,
        conversation_id: String,
        run_id: String,
        message_id: String,
        generation: u64,
        runtime_messages: Vec<Value>,
        plan_mode: bool,
    ) -> AgentRunConfig<'a> {
        // Assemble the per-turn tool set from all sources: core coding tools,
        // MCP server tools, and skill-provided tools. Each source owns its own
        // module (executor / mcp_setup / skill_setup) so they extend disjoint
        // lines here.
        let mut tools = core_tool_definitions();
        tools.extend(self.mcp_tools.clone());
        tools.extend(skill_setup::skill_tool_definitions(&self.skill_registry));

        // web_search is appended only when a Lens web-search provider key is
        // configured (mirrors the GUI gate). Otherwise it is never advertised, so
        // the model can't call a tool that would immediately fail for lack of a key.
        // It is read-only (`is_read_only_tool()`), so it survives the plan-mode
        // filter below — desired: research/planning may search the web.
        if web_search_configured(&self.settings) {
            tools.push(native_web_search_tool());
        }

        // Plan mode: keep only read-only tools (read/ls/grep/find/web_fetch + read-only
        // skill tools); drop write/edit/bash so the turn cannot mutate the workspace.
        if plan_mode {
            tools.retain(|t| t.is_read_only_tool());
        }

        AgentRunConfig {
            entry: AgentRunEntry::Send,
            state,
            tool_conversation_id: conversation_id.clone(),
            conversation_id,
            depth: 0,
            run_id,
            message_id,
            generation,
            provider: self.provider.clone(),
            model: self.model.clone(),
            runtime_messages,
            tools,
            blocked_tool_calls: Vec::new(),
            settings: self.settings.clone(),
            effective_chat_tools: self.effective_chat_tools.clone(),
            language: self.language.clone(),
            has_image: false,
            thinking_enabled: self.thinking_enabled,
            stream_enabled: self.stream_enabled,
            max_output_tokens: self.max_output_tokens,
            retry_attempts: self.retry_attempts,
            skill_registry: self.skill_registry.clone(),
            active_skill_id: None,
            active_skill_detail: None,
            assistant_snapshot: None,
            custom_system_prompt: String::new(),
            provider_tools_fallback_system_prompt: self.system_prompt.clone(),
        }
    }
}

/// Build the cosmetic `<Provider Name> · <model>` display label. Falls back to
/// the provider id when the name is blank. Display-only — never parsed back into
/// a provider (selection/resolution use `providerId:model`).
pub fn provider_model_display(provider_name: &str, provider_id: &str, model: &str) -> String {
    let label = if provider_name.trim().is_empty() {
        provider_id
    } else {
        provider_name
    };
    format!("{label} · {model}")
}

/// Run one print-mode agent turn end to end. Returns the final assistant text.
///
/// `state` must outlive the loop; callers pass an owned `Arc<AppState>` that the
/// borrow here is tied to. The loop streams the answer to stdout via
/// [`CliAgentHost`]; this returns the accumulated content for the exit-code
/// decision (empty answer with no tools is treated as a failure by the bin).
pub async fn run_print(options: PrintOptions, state: &Arc<AppState>) -> Result<String, String> {
    let settings = state.settings_read().clone();
    let mut assembly = TurnAssembly::resolve(
        &settings,
        options.provider.as_deref(),
        options.model.as_deref(),
        &options.cwd,
        !options.no_approve,
    )?;

    // MCP tools are collected asynchronously (server connection is async), then
    // merged into the per-turn tool set by `into_config`. Stub returns empty.
    let mcp_tools = mcp_setup::collect_mcp_tools(state, &settings).await;
    assembly.set_mcp_tools(mcp_tools);

    let runtime_messages: Vec<Value> = vec![
        json!({ "role": "system", "content": assembly.system_prompt.clone() }),
        json!({ "role": "user", "content": options.prompt }),
    ];

    let host = CliAgentHost::new(options.verbose, !options.no_approve);
    let executor = CliToolExecutor::new(
        &options.cwd,
        state.http.clone(),
        assembly.effective_chat_tools.tool_timeout_ms,
        state.clone(),
        assembly.skill_registry.clone(),
        assembly.effective_chat_tools.clone(),
    );

    let config = assembly.into_config(
        state,
        "kivio-code".to_string(),
        "kivio-code-run".to_string(),
        "kivio-code-msg".to_string(),
        host.generation(),
        runtime_messages,
        /* plan_mode */ false,
    );

    // Map the loop error to a concise, actionable message before bubbling it up,
    // so `-p` mode prints a friendly one-liner (the bin just `eprintln!`s it)
    // instead of the raw provider JSON / retry-count noise. Cancellation is
    // passed through unchanged by `friendly_error`.
    let result = run_agent_loop(config, &host, &executor)
        .await
        .map_err(|err| errors::friendly_error(&err))?;
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
        let prompt =
            build_system_prompt(std::path::Path::new("/tmp/project"), &SkillRegistry::default());
        assert!(prompt.contains("/tmp/project"));
        assert!(prompt.contains("Current working directory"));
        assert!(prompt.contains("kivio-code"));
    }

    #[test]
    fn system_prompt_omits_catalog_for_empty_registry() {
        let prompt =
            build_system_prompt(std::path::Path::new("/tmp/project"), &SkillRegistry::default());
        // No skills → no catalog section and no activation hint at all.
        assert!(!prompt.contains("<skills>"));
        assert!(!prompt.contains("<available_skills>"));
        assert!(!prompt.contains("skill_activate"));
    }

    #[test]
    fn system_prompt_embeds_skill_catalog_when_registry_non_empty() {
        use crate::skills::{SkillMeta, SkillRecord};
        let record = SkillRecord {
            meta: SkillMeta {
                id: "pdf".to_string(),
                name: "pdf-wizard".to_string(),
                description: "Extract and edit PDF files".to_string(),
                source: "user".to_string(),
                path: None,
                recommended_tools: vec![],
                disable_model_invocation: false,
                files: vec![],
                triggers: vec![],
                argument_hint: None,
                arguments: vec![],
            },
            location: std::path::PathBuf::from("/skills/pdf/SKILL.md"),
            base_dir: std::path::PathBuf::from("/skills/pdf"),
            body: String::new(),
            allowed_tools: vec![],
        };
        let registry = SkillRegistry {
            records: vec![record],
            warnings: vec![],
        };

        let prompt = build_system_prompt(std::path::Path::new("/tmp/project"), &registry);
        // The catalog names the skill and instructs the model to skill_activate.
        assert!(prompt.contains("<skills>"));
        assert!(prompt.contains("pdf-wizard"));
        assert!(prompt.contains("skill_activate"));
        // Catalog sits before the date/cwd footer.
        let pos_catalog = prompt.find("<skills>").unwrap();
        let pos_cwd = prompt.find("Current working directory").unwrap();
        assert!(pos_catalog < pos_cwd, "skill catalog must precede the cwd footer");
    }

    #[test]
    fn system_prompt_embeds_project_context_when_present() {
        let dir = std::env::temp_dir().join(format!("kivio-sysprompt-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        std::fs::write(dir.join("AGENTS.md"), "be very careful").expect("write AGENTS.md");

        let prompt = build_system_prompt(&dir, &SkillRegistry::default());
        assert!(prompt.contains("<project_context>"));
        assert!(prompt.contains("be very careful"));
        // The date/cwd footer still trails the project context block.
        let pos_ctx = prompt.find("<project_context>").unwrap();
        let pos_cwd = prompt.find("Current working directory").unwrap();
        assert!(pos_ctx < pos_cwd, "project context must precede the cwd footer");

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn assembly_with(provider_id: &str, provider_name: &str, model: &str) -> TurnAssembly {
        let mut p = provider(provider_id);
        p.name = provider_name.to_string();
        let mut settings = Settings::default();
        settings.providers = vec![p.clone()];
        TurnAssembly {
            provider: p,
            model: model.to_string(),
            system_prompt: String::new(),
            effective_chat_tools: settings.chat_tools.clone(),
            language: "en".to_string(),
            thinking_enabled: false,
            stream_enabled: true,
            max_output_tokens: 4096,
            retry_attempts: 1,
            settings,
            mcp_tools: Vec::new(),
            skill_registry: SkillRegistry::default(),
        }
    }

    #[test]
    fn model_label_is_id_based_for_resolution() {
        // The selection / resolution VALUE must stay `providerId:model` so the
        // /model selector and split_model_label can resolve the provider back.
        let a = assembly_with("provider-1780492912291", "DeepSeek Pool", "deepseek-v4-flash");
        assert_eq!(a.model_label(), "provider-1780492912291:deepseek-v4-flash");
    }

    #[test]
    fn model_label_display_uses_provider_name() {
        let a = assembly_with("provider-1780492912291", "DeepSeek Pool", "deepseek-v4-flash");
        assert_eq!(a.model_label_display(), "DeepSeek Pool · deepseek-v4-flash");
        // raw id must NOT leak into the display label.
        assert!(!a.model_label_display().contains("provider-1780492912291"));
    }

    #[test]
    fn model_label_display_falls_back_to_id_when_name_blank() {
        let a = assembly_with("prov-id", "", "m1");
        assert_eq!(a.model_label_display(), "prov-id · m1");
    }

    #[test]
    fn provider_model_display_helper() {
        assert_eq!(provider_model_display("OpenAI", "p1", "gpt-4o"), "OpenAI · gpt-4o");
        assert_eq!(provider_model_display("  ", "p1", "gpt-4o"), "p1 · gpt-4o");
    }

    #[test]
    fn into_config_plan_mode_drops_mutating_tools() {
        let assembly = assembly_with("chat", "Chat", "m1");
        let state = build_app_state(assembly.settings.clone());

        let tool_names = |plan_mode: bool| -> Vec<String> {
            let cfg = assembly.into_config(
                &state,
                "c".to_string(),
                "r".to_string(),
                "msg".to_string(),
                1,
                Vec::new(),
                plan_mode,
            );
            cfg.tools.iter().map(|t| t.name.clone()).collect()
        };

        // Build mode keeps the full core set (incl. write/edit/bash).
        let build = tool_names(false);
        for expected in ["read", "ls", "grep", "find", "write", "edit", "bash", "web_fetch"] {
            assert!(build.iter().any(|n| n == expected), "build missing {expected}: {build:?}");
        }

        // Plan mode drops the mutating tools but keeps the read-only ones.
        let plan = tool_names(true);
        for blocked in ["write", "edit", "bash"] {
            assert!(!plan.iter().any(|n| n == blocked), "plan must drop {blocked}: {plan:?}");
        }
        for kept in ["read", "ls", "grep", "find", "web_fetch"] {
            assert!(plan.iter().any(|n| n == kept), "plan must keep {kept}: {plan:?}");
        }
    }

    fn assembly_with_tavily_key(key: &str) -> TurnAssembly {
        let mut a = assembly_with("chat", "Chat", "m1");
        a.settings.lens.web_search.provider = crate::settings::WebSearchProvider::Tavily;
        a.settings.lens.web_search.tavily_api_key = key.to_string();
        a
    }

    #[test]
    fn web_search_configured_true_with_tavily_key_false_when_empty() {
        let mut settings = Settings::default();
        settings.lens.web_search.provider = crate::settings::WebSearchProvider::Tavily;

        settings.lens.web_search.tavily_api_key = String::new();
        assert!(!web_search_configured(&settings), "empty key must be unconfigured");

        settings.lens.web_search.tavily_api_key = "tvly-abc".to_string();
        assert!(web_search_configured(&settings), "non-empty Tavily key is configured");

        // Whitespace-only is treated as empty (trimmed).
        settings.lens.web_search.tavily_api_key = "   ".to_string();
        assert!(!web_search_configured(&settings), "whitespace key must be unconfigured");

        // Switching provider to Exa with no Exa key is unconfigured even though Tavily had one.
        settings.lens.web_search.provider = crate::settings::WebSearchProvider::Exa;
        settings.lens.web_search.tavily_api_key = "tvly-abc".to_string();
        assert!(!web_search_configured(&settings), "Exa provider needs an Exa key");
        settings.lens.web_search.exa_api_key = "exa-key".to_string();
        assert!(web_search_configured(&settings), "non-empty Exa key is configured");
    }

    #[test]
    fn into_config_includes_web_search_only_when_configured() {
        let configured = assembly_with_tavily_key("tvly-abc");
        let unconfigured = assembly_with("chat", "Chat", "m1");

        let names = |assembly: &TurnAssembly| -> Vec<String> {
            let state = build_app_state(assembly.settings.clone());
            let cfg = assembly.into_config(
                &state,
                "c".to_string(),
                "r".to_string(),
                "msg".to_string(),
                1,
                Vec::new(),
                /* plan_mode */ false,
            );
            cfg.tools.iter().map(|t| t.name.clone()).collect()
        };

        let with = names(&configured);
        assert!(
            with.iter().any(|n| n == "web_search"),
            "web_search must be exposed when a provider key is configured: {with:?}"
        );

        let without = names(&unconfigured);
        assert!(
            !without.iter().any(|n| n == "web_search"),
            "web_search must NOT be exposed without a provider key: {without:?}"
        );
    }

    #[test]
    fn web_search_survives_plan_mode_filter() {
        // web_search is read-only, so a plan-mode turn (read-only filter applied)
        // must still expose it when a key is configured.
        let assembly = assembly_with_tavily_key("tvly-abc");
        let state = build_app_state(assembly.settings.clone());
        let cfg = assembly.into_config(
            &state,
            "c".to_string(),
            "r".to_string(),
            "msg".to_string(),
            1,
            Vec::new(),
            /* plan_mode */ true,
        );
        let names: Vec<String> = cfg.tools.iter().map(|t| t.name.clone()).collect();
        assert!(
            names.iter().any(|n| n == "web_search"),
            "web_search is read-only and must survive plan mode: {names:?}"
        );
        // is_read_only_tool() is the mechanism — assert it directly too.
        assert!(
            native_web_search_tool().is_read_only_tool(),
            "web_search must report read-only"
        );
        // Mutating tools are still dropped in plan mode.
        for blocked in ["write", "edit", "bash"] {
            assert!(!names.iter().any(|n| n == blocked), "plan must drop {blocked}: {names:?}");
        }
    }
}
