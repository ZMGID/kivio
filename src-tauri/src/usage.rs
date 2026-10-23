use crate::state::AppState;
use tauri::{AppHandle, Manager};
#[path = "model_usage.rs"]
mod shared;
pub use shared::*;
include!("usage_ledger.rs");

pub fn record_model_call(state: &AppState, input: UsageRecordInput<'_>) {
    record_model_call_in(&state.usage_ledger, &state.usage_dir, input);
}
impl UsageLedger {
    fn conversation_cost(
        &self,
        dir: &Path,
        conversation_id: &str,
        children: &crate::chat::sub_agent::runtime::Runtime,
    ) -> Result<ConversationCost, String> {
        let mut ids = vec![conversation_id.to_owned()];
        ids.extend(
            children
                .list(conversation_id)?
                .into_iter()
                .map(|c| format!("subagent-{}", c.id)),
        );
        self.conversation_cost_for_ids(dir, &ids)
    }
}

pub fn usage_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir unavailable: {e}"))?
        .join(USAGE_DIR_NAME);
    fs::create_dir_all(&dir).map_err(|e| format!("create usage dir: {e}"))?;
    Ok(dir)
}

#[tauri::command]
pub async fn usage_get_conversation_cost(
    app: AppHandle,
    conversation_id: String,
) -> Result<ConversationCost, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let children = crate::chat::sub_agent::control::runtime(&app)?;
        state
            .usage_ledger
            .conversation_cost(&state.usage_dir, &conversation_id, &children)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn usage_get_stats(
    app: AppHandle,
    query: Option<UsageStatsQuery>,
) -> Result<UsageStatsResponse, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        state
            .usage_ledger
            .stats(&state.usage_dir, query.unwrap_or_default())
    })
    .await
    .map_err(|error| format!("usage_get_stats failed: {error}"))?
}

#[tauri::command]
pub async fn usage_clear(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        state.usage_ledger.clear(&state.usage_dir)
    })
    .await
    .map_err(|error| format!("usage_clear failed: {error}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_cache_write_cost_uses_requested_retention_without_affecting_other_models() {
        let mut provider: ModelProvider = serde_json::from_value(serde_json::json!({
            "id":"p", "name":"test", "baseUrl":"https://example.test", "apiFormat":"anthropic_messages",
            "modelOverrides": {
                "claude-test": {"pricing":{"input":2.0,"output":10.0,"cachedInput":0.2}},
                "deepseek-test": {"pricing":{"input":2.0,"output":10.0,"cachedInput":0.2}}
            }
        })).unwrap();
        for (model, retention, expected) in [
            ("claude-test", "short", 0.003285),
            ("claude-test", "long", 0.00336),
            ("deepseek-test", "short", 0.00326),
        ] {
            provider.request.prompt_cache_retention = retention.into();
            let mut record = base_record("anthropic_messages");
            record.model = model.into();
            apply_cost(&provider, &mut record);
            assert!(
                (record.cost_usd.unwrap() - expected).abs() < 1e-10,
                "{model}/{retention}: {:?}",
                record.cost_usd
            );
        }
    }

    #[test]
    fn parses_openai_cached_and_reasoning_usage() {
        let value = serde_json::json!({
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 20,
                "total_tokens": 120,
                "prompt_tokens_details": { "cached_tokens": 80 },
                "completion_tokens_details": { "reasoning_tokens": 5 }
            }
        });
        let usage = model_usage_from_openai_value(&value).expect("usage");
        assert_eq!(usage.input_tokens, Some(100));
        assert_eq!(usage.cached_input_tokens, Some(80));
        assert_eq!(usage.reasoning_tokens, Some(5));
    }

    #[test]
    fn filter_missing_usage_status() {
        let mut record = UsageRecord {
            id: "usage_test".to_string(),
            created_at: Local::now().timestamp(),
            completed_at: Local::now().timestamp(),
            duration_ms: 1,
            first_token_ms: None,
            source: "chat".to_string(),
            operation: "plain".to_string(),
            provider_id: "p".to_string(),
            provider_name: "Provider".to_string(),
            model: "model".to_string(),
            api_format: "openai_chat".to_string(),
            status: "success".to_string(),
            status_code: Some(200),
            usage_source: "missing".to_string(),
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            cached_input_tokens: None,
            cache_creation_input_tokens: None,
            reasoning_tokens: None,
            reasoning_effort: None,
            cost_usd: None,
            cost_source: "unavailable".to_string(),
            conversation_id: None,
            message_id: None,
            error_kind: None,
        };
        let query = UsageStatsQuery {
            status: Some("missing_usage".to_string()),
            ..UsageStatsQuery::default()
        };
        assert_eq!(filter_records(vec![record.clone()], &query).len(), 1);
        record.usage_source = "provider_reported".to_string();
        assert!(filter_records(vec![record], &query).is_empty());
    }

    fn base_record(api_format: &str) -> UsageRecord {
        UsageRecord {
            id: "usage_test".to_string(),
            created_at: Local::now().timestamp(),
            completed_at: Local::now().timestamp(),
            duration_ms: 1,
            first_token_ms: None,
            source: "chat".to_string(),
            operation: "plain".to_string(),
            provider_id: "p".to_string(),
            provider_name: "Provider".to_string(),
            model: "model".to_string(),
            api_format: api_format.to_string(),
            status: "success".to_string(),
            status_code: Some(200),
            usage_source: "provider_reported".to_string(),
            input_tokens: Some(1_000),
            output_tokens: Some(100),
            total_tokens: Some(1_100),
            cached_input_tokens: Some(800),
            cache_creation_input_tokens: Some(50),
            reasoning_tokens: None,
            reasoning_effort: None,
            cost_usd: None,
            cost_source: "unavailable".to_string(),
            conversation_id: None,
            message_id: None,
            error_kind: None,
        }
    }

    #[test]
    fn stats_pagination_and_cache_follow_append_and_clear() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = UsageLedger::default();
        let query = |offset| UsageStatsQuery {
            range: "all".into(),
            limit: Some(1),
            offset: Some(offset),
            ..Default::default()
        };
        for i in 0..3 {
            let mut record = base_record("openai_responses");
            record.id = format!("record-{i}");
            record.created_at += i;
            ledger.append(dir.path(), &record).unwrap();
        }
        let first = ledger.stats(dir.path(), query(0)).unwrap();
        let second = ledger.stats(dir.path(), query(1)).unwrap();
        assert_eq!(first.total_logs, 3);
        assert_eq!(first.summary.total_requests, second.summary.total_requests);
        assert_ne!(first.logs[0].id, second.logs[0].id);
        ledger
            .append(dir.path(), &base_record("openai_responses"))
            .unwrap();
        assert_eq!(ledger.stats(dir.path(), query(0)).unwrap().total_logs, 4);
        ledger.clear(dir.path()).unwrap();
        let empty = ledger.stats(dir.path(), query(0)).unwrap();
        assert_eq!(empty.total_logs, 0);
        assert!(empty.logs.is_empty());
    }

    #[test]
    fn conversation_cost_includes_durable_children_and_continuations_once() {
        use crate::chat::sub_agent::runtime::{Profile, Runtime};
        let dir = tempfile::tempdir().unwrap();
        let ledger = UsageLedger::default();
        let children_dir = dir.path().join("children");
        let runtime = Runtime::open(children_dir.clone()).unwrap();
        let child = runtime
            .start("parent", "run", "key", "child", Profile::default(), "work")
            .unwrap();
        let unrelated = runtime
            .start(
                "other",
                "run-other",
                "key-other",
                "other",
                Profile::default(),
                "work",
            )
            .unwrap();
        let child_id = format!("subagent-{}", child.id);
        let unrelated_id = format!("subagent-{}", unrelated.id);
        runtime
            .finish(
                "parent",
                &child.id,
                &child.current().id,
                Ok(("done".into(), None)),
            )
            .unwrap();
        let (continued, _) = runtime
            .resume(
                "parent", &child.id, "next-run", "next-key", "user", "continue",
            )
            .unwrap();
        assert_eq!(continued.runs.len(), 2);
        let usage_dir = dir.path().join("usage");
        for (id, cost) in [
            ("parent", Some(0.125)),
            (child_id.as_str(), Some(0.25)),
            (child_id.as_str(), Some(0.5)),
            (child_id.as_str(), None),
            (unrelated_id.as_str(), Some(99.0)),
        ] {
            let mut record = base_record("openai_chat");
            record.conversation_id = Some(id.into());
            record.cost_usd = cost;
            ledger.append(&usage_dir, &record).unwrap();
        }
        drop(runtime);
        let reopened = Runtime::open(children_dir).unwrap();
        let cost = ledger
            .conversation_cost(&usage_dir, "parent", &reopened)
            .unwrap();
        assert_eq!(cost.cost_usd, Some(0.875));
        assert_eq!(cost.unpriced_requests, 1);
        // Warm queries must use the aggregate, not reread the global ledger.
        let ledger_path = usage_dir.join(monthly_file_name(Local::now().timestamp()));
        let hidden = ledger_path.with_extension("held");
        fs::rename(&ledger_path, &hidden).unwrap();
        let cached = ledger
            .conversation_cost(&usage_dir, "parent", &reopened)
            .unwrap();
        assert_eq!(cached.cost_usd, Some(0.875));
        fs::rename(hidden, ledger_path).unwrap();
        let mut extra = base_record("openai_chat");
        extra.conversation_id = Some("parent".into());
        extra.cost_usd = Some(0.125);
        ledger.append(&usage_dir, &extra).unwrap();
        assert_eq!(
            ledger
                .conversation_cost(&usage_dir, "parent", &reopened)
                .unwrap()
                .cost_usd,
            Some(1.0)
        );
        let restart = UsageLedger::default();
        assert_eq!(
            restart
                .conversation_cost(&usage_dir, "parent", &reopened)
                .unwrap()
                .cost_usd,
            Some(1.0)
        );
        ledger.clear(&usage_dir).unwrap();
        assert_eq!(
            ledger
                .conversation_cost(&usage_dir, "parent", &reopened)
                .unwrap()
                .cost_usd,
            None
        );
        ledger.append(&usage_dir, &extra).unwrap();
        assert_eq!(
            ledger
                .conversation_cost(&usage_dir, "parent", &reopened)
                .unwrap()
                .cost_usd,
            Some(0.125)
        );
    }

    #[test]
    fn concurrent_appends_and_failed_writes_preserve_the_cost_aggregate() {
        let dir = tempfile::tempdir().unwrap();
        let children =
            crate::chat::sub_agent::runtime::Runtime::open(dir.path().join("children")).unwrap();
        let ledger = UsageLedger::default();
        ledger
            .conversation_cost(dir.path(), "parent", &children)
            .unwrap();
        let mut record = base_record("openai_chat");
        record.conversation_id = Some("parent".into());
        record.cost_usd = Some(0.125);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for _ in 0..16 {
                        ledger.append(dir.path(), &record).unwrap();
                    }
                });
            }
        });
        assert_eq!(
            ledger
                .conversation_cost(dir.path(), "parent", &children)
                .unwrap()
                .cost_usd,
            Some(16.0)
        );
        assert_eq!(
            UsageLedger::default()
                .conversation_cost(dir.path(), "parent", &children)
                .unwrap()
                .cost_usd,
            Some(16.0)
        );
        // A failed append is not added to the aggregate and can recover afterward.
        let path = dir.path().join(monthly_file_name(record.created_at));
        let held = path.with_extension("held");
        fs::rename(&path, &held).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(ledger.append(dir.path(), &record).is_err());
        let cold = UsageLedger::default();
        assert!(
            cold.conversation_cost(dir.path(), "parent", &children)
                .is_err(),
            "an unreadable ledger must not become a permanent partial aggregate"
        );
        fs::remove_dir(&path).unwrap();
        fs::rename(&held, &path).unwrap();
        assert_eq!(
            cold.conversation_cost(dir.path(), "parent", &children)
                .unwrap()
                .cost_usd,
            Some(16.0)
        );
        assert_eq!(
            ledger
                .conversation_cost(dir.path(), "parent", &children)
                .unwrap()
                .cost_usd,
            Some(16.0)
        );
    }

    #[test]
    fn conversation_cost_covers_all_recorded_requests_without_conflating_unknown_and_free() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = UsageLedger::default();
        let children =
            crate::chat::sub_agent::runtime::Runtime::open(dir.path().join("children")).unwrap();
        for (id, age_days, cost, usage_source) in [
            ("a", 400, Some(0.125), "provider_reported"),
            ("a", 0, Some(0.25), "provider_reported"),
            ("a", 0, None, "provider_reported"),
            ("a", 0, Some(0.0), "missing"),
            ("other", 0, Some(999.0), "provider_reported"),
            ("free", 0, Some(0.0), "provider_reported"),
        ] {
            let mut record = base_record("openai_chat");
            record.conversation_id = Some(id.into());
            record.created_at -= age_days * 86400;
            record.cost_usd = cost;
            record.usage_source = usage_source.into();
            ledger.append(dir.path(), &record).unwrap();
        }
        let cost = ledger
            .conversation_cost(dir.path(), "a", &children)
            .unwrap();
        assert_eq!(cost.cost_usd, Some(0.375));
        assert_eq!(cost.unpriced_requests, 2);
        assert_eq!(cost.skipped_records, 0);
        assert_eq!(
            ledger
                .conversation_cost(dir.path(), "free", &children)
                .unwrap()
                .cost_usd,
            Some(0.0)
        );
        assert_eq!(
            ledger
                .conversation_cost(dir.path(), "absent", &children)
                .unwrap()
                .cost_usd,
            None
        );
    }

    #[test]
    fn today_companion_snapshot_matches_local_day_and_preserves_missing_costs() {
        let dir = tempfile::tempdir().unwrap();
        let mut old = base_record("openai_chat");
        old.created_at = range_start("today").unwrap() - 1;
        old.cost_usd = Some(900.0);
        append_record(dir.path(), &old).unwrap();
        let mut known = base_record("anthropic_messages");
        known.cost_usd = Some(0.125);
        append_record(dir.path(), &known).unwrap();
        append_record(dir.path(), &base_record("openai_chat")).unwrap();
        let snapshot = today_usage(dir.path()).unwrap();
        assert_eq!(snapshot.summary.total_requests, 2);
        assert_eq!(snapshot.summary.total_tokens, 3_050);
        assert_eq!(snapshot.summary.total_cost_usd, 0.125);
        assert_eq!(snapshot.missing_cost_requests, 1);
        assert_eq!(snapshot.skipped_records, 0);
        fs::write(dir.path().join("unreadable.jsonl"), "{broken record}\n").unwrap();
        assert_eq!(today_usage(dir.path()).unwrap().skipped_records, 1);
        let not_directory = dir.path().join("file");
        fs::write(&not_directory, "").unwrap();
        assert!(today_usage(&not_directory).is_err());
    }

    /// 口径对齐 context_estimate::anchor_total_tokens（85a3056）：Anthropic 的
    /// input 是非缓存部分，全量要显式加 cache_read + cache_creation；OpenAI 的
    /// input 已含 cached，直接用。
    #[test]
    fn effective_input_and_total_disambiguate_by_api_format() {
        let anthropic = base_record("anthropic_messages");
        // 1000(非缓存) + 800(cache_read) + 50(cache_creation) = 1850
        assert_eq!(record_effective_input_tokens(&anthropic), 1_850);
        // 全量 = 1850 + 100(out)，不能用漏 cache 的落盘 total_tokens(1100)。
        assert_eq!(record_total_tokens(&anthropic), 1_950);

        let openai = base_record("openai_chat");
        assert_eq!(record_effective_input_tokens(&openai), 1_000);
        assert_eq!(record_total_tokens(&openai), 1_100); // 落盘 total 优先
    }

    #[test]
    fn usage_record_new_fields_are_backward_compatible_and_camel_case() {
        let mut value = serde_json::to_value(base_record("openai_chat")).expect("serialize");
        let object = value.as_object_mut().expect("record object");
        object.remove("firstTokenMs");
        object.remove("reasoningEffort");

        let old_record: UsageRecord = serde_json::from_value(value).expect("old record");
        assert_eq!(old_record.first_token_ms, None);
        assert_eq!(old_record.reasoning_effort, None);

        let mut current = base_record("openai_responses");
        current.first_token_ms = Some(1_250);
        current.reasoning_effort = Some("high".to_string());
        let serialized = serde_json::to_value(current).expect("serialize current record");
        assert_eq!(serialized["firstTokenMs"], 1_250);
        assert_eq!(serialized["reasoningEffort"], "high");
    }

    #[test]
    fn skips_usage_file_before_range_start() {
        let start = Local
            .with_ymd_and_hms(2026, 6, 8, 0, 0, 0)
            .single()
            .expect("valid date")
            .timestamp();

        assert!(usage_file_is_before_start(
            Path::new("usage-2026-05.jsonl"),
            Some(start)
        ));
        assert!(!usage_file_is_before_start(
            Path::new("usage-2026-06.jsonl"),
            Some(start)
        ));
        assert!(!usage_file_is_before_start(
            Path::new("usage-2026-07.jsonl"),
            Some(start)
        ));
        assert!(!usage_file_is_before_start(
            Path::new("other.jsonl"),
            Some(start)
        ));
        assert!(!usage_file_is_before_start(
            Path::new("usage-2026-05.jsonl"),
            None
        ));
    }
}
