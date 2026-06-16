//! `CliToolExecutor` — a `ToolExecutor` for the headless `kivio-code` agent.
//!
//! Dispatches ONLY the core coding tools by `openai_tool_name()` / `name`,
//! calling the pure `native_tools::` functions directly (they need no
//! `AppHandle`) against a `NativeToolWorkspace::global` rooted at the CLI's cwd.
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

use crate::chat::agent::execute::{ToolExecutionContext, ToolExecutor, ToolExecutorFuture};
use crate::mcp::native_registry::text_tool_result;
use crate::mcp::registry::{file_mutation_tool_result, read_file_tool_result};
use crate::mcp::types::McpToolCallResult;
use crate::mcp::ChatToolDefinition;
use crate::native_tools::{
    edit_file, glob_files, list_dir, read_file, search_files, web_fetch, write_file,
    NativeToolWorkspace,
};
use crate::skills;

pub struct CliToolExecutor {
    workspace: NativeToolWorkspace,
    http: Client,
    /// Default bash timeout (ms) when the model omits one.
    default_timeout_ms: u64,
}

impl CliToolExecutor {
    pub fn new(cwd_roots: Vec<String>, http: Client, default_timeout_ms: u64) -> Self {
        Self {
            workspace: NativeToolWorkspace::global(&cwd_roots),
            http,
            default_timeout_ms,
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
            other => Err(format!(
                "kivio-code does not support tool '{other}' in print mode (core tools only)."
            )),
        }
    }
}

impl ToolExecutor for CliToolExecutor {
    fn call<'a>(
        &'a self,
        _ctx: &'a ToolExecutionContext<'a>,
        tool: &'a ChatToolDefinition,
        arguments: Value,
        _skill_cache: Option<&'a mut skills::SkillRunCache>,
    ) -> ToolExecutorFuture<'a> {
        let tool_name = tool.openai_tool_name();
        let fallback = tool.name.clone();
        Box::pin(async move {
            // Prefer the openai_tool_name (what the model called), fall back to
            // the raw name for native tools where they coincide.
            let result = self.dispatch(&tool_name, arguments.clone()).await;
            match result {
                Ok(output) => Ok(output),
                Err(err) if tool_name != fallback => {
                    // Retry under the raw name in case of an alias mismatch.
                    match self.dispatch(&fallback, arguments).await {
                        Ok(output) => Ok(output),
                        Err(_) => Err(err),
                    }
                }
                Err(err) => Err(err),
            }
        })
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
        let executor = CliToolExecutor::new(
            vec![dir.to_string_lossy().into_owned()],
            reqwest::Client::new(),
            120_000,
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
}
