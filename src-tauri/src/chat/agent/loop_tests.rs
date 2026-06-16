    use std::io::{Read as _, Write as _};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    };

    use tokio::time::{sleep, Duration};

    use super::*;
    use crate::chat::agent::types::AgentRunEntry;
    use crate::chat::model::{StreamPart, StreamSink as _};
    use crate::chat::types::ToolCallStatus;
    use crate::mcp::types::{
        native_read_file_tool, native_run_python_tool, native_web_fetch_tool,
        native_write_file_tool, McpToolCallResult,
    };
    use crate::settings::{ChatToolsConfig, ModelProvider};
    use crate::state::AppState;

    #[derive(Clone, Debug)]
    struct RecordedDelta {
        delta: String,
        reasoning_delta: Option<String>,
        segment: Option<ChatMessageSegment>,
    }

    #[derive(Default)]
    struct TestHost {
        records: Mutex<Vec<ToolCallRecord>>,
        deltas: Mutex<Vec<RecordedDelta>>,
        dones: Mutex<Vec<(String, String)>>,
        /// Per-call snapshot sizes from `persist_partial_assistant`:
        /// `(message_id, tool_records_len, segments_len)`.
        persists: Mutex<Vec<(String, usize, usize)>>,
        cancel_after: Option<Duration>,
        cancel_flag: Arc<AtomicBool>,
        cancel_on_first_text_delta: bool,
    }

    impl TestHost {
        fn cancelling_after(delay: Duration) -> Self {
            Self {
                cancel_after: Some(delay),
                ..Self::default()
            }
        }

        fn with_cancel_flag(cancel_flag: Arc<AtomicBool>) -> Self {
            Self {
                cancel_flag,
                ..Self::default()
            }
        }

        fn cancelling_on_first_text_delta() -> Self {
            Self {
                cancel_on_first_text_delta: true,
                ..Self::default()
            }
        }

        fn recorded_deltas(&self) -> Vec<RecordedDelta> {
            self.deltas
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone()
        }

        fn recorded_dones(&self) -> Vec<(String, String)> {
            self.dones
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone()
        }

        fn recorded_tool_records(&self) -> Vec<ToolCallRecord> {
            self.records
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone()
        }

        fn recorded_persists(&self) -> Vec<(String, usize, usize)> {
            self.persists
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone()
        }
    }

    impl AgentHost for TestHost {
        fn emit_stream_delta(
            &self,
            _conversation_id: &str,
            _run_id: &str,
            _message_id: &str,
            delta: &str,
            reasoning_delta: Option<&str>,
            segment: Option<&ChatMessageSegment>,
        ) {
            if self.cancel_on_first_text_delta && !delta.is_empty() {
                self.cancel_flag.store(true, Ordering::SeqCst);
            }
            self.deltas
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .push(RecordedDelta {
                    delta: delta.to_string(),
                    reasoning_delta: reasoning_delta.map(str::to_string),
                    segment: segment.cloned(),
                });
        }

        fn emit_stream_done(
            &self,
            _conversation_id: &str,
            _run_id: &str,
            _message_id: &str,
            reason: &str,
            full: &str,
        ) {
            self.dones
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .push((reason.to_string(), full.to_string()));
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

        fn persist_partial_assistant(
            &self,
            _conversation_id: &str,
            message_id: &str,
            tool_records: &[ToolCallRecord],
            segments: &[ChatMessageSegment],
        ) {
            self.persists
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .push((message_id.to_string(), tool_records.len(), segments.len()));
        }

        fn request_tool_approval<'a>(
            &'a self,
            _ctx: &'a ToolExecutionContext<'a>,
            _record: &'a ToolCallRecord,
        ) -> super::super::host::AgentHostFuture<'a, bool> {
            Box::pin(async { true })
        }

        fn request_user_response<'a>(
            &'a self,
            _ctx: &'a ToolExecutionContext<'a>,
            _record: &'a ToolCallRecord,
            _prompt: crate::chat::ask_user::AskUserPromptPayload,
        ) -> super::super::host::AgentHostFuture<'a, crate::chat::ask_user::AskUserResponseResult>
        {
            Box::pin(async { crate::chat::ask_user::skipped_response() })
        }

        fn is_generation_active(&self, _conversation_id: &str, _generation: u64) -> bool {
            !self.cancel_flag.load(Ordering::SeqCst)
        }

        fn wait_for_generation_inactive<'a>(
            &'a self,
            _conversation_id: &'a str,
            _generation: u64,
        ) -> super::super::host::AgentHostFuture<'a, ()> {
            let cancel_after = self.cancel_after;
            let cancel_flag = self.cancel_flag.clone();
            Box::pin(async move {
                let started = tokio::time::Instant::now();
                loop {
                    if cancel_flag.load(Ordering::SeqCst) {
                        return;
                    }
                    if let Some(delay) = cancel_after {
                        if started.elapsed() >= delay {
                            return;
                        }
                    }
                    sleep(Duration::from_millis(2)).await;
                }
            })
        }
    }

    #[derive(Default)]
    struct RecordingExecutor {
        active: AtomicUsize,
        max_active: AtomicUsize,
        events: Arc<Mutex<Vec<String>>>,
    }

    impl RecordingExecutor {
        fn max_active(&self) -> usize {
            self.max_active.load(Ordering::SeqCst)
        }

        fn events(&self) -> Vec<String> {
            self.events
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone()
        }
    }

    impl ToolExecutor for RecordingExecutor {
        fn call<'a>(
            &'a self,
            _ctx: &'a ToolExecutionContext<'a>,
            tool: &'a ChatToolDefinition,
            _arguments: Value,
            _skill_cache: Option<&'a mut skills::SkillRunCache>,
        ) -> super::super::execute::ToolExecutorFuture<'a> {
            let name = tool.name.clone();
            let events = self.events.clone();
            Box::pin(async move {
                let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
                self.max_active.fetch_max(active, Ordering::SeqCst);
                events
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .push(format!("start:{name}"));
                sleep(Duration::from_millis(25)).await;
                events
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .push(format!("finish:{name}"));
                self.active.fetch_sub(1, Ordering::SeqCst);
                Ok(McpToolCallResult {
                    content: format!("result:{name}"),
                    is_error: false,
                    raw: Value::Null,
                    artifacts: Vec::new(),
                    structured_content: None,
                })
            })
        }
    }

    /// Tool executor that succeeds immediately and flips the shared cancel flag while
    /// executing, so cancellation deterministically lands between the tool round and
    /// the synthesis request (the executor branch wins the `tokio::select!` because it
    /// completes synchronously on its first poll).
    struct CancelAfterToolExecutor {
        cancel_flag: Arc<AtomicBool>,
    }

    impl ToolExecutor for CancelAfterToolExecutor {
        fn call<'a>(
            &'a self,
            _ctx: &'a ToolExecutionContext<'a>,
            tool: &'a ChatToolDefinition,
            _arguments: Value,
            _skill_cache: Option<&'a mut skills::SkillRunCache>,
        ) -> super::super::execute::ToolExecutorFuture<'a> {
            let name = tool.name.clone();
            let cancel_flag = self.cancel_flag.clone();
            Box::pin(async move {
                cancel_flag.store(true, Ordering::SeqCst);
                Ok(McpToolCallResult {
                    content: format!("result:{name}"),
                    is_error: false,
                    raw: Value::Null,
                    artifacts: Vec::new(),
                    structured_content: None,
                })
            })
        }
    }

    /// Scripted HTTP mock for the OpenAI-compatible chat completions endpoint.
    /// Responses are served in connection-accept order; each response closes (or
    /// deliberately breaks) its connection so reqwest opens a fresh one per request.
    enum MockResponse {
        /// Complete JSON chat completion body.
        Json(String),
        /// SSE stream; each entry becomes one `data: <entry>` event.
        Sse(Vec<String>),
        /// Plain HTTP error status with a JSON body.
        Status(u16, String),
        /// Chunked SSE that drops the connection without the chunked terminator,
        /// producing a reqwest decode error (StreamReadInterrupted).
        SseInterrupt(Vec<String>),
        /// Chunked SSE that writes the given events then keeps the connection open,
        /// simulating a hung provider so cancellation paths can win the select.
        SseThenHang(Vec<String>),
    }

    struct MockModelServer {
        base_url: String,
        captured_bodies: Arc<Mutex<Vec<String>>>,
    }

    impl MockModelServer {
        fn start(responses: Vec<MockResponse>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock model server");
            let addr = listener.local_addr().expect("mock model server addr");
            let captured_bodies = Arc::new(Mutex::new(Vec::new()));
            let captured_for_thread = Arc::clone(&captured_bodies);
            std::thread::spawn(move || {
                for response in responses {
                    let Ok((mut stream, _)) = listener.accept() else {
                        return;
                    };
                    stream
                        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                        .ok();
                    match read_http_request(&mut stream) {
                        Ok(body) => {
                            captured_for_thread
                                .lock()
                                .unwrap_or_else(|err| err.into_inner())
                                .push(body);
                        }
                        Err(_) => continue,
                    }
                    serve_mock_response(stream, response);
                }
            });
            Self {
                base_url: format!("http://{addr}/v1"),
                captured_bodies,
            }
        }

        fn captured_bodies(&self) -> Vec<String> {
            self.captured_bodies
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone()
        }
    }

    /// 读完整 HTTP 请求并返回 body 文本（供测试断言请求内容）。
    fn read_http_request(stream: &mut TcpStream) -> std::io::Result<String> {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        let header_end = loop {
            let n = stream.read(&mut chunk)?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "client closed before request end",
                ));
            }
            buf.extend_from_slice(&chunk[..n]);
            if let Some(pos) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
                break pos + 4;
            }
        };
        let headers = String::from_utf8_lossy(&buf[..header_end]).to_ascii_lowercase();
        let content_length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        while buf.len() < header_end + content_length {
            let n = stream.read(&mut chunk)?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        Ok(String::from_utf8_lossy(&buf[header_end..]).into_owned())
    }

    fn sse_body(events: &[String]) -> String {
        events
            .iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect()
    }

    fn serve_mock_response(mut stream: TcpStream, response: MockResponse) {
        match response {
            MockResponse::Json(body) => {
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
            }
            MockResponse::Status(code, body) => {
                let _ = write!(
                    stream,
                    "HTTP/1.1 {code} Mock Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
            }
            MockResponse::Sse(events) => {
                let body = sse_body(&events);
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
            }
            MockResponse::SseInterrupt(events) => {
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n"
                );
                for event in events {
                    let payload = format!("data: {event}\n\n");
                    let _ = write!(stream, "{:x}\r\n{}\r\n", payload.len(), payload);
                }
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Both);
                return;
            }
            MockResponse::SseThenHang(events) => {
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n"
                );
                for event in events {
                    let payload = format!("data: {event}\n\n");
                    let _ = write!(stream, "{:x}\r\n{}\r\n", payload.len(), payload);
                }
                let _ = stream.flush();
                std::thread::sleep(std::time::Duration::from_secs(5));
                return;
            }
        }
        let _ = stream.flush();
    }

    /// Minimal in-memory AppState for run_agent_loop tests. Settings live only in
    /// memory and usage records go to a unique temp dir, so user settings/providers
    /// are never touched.
    fn test_app_state() -> AppState {
        AppState {
            settings: std::sync::RwLock::new(Settings::default()),
            explain_images: Mutex::new(std::collections::HashMap::new()),
            current_explain_image_id: Mutex::new(None),
            lens_busy: AtomicBool::new(false),
            prev_frontmost_pid_lens: std::sync::atomic::AtomicI32::new(0),
            prev_frontmost_pid_main: std::sync::atomic::AtomicI32::new(0),
            explain_stream_generation: std::sync::atomic::AtomicU64::new(0),
            chat_stream_generations: Mutex::new(std::collections::HashMap::new()),
            chat_active_replies: Mutex::new(std::collections::HashSet::new()),
            pending_chat_tool_approvals: Mutex::new(std::collections::HashMap::new()),
            pending_chat_user_prompts: Mutex::new(std::collections::HashMap::new()),
            pending_python_runs: Mutex::new(std::collections::HashMap::new()),
            chat_create_conversation_lock: Mutex::new(()),
            chat_tool_list_cache: Mutex::new(std::collections::HashMap::new()),
            pending_chat_external_sends: Mutex::new(Vec::new()),
            pending_selection: Mutex::new(None),
            lens_freeze_frame_image_id: Mutex::new(None),
            key_cooldowns: Mutex::new(std::collections::HashMap::new()),
            active_key_idx: Mutex::new(std::collections::HashMap::new()),
            mcp_sessions: tokio::sync::Mutex::new(std::collections::HashMap::new()),
            usage_dir: std::env::temp_dir().join(format!(
                "kivio-agent-loop-test-usage-{}",
                uuid::Uuid::new_v4()
            )),
            http: reqwest::Client::new(),
            #[cfg(target_os = "macos")]
            macos_ocr: crate::macos_ocr::MacOcrClient::disabled(),
            rapidocr: crate::rapidocr::RapidOcrClient::disabled(),
            sub_agents: crate::chat::sub_agent::SubAgentManager::default(),
        }
    }

    fn test_provider(base_url: &str) -> ModelProvider {
        ModelProvider {
            id: "test-provider".to_string(),
            name: "Test Provider".to_string(),
            api_keys: vec!["test-key".to_string()],
            api_key_legacy: None,
            base_url: base_url.to_string(),
            available_models: Vec::new(),
            enabled_models: Vec::new(),
            supports_tools: true,
            enabled: true,
            api_format: "openai_chat".to_string(),
            model_overrides: std::collections::HashMap::new(),
        }
    }

    fn test_run_config<'a>(
        state: &'a AppState,
        base_url: &str,
        stream_enabled: bool,
    ) -> AgentRunConfig<'a> {
        AgentRunConfig {
            entry: AgentRunEntry::Send,
            state,
            conversation_id: "conversation".to_string(),
            tool_conversation_id: "conversation".to_string(),
            depth: 0,
            run_id: "run".to_string(),
            message_id: "message".to_string(),
            generation: 1,
            provider: test_provider(base_url),
            model: "test-model".to_string(),
            runtime_messages: vec![
                serde_json::json!({ "role": "system", "content": "system prompt" }),
                serde_json::json!({ "role": "user", "content": "请读取文件" }),
            ],
            tools: vec![native_read_file_tool()],
            blocked_tool_calls: Vec::new(),
            settings: Settings::default(),
            effective_chat_tools: ChatToolsConfig {
                max_tool_rounds: Some(1),
                ..ChatToolsConfig::default()
            },
            language: "zh-CN".to_string(),
            has_image: false,
            thinking_enabled: false,
            stream_enabled,
            max_output_tokens: 1024,
            retry_attempts: 1,
            skill_registry: skills::SkillRegistry::default(),
            active_skill_id: None,
            active_skill_detail: None,
            assistant_snapshot: None,
            custom_system_prompt: String::new(),
            provider_tools_fallback_system_prompt: String::new(),
        }
    }

    /// Streaming planning step: one `read` tool call, then `[DONE]`.
    fn planning_tool_call_sse_events() -> Vec<String> {
        vec![
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_read","function":{"name":"read","arguments":"{\"path\":\"/tmp/kivio-test.txt\"}"}}]}}]}"#
                .to_string(),
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#.to_string(),
            "[DONE]".to_string(),
        ]
    }

    /// Non-stream planning step: one `read` tool call.
    fn planning_tool_call_json() -> String {
        serde_json::json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "content": Value::Null,
                    "tool_calls": [{
                        "id": "call_read",
                        "type": "function",
                        "function": {
                            "name": "read",
                            "arguments": "{\"path\":\"/tmp/kivio-test.txt\"}"
                        }
                    }]
                }
            }]
        })
        .to_string()
    }

    fn test_round_context() -> ToolRoundContext<'static> {
        ToolRoundContext {
            conversation_id: "conversation",
            run_id: "run",
            message_id: "message",
            generation: 1,
            round: 1,
            depth: 0,
            tool_conversation_id: "conversation",
        }
    }

    fn pending_tool_call(id: &str, function_name: &str) -> PendingToolCall {
        let arguments = test_tool_arguments(function_name);
        PendingToolCall {
            id: id.to_string(),
            function_name: function_name.to_string(),
            arguments_raw: serde_json::to_string(&arguments).expect("serialize test arguments"),
            arguments,
            arguments_parse_error: None,
        }
    }

    fn test_tool_arguments(function_name: &str) -> Value {
        match function_name {
            "read" => serde_json::json!({ "path": "/tmp/kivio-test.txt" }),
            "web_fetch" => serde_json::json!({ "url": "https://example.com" }),
            "run_python" => serde_json::json!({ "code": "print(1)" }),
            "ask_user" => serde_json::json!({
                "questions": [
                    {
                        "id": "scope",
                        "prompt": "Which scope should I use?",
                        "options": [
                            { "id": "mvp", "label": "MVP" },
                            { "id": "full", "label": "Full" }
                        ]
                    }
                ]
            }),
            _ => serde_json::json!({}),
        }
    }

    #[test]
    fn visible_tool_segment_calls_skip_hidden_disabled_builtin_feedback() {
        let tools = vec![native_read_file_tool()];
        let blocked = vec![native_run_python_tool()];
        let calls = vec![
            pending_tool_call("call_read", "read"),
            pending_tool_call("call_blocked", "run_python"),
            pending_tool_call("call_hidden_disabled", "web_search"),
            pending_tool_call("call_unknown", "mcp__server__tool"),
        ];

        let visible = visible_tool_segment_calls(&tools, &blocked, &calls);

        assert_eq!(
            visible
                .iter()
                .map(|call| call.id.as_str())
                .collect::<Vec<_>>(),
            vec!["call_read", "call_blocked", "call_unknown"]
        );
    }

    #[test]
    fn reasoning_segment_order_precedes_text_in_same_step() {
        let mut builder = SegmentBuilder::new();
        let reasoning = builder.reserve(
            ChatMessageSegmentKind::Reasoning,
            ChatMessageSegmentPhase::ToolLoop,
            Some(1),
            Some(1),
            "step_1_reasoning",
        );
        let text = builder.reserve(
            ChatMessageSegmentKind::Text,
            ChatMessageSegmentPhase::ToolLoop,
            Some(1),
            Some(1),
            "step_1_text",
        );

        assert!(reasoning.order < text.order);
    }

    fn test_mcp_tool(name: &str, annotations: Value) -> ChatToolDefinition {
        ChatToolDefinition {
            id: format!("mcp__demo__{name}"),
            name: name.to_string(),
            description: "MCP test tool".to_string(),
            source: "mcp".to_string(),
            server_id: Some("demo".to_string()),
            server_name: Some("Demo".to_string()),
            input_schema: serde_json::json!({ "type": "object", "properties": {} }),
            sensitive: false,
            annotations: Some(annotations),
            output_schema: None,
        }
    }

    fn tool_call_ids(messages: &[Value]) -> Vec<&str> {
        messages
            .iter()
            .filter_map(|message| message.get("tool_call_id").and_then(Value::as_str))
            .collect()
    }

    #[test]
    fn tool_round_limit_reached_only_for_finite_limits_at_boundary() {
        assert!(!tool_round_limit_reached(None, 10));
        assert!(!tool_round_limit_reached(Some(3), 2));
        assert!(tool_round_limit_reached(Some(3), 3));
        assert!(tool_round_limit_reached(Some(3), 4));
    }

    #[tokio::test]
    async fn tool_round_runs_parallel_eligible_tools_concurrently_and_keeps_result_order() {
        let host = TestHost::default();
        let executor = RecordingExecutor::default();
        let settings = Settings::default();
        let tools = vec![native_read_file_tool(), native_web_fetch_tool()];
        let mut skill_cache = skills::SkillRunCache::default();

        let result = execute_tool_round(
            &host,
            &executor,
            &settings,
            test_round_context(),
            &tools,
            &[],
            vec![
                pending_tool_call("call_read", "read"),
                pending_tool_call("call_fetch", "web_fetch"),
            ],
            &mut skill_cache,
        )
        .await;

        assert_eq!(executor.max_active(), 2);
        let events = executor.events();
        let first_finish = events
            .iter()
            .position(|event| event.starts_with("finish:"))
            .expect("finish event");
        assert_eq!(
            first_finish, 2,
            "both calls should start before either finishes"
        );
        assert_eq!(result.response_messages.len(), 2);
        assert_eq!(
            result.response_messages[0]
                .get("tool_call_id")
                .and_then(Value::as_str),
            Some("call_read")
        );
        assert_eq!(
            result.response_messages[1]
                .get("tool_call_id")
                .and_then(Value::as_str),
            Some("call_fetch")
        );
        assert_eq!(result.tool_records.len(), 2);
        assert!(result
            .tool_records
            .iter()
            .all(|record| matches!(record.status, ToolCallStatus::Success)));
    }

    #[test]
    fn write_tools_stay_outside_parallel_whitelist() {
        let mut settings = Settings::default();
        settings.chat_tools.approval_policy = "auto".to_string();
        for tool in [
            crate::mcp::types::native_write_file_tool(),
            crate::mcp::types::native_edit_file_tool(),
        ] {
            assert!(
                !tool_call_parallel_eligible(&settings, &tool),
                "{} must stay serial even when approval is auto",
                tool.name
            );
        }
        assert!(
            tool_call_parallel_eligible(&settings, &native_read_file_tool()),
            "read-only tools remain parallel-eligible"
        );
    }

    #[tokio::test]
    async fn tool_round_runs_read_only_mcp_tools_concurrently() {
        let host = TestHost::default();
        let executor = RecordingExecutor::default();
        let settings = Settings::default();
        let tools = vec![
            test_mcp_tool("search_a", serde_json::json!({ "readOnlyHint": true })),
            test_mcp_tool("search_b", serde_json::json!({ "readOnlyHint": true })),
        ];
        let mut skill_cache = skills::SkillRunCache::default();

        let result = execute_tool_round(
            &host,
            &executor,
            &settings,
            test_round_context(),
            &tools,
            &[],
            vec![
                pending_tool_call("call_mcp_a", "search_a"),
                pending_tool_call("call_mcp_b", "search_b"),
            ],
            &mut skill_cache,
        )
        .await;

        assert_eq!(executor.max_active(), 2);
        assert_eq!(
            tool_call_ids(&result.response_messages),
            vec!["call_mcp_a", "call_mcp_b"]
        );
        assert!(result
            .tool_records
            .iter()
            .all(|record| matches!(record.status, ToolCallStatus::Success)));
    }

    #[tokio::test]
    async fn tool_round_keeps_ask_user_serial_between_parallel_safe_tools() {
        let host = TestHost::default();
        let executor = RecordingExecutor::default();
        let settings = Settings::default();
        let mut tools = vec![native_read_file_tool(), native_web_fetch_tool()];
        crate::chat::ask_user::append_tool_definitions(&mut tools);
        let mut skill_cache = skills::SkillRunCache::default();

        let result = execute_tool_round(
            &host,
            &executor,
            &settings,
            test_round_context(),
            &tools,
            &[],
            vec![
                pending_tool_call("call_read", "read"),
                pending_tool_call("call_ask", "ask_user"),
                pending_tool_call("call_fetch", "web_fetch"),
            ],
            &mut skill_cache,
        )
        .await;

        assert_eq!(executor.max_active(), 1);
        assert_eq!(
            executor.events(),
            vec![
                "start:read",
                "finish:read",
                "start:web_fetch",
                "finish:web_fetch"
            ]
        );
        assert_eq!(
            tool_call_ids(&result.response_messages),
            vec!["call_read", "call_ask", "call_fetch"]
        );
        let ask_user_record = result
            .tool_records
            .iter()
            .find(|record| record.id == "call_ask")
            .expect("ask_user record");
        assert!(matches!(ask_user_record.status, ToolCallStatus::Skipped));
        assert_eq!(
            ask_user_record.name,
            crate::chat::ask_user::ASK_USER_TOOL_NAME
        );
        assert_eq!(ask_user_record.trace_id.as_deref(), Some("run"));
        assert_eq!(
            ask_user_record.span_id.as_deref(),
            Some("tool_round_1_call_ask")
        );
    }

    #[tokio::test]
    async fn tool_round_keeps_destructive_mcp_tools_serial() {
        let host = TestHost::default();
        let executor = RecordingExecutor::default();
        let mut settings = Settings::default();
        settings.chat_tools.approval_policy = "auto".to_string();
        let tools = vec![test_mcp_tool(
            "write_remote",
            serde_json::json!({ "destructiveHint": true }),
        )];
        let mut skill_cache = skills::SkillRunCache::default();

        let result = execute_tool_round(
            &host,
            &executor,
            &settings,
            test_round_context(),
            &tools,
            &[],
            vec![
                pending_tool_call("call_mcp_write_1", "write_remote"),
                pending_tool_call("call_mcp_write_2", "write_remote"),
            ],
            &mut skill_cache,
        )
        .await;

        assert_eq!(executor.max_active(), 1);
        assert_eq!(
            tool_call_ids(&result.response_messages),
            vec!["call_mcp_write_1", "call_mcp_write_2"]
        );
    }

    #[tokio::test]
    async fn tool_round_keeps_open_world_mcp_tools_serial_even_when_read_only() {
        let host = TestHost::default();
        let executor = RecordingExecutor::default();
        let mut settings = Settings::default();
        settings.chat_tools.approval_policy = "auto".to_string();
        let tools = vec![test_mcp_tool(
            "remote_search",
            serde_json::json!({ "readOnlyHint": true, "openWorldHint": true }),
        )];
        let mut skill_cache = skills::SkillRunCache::default();

        let result = execute_tool_round(
            &host,
            &executor,
            &settings,
            test_round_context(),
            &tools,
            &[],
            vec![
                pending_tool_call("call_mcp_remote_1", "remote_search"),
                pending_tool_call("call_mcp_remote_2", "remote_search"),
            ],
            &mut skill_cache,
        )
        .await;

        assert_eq!(executor.max_active(), 1);
        assert_eq!(
            tool_call_ids(&result.response_messages),
            vec!["call_mcp_remote_1", "call_mcp_remote_2"]
        );
    }

    #[tokio::test]
    async fn tool_round_preserves_unknown_and_invalid_call_order() {
        let host = TestHost::default();
        let executor = RecordingExecutor::default();
        let settings = Settings::default();
        let tools = vec![native_read_file_tool(), native_web_fetch_tool()];
        let mut skill_cache = skills::SkillRunCache::default();
        let mut invalid_fetch = pending_tool_call("call_bad_args", "web_fetch");
        invalid_fetch.arguments_parse_error = Some("expected compact object".to_string());

        let result = execute_tool_round(
            &host,
            &executor,
            &settings,
            test_round_context(),
            &tools,
            &[],
            vec![
                pending_tool_call("call_read", "read"),
                pending_tool_call("call_fetch", "web_fetch"),
                pending_tool_call("call_missing", "missing_tool"),
                pending_tool_call("call_read_after_unknown", "read"),
                invalid_fetch,
                pending_tool_call("call_final", "read"),
            ],
            &mut skill_cache,
        )
        .await;

        let error_records = result
            .tool_records
            .iter()
            .filter(|record| matches!(record.status, ToolCallStatus::Error))
            .collect::<Vec<_>>();

        assert_eq!(executor.max_active(), 2);
        assert_eq!(
            tool_call_ids(&result.response_messages),
            vec![
                "call_read",
                "call_fetch",
                "call_missing",
                "call_read_after_unknown",
                "call_bad_args",
                "call_final"
            ]
        );
        assert_eq!(
            result
                .tool_records
                .iter()
                .map(|record| record.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "call_read",
                "call_fetch",
                "call_missing",
                "call_read_after_unknown",
                "call_bad_args",
                "call_final"
            ]
        );
        assert_eq!(
            error_records
                .iter()
                .map(|record| record.id.as_str())
                .collect::<Vec<_>>(),
            vec!["call_missing", "call_bad_args"]
        );
        assert!(error_records
            .iter()
            .all(|record| record.trace_id.as_deref() == Some("run")));
        assert_eq!(
            error_records
                .iter()
                .map(|record| record.span_id.as_deref())
                .collect::<Vec<_>>(),
            vec![
                Some("tool_round_1_call_missing"),
                Some("tool_round_1_call_bad_args")
            ]
        );
        let start_events = executor
            .events()
            .into_iter()
            .filter(|event| event.starts_with("start:"))
            .collect::<Vec<_>>();
        assert_eq!(start_events.len(), 4, "only executable tools should run");
    }

    #[tokio::test]
    async fn tool_round_records_plan_blocked_tool_as_skipped() {
        let host = TestHost::default();
        let executor = RecordingExecutor::default();
        let settings = Settings::default();
        let tools = vec![native_read_file_tool()];
        let blocked = vec![native_run_python_tool()];
        let mut skill_cache = skills::SkillRunCache::default();

        let result = execute_tool_round(
            &host,
            &executor,
            &settings,
            test_round_context(),
            &tools,
            &blocked,
            vec![pending_tool_call("call_py", "run_python")],
            &mut skill_cache,
        )
        .await;

        assert_eq!(executor.max_active(), 0);
        assert_eq!(result.response_messages.len(), 1);
        assert!(result.response_messages[0]
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("blocked in Plan mode"));
        assert_eq!(result.tool_records.len(), 1);
        let record = &result.tool_records[0];
        assert_eq!(record.id, "call_py");
        assert_eq!(record.name, "run_python");
        assert!(matches!(record.status, ToolCallStatus::Skipped));
        assert!(record
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("blocked in Plan mode"));
        assert_eq!(record.trace_id.as_deref(), Some("run"));
        assert_eq!(record.span_id.as_deref(), Some("tool_round_1_call_py"));
    }

    #[tokio::test]
    async fn tool_round_cancels_unstarted_calls_after_running_tool_is_cancelled() {
        let host = TestHost::cancelling_after(Duration::from_millis(5));
        let executor = RecordingExecutor::default();
        let settings = Settings::default();
        let tools = vec![native_read_file_tool(), native_run_python_tool()];
        let mut skill_cache = skills::SkillRunCache::default();

        let result = execute_tool_round(
            &host,
            &executor,
            &settings,
            test_round_context(),
            &tools,
            &[],
            vec![
                pending_tool_call("call_read", "read"),
                pending_tool_call("call_py", "run_python"),
            ],
            &mut skill_cache,
        )
        .await;

        assert!(result.cancelled);
        assert_eq!(
            tool_call_ids(&result.response_messages),
            vec!["call_read", "call_py"]
        );
        assert_eq!(
            result
                .tool_records
                .iter()
                .map(|record| record.id.as_str())
                .collect::<Vec<_>>(),
            vec!["call_read", "call_py"]
        );
        assert!(result
            .tool_records
            .iter()
            .all(|record| matches!(record.status, ToolCallStatus::Cancelled)));
        let start_events = executor
            .events()
            .into_iter()
            .filter(|event| event.starts_with("start:"))
            .collect::<Vec<_>>();
        assert_eq!(
            start_events,
            vec!["start:read"],
            "remaining serial tools must not start after cancellation"
        );
    }

    #[test]
    fn cancelled_tool_round_result_preserves_replay_messages_for_storage() {
        let tool_record = ToolCallRecord {
            id: "call_read".to_string(),
            name: "read".to_string(),
            source: "native".to_string(),
            server_id: None,
            arguments: "{}".to_string(),
            status: ToolCallStatus::Cancelled,
            result_preview: None,
            error: Some("Tool call cancelled".to_string()),
            duration_ms: Some(5),
            started_at: Some(10),
            completed_at: Some(11),
            round: 1,
            sensitive: false,
            artifacts: Vec::new(),
            trace_id: Some("run".to_string()),
            span_id: Some("tool_round_1_call_read".to_string()),
            structured_content: None,
        };
        let assistant_message = serde_json::json!({
            "role": "assistant",
            "content": Value::Null,
            "tool_calls": [{
                "id": "call_read",
                "type": "function",
                "function": {
                    "name": "read",
                    "arguments": "{}",
                }
            }],
        });
        let tool_response = tool_message("call_read".to_string(), "Tool call cancelled");
        let result = cancelled_tool_round_run_result(
            "zh-CN",
            &["planning".to_string()],
            vec![tool_record.clone()],
            vec![ChatMessageSegment {
                id: "seg_1000_tool_call_read".to_string(),
                kind: ChatMessageSegmentKind::Tool,
                phase: ChatMessageSegmentPhase::ToolLoop,
                order: 1000,
                step_number: Some(1),
                round: Some(1),
                text: None,
                tool_call_id: Some("call_read".to_string()),
            }],
            vec![assistant_message.clone(), tool_response.clone()],
            vec![AgentStepResult {
                step_number: 1,
                phase: AgentPhase::ToolLoop,
                response_messages: vec![assistant_message.clone(), tool_response.clone()],
                tool_records: vec![tool_record],
                segments: Vec::new(),
                streamed: true,
                stop_reason: Some(AgentStopReason::Cancelled),
            }],
        );

        assert_eq!(result.content, "已停止生成。");
        assert_eq!(result.reasoning.as_deref(), Some("planning"));
        assert_eq!(result.tool_records.len(), 1);
        assert!(result.segments.iter().any(|segment| {
            segment.kind == ChatMessageSegmentKind::Tool
                && segment.tool_call_id.as_deref() == Some("call_read")
        }));
        assert!(result.segments.iter().any(|segment| {
            segment.kind == ChatMessageSegmentKind::Text
                && segment.phase == ChatMessageSegmentPhase::Synthesis
                && segment.text.as_deref() == Some("已停止生成。")
        }));
        assert!(matches!(
            result.tool_records[0].status,
            ToolCallStatus::Cancelled
        ));
        assert_eq!(result.api_messages.len(), 3);
        assert_eq!(
            result.api_messages[0]
                .get("tool_calls")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(1)
        );
        assert_eq!(
            result.api_messages[1]
                .get("tool_call_id")
                .and_then(Value::as_str),
            Some("call_read")
        );
        assert_eq!(
            result.api_messages[2]
                .get("content")
                .and_then(Value::as_str),
            Some("已停止生成。")
        );
        assert_eq!(
            result.steps[0].stop_reason,
            Some(AgentStopReason::Cancelled)
        );
    }

    #[tokio::test]
    async fn tool_round_keeps_serial_only_tools_non_overlapping() {
        let host = TestHost::default();
        let executor = RecordingExecutor::default();
        let settings = Settings::default();
        let tools = vec![native_run_python_tool()];
        let mut skill_cache = skills::SkillRunCache::default();

        let result = execute_tool_round(
            &host,
            &executor,
            &settings,
            test_round_context(),
            &tools,
            &[],
            vec![
                pending_tool_call("call_py_1", "run_python"),
                pending_tool_call("call_py_2", "run_python"),
            ],
            &mut skill_cache,
        )
        .await;

        assert_eq!(executor.max_active(), 1);
        assert_eq!(
            executor.events(),
            vec![
                "start:run_python",
                "finish:run_python",
                "start:run_python",
                "finish:run_python"
            ]
        );
        assert_eq!(result.response_messages.len(), 2);
        assert_eq!(
            result.response_messages[0]
                .get("tool_call_id")
                .and_then(Value::as_str),
            Some("call_py_1")
        );
        assert_eq!(
            result.response_messages[1]
                .get("tool_call_id")
                .and_then(Value::as_str),
            Some("call_py_2")
        );
    }

    // ===== Fallback scenarios (6 synthesis/planning fallbacks in run_agent_loop) =====

    #[test]
    fn fallback_responses_are_bilingual() {
        assert_eq!(
            empty_synthesis_fallback_response("zh-CN"),
            "工具调用已经完成，但模型没有返回最终总结。上方工具结果已保存在本轮回复中，你可以继续追问，或让我重新生成总结。"
        );
        assert_eq!(
            empty_synthesis_fallback_response("en-US"),
            "The tool calls completed, but the model did not return a final summary. The tool results above were saved with this reply; you can continue from them or regenerate the summary."
        );
        assert_eq!(
            synthesis_failed_fallback_response("zh-CN"),
            "最终总结生成失败(可能是模型供应商内容审核拦截)。上方工具结果已保存在本轮回复中,你可以继续追问、让我重新生成,或更换聊天模型再试。"
        );
        assert_eq!(
            synthesis_failed_fallback_response("en-US"),
            "Final summary generation failed (possibly provider content moderation). The tool results above were saved with this reply; you can continue from them, regenerate, or switch the chat model and retry."
        );
        assert_eq!(
            tool_planning_failed_fallback_response("zh-CN"),
            "工具调用参数生成失败，这一步还没有真正执行写入。主对话已保留，你可以让我缩小范围、改用补丁，或重新生成。"
        );
        assert_eq!(
            tool_planning_failed_fallback_response("en-US"),
            "Tool-call argument generation failed before the write actually ran. This conversation was preserved; you can ask me to narrow the scope, use a patch, or regenerate."
        );
        assert_eq!(stopped_generation_content("zh-CN"), "已停止生成。");
        assert_eq!(stopped_generation_content("en-US"), "Generation stopped.");
    }

    /// Fallback A (helper level, deterministic): a planning stream dies while tool
    /// argument drafts are in flight; the run result must mark every draft record
    /// as error, emit the bilingual fallback, and finish with stream_outcome "error".
    #[test]
    fn tool_planning_failed_run_result_marks_drafts_error_and_emits_fallback() {
        let state = test_app_state();
        let config = test_run_config(&state, "http://127.0.0.1:9/v1", true);
        let host = TestHost::default();

        let mut segment_builder = SegmentBuilder::new();
        let _reasoning_segment = segment_builder.reserve(
            ChatMessageSegmentKind::Reasoning,
            ChatMessageSegmentPhase::ToolLoop,
            Some(1),
            Some(1),
            "step_1_reasoning",
        );
        let planning_text_segment = segment_builder.reserve(
            ChatMessageSegmentKind::Text,
            ChatMessageSegmentPhase::ToolLoop,
            Some(1),
            Some(1),
            "step_1_text",
        );
        let tracker = ToolCallDraftTracker::new(
            vec![native_write_file_tool()],
            1,
            Some(1),
            segment_builder.next_order(),
        );
        let mut sink = AgentStreamSink::new(
            &host,
            "conversation",
            "run",
            "message",
            true,
            None,
            None,
            Some(tracker.clone()),
        );
        sink.emit(StreamPart::ToolCallStart {
            id: "call_write".to_string(),
            name: "write".to_string(),
        })
        .expect("tool call start should emit");
        sink.emit(StreamPart::ToolCallDelta {
            id: "call_write".to_string(),
            delta: "{\"path\":\"large.html\",\"content\":\"".to_string(),
        })
        .expect("tool call delta should emit");
        assert!(tracker.has_started());

        let result = tool_planning_failed_run_result(
            &host,
            &config,
            segment_builder,
            planning_text_segment,
            tracker,
            &["planning thought".to_string()],
            Vec::new(),
            Vec::new(),
            "Chat tools planning read body failed".to_string(),
        );

        let fallback = tool_planning_failed_fallback_response("zh-CN");
        assert_eq!(result.stream_outcome, "error");
        assert_eq!(result.content, fallback);
        assert_eq!(result.reasoning.as_deref(), Some("planning thought"));
        assert_eq!(result.tool_records.len(), 1);
        assert_eq!(result.tool_records[0].id, "call_write");
        assert!(matches!(
            result.tool_records[0].status,
            ToolCallStatus::Error
        ));
        assert_eq!(
            result.tool_records[0].error.as_deref(),
            Some("Chat tools planning read body failed")
        );
        assert!(result.tool_records[0].completed_at.is_some());

        // The error record was re-emitted through the host (after the pending draft).
        let emitted = host.recorded_tool_records();
        let last_emitted = emitted.last().expect("error record emitted");
        assert_eq!(last_emitted.id, "call_write");
        assert!(matches!(last_emitted.status, ToolCallStatus::Error));

        // Draft tool segment is preserved and the fallback text segment is
        // synthesis-phase with no round.
        assert!(result.segments.iter().any(|segment| {
            segment.kind == ChatMessageSegmentKind::Tool
                && segment.tool_call_id.as_deref() == Some("call_write")
        }));
        let fallback_segment = result
            .segments
            .iter()
            .find(|segment| segment.kind == ChatMessageSegmentKind::Text)
            .expect("fallback text segment");
        assert_eq!(fallback_segment.phase, ChatMessageSegmentPhase::Synthesis);
        assert_eq!(fallback_segment.round, None);
        assert_eq!(fallback_segment.text.as_deref(), Some(fallback.as_str()));

        // Fallback delta + a single "done" done event.
        assert!(host
            .recorded_deltas()
            .iter()
            .any(|delta| delta.delta == fallback));
        assert_eq!(
            host.recorded_dones(),
            vec![("done".to_string(), fallback.clone())]
        );

        // The final assistant message is pushed unconditionally here.
        assert_eq!(result.api_messages.len(), 1);
        assert_eq!(
            result.api_messages[0]
                .get("content")
                .and_then(Value::as_str),
            Some(fallback.as_str())
        );

        let last_step = result.steps.last().expect("fallback step");
        assert_eq!(last_step.stop_reason, Some(AgentStopReason::Natural));
        assert_eq!(last_step.tool_records.len(), 1);
    }

    /// Fallback A (integration): the provider stream breaks mid-connection after a
    /// tool-call draft has started; run_agent_loop must return Ok with
    /// stream_outcome "error" instead of bubbling an invoke error.
    #[tokio::test]
    async fn run_loop_stream_planning_interrupt_after_tool_draft_returns_error_result() {
        let server = MockModelServer::start(vec![MockResponse::SseInterrupt(vec![
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_read","function":{"name":"read","arguments":"{\"path\":\"/tmp/"}}]}}]}"#
                .to_string(),
        ])]);
        let state = test_app_state();
        let config = test_run_config(&state, &server.base_url, true);
        let host = TestHost::default();
        let executor = RecordingExecutor::default();

        let result = run_agent_loop(config, &host, &executor)
            .await
            .expect("planning interruption with tool draft must not bubble Err");

        let fallback = tool_planning_failed_fallback_response("zh-CN");
        assert_eq!(result.stream_outcome, "error");
        assert_eq!(result.content, fallback);
        assert_eq!(result.tool_records.len(), 1);
        assert_eq!(result.tool_records[0].id, "call_read");
        assert!(matches!(
            result.tool_records[0].status,
            ToolCallStatus::Error
        ));
        assert!(
            executor.events().is_empty(),
            "interrupted tool drafts must never execute"
        );
        assert_eq!(
            host.recorded_dones(),
            vec![("done".to_string(), fallback.clone())]
        );
        assert_eq!(result.api_messages.len(), 1);
        assert_eq!(
            result.api_messages[0]
                .get("content")
                .and_then(Value::as_str),
            Some(fallback.as_str())
        );
    }

    /// Fallback B: streamed synthesis request fails (HTTP 400) after a successful
    /// tool round; the tool records must survive with the bilingual fallback text.
    #[tokio::test]
    async fn run_loop_stream_synthesis_failure_preserves_tool_records_with_fallback() {
        let server = MockModelServer::start(vec![
            MockResponse::Sse(planning_tool_call_sse_events()),
            MockResponse::Status(400, r#"{"error":"mock synthesis failure"}"#.to_string()),
        ]);
        let state = test_app_state();
        let config = test_run_config(&state, &server.base_url, true);
        let host = TestHost::default();
        let executor = RecordingExecutor::default();

        let result = run_agent_loop(config, &host, &executor)
            .await
            .expect("synthesis failure after tool records must not bubble Err");

        // 框架恢复:合成被拒(400)后不再吐静态"失败"文案,而是用已收集的工具结果兜底。
        let recovered =
            crate::chat::agent::recovery::assemble_results_from_tool_records(&result.tool_records, "zh-CN");
        assert!(recovered.contains("result:read"));
        assert_eq!(result.stream_outcome, "recovered");
        assert_eq!(result.content, recovered);
        assert_eq!(result.tool_records.len(), 1);
        assert_eq!(result.tool_records[0].id, "call_read");
        assert!(matches!(
            result.tool_records[0].status,
            ToolCallStatus::Success
        ));

        // assistant tool_calls message + tool result + final recovered message.
        assert_eq!(result.api_messages.len(), 3);
        assert_eq!(
            result.api_messages[2]
                .get("content")
                .and_then(Value::as_str),
            Some(recovered.as_str())
        );

        // Planning never emits done while tool calls are pending; the only done is
        // the recovered "done".
        assert_eq!(
            host.recorded_dones(),
            vec![("done".to_string(), recovered.clone())]
        );
        let fallback_delta = host
            .recorded_deltas()
            .into_iter()
            .find(|delta| delta.delta == recovered)
            .expect("recovered delta emitted");
        assert_eq!(fallback_delta.reasoning_delta, None);
        let segment = fallback_delta.segment.expect("fallback delta has segment");
        assert_eq!(segment.kind, ChatMessageSegmentKind::Text);
        assert_eq!(segment.phase, ChatMessageSegmentPhase::Synthesis);

        assert_eq!(result.steps.len(), 1);
        assert_eq!(result.steps[0].stop_reason, Some(AgentStopReason::StepLimit));
    }

    /// Fallback C: streamed synthesis is cancelled after tool results exist; the run
    /// returns Ok("cancelled") with the stopped-generation placeholder content.
    #[tokio::test]
    async fn run_loop_stream_synthesis_cancelled_returns_cancelled_with_stopped_content() {
        let server = MockModelServer::start(vec![
            MockResponse::Sse(planning_tool_call_sse_events()),
            MockResponse::SseThenHang(Vec::new()),
        ]);
        let state = test_app_state();
        let config = test_run_config(&state, &server.base_url, true);
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let host = TestHost::with_cancel_flag(cancel_flag.clone());
        let executor = CancelAfterToolExecutor { cancel_flag };

        let result = run_agent_loop(config, &host, &executor)
            .await
            .expect("cancelled synthesis with tool records must not bubble Err");

        assert_eq!(result.stream_outcome, "cancelled");
        assert_eq!(result.content, stopped_generation_content("zh-CN"));
        assert_eq!(result.tool_records.len(), 1);
        assert!(matches!(
            result.tool_records[0].status,
            ToolCallStatus::Success
        ));

        let dones = host.recorded_dones();
        assert_eq!(dones.len(), 1, "exactly one done event");
        assert_eq!(dones[0].0, "cancelled");

        assert_eq!(
            result
                .api_messages
                .last()
                .and_then(|message| message.get("content"))
                .and_then(Value::as_str),
            Some("已停止生成。")
        );
        assert!(result.segments.iter().any(|segment| {
            segment.kind == ChatMessageSegmentKind::Text
                && segment.text.as_deref() == Some("已停止生成。")
        }));
    }

    /// Fallback C variant: synthesis streamed some text before cancellation; the
    /// partial text must be kept instead of the stopped-generation placeholder.
    #[tokio::test]
    async fn run_loop_stream_synthesis_cancelled_keeps_partial_content() {
        let server = MockModelServer::start(vec![
            MockResponse::Sse(planning_tool_call_sse_events()),
            MockResponse::SseThenHang(vec![
                r#"{"choices":[{"delta":{"content":"部分回答"}}]}"#.to_string()
            ]),
        ]);
        let state = test_app_state();
        let config = test_run_config(&state, &server.base_url, true);
        let host = TestHost::cancelling_on_first_text_delta();
        let executor = RecordingExecutor::default();

        let result = run_agent_loop(config, &host, &executor)
            .await
            .expect("cancelled synthesis with partial content must not bubble Err");

        assert_eq!(result.stream_outcome, "cancelled");
        assert_eq!(result.content, "部分回答");
        assert_eq!(result.tool_records.len(), 1);
        let dones = host.recorded_dones();
        assert_eq!(dones.len(), 1);
        assert_eq!(dones[0], ("cancelled".to_string(), "部分回答".to_string()));
        assert_eq!(
            result
                .api_messages
                .last()
                .and_then(|message| message.get("content"))
                .and_then(Value::as_str),
            Some("部分回答")
        );
    }

    /// Bug fix regression: a plain-text planning stream (no tool calls started)
    /// cancelled after partial text must keep the generated text as an
    /// Ok("cancelled") run result instead of bubbling Err and dropping the turn.
    #[tokio::test]
    async fn run_loop_stream_planning_cancelled_keeps_partial_text() {
        let server = MockModelServer::start(vec![MockResponse::SseThenHang(vec![
            r#"{"choices":[{"delta":{"content":"这是已经生成的部分回答内容"}}]}"#.to_string(),
        ])]);
        let state = test_app_state();
        let config = test_run_config(&state, &server.base_url, true);
        let host = TestHost::cancelling_on_first_text_delta();
        let executor = RecordingExecutor::default();

        let result = run_agent_loop(config, &host, &executor)
            .await
            .expect("cancelled plain-text planning with partial content must not bubble Err");

        assert_eq!(result.stream_outcome, "cancelled");
        assert_eq!(result.content, "这是已经生成的部分回答内容");
        assert!(result.tool_records.is_empty());
        assert!(
            executor.events().is_empty(),
            "no tools may execute in a cancelled plain-text turn"
        );

        let dones = host.recorded_dones();
        assert_eq!(dones.len(), 1, "exactly one done event");
        assert_eq!(
            dones[0],
            (
                "cancelled".to_string(),
                "这是已经生成的部分回答内容".to_string()
            )
        );

        assert!(result.segments.iter().any(|segment| {
            segment.kind == ChatMessageSegmentKind::Text
                && segment.phase == ChatMessageSegmentPhase::Plain
                && segment.text.as_deref() == Some("这是已经生成的部分回答内容")
        }));
    }

    /// Regression: a cancelled plain-text reply that streamed reasoning before the
    /// answer must persist the reasoning segment ahead of the text segment, so the
    /// reloaded timeline keeps "Thinking" above the answer instead of below it.
    #[tokio::test]
    async fn run_loop_stream_planning_cancelled_orders_reasoning_before_text() {
        let server = MockModelServer::start(vec![MockResponse::SseThenHang(vec![
            r#"{"choices":[{"delta":{"reasoning_content":"先构思一下整体结构","content":"这是已经生成的部分回答内容"}}]}"#.to_string(),
        ])]);
        let state = test_app_state();
        let config = test_run_config(&state, &server.base_url, true);
        let host = TestHost::cancelling_on_first_text_delta();
        let executor = RecordingExecutor::default();

        let result = run_agent_loop(config, &host, &executor)
            .await
            .expect("cancelled plain-text planning with reasoning must not bubble Err");

        assert_eq!(result.stream_outcome, "cancelled");

        let reasoning = result
            .segments
            .iter()
            .find(|segment| segment.kind == ChatMessageSegmentKind::Reasoning)
            .expect("a reasoning segment must be persisted");
        let text = result
            .segments
            .iter()
            .find(|segment| segment.kind == ChatMessageSegmentKind::Text)
            .expect("a text segment must be persisted");
        assert!(
            reasoning.order < text.order,
            "reasoning segment (order {}) must precede the text segment (order {}) so Thinking renders above the answer",
            reasoning.order,
            text.order,
        );
    }

    /// Pins the unchanged path: a plain-text stream cancelled before any text was
    /// generated still bubbles Err("cancelled") (commands.rs handles it as a
    /// successful no-message cancellation).
    #[tokio::test]
    async fn run_loop_stream_planning_cancelled_with_no_text_returns_err() {
        let server = MockModelServer::start(vec![MockResponse::SseThenHang(Vec::new())]);
        let state = test_app_state();
        let config = test_run_config(&state, &server.base_url, true);
        let host = TestHost::cancelling_after(Duration::from_millis(20));
        let executor = RecordingExecutor::default();

        let err = run_agent_loop(config, &host, &executor)
            .await
            .expect_err("cancelled stream with zero generated text keeps returning Err");

        assert_eq!(err, "cancelled");
        let dones = host.recorded_dones();
        assert_eq!(dones.len(), 1, "exactly one done event");
        assert_eq!(dones[0], ("cancelled".to_string(), String::new()));
    }

    /// Bug fix regression: when no tools are configured, the plain synthesis path
    /// cancelled after partial text must also preserve the generated text.
    #[tokio::test]
    async fn run_loop_stream_plain_synthesis_cancelled_keeps_partial_text() {
        let server = MockModelServer::start(vec![MockResponse::SseThenHang(vec![
            r#"{"choices":[{"delta":{"content":"部分回答"}}]}"#.to_string(),
        ])]);
        let state = test_app_state();
        let mut config = test_run_config(&state, &server.base_url, true);
        config.tools = Vec::new();
        let host = TestHost::cancelling_on_first_text_delta();
        let executor = RecordingExecutor::default();

        let result = run_agent_loop(config, &host, &executor)
            .await
            .expect("cancelled plain synthesis with partial content must not bubble Err");

        assert_eq!(result.stream_outcome, "cancelled");
        assert_eq!(result.content, "部分回答");
        assert!(result.tool_records.is_empty());

        let dones = host.recorded_dones();
        assert_eq!(dones.len(), 1, "exactly one done event");
        assert_eq!(dones[0], ("cancelled".to_string(), "部分回答".to_string()));

        assert!(result.segments.iter().any(|segment| {
            segment.kind == ChatMessageSegmentKind::Text
                && segment.phase == ChatMessageSegmentPhase::Plain
                && segment.text.as_deref() == Some("部分回答")
        }));
    }

    /// Tool executor returning a huge text result, to push the loop over a tiny
    /// context window and exercise in-loop compaction.
    struct HugeResultExecutor {
        chars: usize,
    }

    impl ToolExecutor for HugeResultExecutor {
        fn call<'a>(
            &'a self,
            _ctx: &'a ToolExecutionContext<'a>,
            _tool: &'a ChatToolDefinition,
            _arguments: Value,
            _skill_cache: Option<&'a mut skills::SkillRunCache>,
        ) -> super::super::execute::ToolExecutorFuture<'a> {
            Box::pin(async move {
                Ok(McpToolCallResult {
                    content: "A".repeat(self.chars),
                    is_error: false,
                    raw: Value::Null,
                    artifacts: Vec::new(),
                    structured_content: None,
                })
            })
        }
    }

    /// In-loop compaction: an oversized tool output from an EARLIER round (outside
    /// the keep-recent tail) must be snipped in the request sent to the provider,
    /// while nothing persisted ever carries the snip marker.
    #[tokio::test]
    async fn run_loop_compacts_send_view_but_keeps_persisted_messages_raw() {
        let server = MockModelServer::start(vec![
            MockResponse::Sse(planning_tool_call_sse_events()),
            MockResponse::Sse(vec![
                r#"{"choices":[{"delta":{"content":"总结完成，工具输出已分析。"}}]}"#.to_string(),
                "[DONE]".to_string(),
            ]),
        ]);
        let state = test_app_state();
        let mut config = test_run_config(&state, &server.base_url, true);
        // 2400 token 窗口（预算 2040）：9000 字符旧工具输出（est ~2353）触发压缩，
        // L1 snip 后（est ~1798）回到预算内——只走 Layer1，不触发 Layer2 模型摘要。
        config.provider.model_overrides.insert(
            "test-model".to_string(),
            crate::settings::ModelInfo {
                context_window: Some(2_400),
                ..Default::default()
            },
        );
        // 预填早前轮次历史：一条超大 tool 输出 + 8 条近期小消息把它推出保护尾巴。
        // 设计契约：snip 只动 keep-recent(8) 之外的旧 tool 消息，当前轮结果保持原文。
        let huge = "A".repeat(9_000);
        config.runtime_messages.push(serde_json::json!({
            "role": "assistant", "content": "", "tool_calls": [
                {"id": "old_call", "type": "function", "function": {"name": "read", "arguments": "{}"}}
            ]
        }));
        config.runtime_messages.push(serde_json::json!({
            "role": "tool", "tool_call_id": "old_call", "content": huge
        }));
        for i in 0..8 {
            config.runtime_messages.push(serde_json::json!({
                "role": if i % 2 == 0 { "user" } else { "assistant" },
                "content": format!("history {i}")
            }));
        }
        let host = TestHost::default();
        let executor = RecordingExecutor::default();

        let result = run_agent_loop(config, &host, &executor)
            .await
            .expect("compacted run completes");

        assert_eq!(result.stream_outcome, "completed");
        assert_eq!(result.content, "总结完成，工具输出已分析。");

        // 发送视图：两次请求（规划 + 合成）的 body 中旧工具消息都已被 snip。
        let bodies = server.captured_bodies();
        assert_eq!(bodies.len(), 2, "planning + synthesis requests");
        for (idx, body) in bodies.iter().enumerate() {
            assert!(
                body.contains("chars snipped"),
                "request #{idx} must carry the snipped old tool output"
            );
            assert!(
                !body.contains(&"A".repeat(8_000)),
                "request #{idx} must not carry the full 9000-char tool output"
            );
        }

        // 持久化：本轮产生的 api_messages 不含任何 snip 标记（snip 只作用于发送视图）。
        assert!(result
            .api_messages
            .iter()
            .all(|message| !message.to_string().contains("chars snipped")));
        // 本轮工具结果原文留存。
        let persisted_tool = result
            .api_messages
            .iter()
            .find(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
            .expect("persisted tool message from this round");
        assert_eq!(persisted_tool["content"], "result:read");
    }

    /// Crash-safety: after a tool round that returns `Continue` (more rounds
    /// allowed), the loop must checkpoint a partial-assistant snapshot carrying
    /// the round's tool work, so a mid-run crash before the final write keeps the
    /// turn recoverable instead of discarding it.
    #[tokio::test]
    async fn run_loop_persists_partial_assistant_after_completed_tool_round() {
        let server = MockModelServer::start(vec![
            // Round 1 planning: one read tool call. With max_tool_rounds=2
            // this round returns Continue, so the checkpoint fires.
            MockResponse::Sse(planning_tool_call_sse_events()),
            // Round 2 planning: a natural final answer (no tools) ends the loop.
            MockResponse::Sse(vec![
                r#"{"choices":[{"delta":{"content":"完成。"}}]}"#.to_string(),
                "[DONE]".to_string(),
            ]),
        ]);
        let state = test_app_state();
        let mut config = test_run_config(&state, &server.base_url, true);
        config.effective_chat_tools.max_tool_rounds = Some(2);
        let host = TestHost::default();
        let executor = RecordingExecutor::default();

        let result = run_agent_loop(config, &host, &executor)
            .await
            .expect("multi-round run completes");

        // The checkpoint fired after round 1's completed tool round, snapshotting
        // the work done so far under the run's message id.
        let persists = host.recorded_persists();
        let snapshot = persists
            .iter()
            .find(|(_, tool_records_len, _)| *tool_records_len >= 1)
            .expect("a checkpoint snapshot includes the completed round's tool record");
        assert_eq!(snapshot.0, "message", "snapshot keyed by run message id");
        assert!(
            snapshot.1 >= 1,
            "snapshot carries the round's tool record(s)"
        );

        // The run still completed normally end-to-end.
        assert_eq!(result.stream_outcome, "completed");
        assert!(result.tool_records.iter().any(|r| r.id == "call_read"));
    }

    /// Layer2 escalation: when snip alone cannot fit the window, a summary request
    /// fires first and the next provider request carries the summary instead of
    /// the old history; the summary itself stays out of persisted api_messages.
    #[tokio::test]
    async fn run_loop_layer2_replaces_old_history_with_summary() {
        let server = MockModelServer::start(vec![
            // 1) Layer2 摘要请求（非流式 JSON）。
            MockResponse::Json(
                r#"{"choices":[{"message":{"role":"assistant","content":"SUMMARY_MARKER: 早前轮次已读取大文件"}}]}"#
                    .to_string(),
            ),
            // 2) 压缩后的规划请求 → 直接给出最终回答（无工具调用）。
            MockResponse::Sse(vec![
                r#"{"choices":[{"delta":{"content":"基于摘要继续完成任务。"}}]}"#.to_string(),
                "[DONE]".to_string(),
            ]),
        ]);
        let state = test_app_state();
        let mut config = test_run_config(&state, &server.base_url, true);
        // 600 token 窗口（预算 510）：snip 后仍超 → 必然升级 Layer2。
        config.provider.model_overrides.insert(
            "test-model".to_string(),
            crate::settings::ModelInfo {
                context_window: Some(600),
                ..Default::default()
            },
        );
        let huge = "B".repeat(9_000);
        config.runtime_messages.push(serde_json::json!({
            "role": "tool", "tool_call_id": "old_call", "content": huge
        }));
        for i in 0..8 {
            config.runtime_messages.push(serde_json::json!({
                "role": if i % 2 == 0 { "user" } else { "assistant" },
                "content": format!("recent history {i}")
            }));
        }
        let host = TestHost::default();
        let executor = RecordingExecutor::default();

        let result = run_agent_loop(config, &host, &executor)
            .await
            .expect("layer2-compacted run completes");
        assert_eq!(result.stream_outcome, "completed");
        assert_eq!(result.content, "基于摘要继续完成任务。");

        let bodies = server.captured_bodies();
        assert_eq!(bodies.len(), 2, "summary request + planning request");
        // 摘要请求带着压缩指令与旧段样本。
        assert!(bodies[0].contains("compress conversation history"));
        // 压缩后的规划请求：携带摘要，不再携带旧原文，保留最近历史。
        assert!(bodies[1].contains("SUMMARY_MARKER"));
        assert!(!bodies[1].contains(&"B".repeat(1_000)));
        assert!(bodies[1].contains("recent history 7"));
        // 摘要只存在于发送视图/工作副本，不进持久化 api_messages。
        assert!(result
            .api_messages
            .iter()
            .all(|message| !message.to_string().contains("SUMMARY_MARKER")));
    }

    /// Under-budget runs must not be touched by compaction: the request body
    /// carries the tool output verbatim.
    #[tokio::test]
    async fn run_loop_under_budget_sends_messages_untouched() {
        let server = MockModelServer::start(vec![
            MockResponse::Sse(planning_tool_call_sse_events()),
            MockResponse::Sse(vec![
                r#"{"choices":[{"delta":{"content":"done"}}]}"#.to_string(),
                "[DONE]".to_string(),
            ]),
        ]);
        let state = test_app_state();
        let config = test_run_config(&state, &server.base_url, true);
        let host = TestHost::default();
        // 默认窗口（test-model 无覆盖 → 200k fallback），小输出远不达 0.85。
        let executor = HugeResultExecutor { chars: 600 };

        let result = run_agent_loop(config, &host, &executor)
            .await
            .expect("run completes");
        assert_eq!(result.stream_outcome, "completed");

        let bodies = server.captured_bodies();
        assert_eq!(bodies.len(), 2);
        assert!(
            !bodies[1].contains("chars snipped"),
            "under-budget send view must be untouched"
        );
        assert!(bodies[1].contains(&"A".repeat(600)));
    }

    /// Fallback D: streamed synthesis returns an empty answer after tool results;
    /// the loop substitutes the bilingual fallback and completes normally
    /// (stream_outcome "completed", not "error").
    #[tokio::test]
    async fn run_loop_stream_synthesis_empty_output_uses_fallback_and_completes() {
        let server = MockModelServer::start(vec![
            MockResponse::Sse(planning_tool_call_sse_events()),
            MockResponse::Sse(vec!["[DONE]".to_string()]),
        ]);
        let state = test_app_state();
        let config = test_run_config(&state, &server.base_url, true);
        let host = TestHost::default();
        let executor = RecordingExecutor::default();

        let result = run_agent_loop(config, &host, &executor)
            .await
            .expect("empty synthesis with tool records must not bubble Err");

        let recovered =
            crate::chat::agent::recovery::assemble_results_from_tool_records(&result.tool_records, "zh-CN");
        assert!(recovered.contains("result:read"));
        assert_eq!(result.stream_outcome, "completed");
        assert_eq!(result.content, recovered);
        assert_eq!(result.tool_records.len(), 1);
        assert!(matches!(
            result.tool_records[0].status,
            ToolCallStatus::Success
        ));
        assert_eq!(
            host.recorded_dones(),
            vec![("done".to_string(), recovered.clone())]
        );

        assert_eq!(result.steps.len(), 2);
        assert_eq!(result.steps[0].stop_reason, Some(AgentStopReason::StepLimit));
        let final_step = &result.steps[1];
        assert_eq!(final_step.phase, AgentPhase::Synthesis);
        assert_eq!(final_step.stop_reason, Some(AgentStopReason::Natural));

        assert!(result.segments.iter().any(|segment| {
            segment.kind == ChatMessageSegmentKind::Text
                && segment.phase == ChatMessageSegmentPhase::Synthesis
                && segment.text.as_deref() == Some(recovered.as_str())
        }));
        assert_eq!(
            result
                .api_messages
                .last()
                .and_then(|message| message.get("content"))
                .and_then(Value::as_str),
            Some(recovered.as_str())
        );
    }

    /// Fallback E: non-streamed synthesis request fails (HTTP 400) after a
    /// successful tool round; tool records survive with the failure fallback.
    #[tokio::test]
    async fn run_loop_nonstream_synthesis_failure_preserves_tool_records_with_fallback() {
        let server = MockModelServer::start(vec![
            MockResponse::Json(planning_tool_call_json()),
            MockResponse::Status(400, r#"{"error":"mock synthesis failure"}"#.to_string()),
        ]);
        let state = test_app_state();
        let config = test_run_config(&state, &server.base_url, false);
        let host = TestHost::default();
        let executor = RecordingExecutor::default();

        let result = run_agent_loop(config, &host, &executor)
            .await
            .expect("non-stream synthesis failure after tool records must not bubble Err");

        let recovered =
            crate::chat::agent::recovery::assemble_results_from_tool_records(&result.tool_records, "zh-CN");
        assert!(recovered.contains("result:read"));
        assert_eq!(result.stream_outcome, "recovered");
        assert_eq!(result.content, recovered);
        assert_eq!(result.tool_records.len(), 1);
        assert!(matches!(
            result.tool_records[0].status,
            ToolCallStatus::Success
        ));
        assert!(host
            .recorded_deltas()
            .iter()
            .any(|delta| delta.delta == recovered));
        assert_eq!(
            host.recorded_dones(),
            vec![("done".to_string(), recovered.clone())]
        );
        assert_eq!(
            result
                .api_messages
                .last()
                .and_then(|message| message.get("content"))
                .and_then(Value::as_str),
            Some(recovered.as_str())
        );
    }

    /// Fallback F: non-streamed synthesis returns empty content after tool results;
    /// the bilingual fallback replaces it and reasoning is still passed through.
    #[tokio::test]
    async fn run_loop_nonstream_synthesis_empty_output_uses_fallback() {
        let empty_synthesis = serde_json::json!({
            "choices": [{
                "finish_reason": "stop",
                "message": {
                    "role": "assistant",
                    "content": "",
                    "reasoning_content": "synthesis reasoning"
                }
            }]
        })
        .to_string();
        let server = MockModelServer::start(vec![
            MockResponse::Json(planning_tool_call_json()),
            MockResponse::Json(empty_synthesis),
        ]);
        let state = test_app_state();
        let config = test_run_config(&state, &server.base_url, false);
        let host = TestHost::default();
        let executor = RecordingExecutor::default();

        let result = run_agent_loop(config, &host, &executor)
            .await
            .expect("non-stream empty synthesis with tool records must not bubble Err");

        let recovered =
            crate::chat::agent::recovery::assemble_results_from_tool_records(&result.tool_records, "zh-CN");
        assert!(recovered.contains("result:read"));
        assert_eq!(result.stream_outcome, "completed");
        assert_eq!(result.content, recovered);
        assert_eq!(result.reasoning.as_deref(), Some("synthesis reasoning"));
        assert_eq!(result.tool_records.len(), 1);
        assert_eq!(
            host.recorded_dones(),
            vec![("done".to_string(), recovered.clone())]
        );

        let final_message = result.api_messages.last().expect("final api message");
        assert_eq!(
            final_message.get("content").and_then(Value::as_str),
            Some(recovered.as_str())
        );
        assert_eq!(
            final_message
                .get("reasoning_content")
                .and_then(Value::as_str),
            Some("synthesis reasoning")
        );

        let final_step = result.steps.last().expect("final step");
        assert_eq!(final_step.phase, AgentPhase::Synthesis);
        assert_eq!(final_step.stop_reason, Some(AgentStopReason::Natural));
    }

    /// Build a minimal RunState carrying only the tool fields under test.
    fn run_state_with_base(base_tools: Vec<ChatToolDefinition>) -> RunState {
        RunState {
            runtime_messages: Vec::new(),
            tools: base_tools.clone(),
            base_tools,
            blocked_tool_calls: Vec::new(),
            generated_api_messages: Vec::new(),
            tool_records: Vec::new(),
            planning_reasoning_parts: Vec::new(),
            steps: Vec::new(),
            segment_builder: SegmentBuilder::new(),
            step_number: 0,
            provider_tools_unsupported: false,
            tried_skill_only_tools: false,
            planning_final_message: None,
            planning_final_streamed: false,
            skill_cache: skills::SkillRunCache::default(),
            applied_allowed_tools_len: 0,
            usage: None,
        }
    }

    /// FIX 4: T3 dynamic allowed_tools must recompute the effective tool set from the
    /// FULL base list each round, not shrink cumulatively. Activating skill A (which
    /// allows only `alpha`) drops `beta`; a later activation of skill B (which allows
    /// `beta`) must re-permit `beta` — impossible under the old cumulative-shrink impl.
    #[test]
    fn activated_tool_filter_recomputes_from_base_not_cumulatively() {
        let base = vec![
            test_mcp_tool("alpha", Value::Null),
            test_mcp_tool("beta", Value::Null),
        ];
        let mut state = run_state_with_base(base);

        // Activate skill A: allows only `alpha`. `beta` is dropped.
        state
            .skill_cache
            .record_activated_allowed_tools(&["alpha".to_string()]);
        state.apply_activated_tool_filter();
        assert!(state.tools.iter().any(|tool| tool.name == "alpha"));
        assert!(
            !state.tools.iter().any(|tool| tool.name == "beta"),
            "beta should be narrowed out by skill A"
        );

        // Activate skill B: allows `beta`. Recomputing from base with the union
        // {alpha, beta} must restore `beta` (not possible with cumulative shrink).
        state
            .skill_cache
            .record_activated_allowed_tools(&["beta".to_string()]);
        state.apply_activated_tool_filter();
        assert!(state.tools.iter().any(|tool| tool.name == "alpha"));
        assert!(
            state.tools.iter().any(|tool| tool.name == "beta"),
            "beta must be re-permitted after skill B activation (recompute from base)"
        );
    }
