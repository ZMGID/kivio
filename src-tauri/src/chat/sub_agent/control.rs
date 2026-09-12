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

/// The parent conversation is the receipt ledger. A stable message identity and
/// its readable result are committed together; only then is the child outbox
/// acknowledged. A crash between commits retries the idempotent parent upsert.
pub async fn collect_results(
    app: &AppHandle,
    conversation: &str,
    parent_run: &str,
) -> Result<(Vec<Value>, bool), String> {
    let runtime = runtime(app)?;
    let records = runtime.list(conversation)?;
    let mut pending = false;
    let mut incoming = Vec::new();
    for record in records {
        for run in &record.runs {
            pending |= run.parent_run == parent_run && run.status.active();
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
                .ok_or("Missing completed execution")?;
            let message_id = format!("subagent-result-{}", run.id);
            let content = format!(
                "[Sub-agent: {} · {:?}]\n{}",
                record.name,
                run.status,
                run.result
                    .as_deref()
                    .or(run.error.as_deref())
                    .unwrap_or("No result")
            );
            let message: crate::chat::types::ChatMessage = serde_json::from_value(json!({"id":message_id,"role":"assistant","content":content,"timestamp":chrono::Local::now().timestamp()})).map_err(|e| e.to_string())?;
            let inserted = deliver_result(&runtime, conversation, &record.id, &run.id, async {
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
            })
            .await?;
            if inserted {
                incoming.push(json!({"role":"assistant", "content":bounded_output(&content), "subagent_parent_persisted":true}));
            }
        }
    }
    Ok((incoming, pending))
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
                "\n[Output truncated. Full content remains in the sub-agent detail view.]\n",
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
        .is_chat_generation_active(&request.parent_conversation_id, request.parent_generation)
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
    runtime.spawn_task(&record, async move {
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
            Ok(result) => worker_output(
                &result.stream_outcome,
                result.content,
                result.usage.and_then(|u| serde_json::to_value(u).ok()),
                result.degraded.is_some(),
            ),
            Err(error) => Err(error),
        }
    });
}

fn worker_output(
    outcome: &str,
    content: String,
    usage: Option<Value>,
    degraded: bool,
) -> Result<(String, Option<Value>), String> {
    if outcome == "completed" && !degraded {
        Ok((content, usage))
    } else if outcome == "recovered" && !degraded && !content.trim().is_empty() {
        Ok((
            format!("[Recovered response / 恢复后生成的答复]\n\n{content}"),
            usage,
        ))
    } else {
        Err(format!("{outcome}: {content}"))
    }
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
        "get" => Ok(json!(runtime.get(conversation, id)?)),
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
            let mut events = runtime.subscribe_results();
            let cursor = args["cursor"]
                .as_u64()
                .unwrap_or(runtime.result_sequence(conversation));
            let timeout = args["timeout_ms"].as_u64().unwrap_or(30_000).min(60_000);
            let started = tokio::time::Instant::now();
            let deadline = started + std::time::Duration::from_millis(timeout);
            let reason = loop {
                events.borrow_and_update();
                if runtime.result_sequence(conversation) != cursor {
                    break "result_ready";
                }
                if !runtime
                    .list(conversation)?
                    .iter()
                    .any(|r| r.current().status.active())
                {
                    break "all_finished";
                }
                if app.state::<AppState>().has_chat_pending_input(conversation) {
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
                json!({"sequence":runtime.result_sequence(conversation), "agents":runtime.list(conversation)?, "reason":reason,"waited_ms":started.elapsed().as_millis() as u64,"timeout_ms":timeout}),
            )
        }

        _ => Err("Unknown sub-agent operation".into()),
    }
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
    ChatToolDefinition { id: "native__agent_control".into(), name: "agent_control".into(), description: "Control children belonging to this conversation. List/get results; message only adds information (idle children do not run); continue explicitly runs an idle child or supplements an active one; stop requires the current execution_id; wait wakes for completed/failed/interrupted results, user input, or at most 60000 ms; progress alone does not wake it. Use the returned sequence as cursor. waited_ms is actual elapsed time; never infer elapsed time or a stall from the requested timeout. When all children have ended, summarize their results now; do not keep waiting or announce a future summary. Use stable message_id for retries. A user-stopped child requires a new explicit user instruction: only then set user_requested=true on continue. Never set it for automatic retries. The message retains main-agent provenance.".into(), source:"native".into(), server_id: None, server_name:Some("Kivio".into()), input_schema:json!({"type":"object","properties":{"operation":{"type":"string","enum":["list","get","message","continue","stop","wait"]},"id":{"type":"string"},"execution_id":{"type":"string"},"message_id":{"type":"string"},"message":{"type":"string"},"cursor":{"type":"integer"},"timeout_ms":{"type":"integer","minimum":0,"maximum":60000},"user_requested":{"type":"boolean","description":"Only true when the user explicitly instructed continuation after stopping this child; never for automatic retries."}},"required":["operation"]}), sensitive:false, annotations:None, output_schema:None }
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
        Ok(McpToolCallResult {
            content,
            is_error: false,
            structured_content: None,
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
        json!({"id":record["id"],"name":record["name"],"execution_id":run["id"],"status":run["status"],"error":run["error"],"result_available":!run["status"].as_str().is_some_and(|s| matches!(s,"running"|"finishing"|"stopping"))})
    }
    if let Some(records) = value["agents"].as_array() {
        return json!({"sequence":value["sequence"],"agents":records.iter().map(summary).collect::<Vec<_>>(),"waited_ms":value["waited_ms"],"reason":value["reason"],"timeout_ms":value["timeout_ms"]});
    }
    let mut result = summary(&value);
    if matches!(args["operation"].as_str(), Some("message" | "continue")) {
        result["accepted_message_id"] = args["message_id"].clone();
    }
    if args["operation"] == "get" {
        if let Some(run) = value["runs"].as_array().and_then(|runs| runs.last()) {
            result["result"] = run["result"].clone();
            result["usage"] = run["usage"].clone();
        }
        result["messages"] = value["messages"].clone();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let output = worker_output(
            "recovered",
            "A complete report in free-form prose".into(),
            Some(json!({"input_tokens":123})),
            false,
        );
        runtime
            .finish("conv", &child.id, &child.current().id, output)
            .unwrap();
        let saved = runtime.get("conv", &child.id).unwrap();
        assert_eq!(
            saved.current().status,
            super::super::runtime::Status::Completed
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
    fn recovery_does_not_turn_cancellation_or_degraded_fallback_into_success() {
        assert!(worker_output("cancelled", "partial".into(), None, false).is_err());
        assert!(worker_output("recovered", "fallback".into(), None, true).is_err());
        assert!(worker_output("recovered", " ".into(), None, false).is_err());
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
