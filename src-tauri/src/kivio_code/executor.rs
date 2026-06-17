//! `CliToolExecutor` — a `ToolExecutor` for the headless `kivio-code` agent.
//!
//! Dispatches ONLY the core coding tools by `openai_tool_name()` / `name`,
//! calling the pure `native_tools::` functions directly (they need no
//! `AppHandle`) against a `NativeToolWorkspace::project` rooted at the CLI's cwd.
//! A PROJECT workspace makes RELATIVE paths resolve against the project root
//! (the standard coding-agent convention) — a `global` workspace would instead
//! join relative paths to the user HOME, so `.agent/AGENTS.md` would land in
//! `~/.agent/`, not the project. Absolute / `~` paths still go anywhere.
//! Results are wrapped into `McpToolCallResult` via the same registry helpers
//! the Tauri path uses, so model-facing tool output is byte-identical.
//!
//! Tools wired this phase: read_file, write_file, edit_file, list_dir,
//! glob_files, search_files, run_command (bash), stat_path*, web_fetch.
//! (*stat_path has no native tool builder yet; it is dispatched to list_dir-less
//! metadata via the file tools only if a builder is later added. For now the
//! exposed set excludes it — see `mod.rs::core_tool_definitions`.)

use reqwest::Client;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

use crate::chat::agent::execute::{ToolExecutionContext, ToolExecutor, ToolExecutorFuture};
use crate::kivio_code::mcp_setup;
use crate::mcp::native_registry::text_tool_result;
use crate::mcp::registry::{
    effective_skill_script_timeout_ms, file_mutation_tool_result, read_file_tool_result,
};
use crate::mcp::types::McpToolCallResult;
use crate::mcp::ChatToolDefinition;
use crate::native_tools::{
    edit_file, glob_files, list_dir, read_file, search_files, web_fetch, write_file,
    NativeToolWorkspace,
};
use crate::settings::ChatToolsConfig;
use crate::skills::{self, SkillRegistry};
use crate::state::AppState;

/// Build a PROJECT-rooted tool workspace from the CLI's working directory, so
/// RELATIVE tool paths (e.g. `.agent/AGENTS.md`, `src/x.rs`) resolve against the
/// project root rather than the user HOME (which is what `global` would do).
/// `project_id` is a stable synthetic id; `project_name` is the cwd's directory
/// name (fallback "project"); the root is the absolute cwd.
fn project_workspace_for_cwd(cwd: &Path) -> NativeToolWorkspace {
    let project_name = cwd
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "project".to_string());
    NativeToolWorkspace::project(
        "kivio-code-cli".to_string(),
        project_name,
        Some(cwd.to_string_lossy().into_owned()),
    )
}

pub struct CliToolExecutor {
    workspace: NativeToolWorkspace,
    http: Client,
    /// Default bash timeout (ms) when the model omits one.
    default_timeout_ms: u64,
    /// Headless app state, used to dispatch MCP tool calls (the MCP manager
    /// lives here). Held so the executor can route unknown tools through
    /// [`mcp_setup::dispatch_mcp_tool`] before erroring.
    state: Arc<AppState>,
    /// Pre-built skill registry (discovered once when the `TurnAssembly` was
    /// resolved). Skill tools (`skill_activate` / `skill_read_file` /
    /// `skill_run_script`) resolve their `SkillRecord` from here, so skill
    /// dispatch never needs an `AppHandle` (the GUI's `registry_for` does).
    skill_registry: SkillRegistry,
    /// Effective chat-tools config: supplies the skill-enabled gate, the
    /// `skill_run_script` interpreter allowlist, and the base tool timeout.
    chat_tools: ChatToolsConfig,
}

impl CliToolExecutor {
    pub fn new(
        cwd: &Path,
        http: Client,
        default_timeout_ms: u64,
        state: Arc<AppState>,
        skill_registry: SkillRegistry,
        chat_tools: ChatToolsConfig,
    ) -> Self {
        Self {
            workspace: project_workspace_for_cwd(cwd),
            http,
            default_timeout_ms,
            state,
            skill_registry,
            chat_tools,
        }
    }

    /// Resolve a tool call to an `McpToolCallResult`, mapping unknown tools to a
    /// loop-friendly error string (the loop encodes it as a tool error).
    async fn dispatch(
        &self,
        tool_name: &str,
        arguments: Value,
    ) -> Result<McpToolCallResult, String> {
        match tool_name {
            "read_file" | "read" => {
                read_file_tool_result(read_file(&self.workspace, &arguments)?)
            }
            "write_file" | "write" => {
                let result = write_file(&self.workspace, &arguments)?;
                file_mutation_tool_result(result)
            }
            "edit_file" | "edit" => {
                let result = edit_file(&self.workspace, &arguments)?;
                file_mutation_tool_result(result)
            }
            "list_dir" | "ls" => Ok(text_tool_result(list_dir(&self.workspace, &arguments)?)),
            "glob_files" | "find" => {
                Ok(text_tool_result(glob_files(&self.workspace, &arguments)?))
            }
            "search_files" | "grep" => {
                Ok(text_tool_result(search_files(&self.workspace, &arguments)?))
            }
            "run_command" | "bash" => Ok(text_tool_result(
                crate::native_tools::run_command(
                    &self.workspace,
                    self.default_timeout_ms,
                    &arguments,
                )
                .await?,
            )),
            "web_fetch" => Ok(text_tool_result(web_fetch(&self.http, &arguments).await?)),
            "web_search" => self.dispatch_web_search(&arguments).await,
            other => Err(format!(
                "kivio-code does not support tool '{other}' in print mode (core tools only)."
            )),
        }
    }

    /// Dispatch a `web_search` call. Mirrors `mcp::native_registry::call_web_search`:
    /// reads the `query` arg (error if empty — no network call), derives the retry
    /// count from settings, runs the configured Lens web-search provider, and returns
    /// the formatted web context. The tool is only ever advertised when a provider
    /// key is configured (see `mod.rs::web_search_configured`), so this assumes a key.
    async fn dispatch_web_search(
        &self,
        arguments: &Value,
    ) -> Result<McpToolCallResult, String> {
        let query = arguments
            .get("query")
            .and_then(|query| query.as_str())
            .unwrap_or_default()
            .trim()
            .to_string();
        if query.is_empty() {
            return Err("web_search query is empty".to_string());
        }
        let settings = self.state.settings_read().clone();
        let retry_attempts = if settings.retry_enabled {
            settings.retry_attempts as usize
        } else {
            1
        };
        let results = crate::web_search::search_web(
            &self.state,
            &settings.lens.web_search,
            &query,
            retry_attempts,
        )
        .await?;
        Ok(text_tool_result(crate::web_search::format_web_context(
            &results,
        )))
    }
}

impl ToolExecutor for CliToolExecutor {
    fn call<'a>(
        &'a self,
        _ctx: &'a ToolExecutionContext<'a>,
        tool: &'a ChatToolDefinition,
        arguments: Value,
        skill_cache: Option<&'a mut skills::SkillRunCache>,
    ) -> ToolExecutorFuture<'a> {
        let tool_name = tool.openai_tool_name();
        let fallback = tool.name.clone();
        Box::pin(async move {
            // Skill tools are dispatched here (not in `dispatch`) because they
            // need the mutable `skill_cache` to record activation / re-permit
            // tools — mirroring `mcp::registry::call_skill_tool`.
            if tool.source == "skill" {
                return self
                    .dispatch_skill(tool, arguments, skill_cache)
                    .await;
            }

            // Prefer the openai_tool_name (what the model called), fall back to
            // the raw name for native tools where they coincide.
            let result = self.dispatch(&tool_name, arguments.clone()).await;
            match result {
                Ok(output) => Ok(output),
                Err(err) if tool_name != fallback => {
                    // Retry under the raw name in case of an alias mismatch.
                    match self.dispatch(&fallback, arguments.clone()).await {
                        Ok(output) => Ok(output),
                        Err(_) => self.dispatch_mcp_or_error(tool, &arguments, err).await,
                    }
                }
                Err(err) => self.dispatch_mcp_or_error(tool, &arguments, err).await,
            }
        })
    }
}

impl CliToolExecutor {
    /// Dispatch a skill tool (`skill_activate` / `skill_read_file` /
    /// `skill_run_script`). Mirrors `mcp::registry::call_skill_tool`, but
    /// resolves the `SkillRecord` from the pre-built `skill_registry` (so no
    /// `AppHandle` is required) and uses the `skill_cache` exactly as the GUI
    /// does: `skill_activate` records the activated skill's `allowed_tools` so
    /// the loop re-permits them on the next planning round (mid-run activation),
    /// and read/activate results are de-duplicated through the cache when present.
    async fn dispatch_skill(
        &self,
        tool: &ChatToolDefinition,
        arguments: Value,
        skill_cache: Option<&mut skills::SkillRunCache>,
    ) -> Result<McpToolCallResult, String> {
        let skill_name = skills::extract_skill_name(&arguments)?;

        // Resolve the SkillRecord from the run's pre-built registry. Clone it so
        // the immutable borrow on the registry is dropped before we take a
        // mutable borrow of the cache for activate / read dispatch.
        let record = skills::lookup_skill(&self.skill_registry, &skill_name)
            .cloned()
            .ok_or_else(|| format!("Skill not found: {skill_name}"))?;

        if !crate::settings::is_skill_enabled(&self.chat_tools, &record.meta.id) {
            return Err(format!("Skill is disabled in Settings: {skill_name}"));
        }

        let mut skill_cache = skill_cache;
        // Assistant skill allow-list hard gate (defense in depth), as in the GUI.
        if let Some(cache) = skill_cache.as_deref() {
            if !cache.skill_id_allowed(&record.meta.id) {
                return Err(format!(
                    "Skill is not allowed for the active assistant: {skill_name}"
                ));
            }
        }

        let content = match tool.name.as_str() {
            "skill_activate" => {
                if let Some(cache) = skill_cache.as_deref_mut() {
                    // T3: a model-activated skill narrows the tool set on later
                    // rounds — record its allowed_tools before activating.
                    cache.record_activated_allowed_tools(&record.allowed_tools);
                    cache.activate_with_cache(&record)
                } else {
                    skills::activate_skill(&record)
                }
            }
            "skill_read_file" => {
                let relative_path = skills::extract_relative_path(&arguments)?;
                if let Some(cache) = skill_cache.as_deref_mut() {
                    cache.read_file_with_cache(&record, &relative_path)?
                } else {
                    skills::read_skill_file(&record, &relative_path)?
                }
            }
            "skill_run_script" => {
                let relative_path = skills::extract_relative_path(&arguments)?;
                let args = skills::extract_script_args(&arguments);
                let timeout_ms = effective_skill_script_timeout_ms(
                    self.chat_tools.tool_timeout_ms,
                    arguments.get("timeout_ms").and_then(|value| value.as_u64()),
                );
                skills::run_skill_script(
                    &record,
                    &relative_path,
                    &args,
                    timeout_ms,
                    &self.chat_tools.skill_script_allowlist,
                )
                .await?
            }
            other => return Err(format!("Unknown skill tool: {other}")),
        };

        Ok(text_tool_result(content))
    }

    /// Final fallthrough for a tool the core set did not handle: try the MCP
    /// dispatch seam first (an MCP server may own this tool), and only return
    /// the original unknown-tool error if MCP did not handle it. The stub MCP
    /// dispatch always errs, so today this is equivalent to returning `err`.
    async fn dispatch_mcp_or_error(
        &self,
        tool: &ChatToolDefinition,
        arguments: &Value,
        err: String,
    ) -> Result<McpToolCallResult, String> {
        match mcp_setup::dispatch_mcp_tool(&self.state, tool, arguments).await {
            Ok(output) => Ok(output),
            Err(_) => Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str) -> ChatToolDefinition {
        ChatToolDefinition {
            id: format!("native__{name}"),
            name: name.to_string(),
            description: String::new(),
            source: "native".to_string(),
            server_id: None,
            server_name: Some("Kivio".to_string()),
            input_schema: serde_json::json!({}),
            sensitive: false,
            annotations: None,
            output_schema: None,
        }
    }

    fn ctx() -> ToolExecutionContext<'static> {
        ToolExecutionContext {
            conversation_id: "kivio-code",
            run_id: "run",
            message_id: "msg",
            generation: 1,
            round: 1,
            depth: 0,
            tool_conversation_id: "kivio-code",
            tool_call_id: "call",
        }
    }

    fn temp_workspace() -> (std::path::PathBuf, CliToolExecutor) {
        let dir = std::env::temp_dir().join(format!("kivio-code-exec-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let settings = crate::settings::Settings::default();
        let chat_tools = settings.chat_tools.clone();
        let state = Arc::new(AppState::new_headless(settings, dir.join("usage")));
        let executor = CliToolExecutor::new(
            &dir,
            reqwest::Client::new(),
            120_000,
            state,
            SkillRegistry::default(),
            chat_tools,
        );
        (dir, executor)
    }

    #[tokio::test]
    async fn read_file_dispatch_returns_numbered_content() {
        let (dir, executor) = temp_workspace();
        let file = dir.join("hello.txt");
        std::fs::write(&file, "line one\nline two\n").expect("write file");

        let result = executor
            .call(
                &ctx(),
                &tool("read_file"),
                serde_json::json!({ "path": file.to_string_lossy() }),
                None,
            )
            .await
            .expect("read_file ok");

        assert!(!result.is_error);
        assert!(result.content.contains("line one"));
        assert!(result.content.contains("line two"));
        // cat -n style line numbers present.
        assert!(result.content.contains('1') && result.content.contains('2'));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn list_dir_dispatch_lists_entries() {
        let (dir, executor) = temp_workspace();
        std::fs::write(dir.join("a.txt"), "a").expect("write a");
        std::fs::create_dir(dir.join("sub")).expect("mkdir sub");

        let result = executor
            .call(
                &ctx(),
                &tool("list_dir"),
                serde_json::json!({ "path": dir.to_string_lossy(), "include_hidden": false }),
                None,
            )
            .await
            .expect("list_dir ok");

        assert!(!result.is_error);
        assert!(result.content.contains("a.txt"));
        assert!(result.content.contains("sub"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn write_file_dispatch_creates_file() {
        let (dir, executor) = temp_workspace();
        let file = dir.join("out.txt");

        let result = executor
            .call(
                &ctx(),
                &tool("write_file"),
                serde_json::json!({ "path": file.to_string_lossy(), "content": "written" }),
                None,
            )
            .await
            .expect("write_file ok");

        assert!(!result.is_error);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "written");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn write_file_relative_path_lands_under_project_not_home() {
        // Regression guard: a RELATIVE write path must resolve against the CLI's
        // project cwd, NOT the user HOME. A `global` workspace joins relative
        // paths to HOME (so `.agent/AGENTS.md` would land in `~/.agent/`); the
        // project workspace must keep it inside the temp project dir.
        let (dir, executor) = temp_workspace();

        let result = executor
            .call(
                &ctx(),
                &tool("write_file"),
                serde_json::json!({ "path": ".agent/AGENTS.md", "content": "project-rooted" }),
                None,
            )
            .await
            .expect("write_file ok");
        assert!(!result.is_error);

        // The file exists under the project cwd, and NOT under HOME.
        let in_project = dir.join(".agent").join("AGENTS.md");
        assert_eq!(
            std::fs::read_to_string(&in_project).unwrap(),
            "project-rooted",
            "relative write must land under the project cwd"
        );
        // The HOME-rooted location a `global` workspace would have used must not
        // hold this content (the temp project dir lives outside HOME).
        let in_home = crate::native_tools::user_home_dir()
            .unwrap()
            .join(".agent")
            .join("AGENTS.md");
        let home_has_our_content = std::fs::read_to_string(&in_home)
            .map(|content| content == "project-rooted")
            .unwrap_or(false);
        assert!(
            !home_has_our_content || in_home == in_project,
            "relative write must NOT land under the user HOME"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn relative_read_and_list_resolve_against_project_cwd() {
        let (dir, executor) = temp_workspace();
        std::fs::create_dir_all(dir.join("sub")).expect("mkdir sub");
        std::fs::write(dir.join("sub").join("x.txt"), "relative-read").expect("write x");

        // Relative read resolves against the project cwd.
        let read = executor
            .call(
                &ctx(),
                &tool("read_file"),
                serde_json::json!({ "path": "sub/x.txt" }),
                None,
            )
            .await
            .expect("read_file ok");
        assert!(!read.is_error);
        assert!(read.content.contains("relative-read"));

        // Relative list ("." = project root) resolves against the project cwd.
        let list = executor
            .call(
                &ctx(),
                &tool("list_dir"),
                serde_json::json!({ "path": ".", "include_hidden": false }),
                None,
            )
            .await
            .expect("list_dir ok");
        assert!(!list.is_error);
        assert!(list.content.contains("sub"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn absolute_path_still_writes_and_reads_at_that_location() {
        // An absolute path must bypass project-root confinement (the standard
        // convention: absolute / ~ may go anywhere), so a write to an absolute
        // path outside the project cwd still lands there.
        let (dir, executor) = temp_workspace();
        let other = std::env::temp_dir().join(format!("kivio-code-abs-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&other).expect("create other dir");
        let abs_file = other.join("absolute.txt");

        let write = executor
            .call(
                &ctx(),
                &tool("write_file"),
                serde_json::json!({ "path": abs_file.to_string_lossy(), "content": "absolute" }),
                None,
            )
            .await
            .expect("write_file ok");
        assert!(!write.is_error);
        assert_eq!(std::fs::read_to_string(&abs_file).unwrap(), "absolute");

        let read = executor
            .call(
                &ctx(),
                &tool("read_file"),
                serde_json::json!({ "path": abs_file.to_string_lossy() }),
                None,
            )
            .await
            .expect("read_file ok");
        assert!(!read.is_error);
        assert!(read.content.contains("absolute"));

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&other);
    }

    #[tokio::test]
    async fn unknown_tool_errors() {
        let (dir, executor) = temp_workspace();
        let result = executor
            .call(
                &ctx(),
                &tool("run_python"),
                serde_json::json!({ "code": "print(1)" }),
                None,
            )
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("does not support tool"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn web_search_empty_query_errors_without_network() {
        // An empty/whitespace query must short-circuit with an error BEFORE any
        // network call — this test makes no real request.
        let (dir, executor) = temp_workspace();

        for empty in [serde_json::json!({}), serde_json::json!({ "query": "   " })] {
            let result = executor
                .call(&ctx(), &tool("web_search"), empty, None)
                .await;
            assert!(result.is_err(), "empty query must error");
            assert!(
                result.unwrap_err().contains("query is empty"),
                "error must name the empty query"
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- Skill-tool dispatch ------------------------------------------------

    fn skill_tool(name: &str) -> ChatToolDefinition {
        crate::mcp::types::native_skill_tools()
            .into_iter()
            .find(|tool| tool.name == name)
            .expect("skill tool exists")
    }

    /// Write a minimal valid skill with a `references/guide.md` resource, build a
    /// registry from it (mirroring `TurnAssembly`), and construct an executor
    /// holding that registry + the matching chat-tools config (scan path set so
    /// the skill is discoverable and enabled).
    fn skill_workspace() -> (std::path::PathBuf, CliToolExecutor) {
        let dir = std::env::temp_dir().join(format!("kivio-code-skill-{}", uuid::Uuid::new_v4()));
        let skill_dir = dir.join("demo-skill");
        std::fs::create_dir_all(skill_dir.join("references")).expect("create skill dir");
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: demo-skill\ndescription: A demo skill for tests.\nallowed-tools: read web_fetch\n---\n\n# demo-skill\nDo the thing.\n",
        )
        .expect("write SKILL.md");
        std::fs::write(
            skill_dir.join("references").join("guide.md"),
            "GUIDE CONTENTS",
        )
        .expect("write guide");

        let mut settings = crate::settings::Settings::default();
        settings.chat_tools.skill_scan_paths = vec![dir.to_string_lossy().into_owned()];
        let chat_tools = settings.chat_tools.clone();
        let registry = crate::kivio_code::skill_setup::build_skill_registry(
            &settings,
            std::path::Path::new("/tmp"),
        );
        assert!(
            !registry.records.is_empty(),
            "precondition: demo-skill discovered"
        );

        let state = Arc::new(AppState::new_headless(settings, dir.join("usage")));
        let executor = CliToolExecutor::new(
            &dir,
            reqwest::Client::new(),
            120_000,
            state,
            registry,
            chat_tools,
        );
        (dir, executor)
    }

    #[tokio::test]
    async fn skill_activate_records_allowed_tools_and_returns_instructions() {
        let (dir, executor) = skill_workspace();
        let mut cache = skills::SkillRunCache::default();

        let result = executor
            .call(
                &ctx(),
                &skill_tool("skill_activate"),
                serde_json::json!({ "name": "demo-skill" }),
                Some(&mut cache),
            )
            .await
            .expect("skill_activate ok");

        assert!(!result.is_error);
        // Activation returns the skill instructions / body.
        assert!(result.content.contains("Do the thing"));
        // Mid-run activation re-permits the skill's tools through the cache so
        // the loop narrows the tool set on the next round (T3): the skill's
        // declared allowed-tools must now be recorded in the cache.
        assert!(
            cache.activated_allowed_tools().iter().any(|t| t == "read"),
            "skill's allowed_tools should be recorded for loop re-permit"
        );
        // A second activation is recognized as already active — proving the
        // cache was mutated (activation state persisted).
        let again = executor
            .call(
                &ctx(),
                &skill_tool("skill_activate"),
                serde_json::json!({ "name": "demo-skill" }),
                Some(&mut cache),
            )
            .await
            .expect("re-activate ok");
        assert!(again.content.contains("already active"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn skill_activate_unknown_skill_errors() {
        let (dir, executor) = skill_workspace();
        let mut cache = skills::SkillRunCache::default();

        let result = executor
            .call(
                &ctx(),
                &skill_tool("skill_activate"),
                serde_json::json!({ "name": "no-such-skill" }),
                Some(&mut cache),
            )
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Skill not found"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn skill_read_file_reads_skill_resource() {
        let (dir, executor) = skill_workspace();
        let mut cache = skills::SkillRunCache::default();

        let result = executor
            .call(
                &ctx(),
                &skill_tool("skill_read_file"),
                serde_json::json!({ "name": "demo-skill", "relative_path": "references/guide.md" }),
                Some(&mut cache),
            )
            .await
            .expect("skill_read_file ok");

        assert!(!result.is_error);
        assert!(result.content.contains("GUIDE CONTENTS"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
