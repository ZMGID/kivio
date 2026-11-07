#[cfg(test)]
mod tests {
    use super::*;
    use super::ToolDispatcher as ToolExecutor;
    use crate::chat::agent::execute::unknown_tool_record;
    use std::time::Duration;
    use crate::chat::types::{ToolCallRecord, ToolCallStatus, ChatMessageSegment};
    use crate::chat::agent::control::{AgentRunControl, AgentRunHandle, InputAdmission, StopAdmission};
    use crate::chat::agent::events::{AgentEvent, AgentEventKind, AgentEventSink, AgentRunIdentity, EventDelivery};
    use crate::chat::agent::content::InputKind;
    use crate::chat::agent::steering::SteeringMessage;
    use crate::chat::agent::context::AgentContext;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    };

    use crate::chat::agent::host::AgentHostFuture;
    use crate::mcp::types::native_skill_activate_tool;

    #[derive(Default)]
    struct ExecuteTestHost {
        stopped: AtomicBool,
        finish_tool: bool,
        workflow: Option<crate::chat::workflow_hooks::Runtime>,
        approvals: AtomicUsize,
        consents: AtomicUsize,
        deny_approval: bool,
        deny_consent: bool,
        parallel_arm: bool,
        records: Mutex<Vec<ToolCallRecord>>,
    }

    impl AgentHost for ExecuteTestHost {
        fn requires_tool_completion(&self) -> bool { self.finish_tool }









    }

    impl crate::chat::tool_runtime::ApplicationHost for ExecuteTestHost {
        fn supports_interactive_approval(&self) -> bool {
            !self.parallel_arm
        }

        fn workflow_hooks(&self) -> Option<&crate::chat::workflow_hooks::Runtime> {
            self.workflow.as_ref()
        }

        fn request_tool_approval<'a>(
            &'a self,
            _ctx: &'a crate::chat::tool_runtime::ToolExecutionContext<'a>,
            _record: &'a AgentToolCall,
        ) -> AgentHostFuture<'a, bool> {
            self.approvals.fetch_add(1, Ordering::SeqCst);
            let granted = !self.deny_approval;
            Box::pin(async move { granted })
        }

        fn request_session_consent<'a>(
            &'a self,
            _ctx: &'a crate::chat::tool_runtime::ToolExecutionContext<'a>,
        ) -> AgentHostFuture<'a, bool> {
            self.consents.fetch_add(1, Ordering::SeqCst);
            let granted = !self.deny_consent;
            Box::pin(async move { granted })
        }

        fn request_user_response<'a>(
            &'a self,
            _ctx: &'a crate::chat::tool_runtime::ToolExecutionContext<'a>,
            _record: &'a AgentToolCall,
            _prompt: crate::chat::ask_user::AskUserPromptPayload,
        ) -> AgentHostFuture<'a, crate::chat::ask_user::AskUserResponseResult> {
            Box::pin(async { crate::chat::ask_user::skipped_response() })
        }
    }

impl ExecuteTestHost {
        fn emit_stream_delta(
            &self,
            _conversation_id: &str,
            _run_id: &str,
            _message_id: &str,
            _delta: &str,
            _reasoning_delta: Option<&str>,
            _segment: Option<&ChatMessageSegment>,
        ) {
        }
        fn emit_tool_record(
            &self,
            _conversation_id: &str,
            _run_id: &str,
            _message_id: &str,
            record: &ToolCallRecord,
        ) {
            self.records
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .push(record.clone());
        }

}

    #[derive(Default)]
    struct ExecuteTestExecutor {
        calls: AtomicUsize,
        structured_content: Option<Value>,
    }

    impl ToolExecutor for ExecuteTestExecutor {
        fn call<'a>(
            &'a self, _progress: &'a ToolProgress<'a>,
            _ctx: &'a ToolExecutionContext<'a>,
            tool: &'a ChatToolDefinition,
            _arguments: Value,
        ) -> ToolExecutorFuture<'a> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let content = format!("result:{}", tool.name);
            let structured_content = self.structured_content.clone();
            Box::pin(async move {
                crate::chat::tool_output::from_mcp(tool, crate::mcp::types::McpToolCallResult {
                    content,
                    is_error: false,
                    raw: Value::Null,
                    artifacts: Vec::new(),
                    structured_content,
                    follow_up_user_messages: Vec::new(),
                })
            })
        }
    }

    fn test_execution_context() -> ToolExecutionContext<'static> {
        ToolExecutionContext {
            conversation_id: "conversation",
            run_id: "run",
            message_id: "message",
            provider: {
                static PROVIDER: std::sync::LazyLock<crate::settings::ModelProvider> =
                    std::sync::LazyLock::new(|| {
                        serde_json::from_value(serde_json::json!({
                            "id": "test-provider", "name": "Test", "baseUrl": ""
                        }))
                        .unwrap()
                    });
                &PROVIDER
            },
            model: "test-model",
            generation: 1,
            round: 2,
            depth: 0,
            tool_conversation_id: "conversation",
            plan_state: {
                static PLAN_STATE: std::sync::LazyLock<crate::chat::types::AgentPlanState> =
                    std::sync::LazyLock::new(crate::chat::types::AgentPlanState::default);
                &PLAN_STATE
            },
            mcp_tool_ids: &[],
            mcp_server_ids: &[],
            approval_policy: "auto",
            interactive_host: false,
            tool_call_id: "call",
        }
    }

    fn test_pending_call(id: &str, function_name: &str, arguments: Value) -> PendingToolCall {
        PendingToolCall {
            id: id.to_string(),
            function_name: function_name.to_string(),
            arguments_raw: serde_json::to_string(&arguments).expect("serialize test args"),
            arguments,
            arguments_parse_error: None,
            signature: None,
        }
    }

    fn sensitive_test_tool() -> ChatToolDefinition {
        ChatToolDefinition {
            id: "native__write_file".to_string(),
            name: "write".to_string(),
            description: "Write file".to_string(),
            source: "native".to_string(),
            server_id: None,
            server_name: Some("Kivio".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
            }),
            sensitive: true,
            annotations: None,
            output_schema: None,
        }
    }

    fn named_test_tool(name: &str) -> ChatToolDefinition {
        ChatToolDefinition {
            id: format!("native__{name}"),
            name: name.to_string(),
            description: String::new(),
            source: "native".to_string(),
            server_id: None,
            server_name: Some("Kivio".to_string()),
            input_schema: serde_json::json!({ "type": "object" }),
            sensitive: false,
            annotations: None,
            output_schema: None,
        }
    }

    #[test]
    fn match_tool_call_falls_back_to_case_insensitive_unique_hit() {
        // Cursor 系模型按训练时的大写工具名出牌（Grep/Read/Bash）——唯一命中时采用。
        let tools = vec![named_test_tool("grep"), named_test_tool("read")];
        assert_eq!(match_tool_call(&tools, "grep").unwrap().name, "grep");
        assert_eq!(match_tool_call(&tools, "Grep").unwrap().name, "grep");
        assert_eq!(match_tool_call(&tools, "READ").unwrap().name, "read");
        assert!(match_tool_call(&tools, "Glob").is_none());
    }

    #[test]
    fn match_tool_call_routes_legacy_names_to_current_tools() {
        // 旧名（移除/合并/改名前的名字）经归一化路由到现工具。
        let tools = vec![
            named_test_tool("read"),
            named_test_tool("write"),
            named_test_tool("edit"),
            named_test_tool("grep"),
            named_test_tool("glob"),
            named_test_tool("bash"),
            named_test_tool("bash_output"),
            named_test_tool("todo_write"),
        ];
        assert_eq!(match_tool_call(&tools, "read_file").unwrap().name, "read");
        assert_eq!(match_tool_call(&tools, "write_file").unwrap().name, "write");
        assert_eq!(match_tool_call(&tools, "edit_file").unwrap().name, "edit");
        assert_eq!(match_tool_call(&tools, "list_dir").unwrap().name, "read");
        assert_eq!(match_tool_call(&tools, "ls").unwrap().name, "read");
        assert_eq!(
            match_tool_call(&tools, "search_files").unwrap().name,
            "grep"
        );
        assert_eq!(match_tool_call(&tools, "glob_files").unwrap().name, "glob");
        assert_eq!(match_tool_call(&tools, "find").unwrap().name, "glob");
        assert_eq!(match_tool_call(&tools, "run_command").unwrap().name, "bash");
        assert_eq!(
            match_tool_call(&tools, "list_background").unwrap().name,
            "bash_output"
        );
        assert_eq!(
            match_tool_call(&tools, "todo_update").unwrap().name,
            "todo_write"
        );
    }

    #[test]
    fn match_tool_call_refuses_ambiguous_case_variants() {
        // MCP 可能真有同名不同大小写的工具：精确匹配照常，忽略大小写多义时不猜。
        let tools = vec![named_test_tool("fetch"), named_test_tool("Fetch")];
        assert_eq!(match_tool_call(&tools, "fetch").unwrap().name, "fetch");
        assert_eq!(match_tool_call(&tools, "Fetch").unwrap().name, "Fetch");
        assert!(match_tool_call(&tools, "FETCH").is_none());
    }

    #[test]
    fn unknown_tool_record_is_error_metadata() {
        let call = PendingToolCall {
            id: "call_1".to_string(),
            function_name: "missing".to_string(),
            arguments: Value::Null,
            arguments_raw: "{}".to_string(),
            arguments_parse_error: None,
            signature: None,
        };

        let record = unknown_tool_record(&call, 2, "Unknown tool requested: missing".to_string());
        let record = crate::chat::agent_presentation::project_tool(&record, None);

        assert!(matches!(record.status, ToolCallStatus::Error));
        assert_eq!(record.round, 2);
        assert_eq!(record.source, "unknown");
    }

    #[tokio::test]
    async fn schema_validation_fails_before_approval_or_execution() {
        let host = ExecuteTestHost::default();
        let executor = ExecuteTestExecutor::default();
        let settings = Settings::default();
        let tool = sensitive_test_tool();
        let call = test_pending_call("call_invalid", "write", serde_json::json!({}));

        let (record, content, _) = execute_tool_call(
            &host,
            &executor,
            &settings,
            &test_execution_context(),
            &tool,
            call,
        )
        .await;

        assert!(matches!(record.status, ToolCallStatus::Error));
        assert!(record
            .error
            .as_deref()
            .is_some_and(|error| error.contains("arguments.path is required")));
        assert!(content.contains("schema validation"));
        assert_eq!(host.approvals.load(Ordering::SeqCst), 0);
        assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
        let records = host.records.lock().unwrap_or_else(|err| err.into_inner());
        assert_eq!(records.len(), 2);
        assert!(matches!(records[0].status, ToolCallStatus::Pending));
        assert!(matches!(records[1].status, ToolCallStatus::Error));
    }

    #[tokio::test]
    async fn native_task_validation_runs_after_approval_without_coercing_batch() {
        struct TaskExecutor(Mutex<Vec<Value>>);
        impl ToolExecutor for TaskExecutor {
            fn call<'a>(
                &'a self, _progress: &'a ToolProgress<'a>,
                _ctx: &'a ToolExecutionContext<'a>,
                tool: &'a ChatToolDefinition,
                arguments: Value,
            ) -> ToolExecutorFuture<'a> {
                self.0.lock().unwrap().push(arguments);
                Box::pin(async {
                    crate::chat::tool_output::from_mcp(tool, crate::mcp::types::McpToolCallResult {
                        content: "dedicated batch preflight error".into(),
                        is_error: true,
                        ..Default::default()
                    })
                })
            }
        }
        let tool = crate::chat::sub_agent::task::tool_definition(&[], &Settings::default());
        let arguments = serde_json::json!({
            "context": 12, "agent": "task", "tasks": [{"task": "work", "solutionSpace": 5}]
        });
        let mut settings = Settings::default();
        settings.chat_tools.approval_policy = "always_confirm".into();
        let executor = TaskExecutor(Mutex::new(Vec::new()));
        let denied = ExecuteTestHost { deny_approval: true, ..Default::default() };
        let (record, _, _) = execute_tool_call(
            &denied, &executor, &settings, &test_execution_context(), &tool,
            test_pending_call("denied", "task", arguments.clone()),
        ).await;
        assert!(matches!(record.status, ToolCallStatus::Skipped));
        assert_eq!(denied.approvals.load(Ordering::SeqCst), 1);
        assert!(executor.0.lock().unwrap().is_empty());

        let allowed = ExecuteTestHost::default();
        let (record, content, _) = execute_tool_call(
            &allowed, &executor, &settings, &test_execution_context(), &tool,
            test_pending_call("allowed", "task", arguments.clone()),
        ).await;
        assert!(matches!(record.status, ToolCallStatus::Error));
        assert!(content.contains("dedicated batch preflight error"));
        assert_eq!(allowed.approvals.load(Ordering::SeqCst), 1);
        assert_eq!(*executor.0.lock().unwrap(), vec![arguments.clone()]);

        let mut remote = tool.clone();
        remote.source = "mcp".into();
        let (record, content, _) = execute_tool_call(
            &allowed, &executor, &settings, &test_execution_context(), &remote,
            test_pending_call("remote", "task", arguments),
        ).await;
        assert!(matches!(record.status, ToolCallStatus::Error));
        assert!(content.contains("schema validation"));
        assert_eq!(executor.0.lock().unwrap().len(), 1);
    }

    fn host_with_hook(dir: &std::path::Path, event: &str, output: Value) -> ExecuteTestHost {
        let file = dir.join("result.json");
        std::fs::write(&file, output.to_string()).unwrap();
        // Use the production shell choice; Git Bash and sh both support cat.
        // On Windows PowerShell also aliases cat to Get-Content.
        let command = format!("cat \"{}\"", file.to_string_lossy().replace('\\', "/"));
        let hooks = crate::chat::workflow_hooks::parse(
            &serde_json::json!({(event):[{"hooks":[{"type":"command","command":command}]}]}),
            "test",
            dir,
            dir,
        )
        .unwrap();
        ExecuteTestHost {
            workflow: Some(crate::chat::workflow_hooks::Runtime {
                hooks,
                cwd: dir.into(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn workflow_hook_denial_never_reaches_executor() {
        let dir = tempfile::tempdir().unwrap();
        let host = host_with_hook(
            dir.path(),
            "PreToolUse",
            serde_json::json!({"hookSpecificOutput":{"permissionDecision":"deny","permissionDecisionReason":"protected"}}),
        );
        let executor = ExecuteTestExecutor::default();
        let (record, _, _) = execute_tool_call(
            &host,
            &executor,
            &Settings::default(),
            &test_execution_context(),
            &sensitive_test_tool(),
            test_pending_call(
                "c",
                "write",
                serde_json::json!({"path":"file","content":"text"}),
            ),
        )
        .await;
        assert!(matches!(record.status, ToolCallStatus::Skipped));
        assert_eq!(record.error.as_deref(), Some("protected"));
        assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
        assert_eq!(host.approvals.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn workflow_hook_errors_fail_with_source_and_preserve_executed_tool_results() {
        for (event, hook, calls) in [("PreToolUse", "before_tool_call", 0), ("PostToolUse", "after_tool_call", 1)] {
            let dir = tempfile::tempdir().unwrap();
            let host = host_with_hook(dir.path(), event, serde_json::json!({"hookSpecificOutput":{"hookEventName":"invalid"}}));
            let executor = ExecuteTestExecutor { structured_content: Some(serde_json::json!({"raw":true})), ..Default::default() };
            let mut settings = Settings::default();
            settings.chat_tools.approval_policy = "auto".into();
            let result = execute_tool_result(&host, &executor, &settings, &test_execution_context(), &sensitive_test_tool(), test_pending_call("c", "write", serde_json::json!({"path":"file","content":"text"}))).await;
            assert!(matches!(result.failure, Some(kivio_agent_sdk::agent::AgentFailure::Hook { hook: actual, .. }) if actual == hook));
            assert_eq!(executor.calls.load(Ordering::SeqCst), calls);
            if calls == 1 {
                assert_eq!(result.record.status, AgentToolStatus::Success);
                assert_eq!(result.content, "result:write");
                assert_eq!(result.record.output.as_deref(), Some("result:write"));
                assert_eq!(result.record.structured_content, Some(serde_json::json!({"raw":true})));
            } else {
                assert_eq!(result.record.status, AgentToolStatus::Skipped);
            }
        }
    }

    #[tokio::test]
    async fn workflow_hook_changes_are_validated_before_approval() {
        let dir = tempfile::tempdir().unwrap();
        let host = host_with_hook(
            dir.path(),
            "PreToolUse",
            serde_json::json!({"hookSpecificOutput":{"permissionDecision":"allow","updatedInput":{"content":"missing path"}}}),
        );
        let executor = ExecuteTestExecutor::default();
        let (record, _, _) = execute_tool_call(
            &host,
            &executor,
            &Settings::default(),
            &test_execution_context(),
            &sensitive_test_tool(),
            test_pending_call(
                "c",
                "write",
                serde_json::json!({"path":"file","content":"text"}),
            ),
        )
        .await;
        assert!(matches!(record.status, ToolCallStatus::Error));
        assert!(record.error.unwrap().contains("path"));
        assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
        assert_eq!(host.approvals.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn workflow_hook_allow_cannot_bypass_host_consent() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = host_with_hook(
            dir.path(),
            "PreToolUse",
            serde_json::json!({"hookSpecificOutput":{"permissionDecision":"allow","updatedInput":{"path":"changed","content":"text"}}}),
        );
        host.deny_consent = true;
        let executor = ExecuteTestExecutor::default();
        let mut settings = Settings::default();
        settings.chat_tools.approval_policy = "readonly_auto_sensitive_confirm".into();
        let (record, _, _) = execute_tool_call(
            &host,
            &executor,
            &settings,
            &test_execution_context(),
            &sensitive_test_tool(),
            test_pending_call(
                "c",
                "write",
                serde_json::json!({"path":"file","content":"text"}),
            ),
        )
        .await;
        assert!(matches!(record.status, ToolCallStatus::Skipped));
        assert_eq!(
            serde_json::from_str::<Value>(&record.arguments).unwrap()["path"],
            "changed"
        );
        assert_eq!(host.consents.load(Ordering::SeqCst), 1);
        assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn parallel_answer_approval_never_executes_sensitive_or_remote_tools_implicitly() {
        let host = ExecuteTestHost {
            parallel_arm: true,
            ..Default::default()
        };
        let executor = ExecuteTestExecutor::default();
        let mut settings = Settings::default();
        let tool = sensitive_test_tool();
        let call = || {
            test_pending_call(
                "c",
                "write",
                serde_json::json!({"path":"file","content":"text"}),
            )
        };
        let (record, feedback, _) = execute_tool_call(
            &host,
            &executor,
            &settings,
            &test_execution_context(),
            &tool,
            call(),
        )
        .await;
        assert!(matches!(record.status, ToolCallStatus::Skipped));
        assert!(feedback.contains("single-model"));
        assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
        assert_eq!(host.approvals.load(Ordering::SeqCst), 0);
        let mut remote = tool.clone();
        remote.source = "mcp".into();
        remote.sensitive = false;
        settings.chat_tools.approval_policy = "readonly_auto_sensitive_confirm".into();
        let (record, _, _) = execute_tool_call(
            &host,
            &executor,
            &settings,
            &test_execution_context(),
            &remote,
            call(),
        )
        .await;
        assert!(matches!(record.status, ToolCallStatus::Skipped));
        assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
        settings.chat_tools.approval_policy = "auto".into();
        let (record, _, _) = execute_tool_call(
            &host,
            &executor,
            &settings,
            &test_execution_context(),
            &tool,
            call(),
        )
        .await;
        assert!(matches!(record.status, ToolCallStatus::Success));
        assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn workflow_hook_ask_requires_approval_even_under_auto_policy() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = host_with_hook(
            dir.path(),
            "PreToolUse",
            serde_json::json!({"hookSpecificOutput":{"permissionDecision":"ask","permissionDecisionReason":"review"}}),
        );
        let executor = ExecuteTestExecutor::default();
        let mut settings = Settings::default();
        settings.chat_tools.approval_policy = "auto".into();
        let (record, _, _) = execute_tool_call(
            &host,
            &executor,
            &settings,
            &test_execution_context(),
            &sensitive_test_tool(),
            test_pending_call(
                "c",
                "write",
                serde_json::json!({"path":"file","content":"text"}),
            ),
        )
        .await;
        assert!(matches!(record.status, ToolCallStatus::Success));
        assert_eq!(host.approvals.load(Ordering::SeqCst), 1);
        assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
        host.parallel_arm = true;
        let (record, _, _) = execute_tool_call(
            &host,
            &executor,
            &settings,
            &test_execution_context(),
            &sensitive_test_tool(),
            test_pending_call(
                "other",
                "write",
                serde_json::json!({"path":"file","content":"text"}),
            ),
        )
        .await;
        assert!(matches!(record.status, ToolCallStatus::Skipped));
        assert_eq!(host.approvals.load(Ordering::SeqCst), 1);
        assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn schema_validation_rejects_additional_properties_without_declared_properties() {
        let mut tool = sensitive_test_tool();
        tool.input_schema = serde_json::json!({
            "type": "object",
            "additionalProperties": false
        });

        let err = validate_tool_arguments(&tool, &serde_json::json!({ "extra": true }))
            .expect_err("extra properties should be rejected");

        assert!(err.contains("arguments.extra is not allowed"));
    }

    #[tokio::test]
    async fn successful_tool_records_trace_and_structured_content() {
        let host = ExecuteTestHost::default();
        let executor = ExecuteTestExecutor {
            calls: AtomicUsize::new(0),
            structured_content: Some(serde_json::json!({ "answer": 42 })),
        };
        let settings = Settings::default();
        let mut tool = sensitive_test_tool();
        tool.sensitive = false;
        let call = test_pending_call(
            "call_ok",
            "write",
            serde_json::json!({ "path": "/tmp/out.txt", "content": "hello" }),
        );

        let (record, content, _) = execute_tool_call(
            &host,
            &executor,
            &settings,
            &test_execution_context(),
            &tool,
            call,
        )
        .await;

        assert!(matches!(record.status, ToolCallStatus::Success));
        assert_eq!(record.trace_id.as_deref(), Some("run"));
        assert_eq!(record.span_id.as_deref(), Some("tool_round_2_call_ok"));
        assert_eq!(
            record.structured_content.as_ref(),
            Some(&serde_json::json!({ "answer": 42 }))
        );
        // native 工具的 content 是为模型格式化的文本，不再追加整包 structuredContent
        // JSON（record/前端仍持有完整 structured_content）。MCP 契约见下一个测试。
        assert!(!content.contains("structuredContent"));
        assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn file_tool_skipped_when_session_consent_denied() {
        let host = ExecuteTestHost {
            deny_consent: true,
            ..Default::default()
        };
        let executor = ExecuteTestExecutor::default();
        let mut settings = Settings::default();
        // Exercise the per-conversation consent gate (default policy is now "auto",
        // which bypasses it).
        settings.chat_tools.approval_policy = "readonly_auto_sensitive_confirm".to_string();
        let tool = sensitive_test_tool(); // native "write" — consent-gated
        let call = test_pending_call(
            "call_write",
            "write",
            serde_json::json!({ "path": "/tmp/out.txt", "content": "hi" }),
        );

        let (record, _content, _) = execute_tool_call(
            &host,
            &executor,
            &settings,
            &test_execution_context(),
            &tool,
            call,
        )
        .await;

        assert!(matches!(record.status, ToolCallStatus::Skipped));
        assert_eq!(host.consents.load(Ordering::SeqCst), 1);
        // No per-call approval under default policy, and the tool never ran.
        assert_eq!(host.approvals.load(Ordering::SeqCst), 0);
        assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn file_tool_uses_session_consent_not_per_call_approval() {
        let host = ExecuteTestHost::default(); // grants consent
        let executor = ExecuteTestExecutor::default();
        let mut settings = Settings::default();
        // Per-conversation consent policy (default is now "auto", which bypasses it).
        settings.chat_tools.approval_policy = "readonly_auto_sensitive_confirm".to_string();
        let tool = sensitive_test_tool();
        let call = test_pending_call(
            "call_write",
            "write",
            serde_json::json!({ "path": "/tmp/out.txt", "content": "hi" }),
        );

        let (record, _content, _) = execute_tool_call(
            &host,
            &executor,
            &settings,
            &test_execution_context(),
            &tool,
            call,
        )
        .await;

        assert!(matches!(record.status, ToolCallStatus::Success));
        assert_eq!(host.consents.load(Ordering::SeqCst), 1);
        assert_eq!(host.approvals.load(Ordering::SeqCst), 0);
        assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn task_obeys_session_consent_before_execution() {
        for denied in [true, false] {
            let host = ExecuteTestHost {
                deny_consent: denied,
                ..Default::default()
            };
            let executor = ExecuteTestExecutor::default();
            let mut settings = Settings::default();
            settings.chat_tools.approval_policy = "readonly_auto_sensitive_confirm".into();
            let tool = crate::chat::sub_agent::task::tool_definition(&[], &settings);
            let call = test_pending_call("call_task", "task", serde_json::json!({
                "context": "shared",
                "tasks": [{"task": "work", "solutionSpace": "one fix"}]
            }));
            let (record, content, _) = execute_tool_call(
                &host, &executor, &settings, &test_execution_context(), &tool, call,
            ).await;
            assert_eq!(host.consents.load(Ordering::SeqCst), 1);
            assert_eq!(host.approvals.load(Ordering::SeqCst), 0);
            assert_eq!(executor.calls.load(Ordering::SeqCst), usize::from(!denied));
            if denied {
                assert!(matches!(record.status, ToolCallStatus::Skipped));
                assert!(!content.is_empty());
            } else {
                assert!(matches!(record.status, ToolCallStatus::Success));
                assert_eq!(content, "result:task");
            }
        }
    }

    /// MCP structured-content 契约（agent-runtime spec）：MCP 结果的 structuredContent
    /// 必须进入模型可见文本（除非文本已含同样 JSON）。
    #[tokio::test]
    async fn mcp_tool_content_still_carries_structured_content() {
        let host = ExecuteTestHost::default();
        let executor = ExecuteTestExecutor {
            calls: AtomicUsize::new(0),
            structured_content: Some(serde_json::json!({ "answer": 42 })),
        };
        let settings = Settings::default();
        let mut tool = sensitive_test_tool();
        tool.sensitive = false;
        tool.source = "mcp".to_string();
        tool.server_id = Some("server-1".to_string());
        let call = test_pending_call(
            "call_mcp",
            "write",
            serde_json::json!({ "path": "/tmp/out.txt", "content": "hello" }),
        );

        let (record, content, _) = execute_tool_call(
            &host,
            &executor,
            &settings,
            &test_execution_context(),
            &tool,
            call,
        )
        .await;

        assert!(matches!(record.status, ToolCallStatus::Success));
        assert!(content.contains("structuredContent"));
        assert!(content.contains("\"answer\":42"));
    }

    #[test]
    fn untrusted_mcp_read_only_hint_does_not_bypass_approval() {
        let mut settings = Settings::default();
        settings.chat_tools.approval_policy = "readonly_auto_sensitive_confirm".to_string();
        let mut tool = sensitive_test_tool();
        tool.source = "mcp".to_string();
        tool.sensitive = false;
        tool.annotations = Some(serde_json::json!({ "readOnlyHint": true }));

        assert!(tool_requires_approval(&settings, &tool));
        settings.chat_tools.approval_policy = "auto".to_string();
        assert!(!tool_requires_approval(&settings, &tool));
    }

    #[test]
    fn native_skill_tools_match_by_openai_name() {
        let tool = native_skill_activate_tool();
        // 现名 skill 精确命中，旧名 skill_activate 经 canonical alias 归一化命中。
        assert!(match_tool_call(std::slice::from_ref(&tool), "skill").is_some());
        assert!(match_tool_call(&[tool], "skill_activate").is_some());
    }

    #[test]
    fn format_tool_result_preview_summarizes_tavily_search_json() {
        let raw = r#"stdout:
{
  "answer": null,
  "query": "吉林市 明天 天气",
  "results": [
    {
      "title": "吉林市天气预报",
      "content": "明天晴有时多云，最高33℃"
    }
  ]
}"#;
        let preview = crate::chat::tools::summarize_tool_text(raw);
        assert!(preview.contains("1 条结果"));
        assert!(preview.contains("吉林市天气预报"));
        assert!(preview.contains("33"));
        assert!(!preview.contains("\"answer\": null"));
    }

    #[test]
    fn truncate_tool_content_for_model_marks_truncated_output() {
        // 头+尾保留：12 字符预算 → 头 6 + 尾 3，中间替换为截断说明。
        let content = "HEADxxOMITTEDyyTAIL_abc";
        let truncated = limit_tool_text_for_model(content, Some(12));

        assert!(truncated.starts_with("HEADxx"));
        assert!(truncated.ends_with("abc"));
        assert!(truncated.contains("Tool output truncated"));
        assert!(truncated.contains("original 23 chars"));
        assert!(truncated.contains("first 6 and last 3"));
        assert!(!truncated.contains("OMITTEDyy"));
    }

    #[test]
    fn truncate_tool_content_for_model_keeps_short_output_unchanged() {
        assert_eq!(limit_tool_text_for_model("abc", Some(3)), "abc");
    }

    #[test]
    fn mixer_image_generation_uses_extended_timeout() {
        let mut settings = Settings::default();
        settings.chat_tools.tool_timeout_ms = 60_000;
        let tool = crate::mcp::types::mixer_generate_image_tool();
        let arguments = serde_json::json!({ "prompt": "draw a quiet desktop assistant" });

        assert_eq!(
            effective_tool_timeout_ms(&settings, &tool, &arguments),
            crate::chat::image_generation::IMAGE_GENERATION_TIMEOUT_MS
        );
    }

    #[test]
    fn voice_orchestration_wait_owns_its_timeout() {
        let mut tool = crate::mcp::types::native_save_assistant_tool();
        tool.name = "kivio_send_message".into();
        assert_eq!(
            effective_tool_timeout_ms(
                &Settings::default(),
                &tool,
                &serde_json::json!({"wait": true})
            ),
            NO_OUTER_TOOL_TIMEOUT
        );
    }

    #[test]
    fn native_subagent_wait_owns_its_timeout() {
        let mut settings = Settings::default();
        settings.chat_tools.tool_timeout_ms = 1_000;
        let mut tool = crate::chat::sub_agent::wait::definition();
        let arguments = serde_json::json!({});

        assert_eq!(
            effective_tool_timeout_ms(&settings, &tool, &arguments),
            NO_OUTER_TOOL_TIMEOUT
        );
        tool.source = "mcp".into();
        assert_eq!(
            effective_tool_timeout_ms(&settings, &tool, &arguments),
            1_000
        );
    }

    #[tokio::test]
    async fn blocking_task_is_not_cut_off_by_the_ordinary_tool_timeout() {
        struct SlowTask;
        impl ToolExecutor for SlowTask {
            fn call<'a>(
                &'a self, _progress: &'a ToolProgress<'a>, _ctx: &'a ToolExecutionContext<'a>, tool: &'a ChatToolDefinition,
                _arguments: Value,
            ) -> ToolExecutorFuture<'a> {
                Box::pin(async {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    crate::chat::tool_output::from_mcp(tool, crate::mcp::types::McpToolCallResult { content: "completed child result".into(), ..Default::default() })
                })
            }
        }
        let mut settings = Settings::default();
        settings.chat_tools.approval_policy = "auto".into();
        settings.chat_tools.tool_timeout_ms = 1;
        let tool = crate::chat::sub_agent::task::tool_definition(&[], &settings);
        let call = test_pending_call("blocking", "task", serde_json::json!({
            "context": "shared", "tasks": [{"task": "work", "solutionSpace": "one fix"}]
        }));
        let (record, content, _) = execute_tool_call(
            &ExecuteTestHost::default(), &SlowTask, &settings, &test_execution_context(), &tool, call,
        ).await;
        assert!(matches!(record.status, ToolCallStatus::Success), "{content}");
        assert_eq!(content, "completed child result");
    }

    #[test]
    fn automation_run_uses_requested_timeout_plus_buffer() {
        let mut settings = Settings::default();
        settings.chat_tools.tool_timeout_ms = 60_000;
        let tool = crate::mcp::types::native_automation_run_tool();
        let arguments = serde_json::json!({ "id": "a1", "timeout_seconds": 120 });
        assert_eq!(
            effective_tool_timeout_ms(&settings, &tool, &arguments),
            125_000
        );
    }

    #[test]
    fn bash_skips_generic_outer_tool_timeout() {
        let mut settings = Settings::default();
        settings.chat_tools.tool_timeout_ms = 60_000;
        let bash = crate::mcp::types::native_run_command_tool();

        assert_eq!(
            effective_tool_timeout_ms(&settings, &bash, &serde_json::json!({})),
            u64::MAX
        );
        assert_eq!(
            effective_tool_timeout_ms(
                &settings,
                &bash,
                &serde_json::json!({ "timeout_ms": 5_000 })
            ),
            u64::MAX
        );
    }

    #[test]
    fn bash_output_timeout_covers_requested_wait() {
        let mut settings = Settings::default();
        settings.chat_tools.tool_timeout_ms = 60_000;
        let tool = crate::mcp::types::native_bash_output_tool();

        assert_eq!(
            effective_tool_timeout_ms(&settings, &tool, &serde_json::json!({})),
            60_000
        );
        assert_eq!(
            effective_tool_timeout_ms(&settings, &tool, &serde_json::json!({ "wait_ms": 300_000 })),
            300_000
        );
    }

    #[test]
    fn mcp_tool_timeout_follows_settings_like_other_tools() {
        let mut settings = Settings::default();
        settings.chat_tools.tool_timeout_ms = 300_000;
        let tool = ChatToolDefinition {
            id: "mcp__plugin-officecli__officecli".into(),
            name: "officecli".into(),
            description: String::new(),
            source: "mcp".into(),
            server_id: Some("plugin-officecli".into()),
            server_name: Some("OfficeCLI".into()),
            input_schema: serde_json::json!({}),
            sensitive: false,
            annotations: None,
            output_schema: None,
        };
        assert_eq!(
            effective_tool_timeout_ms(
                &settings,
                &tool,
                &serde_json::json!({ "command": "batch …" })
            ),
            300_000
        );
    }

    #[test]
    fn tool_timeout_error_mentions_tool_and_actionable_hint() {
        let tool = ChatToolDefinition {
            id: "mcp__plugin-officecli__officecli".into(),
            name: "officecli".into(),
            description: String::new(),
            source: "mcp".into(),
            server_id: Some("plugin-officecli".into()),
            server_name: Some("OfficeCLI".into()),
            input_schema: serde_json::json!({}),
            sensitive: false,
            annotations: None,
            output_schema: None,
        };
        let err = format_tool_timeout_error(
            &tool,
            300_000,
            &serde_json::json!({
                "command": "batch \"deck.pptx\" --commands \"[{...}]\""
            }),
        );
        assert!(err.contains("超时"));
        assert!(err.contains("officecli"));
        assert!(err.contains("300") || err.contains("300s") || err.contains("300.0"));
        assert!(err.contains("勿原样重试") || err.contains("缩小"));
    }

    #[test]
    fn coercion_fixes_string_number_and_bool() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "offset": { "type": "integer" },
                "ratio": { "type": "number" },
                "recursive": { "type": "boolean" },
                "name": { "type": "string" }
            }
        });
        let coerced = coerce_tool_arguments(
            &schema,
            serde_json::json!({
                "offset": "5",
                "ratio": "0.5",
                "recursive": "true",
                "name": 42
            }),
        );
        assert_eq!(
            coerced,
            serde_json::json!({ "offset": 5, "ratio": 0.5, "recursive": true, "name": "42" })
        );
        // coercion 后必须能过既有校验（这就是它存在的意义）。
        assert!(crate::chat::tools::schema::validate_arguments(&schema, &coerced).is_ok());
    }

    #[test]
    fn coercion_drops_null_optional_properties_instead_of_inventing_defaults() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "domain_type": { "type": "string" },
                "pub_year_min": { "type": "integer" },
                "pub_year_max": { "type": "integer" },
                "nullable_note": { "type": ["string", "null"] }
            },
            "required": ["query"]
        });
        let coerced = coerce_tool_arguments(
            &schema,
            serde_json::json!({
                "query": "Kivio",
                "domain_type": "web",
                "pub_year_min": null,
                "pub_year_max": null,
                "nullable_note": null
            }),
        );
        assert_eq!(
            coerced,
            serde_json::json!({
                "query": "Kivio",
                "domain_type": "web",
                "nullable_note": null
            })
        );
    }

    #[test]
    fn coercion_does_not_turn_required_null_into_a_zero_value() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "count": { "type": "integer" } },
            "required": ["count"]
        });
        let coerced = coerce_tool_arguments(&schema, serde_json::json!({ "count": null }));
        assert_eq!(coerced, serde_json::json!({ "count": null }));
        assert!(crate::chat::tools::schema::validate_arguments(&schema, &coerced).is_err());
    }

    #[test]
    fn coercion_recurses_into_arrays_and_nested_objects() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "edits": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": { "line": { "type": "integer" } }
                    }
                }
            }
        });
        let coerced = coerce_tool_arguments(
            &schema,
            serde_json::json!({ "edits": [ { "line": "12" }, { "line": 3 } ] }),
        );
        assert_eq!(
            coerced,
            serde_json::json!({ "edits": [ { "line": 12 }, { "line": 3 } ] })
        );
    }

    #[test]
    fn coercion_leaves_valid_and_hopeless_values_alone() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "count": { "type": "integer" } }
        });
        // 已合法：原样。
        let valid = serde_json::json!({ "count": 7 });
        assert_eq!(coerce_tool_arguments(&schema, valid.clone()), valid);
        // 掰不动的（非数字字符串）：原样保留，交给校验去报错。
        let hopeless = serde_json::json!({ "count": "many" });
        assert_eq!(coerce_tool_arguments(&schema, hopeless.clone()), hopeless);
        assert!(crate::chat::tools::schema::validate_arguments(&schema, &hopeless).is_err());
    }

    #[test]
    fn coercion_resolves_any_of_by_member_validation() {
        let schema = serde_json::json!({
            "anyOf": [
                { "type": "object", "properties": { "path": { "type": "string" } }, "required": ["path"] },
                { "type": "integer" }
            ]
        });
        // 原值已匹配成员一：不动。
        let object = serde_json::json!({ "path": "a.rs" });
        assert_eq!(coerce_tool_arguments(&schema, object.clone()), object);
        // 字符串数字：coerce 后命中成员二。
        assert_eq!(
            coerce_tool_arguments(&schema, serde_json::json!("12")),
            serde_json::json!(12)
        );
    }

impl AgentRunControl for ExecuteTestHost {
    fn active(&self) -> bool { !self.stopped.load(Ordering::SeqCst) }
    fn stop(&self) -> StopAdmission { self.stopped.store(true, Ordering::SeqCst); StopAdmission::Requested }
    fn submit(&self, _kind: InputKind, _message: SteeringMessage) -> InputAdmission { InputAdmission::Unsupported }
    fn take(&self, _kind: InputKind) -> Vec<SteeringMessage> { Vec::new() }
    fn wait_inactive(&self) -> AgentHostFuture<'_, ()> { Box::pin(std::future::pending()) }
}
impl AgentEventSink for ExecuteTestHost {
    fn on_event(&self, event: &AgentEvent<'_>) -> EventDelivery {
        let id = event.identity;
        match &event.kind {
            AgentEventKind::Content { delta, reasoning_delta, block } => {
                let segment = block.map(crate::chat::agent_presentation::project_block);
                self.emit_stream_delta(id.conversation_id, id.run_id, id.message_id, delta, *reasoning_delta, segment.as_ref());
                if let Some(record) = block.and_then(crate::chat::agent_presentation::project_embedded) { self.emit_tool_record(id.conversation_id, id.run_id, id.message_id, &record); }
            }
            AgentEventKind::SearchUpdated { block, announce } => {
                if *announce {
                    self.emit_stream_delta(id.conversation_id, id.run_id, id.message_id, "", None, Some(&crate::chat::agent_presentation::project_block(block)));
                }
                if let Some(record) = crate::chat::agent_presentation::project_embedded(block) { self.emit_tool_record(id.conversation_id, id.run_id, id.message_id, &record); }
            }
            AgentEventKind::ToolUpdated(call) => self.emit_tool_record(id.conversation_id, id.run_id, id.message_id, &crate::chat::agent_presentation::project_tool(call, Some(id.run_id))),
            _ => {}
        }
        EventDelivery::Accepted
    }
}
fn test_context(host: &ExecuteTestHost) -> AgentContext<'_> {
    let identity = AgentRunIdentity { conversation_id: "conversation", run_id: "run", message_id: "message", generation: 1 };
    let control = AgentRunHandle::new(identity, host);
    AgentContext::new(host, host, control)
}

async fn execute_tool_call(host: &ExecuteTestHost, executor: &dyn ToolExecutor, settings: &Settings, ctx: &ToolExecutionContext<'_>, tool: &ChatToolDefinition, call: PendingToolCall) -> (ToolCallRecord, String, Vec<crate::chat::model::ModelMessage>) {
    let result = execute_tool_result(host, executor, settings, ctx, tool, call).await;
    (crate::chat::agent_presentation::project_tool(&result.record, Some(ctx.run_id)), result.content, result.follow_up_messages)
}

async fn execute_tool_result(host: &ExecuteTestHost, executor: &dyn ToolExecutor, settings: &Settings, ctx: &ToolExecutionContext<'_>, tool: &ChatToolDefinition, call: PendingToolCall) -> crate::chat::agent::execute::ToolCallResult {
    let context = test_context(host);
    let tools = std::slice::from_ref(tool);
    let adapter = ApplicationToolExecutor { settings, authority: ToolAuthority::from_context(ctx), tools, host, dispatcher: executor };
    let core_ctx = crate::chat::agent::execute::ToolExecutionContext {
        conversation_id: ctx.conversation_id, run_id: ctx.run_id, message_id: ctx.message_id,
        generation: ctx.generation, round: ctx.round, tool_call_id: ctx.tool_call_id,
    };
    crate::chat::agent::execute::execute_tool_call(&context, &adapter, &core_ctx, &tool.for_run(settings), call).await
}

fn match_tool_call<'a>(tools: &'a [ChatToolDefinition], name: &str) -> Option<&'a ChatToolDefinition> {
    let definitions: Vec<_> = tools.iter().map(|tool| tool.for_run(&Settings::default())).collect();
    let found = crate::chat::agent::execute::match_tool_call(&definitions, name)?;
    tools.iter().find(|tool| tool.id == found.key)
}

    #[tokio::test]
    async fn stopped_host_discards_ordinary_completion_but_drains_mcp_and_required_completion() {
        struct StoppingExecutor;
        impl ToolExecutor for StoppingExecutor {
            fn call<'a>(&'a self, progress: &'a ToolProgress<'a>, _ctx: &'a ToolExecutionContext<'a>, _tool: &'a ChatToolDefinition, _arguments: Value) -> ToolExecutorFuture<'a> {
                Box::pin(async move {
                    progress.control.stop();
                    progress.control.wait_inactive().await;
                    tokio::task::yield_now().await;
                    Ok(crate::chat::tools::ToolOutput::text("late result"))
                })
            }
        }
        for (finish_tool, mcp) in [(false, false), (true, false), (false, true)] {
            let host = ExecuteTestHost { finish_tool, ..Default::default() };
            let mut tool = named_test_tool("echo");
            if mcp {
                tool.source = "mcp".into();
            }
            let mut settings = Settings::default();
            settings.chat_tools.approval_policy = "auto".into();
            let name = tool.openai_tool_name();
            let (record, content, follow_ups) = execute_tool_call(&host, &StoppingExecutor, &settings, &test_execution_context(), &tool, test_pending_call("call", &name, serde_json::json!({}))).await;
            let drained = finish_tool || mcp;
            assert!(host.stopped.load(Ordering::SeqCst), "dispatcher must use the issuing run control");
            assert_eq!(record.status, if drained { ToolCallStatus::Success } else { ToolCallStatus::Cancelled });
            assert_eq!(record.result_preview.as_deref(), drained.then_some("late result"));
            assert_eq!(content, if drained { "late result" } else { "Tool call cancelled" });
            assert!(follow_ups.is_empty());
        }
    }

}
