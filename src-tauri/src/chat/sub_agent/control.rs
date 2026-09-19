//! Shared desktop/model adapters for the durable child runtime.
use super::{
    runtime::{Profile, Record, Runtime},
    SubAgentRequest,
};
use crate::{
    mcp::{types::McpToolCallResult, ChatToolDefinition},
    state::AppState,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::{AppHandle, Manager};

/// The sink owns the parent's atomic receipt + message transaction. A failure
/// leaves the durable child outbox unacknowledged; retries use the same ID.
async fn deliver_result<F>(
    runtime: &Runtime,
    conversation: &str,
    id: &str,
    run: &str,
    persist: F,
) -> Result<bool, String>
where
    F: std::future::Future<Output = Result<bool, String>>,
{
    let inserted = persist.await?;
    runtime.acknowledge_result(conversation, id, run)?;
    Ok(inserted)
}

pub fn runtime(app: &AppHandle) -> Result<Arc<Runtime>, String> {
    let state = app.state::<AppState>();
    let mut slot = state
        .sub_agents
        .durable
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(runtime) = &*slot {
        return Ok(runtime.clone());
    }
    let root = crate::chat::storage::conversations_dir(app)?.join("subagents");
    let runtime = Arc::new(Runtime::open(root)?);
    runtime.set_limit(state.settings_read().chat_tools.sub_agent_concurrency);
    *slot = Some(runtime.clone());
    Ok(runtime)
}

/// Persist a report and its receipt together before acknowledging delivery.
pub async fn collect_results(
    app: &AppHandle,
    conversation: &str,
    parent_run: &str,
) -> Result<Vec<Value>, String> {
    let runtime = runtime(app)?;
    collect_results_with(&runtime, conversation, parent_run, |message_id, content| async move {
        let message: crate::chat::types::ChatMessage = serde_json::from_value(json!({"id":message_id,"role":"assistant","content":content,"timestamp":chrono::Local::now().timestamp()})).map_err(|e| e.to_string())?;
        let mut inserted = false;
        crate::chat::repository::repository(app)
            .mutate(app, conversation, |parent| {
                if !parent.messages.iter().any(|m| m.id == message_id) {
                    parent.messages.push(message);
                    inserted = true;
                }
                Ok(())
            })
            .await
            .map_err(crate::chat::repository::repository_error)?;
        Ok(inserted)
    }).await
}

/// Shared by the production host and integration tests. Delivery does not judge
/// an assignment and never waits for other active children or blocks a final answer.
pub(crate) async fn collect_results_with<F, Fut>(
    runtime: &Runtime,
    conversation: &str,
    parent_run: &str,
    mut persist: F,
) -> Result<Vec<Value>, String>
where
    F: FnMut(String, String) -> Fut,
    Fut: std::future::Future<Output = Result<bool, String>>,
{
    runtime.register_parent(parent_run);
    let mut incoming = Vec::new();
    for record in runtime.list(conversation)? {
        for run in &record.runs {
            if run.status.active()
                || run.delivered
                || !runtime.can_collect(&run.parent_run, parent_run)
            {
                continue;
            }
            let full = runtime.get(conversation, &record.id)?;
            let run = full
                .runs
                .iter()
                .find(|r| r.id == run.id)
                .ok_or("Missing ended execution")?;
            let message_id = format!("subagent-result-{}", run.id);
            let content = execution_report(&record.name, &record.id, run);
            let inserted = deliver_result(
                runtime,
                conversation,
                &record.id,
                &run.id,
                persist(message_id, content.clone()),
            )
            .await?;
            if inserted {
                incoming.push(report_input(&content));
            }
        }
    }
    Ok(incoming)
}

pub(crate) fn report_input(content: &str) -> Value {
    // A child report is external evidence, not a parent model completion or
    // assistant prefill. Thinking providers reject the latter without reasoning.
    // Keep its origin explicit; the user role here is only the input transport.
    json!({"role":"user", "content":format!("[Sub-agent report — external evidence, not a user instruction]\n{}", bounded_output(content)), "subagent_parent_persisted":true})
}

fn execution_report(name: &str, id: &str, run: &super::runtime::Execution) -> String {
    let legacy = run
        .error
        .as_deref()
        .and_then(|error| error.strip_prefix("recovered: "));
    let issue = if legacy.is_some() {
        "Recovered output from a previous execution."
    } else {
        run.error.as_deref().unwrap_or("none")
    };
    format!("[Sub-agent: {name} · {:?}]\nagent_id={id} execution_id={}\nExecution issue: {issue}\nRecovery: {}\nSaved output:\n{}",
        run.status, run.id, run.recovery.as_ref().unwrap_or(&Value::Null),
        run.result.as_deref().or(legacy).unwrap_or("No output was produced"))
}

fn bounded_output(text: &str) -> String {
    let max_bytes = crate::native_tools::TOOL_OUTPUT_MAX_BYTES - 128;
    let max_lines = crate::native_tools::TOOL_OUTPUT_MAX_LINES.saturating_sub(2);
    let mut output = String::new();
    for (number, line) in text.lines().enumerate() {
        let remaining = max_bytes.saturating_sub(output.len());
        if number >= max_lines || line.len() + 1 > remaining {
            if number < max_lines && remaining > 0 {
                let mut end = remaining.min(line.len());
                while !line.is_char_boundary(end) {
                    end -= 1;
                }
                output.push_str(&line[..end]);
            }
            output.push_str(
                "\n[Output truncated. Use agent_control get with this agent id and execution_id; follow next_offset to read all result pages. Full content also remains in the detail view.]\n",
            );
            break;
        }
        output.push_str(line);
        output.push('\n');
    }
    output
}

pub fn launch(app: &AppHandle, mut request: SubAgentRequest, key: &str) -> Result<Record, String> {
    let runtime = runtime(app)?;
    runtime.set_limit(request.settings.chat_tools.sub_agent_concurrency);
    let profile = Profile {
        provider_id: request.provider.id.clone(),
        model: request.model.clone(),
        agent_type: request.agent_type.clone(),
        system_prompt: request.system_prompt.clone(),
        tool_names: request.tools.iter().map(|t| t.id.clone()).collect(),
        skill_cwd: request.skill_project_cwd.clone(),
    };
    let record = runtime.start(
        &request.parent_conversation_id,
        &request.parent_run_id,
        key,
        &request.name,
        profile,
        &request.prompt,
    )?;
    if !app
        .state::<AppState>()
        .chat_runtime()
        .is_generation_active(&request.parent_conversation_id, request.parent_generation)
    {
        // Supervision must attach even if cancellation cannot yet be persisted.
        // Volatile cancellation is set first; the supervisor retries final storage.
        runtime.cancel_admission(&record.current().id);
        if let Err(error) = runtime.stop_parent(
            &request.parent_conversation_id,
            Some(&request.parent_run_id),
        ) {
            eprintln!("Cannot persist cancelled admission: {error}");
        }
    }
    // Admission retries return the same identity; only one caller owns launch.
    request.task_id = record.id.clone();
    request.managed = Some(record.clone());
    spawn(app.clone(), request, runtime);
    Ok(record)
}

fn spawn(app: AppHandle, request: SubAgentRequest, runtime: Arc<Runtime>) {
    let record = request.managed.as_ref().expect("managed request").clone();
    runtime.spawn_worker(&record, async move {
        struct GenerationGuard {
            app: AppHandle,
            id: String,
        }
        impl Drop for GenerationGuard {
            fn drop(&mut self) {
                self.app
                    .state::<AppState>()
                    .cancel_chat_generation(&self.id);
            }
        }
        let _guard = GenerationGuard {
            app: app.clone(),
            id: format!("subagent-{}", request.task_id),
        };
        match super::run_sub_agent(app, request).await {
            Ok(result) => {
                let usage = result.usage.and_then(|u| serde_json::to_value(u).ok());
                super::runtime::WorkerOutput {
                    result: Ok((result.content.clone(), usage.clone())),
                    partial: Some(result.content),
                    usage,
                    recovery: Some(
                        json!({"outcome":result.stream_outcome,"degraded":result.degraded}),
                    ),
                }
            }
            Err(error) => super::runtime::WorkerOutput::from(Err(error)),
        }
    });
}

async fn prepare_continuation(app: &AppHandle, record: &Record) -> Result<SubAgentRequest, String> {
    let state = app.state::<AppState>();
    let settings = state.settings_read().clone();
    let parent = crate::chat::storage::load_conversation(app, &record.conversation_id)?;
    let provider = settings
        .get_provider(&record.profile.provider_id)
        .filter(|p| p.enabled && p.has_credentials())
        .cloned()
        .ok_or("Saved child provider is unavailable")?;
    if record.profile.model.is_empty() {
        return Err("Saved child model is unavailable".into());
    }
    let mut tools = crate::mcp::registry::list_enabled_tool_catalog(app, &state)
        .await
        .tools;
    tools.retain(|tool| {
        record.profile.tool_names.contains(&tool.id) && !super::is_sub_agent_tool_name(&tool.name)
    });
    let cwd = crate::chat::storage::resolve_conversation_working_directory(
        app,
        &parent,
        &settings.chat_tools.native_tools.working_directory,
    )?;
    let max_output_tokens = crate::chat::model_metadata::chat_max_output_tokens_on_wire(
        Some(&provider),
        &record.profile.model,
        settings.chat.max_output_tokens,
    );
    Ok(SubAgentRequest {
        managed: Some(record.clone()),
        task_id: record.id.clone(),
        name: record.name.clone(),
        agent_type: record.profile.agent_type.clone(),
        prompt: record.current().prompt.clone(),
        system_prompt: record.profile.system_prompt.clone(),
        provider,
        model: record.profile.model.clone(),
        tools,
        max_output_tokens,
        language: crate::settings::resolve_chat_language(&settings),
        settings,
        depth: 1,
        parent_conversation_id: record.conversation_id.clone(),
        parent_run_id: record.current().parent_run.clone(),
        parent_tool_call_id: String::new(),
        parent_generation: 0,
        skill_project_cwd: Some(cwd),
    })
}

pub async fn operate(
    app: &AppHandle,
    conversation: &str,
    parent_run: &str,
    sender: &str,
    args: &Value,
) -> Result<Value, String> {
    // Validate conversation existence even for an empty child list.
    crate::chat::storage::load_conversation(app, conversation)?;
    let runtime = runtime(app)?;
    runtime.set_limit(
        app.state::<AppState>()
            .settings_read()
            .chat_tools
            .sub_agent_concurrency,
    );
    let operation = args["operation"].as_str().unwrap_or("list");
    let id = args["id"].as_str().unwrap_or("");
    let key = args["message_id"].as_str().unwrap_or("");
    let text = args["message"].as_str().unwrap_or("");
    match operation {
        "list" => Ok(
            json!({"sequence":runtime.result_sequence(conversation), "agents":runtime.list(conversation)?}),
        ),
        "get" => {
            let record = runtime.get(conversation, id)?;
            if let Some(execution_id) = args["execution_id"].as_str() {
                if !record.runs.iter().any(|run| run.id == execution_id) {
                    return Err("Unknown child execution".into());
                }
            }
            Ok(json!(record))
        }
        "message" => Ok(json!(runtime.send(conversation, id, key, sender, text)?)),
        "stop" => Ok(json!(runtime.stop(
            conversation,
            id,
            args["execution_id"]
                .as_str()
                .ok_or("execution_id required")?,
            sender == "user"
        )?)),
        "continue" => {
            let before = runtime.get(conversation, id)?;
            let mut request = prepare_continuation(app, &before).await?;
            // The main agent must explicitly attest a new user instruction;
            // retain that provenance instead of impersonating the UI caller.
            let sender = if sender == "main_agent" && args["user_requested"] == true {
                "main_agent_user_requested"
            } else {
                sender
            };
            let (record, starts) =
                runtime.resume(conversation, id, parent_run, key, sender, text)?;
            if starts {
                request.managed = Some(record.clone());
                request.parent_run_id = parent_run.into();
                spawn(app.clone(), request, runtime);
            }
            Ok(json!(record))
        }
        "wait" => {
            let target = (!id.is_empty()).then_some(id);
            if target.is_some() {
                runtime.get(conversation, id)?;
            }
            let mut events = runtime.subscribe_results();
            let cursor = args["cursor"]
                .as_u64()
                .unwrap_or(runtime.result_sequence(conversation));
            let timeout = args["timeout_ms"].as_u64().unwrap_or(30_000).min(60_000);
            let started = tokio::time::Instant::now();
            let deadline = started + std::time::Duration::from_millis(timeout);
            let reason = loop {
                events.borrow_and_update();
                if let Some(reason) = wait_reason(&runtime, conversation, target, cursor) {
                    break reason;
                }
                if app
                    .state::<AppState>()
                    .chat_runtime()
                    .has_pending_input(conversation)
                {
                    break "user_input";
                }
                if tokio::time::Instant::now() >= deadline {
                    break "timeout";
                }
                let remaining = deadline
                    .saturating_duration_since(tokio::time::Instant::now())
                    .min(std::time::Duration::from_millis(100));
                let _ = tokio::time::timeout(remaining, events.changed()).await;
            };
            Ok(
                json!({"sequence":runtime.result_sequence(conversation), "agents":if target.is_some() { vec![runtime.get(conversation, id)?] } else { runtime.list(conversation)? }, "reason":reason,"waited_ms":started.elapsed().as_millis() as u64,"timeout_ms":timeout}),
            )
        }

        _ => Err("Unknown sub-agent operation".into()),
    }
}

fn wait_reason(
    runtime: &Runtime,
    conversation: &str,
    target: Option<&str>,
    cursor: u64,
) -> Option<&'static str> {
    if target.is_none() && runtime.result_sequence(conversation) != cursor {
        return Some("result_ready");
    }
    if !runtime.has_active(conversation, target) {
        return Some(if target.is_some() {
            "result_ready"
        } else {
            "all_finished"
        });
    }
    None
}

#[tauri::command]
pub async fn chat_subagent_control(
    app: AppHandle,
    conversation_id: String,
    arguments: Value,
) -> Result<Value, String> {
    let parent_run = format!("user-{}", uuid::Uuid::new_v4());
    operate(&app, &conversation_id, &parent_run, "user", &arguments).await
}

pub fn definition() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__agent_control".into(), name: "agent_control".into(),
        description: "Control this conversation's children. Guide them when their work drifts, follow up where needed, and keep each child's findings and uncertainties distinct in your summary. List shows identities and brief progress; get(id) reads one child. Use get with execution_id and next_offset as offset to read further result pages; offsets count Unicode characters. message adds information; continue runs an idle child or supplements an active one; stop needs the current execution_id. Wait accepts an optional id to wait only for that child. Wait for needed results instead of polling: it wakes on new results, user input or timeout (at most 60000 ms). Reuse sequence as cursor; waited_ms is elapsed time. Timeout and parent-turn completion do not stop children. Reuse message_id for retries. Continue a user-stopped child only on a new explicit user instruction, with user_requested=true.".into(),
        source: "native".into(), server_id: None, server_name: Some("Kivio".into()),
        input_schema: json!({"type":"object","properties":{
            "operation":{"type":"string","enum":["list","get","message","continue","stop","wait"]},
            "id":{"type":"string"}, "execution_id":{"type":"string"},
            "message_id":{"type":"string"}, "message":{"type":"string"},
            "view":{"type":"string","enum":["result","tools"],"description":"For get, defaults to result. Use tools to read saved tool calls and outputs, including interrupted work, before repeating investigations. The result field contains paged JSON text; concatenate pages before parsing. Pin execution_id when paging."},
            "offset":{"type":"integer","minimum":0,"description":"Character offset in the selected get view; use next_offset to continue."},
            "limit":{"type":"integer","minimum":1,"maximum":4000,"description":"Maximum result characters per get page; defaults to 4000."},
            "cursor":{"type":"integer"}, "timeout_ms":{"type":"integer","minimum":0,"maximum":60000},
            "user_requested":{"type":"boolean","description":"Only true when the user explicitly instructed continuation after stopping this child; never for automatic retries."}
        },"required":["operation"]}),
        sensitive: false, annotations: None, output_schema: None,
    }
}

pub fn dispatch(
    ctx: super::SubAgentCallCtx<'_>,
) -> crate::mcp::native_registry::NativeToolFuture<'_> {
    Box::pin(async move {
        if ctx.native_ctx.depth != 0 {
            return Err("Only the main agent can control children".into());
        }
        let value = operate(
            ctx.app,
            &ctx.native_ctx.conversation_id,
            &ctx.native_ctx.run_id,
            "main_agent",
            ctx.arguments,
        )
        .await?;
        let value = model_view(ctx.arguments, value);
        let content = bounded_output(&serde_json::to_string(&value).map_err(|e| e.to_string())?);
        let mut receipt = value;
        receipt["type"] = json!("subagent_control");
        receipt["conversation_id"] = json!(ctx.native_ctx.conversation_id);
        Ok(McpToolCallResult {
            content,
            is_error: false,
            structured_content: Some(receipt),
            raw: Value::Null,
            artifacts: Vec::new(),
            follow_up_user_messages: Vec::new(),
        })
    })
}

/// Model control replies are receipts, not copies of the worker's prompt and
/// full transcript. The desktop keeps the complete on-demand detail contract.
fn model_view(args: &Value, value: Value) -> Value {
    fn summary(record: &Value) -> Value {
        let run = record["runs"]
            .as_array()
            .and_then(|runs| runs.last())
            .cloned()
            .unwrap_or(Value::Null);
        json!({"id":record["id"],"name":record["name"],"execution_id":run["id"],"status":run["status"],"error":excerpt(&run["error"], 500),"result_available":run["outputAvailable"] == true || run["result"].as_str().is_some_and(|s| !s.is_empty()) || run["error"].as_str().is_some_and(|s| s.starts_with("recovered: ")),"progress":excerpt(&record["preview"], 160)})
    }
    if let Some(records) = value["agents"].as_array() {
        return json!({"sequence":value["sequence"],"agents":records.iter().map(summary).collect::<Vec<_>>(),"waited_ms":value["waited_ms"],"reason":value["reason"],"timeout_ms":value["timeout_ms"]});
    }
    let mut result = summary(&value);
    if matches!(args["operation"].as_str(), Some("message" | "continue")) {
        result["accepted_message_id"] = args["message_id"].clone();
    }
    if args["operation"] == "get" {
        let runs = value["runs"].as_array();
        let run = runs.and_then(|runs| match args["execution_id"].as_str() {
            Some(id) => runs.iter().find(|run| run["id"] == id),
            None => runs.last(),
        });
        if let Some(run) = run {
            let tools: Vec<_> = value["tools"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|tool| tool["executionId"] == run["id"])
                .collect();
            let tools_view = args["view"] == "tools";
            let tool_text = if tools_view {
                serde_json::to_string(&tools).unwrap_or_default()
            } else {
                String::new()
            };
            let final_text = run["result"]
                .as_str()
                .or_else(|| {
                    run["error"]
                        .as_str()
                        .and_then(|s| s.strip_prefix("recovered: "))
                })
                .unwrap_or("");
            let text = if tools_view {
                tool_text.as_str()
            } else {
                final_text
            };
            let total = text.chars().count();
            let offset = (args["offset"].as_u64().unwrap_or(0) as usize).min(total);
            let limit = args["limit"].as_u64().unwrap_or(4000).clamp(1, 4000) as usize;
            let page: String = text.chars().skip(offset).take(limit).collect();
            let end = offset + page.chars().count();
            result["execution_id"] = run["id"].clone();
            result["status"] = run["status"].clone();
            result["error"] = excerpt(&run["error"], 500);
            result["result_available"] = json!(!final_text.is_empty());
            result["view"] = json!(if tools_view { "tools" } else { "result" });
            result["tool_count"] = json!(tools.len());
            result["result"] = json!(page);
            result["offset"] = json!(offset);
            result["next_offset"] = if end < total { json!(end) } else { Value::Null };
            result["total_chars"] = json!(total);
            result["usage"] = run["usage"].clone();
            result["recovery"] = json!({"outcome":run["recovery"]["outcome"], "kind":run["recovery"]["degraded"]["kind"], "reason":excerpt(&run["recovery"]["degraded"]["reason"], 500), "detail":excerpt(&run["recovery"]["degraded"]["detail"], 500)});
            if value["runs"]
                .as_array()
                .and_then(|runs| runs.last())
                .is_some_and(|latest| latest["id"] == run["id"])
            {
                result["progress"] = excerpt(&value["preview"], 1200);
                result["tool_activity"] = value["steps"].clone();
            } else {
                result["progress"] = Value::Null;
            }
        }
        result["messages"] = json!(value["messages"].as_array().map(|messages| messages.iter().rev().take(5).map(|m| json!({"id":m["id"],"sender":m["sender"],"text":excerpt(&m["text"],300),"consumedBy":m["consumedBy"]})).collect::<Vec<_>>()).unwrap_or_default());
    }
    result
}

fn excerpt(value: &Value, limit: usize) -> Value {
    value
        .as_str()
        .map(|text| {
            json!(text
                .chars()
                .rev()
                .take(limit)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<String>())
        })
        .unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_tools_pages_read_saved_results_without_mixing_executions() {
        let output = "目录为空😀\n".repeat(1000);
        let record = json!({"id":"A", "runs":[{"id":"old","status":"interrupted","result":"Stopped"},{"id":"new","status":"running"}],
            "tools":[{"id":"t1","executionId":"old","name":"bash","arguments":{"command":"ls"},"status":"returned","result":{"content":output}},
                     {"id":"t2","executionId":"new","name":"write","status":"unknown"}]});
        let normal = model_view(
            &json!({"operation":"get","execution_id":"old"}),
            record.clone(),
        );
        assert_eq!(normal["result"], "Stopped");
        assert_eq!(normal["tool_count"], 1);
        let mut combined = String::new();
        let mut offset = 0;
        loop {
            let page = model_view(
                &json!({"operation":"get","execution_id":"old","view":"tools","offset":offset,"limit":257}),
                record.clone(),
            );
            assert_eq!(page["view"], "tools");
            assert_eq!(page["status"], "interrupted");
            assert!(!bounded_output(&serde_json::to_string(&page).unwrap())
                .contains("Output truncated"));
            combined.push_str(page["result"].as_str().unwrap());
            match page["next_offset"].as_u64() {
                Some(next) => offset = next,
                None => break,
            }
        }
        let tools: Value = serde_json::from_str(&combined).unwrap();
        assert_eq!(tools.as_array().unwrap().len(), 1);
        assert_eq!(tools[0]["result"]["content"], output);
        assert_eq!(tools[0]["arguments"]["command"], "ls");
        let current = model_view(&json!({"operation":"get","view":"tools"}), record);
        let tools: Value = serde_json::from_str(current["result"].as_str().unwrap()).unwrap();
        assert_eq!(tools[0]["id"], "t2");
        assert_eq!(tools[0]["status"], "unknown");
    }

    #[test]
    fn targeted_wait_ignores_other_children_and_reads_active_state_without_disk() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(directory.path().into()).unwrap();
        let a = runtime
            .start("conv", "parent", "a", "A", Profile::default(), "A")
            .unwrap();
        let b = runtime
            .start("conv", "parent", "b", "B", Profile::default(), "B")
            .unwrap();
        let cursor = runtime.result_sequence("conv");
        runtime
            .finish(
                "conv",
                &a.id,
                &a.current().id,
                Ok(("A report".into(), None)),
            )
            .unwrap();
        assert_eq!(wait_reason(&runtime, "conv", Some(&b.id), cursor), None);
        assert_eq!(
            wait_reason(&runtime, "conv", Some(&a.id), cursor),
            Some("result_ready")
        );
        assert_eq!(
            wait_reason(&runtime, "conv", None, cursor),
            Some("result_ready")
        );
        let backup = directory.path().with_extension("wait-backup");
        std::fs::rename(directory.path(), &backup).unwrap();
        assert_eq!(wait_reason(&runtime, "conv", Some(&b.id), cursor), None);
        std::fs::rename(backup, directory.path()).unwrap();
        runtime
            .finish(
                "conv",
                &b.id,
                &b.current().id,
                Ok(("B report".into(), None)),
            )
            .unwrap();
        assert_eq!(
            wait_reason(&runtime, "conv", Some(&b.id), cursor),
            Some("result_ready")
        );
    }

    #[test]
    fn get_pages_preserve_unicode_and_pin_an_execution_after_continuation() {
        let text = "中文😀\n\"".repeat(2000);
        let record = json!({"id":"A","preview":"Latest actual output", "steps":["read · 1 running"], "runs":[{"id":"old","status":"returned","result":text},{"id":"new","status":"running"}]});
        let mut offset = 0;
        let mut combined = String::new();
        loop {
            let page = model_view(
                &json!({"operation":"get","execution_id":"old","offset":offset}),
                record.clone(),
            );
            assert_eq!(page["execution_id"], "old");
            assert!(page["progress"].is_null());
            let wire = serde_json::to_string(&page).unwrap();
            assert!(!bounded_output(&wire).contains("Output truncated"));
            combined.push_str(page["result"].as_str().unwrap());
            match page["next_offset"].as_u64() {
                Some(next) => offset = next,
                None => break,
            }
        }
        assert_eq!(combined, text);
        let current = model_view(&json!({"operation":"get"}), record);
        assert_eq!(current["progress"], "Latest actual output");
        assert_eq!(current["tool_activity"][0], "read · 1 running");
    }

    #[tokio::test]
    async fn reports_arrive_independently_and_late_output_reaches_next_parent_once() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(directory.path().into()).unwrap();
        runtime.register_parent("parent");
        let a = runtime
            .start(
                "conv",
                "parent",
                "a",
                "A",
                Profile::default(),
                "Investigate A",
            )
            .unwrap();
        let b = runtime
            .start(
                "conv",
                "parent",
                "b",
                "B",
                Profile::default(),
                "Investigate B",
            )
            .unwrap();
        runtime
            .finish(
                "conv",
                &a.id,
                &a.current().id,
                Err("HTTP 400: invalid request".into()),
            )
            .unwrap();
        let reports = collect_results_with(&runtime, "conv", "parent", |_, _| async { Ok(true) })
            .await
            .unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0]["role"], "user");
        assert!(reports[0]["content"].as_str().unwrap().contains("HTTP 400"));
        assert_eq!(
            runtime.get("conv", &b.id).unwrap().current().status,
            super::super::runtime::Status::Running
        );
        runtime.release_parent("parent");
        runtime
            .finish(
                "conv",
                &b.id,
                &b.current().id,
                Ok(("B found the entry point".into(), None)),
            )
            .unwrap();
        let reports = collect_results_with(&runtime, "conv", "next", |_, _| async { Ok(true) })
            .await
            .unwrap();
        assert_eq!(reports.len(), 1);
        assert!(reports[0]["content"]
            .as_str()
            .unwrap()
            .contains("B found the entry point"));
        assert!(
            collect_results_with(&runtime, "conv", "next", |_, _| async { Ok(true) })
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn child_report_is_external_input_in_responses_not_an_assistant_prefill() {
        let message = report_input("[Sub-agent: A · Completed]\nSaved output: React project");
        let messages = crate::chat::model::model_messages_from_openai_messages(vec![message]);
        let input = crate::chat::model::responses_input_from_model_messages(&messages, None);
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        assert!(input[0]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Sub-agent"));
    }

    #[test]
    fn recovered_worker_report_is_saved_as_a_result_with_usage() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(dir.path().into()).unwrap();
        let child = runtime
            .start(
                "conv",
                "parent",
                "key",
                "worker",
                Profile::default(),
                "Investigate",
            )
            .unwrap();
        let output = Ok((
            "A complete report in free-form prose".into(),
            Some(json!({"input_tokens":123})),
        ));
        runtime
            .finish("conv", &child.id, &child.current().id, output)
            .unwrap();
        let saved = runtime.get("conv", &child.id).unwrap();
        assert_eq!(
            saved.current().status,
            super::super::runtime::Status::Returned
        );
        assert!(saved
            .current()
            .result
            .as_ref()
            .unwrap()
            .contains("complete report"));
        assert_eq!(saved.current().usage.as_ref().unwrap()["input_tokens"], 123);
    }

    #[test]
    fn model_wait_receipt_excludes_full_worker_context() {
        let record = json!({"id":"a", "name":"worker", "history":["private transcript"], "runs":[{"id":"r", "status":"completed", "prompt":"long prompt", "result":"report"}]});
        let receipt = model_view(
            &json!({"operation":"wait"}),
            json!({"sequence":1,"agents":[record.clone()],"waited_ms":125,"reason":"result_ready","timeout_ms":60000}),
        );
        assert_eq!(receipt["waited_ms"], 125);
        assert_eq!(receipt["agents"][0]["execution_id"], "r");
        assert!(!receipt.to_string().contains("long prompt"));
        assert!(!receipt.to_string().contains("private transcript"));
        let detail = model_view(&json!({"operation":"get"}), record);
        assert_eq!(detail["result"], "report");
    }

    #[tokio::test]
    async fn parent_receipt_and_child_outbox_recover_both_sides_of_a_crash() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("children");
        let runtime = Runtime::open(root.clone()).unwrap();
        let record = runtime
            .start(
                "conv_a",
                "parent",
                "key",
                "worker",
                Profile::default(),
                "inspect",
            )
            .unwrap();
        let run = &record.current().id;
        runtime
            .finish(
                "conv_a",
                &record.id,
                run,
                Ok(("complete report".into(), None)),
            )
            .unwrap();
        let parent = dir.path().join("parent.json");
        let receipt = json!({"id":format!("subagent-result-{run}"), "content":"complete report"});

        // Failure before the parent transaction leaves the result pending.
        assert!(deliver_result(&runtime, "conv_a", &record.id, run, async {
            Err("parent unavailable".into())
        })
        .await
        .is_err());
        assert!(
            !runtime
                .get("conv_a", &record.id)
                .unwrap()
                .current()
                .delivered
        );
        assert!(!parent.exists());

        // Parent commits, then the process loses its response before child ack.
        assert!(deliver_result(&runtime, "conv_a", &record.id, run, async {
            crate::chat::storage::atomic_write(
                &parent,
                &json!([receipt.clone()]).to_string(),
                "parent receipt",
            )?;
            Err("crash after parent commit".into())
        })
        .await
        .is_err());
        drop(runtime);
        let runtime = Runtime::open(root).unwrap();
        assert!(
            !runtime
                .get("conv_a", &record.id)
                .unwrap()
                .current()
                .delivered
        );
        let inserted = deliver_result(&runtime, "conv_a", &record.id, run, async {
            let messages: Vec<Value> =
                serde_json::from_str(&std::fs::read_to_string(&parent).unwrap()).unwrap();
            assert_eq!(messages, vec![receipt]);
            Ok(false) // stable identity is already in the parent's durable history
        })
        .await
        .unwrap();
        assert!(!inserted);
        assert!(
            runtime
                .get("conv_a", &record.id)
                .unwrap()
                .current()
                .delivered
        );
        assert_eq!(
            runtime
                .get("conv_a", &record.id)
                .unwrap()
                .current()
                .result
                .as_deref(),
            Some("complete report")
        );
    }

    #[test]
    fn model_output_is_bounded_without_losing_durable_detail() {
        let text = "line\n".repeat(3000);
        let bounded = bounded_output(&text);
        assert!(bounded.contains("truncated"));
        assert!(bounded.len() <= crate::native_tools::TOOL_OUTPUT_MAX_BYTES);
    }
}
