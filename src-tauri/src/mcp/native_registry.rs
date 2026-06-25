//! Static registry for Kivio's builtin (native-source) tools.
//!
//! This table is the single source of truth that replaces the previously
//! drifting hardcoded lists:
//! - exposure if-chain in `types.rs::list_native_builtin_tool_defs`
//! - dispatch match in `registry.rs::call_native_tool`
//! - `BUILTIN_NAMES` in `chat/agent/prepare.rs::disabled_builtin_tool_feedback`
//! - approval bypass list in `chat/agent/prepare.rs::builtin_tool_bypasses_approval`
//! - parallel whitelist in `chat/agent/rounds.rs::tool_call_parallel_eligible`
//! - native read-only arm in `types.rs::ChatToolDefinition::is_read_only_tool`
//!
//! Contract notes (see `.trellis/spec/backend/agent-runtime.md` and
//! `.trellis/spec/backend/file-tools.md`):
//! - The `parallel_safe` set is intentionally narrow: web_search/web_fetch/
//!   read plus the read-side project tools (ls/grep/find), and only when
//!   approval-free. Do not widen or narrow it here without a spec change.
//!   memory_read is read-only and approval-free but deliberately NOT
//!   parallel-safe. `agent` is also parallel-safe (multi-agent fan-out): each
//!   spawn runs isolated and is capped by the SubAgentManager semaphore
//!   (default `DEFAULT_SUB_AGENT_CONCURRENCY` = 12, user-configurable). Each
//!   `agent` call blocks until its sub-agent finishes and returns the result
//!   inline; parallelism comes from the model emitting several `agent` calls in
//!   one round.
//! - Table order is the model-facing tool list order; keep it stable.

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;
use tauri::AppHandle;

use crate::native_tools::{FileMutationResult, NativeToolWorkspace};
use crate::settings::{ChatNativeToolsConfig, Settings};
use crate::state::AppState;

use super::registry::NativeToolContext;
use super::types::{
    native_bash_output_tool, native_edit_file_tool, native_glob_files_tool,
    native_kill_background_tool, native_list_background_tool, native_list_dir_tool,
    native_knowledge_search_tool, native_memory_modify_tool, native_memory_read_tool,
    native_memory_search_tool, native_read_file_tool, native_run_command_tool,
    native_run_python_tool, native_save_assistant_tool, native_search_files_tool,
    native_web_fetch_tool, native_web_search_tool, native_write_file_tool, ChatToolDefinition,
    McpToolCallResult,
};

/// Gate signature mirrors `list_native_builtin_tool_defs(native,
/// web_search_configured, memory_enabled)` so exposure stays bit-identical.
pub type NativeToolEnabledFn = fn(&ChatNativeToolsConfig, bool, bool) -> bool;

pub type NativeToolFuture<'a> =
    Pin<Box<dyn Future<Output = Result<McpToolCallResult, String>> + Send + 'a>>;

/// Full-context async call. `workspace` is already resolved by
/// `call_native_tool` before dispatch, matching the legacy behavior where
/// every native tool resolved the workspace at entry.
pub struct NativeCallCtx<'a> {
    pub app: &'a AppHandle,
    pub state: &'a AppState,
    pub settings: &'a Settings,
    pub workspace: &'a NativeToolWorkspace,
    pub arguments: &'a Value,
    pub native_ctx: Option<&'a NativeToolContext>,
}

/// How `call_native_tool` dispatches a registry entry. The blocking-vs-sync
/// split is kept explicit here so "which tools run on the blocking thread
/// pool" stays auditable in one place (see `run_blocking_file_mutation`).
pub enum NativeToolCall {
    /// Synchronous, workspace-scoped, plain-text result.
    SyncText(fn(&NativeToolWorkspace, &Value) -> Result<String, String>),
    /// Synchronous, workspace-scoped, custom tool result.
    SyncResult(fn(&NativeToolWorkspace, &Value) -> Result<McpToolCallResult, String>),
    /// spawn_blocking, plain-text result (path mutations with lock waits).
    BlockingText(fn(&NativeToolWorkspace, &Value) -> Result<String, String>),
    /// spawn_blocking, structured `FileMutationResult` (write_file/edit_file).
    BlockingMutation(fn(&NativeToolWorkspace, &Value) -> Result<FileMutationResult, String>),
    /// Full-context async call (web/memory/shell/python).
    Async(for<'a> fn(NativeCallCtx<'a>) -> NativeToolFuture<'a>),
    /// Conversation-scoped call (todo tools): runs before workspace
    /// resolution because it only needs the conversation id, matching the
    /// legacy `RegistryToolExecutor` special case which never resolved a
    /// workspace for todo tools.
    Conversation(fn(&AppHandle, &str, &str, Value) -> Result<McpToolCallResult, String>),
    /// Host-mediated tool (ask_user): intercepted in
    /// `chat/agent/execute.rs::execute_ask_user_call` and must never reach
    /// the registry dispatcher.
    HostMediated,
    /// Sub-agent spawn tool (`agent`): dispatched before workspace resolution
    /// (it manages agents, not files) with the parent run context from
    /// `NativeToolContext` (depth, run_id, generation, parent conversation/
    /// tool-call id).
    SubAgent(for<'a> fn(crate::chat::sub_agent::SubAgentCallCtx<'a>) -> NativeToolFuture<'a>),
}

pub struct NativeToolEntry {
    pub name: &'static str,
    pub def: fn() -> ChatToolDefinition,
    /// Whether the tool is exposed in the model tool list for the given
    /// settings. todo/ask_user return false here because they are appended
    /// separately in `chat/commands.rs` (`append_agent_todo_tools` /
    /// `append_agent_ask_user_tools`), not via
    /// `list_native_builtin_tool_defs`.
    pub enabled: NativeToolEnabledFn,
    pub parallel_safe: bool,
    pub bypasses_approval: bool,
    pub read_only: bool,
    /// File/shell tools (read/write/edit/bash/grep/find/ls) gated by one-time
    /// per-conversation session consent. The flag lives on the entry so a rename
    /// or a newly-added tool can't silently bypass the consent gate.
    pub requires_session_consent: bool,
    pub call: NativeToolCall,
}

/// Declaration order is the model-facing exposure order (the legacy push
/// order of `list_native_builtin_tool_defs`), followed by the
/// conversation-level tools that are appended elsewhere.
pub static NATIVE_TOOLS: &[NativeToolEntry] = &[
    NativeToolEntry {
        name: "web_search",
        def: native_web_search_tool,
        enabled: |native, web_search_configured, _| native.web_search && web_search_configured,
        parallel_safe: true,
        bypasses_approval: false,
        read_only: true,
        requires_session_consent: false,
        call: NativeToolCall::Async(call_web_search),
    },
    NativeToolEntry {
        name: "web_fetch",
        def: native_web_fetch_tool,
        enabled: |native, _, _| native.web_fetch,
        parallel_safe: true,
        bypasses_approval: false,
        read_only: true,
        requires_session_consent: false,
        call: NativeToolCall::Async(call_web_fetch),
    },
    NativeToolEntry {
        name: "knowledge_search",
        def: native_knowledge_search_tool,
        enabled: |native, _, _| native.knowledge_search,
        parallel_safe: true,
        bypasses_approval: false,
        read_only: true,
        requires_session_consent: false,
        call: NativeToolCall::Async(call_knowledge_search),
    },
    NativeToolEntry {
        name: "read",
        def: native_read_file_tool,
        enabled: |native, _, _| native.read_file,
        parallel_safe: true,
        bypasses_approval: false,
        read_only: true,
        requires_session_consent: true,
        call: NativeToolCall::SyncResult(call_read_file),
    },
    NativeToolEntry {
        name: "ls",
        def: native_list_dir_tool,
        enabled: |native, _, _| native.read_file,
        parallel_safe: true,
        bypasses_approval: false,
        read_only: true,
        requires_session_consent: true,
        call: NativeToolCall::SyncText(crate::native_tools::list_dir),
    },
    NativeToolEntry {
        name: "grep",
        def: native_search_files_tool,
        enabled: |native, _, _| native.read_file,
        parallel_safe: true,
        bypasses_approval: false,
        read_only: true,
        requires_session_consent: true,
        call: NativeToolCall::SyncText(crate::native_tools::search_files),
    },
    NativeToolEntry {
        name: "find",
        def: native_glob_files_tool,
        enabled: |native, _, _| native.read_file,
        parallel_safe: true,
        bypasses_approval: false,
        read_only: true,
        requires_session_consent: true,
        call: NativeToolCall::SyncText(crate::native_tools::glob_files),
    },
    NativeToolEntry {
        name: "write",
        def: native_write_file_tool,
        enabled: |native, _, _| native.write_file,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        requires_session_consent: true,
        call: NativeToolCall::BlockingMutation(crate::native_tools::write_file),
    },
    NativeToolEntry {
        name: "edit",
        def: native_edit_file_tool,
        enabled: |native, _, _| native.edit_file,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        requires_session_consent: true,
        call: NativeToolCall::BlockingMutation(crate::native_tools::edit_file),
    },
    NativeToolEntry {
        name: "bash",
        def: native_run_command_tool,
        enabled: |native, _, _| native.run_command,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        requires_session_consent: true,
        call: NativeToolCall::Async(call_run_command),
    },
    // Background-command observability tools (PR2). Gated by the same
    // `run_command` toggle + session consent as `bash` (they expose host-shell
    // job state/control). bash_output/list_background are read-only and
    // parallel-safe (pure registry/log reads); kill_background is a control
    // action and is neither. None bypass approval — they ride bash's consent.
    NativeToolEntry {
        name: "bash_output",
        def: native_bash_output_tool,
        enabled: |native, _, _| native.run_command,
        parallel_safe: true,
        bypasses_approval: false,
        read_only: true,
        requires_session_consent: true,
        call: NativeToolCall::Async(call_bash_output),
    },
    NativeToolEntry {
        name: "list_background",
        def: native_list_background_tool,
        enabled: |native, _, _| native.run_command,
        parallel_safe: true,
        bypasses_approval: false,
        read_only: true,
        requires_session_consent: true,
        call: NativeToolCall::Async(call_list_background),
    },
    NativeToolEntry {
        name: "kill_background",
        def: native_kill_background_tool,
        enabled: |native, _, _| native.run_command,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        requires_session_consent: true,
        call: NativeToolCall::Async(call_kill_background),
    },
    NativeToolEntry {
        // 仅在「对话搭建专家」会话里手动 append(见 commands.rs);全局列表永不暴露。
        // call_native_tool 按名分发、调用期不复查 enabled,故手动 append 仍可执行。
        name: "save_assistant",
        def: native_save_assistant_tool,
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        requires_session_consent: false,
        call: NativeToolCall::Async(call_save_assistant),
    },
    NativeToolEntry {
        name: "run_python",
        def: native_run_python_tool,
        enabled: |native, _, _| native.run_python,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        requires_session_consent: false,
        call: NativeToolCall::Async(call_run_python),
    },
    // Unified file delivery is no longer a tool. Deliverables are surfaced by a
    // path-driven channel instead: `write_file` writing into (or `run_python`
    // producing artifacts in) the persistent per-conversation delivery directory
    // `~/Kivio/outputs/<conversation>/` automatically renders a file card. There
    // is no `deliver_file` tool / flag.
    NativeToolEntry {
        name: "memory_read",
        def: native_memory_read_tool,
        enabled: |_, _, memory_enabled| memory_enabled,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: true,
        requires_session_consent: false,
        call: NativeToolCall::Async(call_memory_read),
    },
    NativeToolEntry {
        name: "memory_modify",
        def: native_memory_modify_tool,
        enabled: |_, _, memory_enabled| memory_enabled,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: false,
        requires_session_consent: false,
        call: NativeToolCall::Async(call_memory_modify),
    },
    NativeToolEntry {
        name: "memory_search",
        def: native_memory_search_tool,
        enabled: |_, _, memory_enabled| memory_enabled,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: true,
        requires_session_consent: false,
        call: NativeToolCall::Async(call_memory_search),
    },
    // Conversation-level tools below are appended in chat/commands.rs and
    // never exposed via list_native_builtin_tool_defs (enabled = false).
    NativeToolEntry {
        name: crate::chat::todo::TODO_WRITE_TOOL_NAME,
        def: crate::chat::todo::todo_write_tool,
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: false,
        requires_session_consent: false,
        call: NativeToolCall::Conversation(crate::chat::todo::handle_conversation_tool_call),
    },
    NativeToolEntry {
        name: crate::chat::todo::TODO_UPDATE_TOOL_NAME,
        def: crate::chat::todo::todo_update_tool,
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: false,
        requires_session_consent: false,
        call: NativeToolCall::Conversation(crate::chat::todo::handle_conversation_tool_call),
    },
    NativeToolEntry {
        name: crate::chat::ask_user::ASK_USER_TOOL_NAME,
        def: crate::chat::ask_user::ask_user_tool,
        enabled: |_, _, _| false,
        parallel_safe: false, // spec: ask_user is forced serial with batch flush
        bypasses_approval: true,
        read_only: false,
        requires_session_consent: false,
        call: NativeToolCall::HostMediated,
    },
    // Sub-agent spawn tool (P3). Appended in chat/commands.rs (via
    // `sub_agent::append_tool_definitions`) when the multi-agent toggle is on, so
    // enabled = false here. bypasses_approval = true: spawning sub-agents is
    // governed by depth + concurrency caps, not per-call approval prompts.
    NativeToolEntry {
        name: crate::chat::sub_agent::AGENT_TOOL_NAME,
        def: crate::chat::sub_agent::agent_tool,
        enabled: |_, _, _| false,
        // parallel_safe = true: each `agent` spawn runs in isolation (its own
        // synthetic conversation/generation/message history), bypasses approval,
        // and is capped by the SubAgentManager semaphore (default 12, user-
        // configurable). Concurrent fan-out is the core value of multi-agent: a
        // single round may dispatch several `agent` calls in parallel (scheduler
        // caps at MAX_PARALLEL_TOOL_CALLS_PER_ROUND = 12, semaphore at the
        // setting). Each call blocks until its sub-agent finishes and returns the
        // full result inline (Claude Code Task model).
        parallel_safe: true,
        bypasses_approval: true,
        read_only: false,
        requires_session_consent: false,
        call: NativeToolCall::SubAgent(crate::chat::sub_agent::dispatch_agent_spawn),
    },
];

pub fn find_entry(name: &str) -> Option<&'static NativeToolEntry> {
    NATIVE_TOOLS.iter().find(|entry| entry.name == name)
}

/// The native file/shell tools (Pi's 7) are gated by a single one-time
/// per-conversation **session consent** prompt — granting one authorizes
/// full-disk read/write and arbitrary command execution for the rest of that
/// conversation. Everything else (web/python/memory/todo/sub-agent/...) keeps
/// its own gating and is NOT behind this consent. Driven by the per-entry
/// `requires_session_consent` flag so a rename or new tool can't drift.
pub fn native_tool_requires_session_consent(name: &str) -> bool {
    find_entry(name).is_some_and(|entry| entry.requires_session_consent)
}

pub fn text_tool_result(content: String) -> McpToolCallResult {
    McpToolCallResult {
        content,
        is_error: false,
        raw: Value::Null,
        artifacts: Vec::new(),
        structured_content: None,
    }
}

fn call_read_file(
    workspace: &NativeToolWorkspace,
    arguments: &Value,
) -> Result<McpToolCallResult, String> {
    let result = crate::native_tools::read_file(workspace, arguments)?;
    super::registry::read_file_tool_result(result)
}

fn call_web_search(ctx: NativeCallCtx<'_>) -> NativeToolFuture<'_> {
    Box::pin(async move {
        let query = ctx
            .arguments
            .get("query")
            .and_then(|query| query.as_str())
            .unwrap_or_default()
            .trim()
            .to_string();
        if query.is_empty() {
            return Err("web_search query is empty".to_string());
        }
        let retry_attempts = if ctx.settings.retry_enabled {
            ctx.settings.retry_attempts as usize
        } else {
            1
        };
        let results = crate::web_search::search_web(
            ctx.state,
            &ctx.settings.lens.web_search,
            &query,
            retry_attempts,
        )
        .await?;
        let raw = serde_json::to_value(&results).unwrap_or(Value::Null);
        Ok(McpToolCallResult {
            content: crate::web_search::format_web_context(&results),
            is_error: false,
            raw,
            artifacts: Vec::new(),
            structured_content: None,
        })
    })
}

fn call_web_fetch(ctx: NativeCallCtx<'_>) -> NativeToolFuture<'_> {
    Box::pin(async move {
        let content = crate::native_tools::web_fetch(&ctx.state.http, ctx.arguments).await?;
        Ok(text_tool_result(content))
    })
}

/// Knowledge-base retrieval (RAG). Embeds the query with each target library's
/// own embedding model, runs a cosine search, merges the hits, and returns
/// passages tagged with `[n]` citation markers plus structured hits for the UI.
fn call_knowledge_search(ctx: NativeCallCtx<'_>) -> NativeToolFuture<'_> {
    Box::pin(async move {
        use crate::chat::knowledge_base as kb;
        use std::collections::BTreeMap;
        use std::cmp::Ordering;

        let query = ctx
            .arguments
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim()
            .to_string();
        if query.is_empty() {
            return Err("knowledge_search query is empty".to_string());
        }
        let top_k = ctx
            .arguments
            .get("top_k")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .filter(|n| *n > 0)
            .unwrap_or(5)
            .min(20);

        // Target libraries: explicit arg > conversation mount > all libraries.
        let mut kb_ids: Vec<String> = ctx
            .arguments
            .get("kb_ids")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        if kb_ids.is_empty() {
            if let Some(nc) = ctx.native_ctx {
                if let Ok(conv) = crate::chat::storage::load_conversation(ctx.app, &nc.conversation_id)
                {
                    kb_ids = conv.knowledge_base_ids.clone();
                }
            }
        }
        let libs = kb::load_libraries(ctx.app)?;
        if kb_ids.is_empty() {
            kb_ids = libs.iter().map(|l| l.id.clone()).collect();
        }
        if kb_ids.is_empty() {
            return Ok(text_tool_result(
                "No knowledge base is configured. Ask the user to create one and add documents."
                    .to_string(),
            ));
        }

        // Group by (provider, model) so each group is embedded with its own
        // model; scores (all cosine) are merged across groups.
        let mut groups: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
        for id in &kb_ids {
            if let Some(l) = libs.iter().find(|l| &l.id == id) {
                groups
                    .entry((l.embedding_provider_id.clone(), l.embedding_model.clone()))
                    .or_default()
                    .push(id.clone());
            }
        }
        let attempts = if ctx.settings.retry_enabled {
            ctx.settings.retry_attempts as usize
        } else {
            1
        };

        let mut all_hits: Vec<kb::ScoredChunk> = Vec::new();
        for ((provider_id, model), ids) in groups {
            let Some(provider) = ctx.settings.get_provider(&provider_id).cloned() else {
                continue;
            };
            let qvec =
                kb::embeddings::embed_query(ctx.state, &provider, &model, &query, attempts).await?;
            all_hits.extend(kb::search(ctx.app, &ids, &qvec, top_k)?);
        }
        all_hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
        all_hits.truncate(top_k);
        all_hits.retain(|h| h.score > 0.0);

        if all_hits.is_empty() {
            return Ok(text_tool_result(
                "No relevant passages found in the knowledge base.".to_string(),
            ));
        }

        let mut content = format!(
            "Found {} relevant passage(s). Cite each with its [n] marker.\n\n",
            all_hits.len()
        );
        let mut struct_hits = Vec::with_capacity(all_hits.len());
        for (i, h) in all_hits.iter().enumerate() {
            let n = i + 1;
            let src = match &h.chunk.heading_path {
                Some(hp) => format!("{} — {}", h.chunk.doc_name, hp),
                None => h.chunk.doc_name.clone(),
            };
            content.push_str(&format!("[{n}] {src}\n{}\n\n", h.chunk.text.trim()));
            struct_hits.push(serde_json::json!({
                "n": n,
                "kbId": h.kb_id,
                "docId": h.chunk.doc_id,
                "docName": h.chunk.doc_name,
                "headingPath": h.chunk.heading_path,
                "score": h.score,
                "text": h.chunk.text,
            }));
        }
        let payload = serde_json::json!({ "hits": struct_hits });
        Ok(McpToolCallResult {
            content,
            is_error: false,
            raw: payload.clone(),
            artifacts: Vec::new(),
            structured_content: Some(payload),
        })
    })
}


fn call_memory_read(ctx: NativeCallCtx<'_>) -> NativeToolFuture<'_> {
    Box::pin(async move {
        // Runtime second gate kept on purpose: the tool list is cached, so a
        // user can disable memory between listing and calling.
        if !ctx.settings.chat_memory.enabled {
            return Err("Chat memory is disabled in Settings".to_string());
        }
        crate::chat::memory::tool_read(ctx.app, ctx.arguments)
    })
}

fn call_memory_modify(ctx: NativeCallCtx<'_>) -> NativeToolFuture<'_> {
    Box::pin(async move {
        if !ctx.settings.chat_memory.enabled {
            return Err("Chat memory is disabled in Settings".to_string());
        }
        crate::chat::memory::tool_modify(ctx.app, ctx.arguments)
    })
}

fn call_memory_search(ctx: NativeCallCtx<'_>) -> NativeToolFuture<'_> {
    Box::pin(async move {
        if !ctx.settings.chat_memory.enabled {
            return Err("Chat memory is disabled in Settings".to_string());
        }
        crate::chat::memory::tool_search(ctx.app, ctx.arguments)
    })
}

fn call_run_command(ctx: NativeCallCtx<'_>) -> NativeToolFuture<'_> {
    Box::pin(async move {
        let content = crate::native_tools::run_command(
            ctx.workspace,
            ctx.settings.chat_tools.tool_timeout_ms,
            ctx.arguments,
            Some(ctx.state),
        )
        .await?;
        Ok(text_tool_result(content))
    })
}

fn call_bash_output(ctx: NativeCallCtx<'_>) -> NativeToolFuture<'_> {
    Box::pin(async move {
        let content = crate::native_tools::bash_output(ctx.state, ctx.arguments)?;
        Ok(text_tool_result(content))
    })
}

fn call_list_background(ctx: NativeCallCtx<'_>) -> NativeToolFuture<'_> {
    Box::pin(async move {
        let content = crate::native_tools::list_background(ctx.state, ctx.arguments)?;
        Ok(text_tool_result(content))
    })
}

fn call_kill_background(ctx: NativeCallCtx<'_>) -> NativeToolFuture<'_> {
    Box::pin(async move {
        let content = crate::native_tools::kill_background(ctx.state, ctx.arguments)?;
        Ok(text_tool_result(content))
    })
}

fn call_save_assistant(ctx: NativeCallCtx<'_>) -> NativeToolFuture<'_> {
    Box::pin(async move {
        let content = crate::chat::commands::create_assistant_via_builder(ctx.app, ctx.arguments)?;
        Ok(text_tool_result(content))
    })
}

fn call_run_python(ctx: NativeCallCtx<'_>) -> NativeToolFuture<'_> {
    Box::pin(async move {
        super::registry::run_python_via_pyodide(
            ctx.app,
            ctx.state,
            ctx.settings,
            ctx.workspace,
            ctx.arguments,
            ctx.native_ctx.cloned(),
        )
        .await
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPECTED_ORDER: &[&str] = &[
        "web_search",
        "web_fetch",
        "knowledge_search",
        "read",
        "ls",
        "grep",
        "find",
        "write",
        "edit",
        "bash",
        "bash_output",
        "list_background",
        "kill_background",
        "save_assistant",
        "run_python",
        "memory_read",
        "memory_modify",
        "memory_search",
        "todo_write",
        "todo_update",
        "ask_user",
        "agent",
    ];

    #[test]
    fn session_consent_set_is_exactly_the_seven_file_shell_tools() {
        let consent: Vec<&str> = NATIVE_TOOLS
            .iter()
            .filter(|entry| entry.requires_session_consent)
            .map(|entry| entry.name)
            .collect();
        assert_eq!(
            consent,
            [
                "read",
                "ls",
                "grep",
                "find",
                "write",
                "edit",
                "bash",
                "bash_output",
                "list_background",
                "kill_background"
            ],
            "session-consent set must be exactly Pi's 7 file/shell tools plus the \
             background-command observability trio (gated identically to bash); a \
             new file/shell tool MUST set requires_session_consent or it silently \
             bypasses the consent gate"
        );
        // The predicate agrees with the flag, and non-file tools are excluded.
        assert!(native_tool_requires_session_consent("bash"));
        assert!(!native_tool_requires_session_consent("web_search"));
        assert!(!native_tool_requires_session_consent("run_python"));
        assert!(!native_tool_requires_session_consent("memory_read"));
    }

    #[test]
    fn registry_order_and_names_match_legacy_exposure_order() {
        let names: Vec<&str> = NATIVE_TOOLS.iter().map(|entry| entry.name).collect();
        assert_eq!(names, EXPECTED_ORDER);
    }

    #[test]
    fn registry_defs_match_entry_names() {
        for entry in NATIVE_TOOLS {
            let def = (entry.def)();
            assert_eq!(def.name, entry.name, "def() name must equal entry name");
            assert_eq!(def.source, "native");
        }
    }

    #[test]
    fn parallel_safe_set_is_exactly_the_narrow_read_whitelist() {
        let parallel: Vec<&str> = NATIVE_TOOLS
            .iter()
            .filter(|entry| entry.parallel_safe)
            .map(|entry| entry.name)
            .collect();
        assert_eq!(
            parallel,
            [
                "web_search",
                "web_fetch",
                "knowledge_search",
                "read",
                "ls",
                "grep",
                "find",
                "bash_output",
                "list_background",
                "agent",
            ],
            "parallel-safe set is intentionally narrow per agent-runtime spec; \
             bash_output/list_background join it because they are pure read-only \
             registry/log reads; `agent` joins it because each spawn runs in \
             isolation (own conversation/generation/message history), bypasses \
             approval, and is capped by the SubAgentManager semaphore (default \
             12), making concurrent fan-out the core multi-agent value"
        );
    }

    #[test]
    fn approval_bypass_set_matches_legacy_list() {
        let bypass: Vec<&str> = NATIVE_TOOLS
            .iter()
            .filter(|entry| entry.bypasses_approval)
            .map(|entry| entry.name)
            .collect();
        assert_eq!(
            bypass,
            [
                "memory_read",
                "memory_modify",
                "memory_search",
                "todo_write",
                "todo_update",
                "ask_user",
                "agent",
            ]
        );
    }

    #[test]
    fn read_only_set_matches_legacy_is_read_only_tool_arm() {
        let read_only: Vec<&str> = NATIVE_TOOLS
            .iter()
            .filter(|entry| entry.read_only)
            .map(|entry| entry.name)
            .collect();
        assert_eq!(
            read_only,
            [
                "web_search",
                "web_fetch",
                "knowledge_search",
                "read",
                "ls",
                "grep",
                "find",
                "bash_output",
                "list_background",
                "memory_read",
                "memory_search",
            ],
            "memory_read/memory_search are read-only but deliberately not parallel-safe"
        );
    }

    #[test]
    fn conversation_tools_are_never_listed_via_builtin_exposure() {
        let native = crate::settings::ChatNativeToolsConfig {
            web_search: true,
            web_fetch: true,
            skill_runtime: true,
            read_file: true,
            write_file: true,
            edit_file: true,
            run_command: true,
            run_python: true,
            knowledge_search: true,
            workspace_roots: Vec::new(),
        };
        for entry in NATIVE_TOOLS {
            if matches!(
                entry.call,
                NativeToolCall::Conversation(_)
                    | NativeToolCall::HostMediated
                    | NativeToolCall::SubAgent(_)
            ) {
                assert!(
                    !(entry.enabled)(&native, true, true),
                    "{} must be appended via chat/commands.rs, not listed here",
                    entry.name
                );
            }
        }
    }

    /// Exposure-surface snapshot: for fixed settings combinations, the exact
    /// ordered tool-name list returned by `list_native_builtin_tool_defs`
    /// must stay frozen. This is the primary regression guard for the
    /// registry refactor.
    #[test]
    fn builtin_exposure_snapshot_per_settings_combination() {
        use crate::mcp::types::list_native_builtin_tool_defs;
        use crate::settings::ChatNativeToolsConfig;

        fn names(
            native: &ChatNativeToolsConfig,
            web_search_configured: bool,
            memory_enabled: bool,
        ) -> Vec<String> {
            list_native_builtin_tool_defs(native, web_search_configured, memory_enabled)
                .into_iter()
                .map(|tool| tool.name)
                .collect()
        }

        let off = ChatNativeToolsConfig {
            web_search: false,
            web_fetch: false,
            skill_runtime: false,
            read_file: false,
            write_file: false,
            edit_file: false,
            run_command: false,
            run_python: false,
            knowledge_search: false,
            workspace_roots: Vec::new(),
        };
        assert!(names(&off, true, false).is_empty());

        // web_search requires both the toggle and a configured provider key.
        let mut search_only = off.clone();
        search_only.web_search = true;
        assert!(names(&search_only, false, false).is_empty());
        assert_eq!(names(&search_only, true, false), ["web_search"]);

        // read_file gate exposes the whole read-side group, in order.
        let mut read_only = off.clone();
        read_only.read_file = true;
        assert_eq!(
            names(&read_only, false, false),
            ["read", "ls", "grep", "find"]
        );

        // write gate exposes the whole-file write tool only. Deliverables are a
        // path-driven channel (writing into ~/Kivio/outputs/<conv>/), not a tool.
        let mut write_only = off.clone();
        write_only.write_file = true;
        assert_eq!(names(&write_only, false, false), ["write"]);

        // memory gate is independent of native toggles.
        assert_eq!(
            names(&off, false, true),
            ["memory_read", "memory_modify", "memory_search"]
        );

        // Everything on: full ordered surface.
        let all = ChatNativeToolsConfig {
            web_search: true,
            web_fetch: true,
            skill_runtime: true,
            read_file: true,
            write_file: true,
            edit_file: true,
            run_command: true,
            run_python: true,
            knowledge_search: true,
            workspace_roots: Vec::new(),
        };
        assert_eq!(
            names(&all, true, true),
            [
                "web_search",
                "web_fetch",
                "knowledge_search",
                "read",
                "ls",
                "grep",
                "find",
                "write",
                "edit",
                "bash",
                "bash_output",
                "list_background",
                "kill_background",
                "run_python",
                "memory_read",
                "memory_modify",
                "memory_search",
            ]
        );
    }
}
