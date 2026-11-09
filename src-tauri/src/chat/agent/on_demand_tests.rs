use super::*;
use serde_json::json;

fn tools() -> Vec<ChatToolDefinition> {
    let mut tools = vec![
        native_read_file_tool(),
        crate::mcp::types::native_write_file_tool(),
        crate::mcp::types::native_automation_tool(),
    ];
    crate::chat::tool_on_demand::configure_tools(&mut tools, true);
    tools
}

#[tokio::test]
async fn on_demand_direct_legacy_call_executes_without_provider_declaration() {
    let server = MockModelServer::start(vec![
        MockResponse::Sse(vec![
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"direct","function":{"name":"automation_list","arguments":"{}"}}]},"finish_reason":"tool_calls"}]}).to_string(),
            "[DONE]".into(),
        ]),
        MockResponse::Sse(vec![json!({"choices":[{"delta":{"content":"finished"},"finish_reason":"stop"}]}).to_string(), "[DONE]".into()]),
    ]);
    let state = test_app_state();
    let mut config = test_run_config(&state, &server.base_url);
    config.tools = tools();
    config.settings.chat_tools.approval_policy = "auto".into();
    config.effective_chat_tools.approval_policy = "auto".into();
    let result = run_agent_loop(
        config,
        &Arc::new(TestHost::default()),
        &Arc::new(RecordingExecutor::default()),
    )
    .await
    .unwrap();
    assert_eq!(result.tool_records.len(), 1);
    assert_eq!(result.tool_records[0].name, "automation");
    assert_eq!(result.tool_records[0].status, ToolCallStatus::Success);
    assert_eq!(
        serde_json::from_str::<Value>(&result.tool_records[0].arguments).unwrap()["action"],
        "list"
    );
    for body in server.captured_bodies() {
        let body: Value = serde_json::from_str(&body).unwrap();
        let names: Vec<_> = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["function"]["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["read", "write"]);
    }
}

#[cfg(target_os = "windows")]
#[test]
#[ignore = "requires configured provider; sends only synthetic tool catalog and isolated empty automation list"]
fn live_on_demand_catalog_then_automation_list() {
    use tauri::Manager;
    struct NativeDispatcher(tauri::AppHandle);
    impl ToolExecutor for NativeDispatcher {
        fn call<'a>(
            &'a self,
            progress: &'a crate::chat::agent::execute::ToolProgress,
            ctx: &'a ToolExecutionContext<'a>,
            tool: &'a ChatToolDefinition,
            arguments: Value,
        ) -> crate::chat::agent::execute::ToolExecutorFuture<'a> {
            Box::pin(async move {
                match tool.name.as_str() {
                    "read"
                        if arguments["path"]
                            .as_str()
                            .is_some_and(|path| path.starts_with("tool://")) => {}
                    "automation" if arguments["action"] == "list" => {}
                    _ => {
                        return Err(
                            "Live test only permits tool catalog reads and automation list".into(),
                        )
                    }
                }
                let state = self.0.state::<AppState>();
                let output = crate::mcp::registry::call_tool(
                    &self.0,
                    &state,
                    tool,
                    arguments,
                    None,
                    Some(crate::mcp::registry::NativeToolContext::from_execution(
                        ctx,
                        progress.control.clone(),
                    )),
                )
                .await?;
                crate::chat::tool_output::from_mcp(tool, output)
            })
        }
    }
    let path = std::env::var("KIVIO_TOOL_LIVE_SETTINGS").expect("set KIVIO_TOOL_LIVE_SETTINGS");
    let value: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let settings: Settings =
        serde_json::from_value(value.get("settings").unwrap_or(&value).clone()).unwrap();
    let chat = &settings.default_models.chat;
    let provider_id =
        std::env::var("KIVIO_TOOL_LIVE_PROVIDER").unwrap_or_else(|_| chat.provider_id.clone());
    let provider = settings
        .get_provider(&provider_id)
        .expect("configured provider")
        .clone();
    let model = std::env::var("KIVIO_TOOL_LIVE_MODEL").unwrap_or_else(|_| chat.model.clone());
    assert!(provider.authentication_ready());
    let directory = tempfile::tempdir().unwrap();
    let mut context = tauri::generate_context!();
    context.config_mut().identifier = directory.path().to_string_lossy().into_owned();
    context.config_mut().app.windows.clear();
    let application = tauri::Builder::default()
        .any_thread()
        .manage(test_app_state())
        .build(context)
        .unwrap();
    let app = application.handle();
    tauri::async_runtime::block_on(async {
        let state = app.state::<AppState>();
        crate::settings::update_settings_in_memory(&state, |settings| {
            settings.chat_tools.native_tools.working_directory =
                directory.path().to_string_lossy().into_owned();
            Ok(())
        })
        .unwrap();
        let mut config = test_run_config(&state, &provider.base_url);
        config.tool_conversation_id = "auto_adr0008_live".into();
        config.provider = provider;
        config.model = model;
        config.tools = tools();
        config.settings.chat_tools.approval_policy = "auto".into();
        config.effective_chat_tools.approval_policy = "auto".into();
        config.max_output_tokens = 2048;
        let mut context = String::new();
        crate::chat::tool_on_demand::update_prompt(&mut context, &config.tools);
        config.runtime_context = Some(context);
        config.runtime_messages = vec![
            json!({"role":"system","content":"You are testing the Kivio on-demand tool protocol. Follow the user exactly and use the real tools."}),
            json!({"role":"user","content":"First call read with path tool:// to discover tools. Then call write with path tool://automation and content {\"action\":\"list\"} encoded as a JSON string. Do not directly call automation. After both calls succeed, reply DONE. No other actions."}),
        ];
        let result = tokio::time::timeout(
            Duration::from_secs(120),
            run_agent_loop(
                config,
                &Arc::new(TestHost::default()),
                &Arc::new(NativeDispatcher(app.clone())),
            ),
        )
        .await
        .expect("live check exceeded 120 seconds")
        .expect("live run failed");
        for message in &result.api_messages {
            if let Some(calls) = message["tool_calls"].as_array() {
                for call in calls {
                    println!(
                        "LIVE_MODEL_CALL {} {}",
                        call["function"]["name"], call["function"]["arguments"]
                    );
                }
            }
        }
        for record in &result.tool_records {
            println!(
                "LIVE_TOOL {} {} => {:?} {}",
                record.name,
                record.arguments,
                record.status,
                record
                    .result_preview
                    .as_deref()
                    .or(record.error.as_deref())
                    .unwrap_or("")
            );
        }
        println!("LIVE_FINAL {}", result.content);
        let read = result
            .tool_records
            .iter()
            .position(|record| {
                record.name == "read"
                    && serde_json::from_str::<Value>(&record.arguments).unwrap()["path"]
                        == "tool://"
            })
            .expect("catalog read");
        let execute = result
            .tool_records
            .iter()
            .position(|record| record.name == "automation")
            .expect("automation execution");
        assert!(read < execute);
        assert!(result
            .tool_records
            .iter()
            .all(|record| record.status == ToolCallStatus::Success));
        assert!(result.tool_records[execute]
            .result_preview
            .as_deref()
            .unwrap()
            .contains("automations"));
        assert!(result
            .api_messages
            .iter()
            .any(|message| message["role"] == "assistant"
                && message["tool_calls"].as_array().is_some_and(|calls| calls
                    .iter()
                    .any(|call| call["function"]["name"] == "write"))));
    });
}
