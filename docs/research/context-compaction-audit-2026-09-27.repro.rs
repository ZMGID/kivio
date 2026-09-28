// Audit-only regression cases. Include temporarily from agent/loop_tests.rs.
// These assert desired behavior and are expected to fail on the audited revision.

#[tokio::test]
async fn audit_compaction_rejects_length_terminated_summary() {
    let server = MockModelServer::start(vec![MockResponse::Sse(vec![
        long_summary_sse("INCOMPLETE CHECKPOINT "),
        r#"{"choices":[{"delta":{},"finish_reason":"length"}]}"#.to_string(),
        "[DONE]".to_string(),
    ])]);
    let state = test_app_state();
    let provider = test_provider(&server.base_url);
    let messages = vec![
        serde_json::json!({"role":"system", "content":"system"}),
        serde_json::json!({"role":"user", "content":"old request ".repeat(500)}),
        serde_json::json!({"role":"assistant", "content":"old answer ".repeat(500)}),
        serde_json::json!({"role":"user", "content":"continue"}),
    ];
    let outcome = crate::chat::agent::compaction::summarize_history(
        &state, &provider, "test-model", &messages, 100, 40_000,
        8192, 1, "audit", "message", None, None,
    ).await;
    assert!(matches!(outcome, crate::chat::agent::compaction::CompactOutcome::Failed),
        "length-terminated summary was accepted as a complete replacement");
}

#[tokio::test]
async fn audit_compaction_overflow_forces_summary_below_local_threshold() {
    let server = MockModelServer::start(vec![
        MockResponse::Sse(planning_tool_call_sse_events()),
        MockResponse::Status(400,
            r#"{"error":{"message":"This model's maximum context length is 8192 tokens"}}"#.to_string()),
        MockResponse::Sse(vec![
            r#"{"choices":[{"delta":{"content":"retry accepted by mock"},"finish_reason":"stop"}]}"#.to_string(),
            "[DONE]".to_string(),
        ]),
    ]);
    let state = test_app_state();
    let mut config = test_run_config(&state, &server.base_url);
    config.provider.model_overrides.insert("test-model".to_string(), crate::settings::ModelInfo {
        context_window: Some(200_000), ..Default::default()
    });
    config.runtime_messages.insert(1, serde_json::json!({"role":"user", "content":"OLD ".repeat(30_000)}));
    config.runtime_messages.insert(2, serde_json::json!({"role":"assistant", "content":"old task completed"}));
    let host = TestHost::default();
    let executor = RecordingExecutor::default();
    let result = run_agent_loop(config, &host, &executor).await.unwrap();
    assert_eq!(result.stream_outcome, "recovered");
    let bodies = server.captured_bodies();
    assert!(bodies.iter().any(|body| body.contains("context summarization assistant")),
        "provider reported overflow, but all {} requests bypassed summarization", bodies.len());
}

#[tokio::test]
async fn audit_compaction_second_summary_call_remains_cancellable() {
    let server = MockModelServer::start(vec![
        MockResponse::Sse(vec![long_summary_sse("HISTORY "), "[DONE]".to_string()]),
        MockResponse::SseThenHang(vec![]),
    ]);
    let state = test_app_state();
    let provider = test_provider(&server.base_url);
    let messages = vec![
        serde_json::json!({"role":"system", "content":"system"}),
        serde_json::json!({"role":"user", "content":"old request ".repeat(500)}),
        serde_json::json!({"role":"assistant", "content":"old answer"}),
        serde_json::json!({"role":"user", "content":"current request ".repeat(500)}),
        serde_json::json!({"role":"assistant", "content":"recent suffix"}),
    ];
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let captured = Arc::clone(&server.captured_bodies);
    let cancel_task = tokio::spawn(async move {
        loop {
            if captured.lock().unwrap().len() >= 2 { break; }
            sleep(Duration::from_millis(5)).await;
        }
        let _ = tx.send(());
    });
    let outcome = tokio::time::timeout(Duration::from_secs(1),
        crate::chat::agent::compaction::summarize_history(
            &state, &provider, "test-model", &messages, 100, 40_000,
            8192, 1, "audit", "message", None,
            Some(Box::pin(async { let _ = rx.await; })),
        )).await;
    cancel_task.abort();
    assert_eq!(server.captured_bodies().len(), 2, "must exercise both summary calls");
    assert!(matches!(outcome, Ok(crate::chat::agent::compaction::CompactOutcome::Cancelled)),
        "cancel signal did not interrupt the second summary request");
}
