use std::time::Duration;

use tauri::{AppHandle, Manager, State};
use tokio::time::timeout;

use crate::chat::agent::{execute::truncate_chars, stop as agent_stop};
use crate::chat::attachments::title_source_for_user_message;
use crate::chat::model::{
    generate_request_from_openai_messages, GenerateOptions, GenerateRequestContext,
};
use crate::chat::model_metadata::{
    chat_max_output_tokens_on_wire, model_can_generate_images_directly,
    thinking_capabilities_for_model, ThinkingOffMode,
};
use crate::chat::repository::ConversationMetadataMutation;
use crate::chat::{ChatMessage, Conversation};
use crate::settings::{SessionModel, Settings};
use crate::state::AppState;

pub(super) async fn resolve_conversation_title(
    settings: &Settings,
    state: &AppState,
    conversation: &Conversation,
    user_content: &str,
    assistant_content: &str,
) -> String {
    let session = SessionModel {
        provider_id: conversation.provider_id.as_str(),
        model: conversation.model.as_str(),
    };
    match timeout(
        Duration::from_secs(30),
        generate_title_with_model(
            settings,
            state,
            &conversation.id,
            Some(session),
            user_content,
            assistant_content,
        ),
    )
    .await
    {
        Ok(Some(title)) => title,
        // 兜底必须留痕：这三条路都会安静地把标题变成「第一句用户消息」，用户只能看到
        // 「标题没总结」，而用量日志里连一条请求都没有（超时会把 future 丢掉、来不及记账）。
        Ok(None) => {
            eprintln!("[title] 跳过模型总结（provider/model 未解析或被门控），退回截断标题");
            generate_title(user_content)
        }
        Err(_) => {
            eprintln!("[title] 模型总结 30s 超时，退回截断标题");
            generate_title(user_content)
        }
    }
}

/// The assistant reply owns its terminal path; title summarization is a best-effort
/// metadata follow-up and must never delay that terminal or overwrite a rename.
pub(super) fn schedule_auto_title_summary(
    app: AppHandle,
    settings: Settings,
    conversation: Conversation,
    first_user: String,
    assistant_content: String,
) {
    tauri::async_runtime::spawn(async move {
        let title = resolve_conversation_title(
            &settings,
            app.state::<AppState>().inner(),
            &conversation,
            &first_user,
            &assistant_content,
        )
        .await;
        let previous_auto_title = generate_title(&first_user);
        if title == previous_auto_title || title.trim().is_empty() {
            return;
        }
        let conversation_id = conversation.id.clone();
        let updated = crate::chat::repository::repository(&app)
            .mutate(&app, &conversation_id, |latest| {
                apply_auto_title(
                    latest,
                    &conversation,
                    &first_user,
                    &assistant_content,
                    &title,
                )
            })
            .await;
        match updated {
            Ok(updated) => crate::chat::protocol::emit_conversation_event(
                &app,
                &conversation_id,
                updated.revision,
                crate::chat::protocol::ChatConversationEvent::TitleUpdated { title },
            ),
            Err(crate::chat::repository::ConversationRepositoryError::Storage(message))
                if message == STALE_AUTO_TITLE => {}
            Err(error) => eprintln!("[title] 后台标题写入失败: {error}"),
        }
    });
}

const STALE_AUTO_TITLE: &str = "conversation title or source changed before summary completed";

/// Called inside the repository mutation lock, against the latest stored conversation.
fn apply_auto_title(
    latest: &mut Conversation,
    source: &Conversation,
    first_user: &str,
    assistant_content: &str,
    title: &str,
) -> Result<(), String> {
    let source_user = source
        .messages
        .iter()
        .find(|message| message.role == "user");
    let latest_user = latest
        .messages
        .iter()
        .find(|message| message.role == "user");
    let user_unchanged = source_user.zip(latest_user).is_some_and(|(before, now)| {
        before.id == now.id
            && title_source_for_user_message(&before.content, &before.attachments) == first_user
            && title_source_for_user_message(&now.content, &now.attachments) == first_user
    });
    // A tool-using first turn may contain several assistant messages. Pin the actual
    // final answer used by this task, not simply the first assistant in the history.
    let source_answer = source
        .messages
        .iter()
        .rev()
        .find(|message| message.role == "assistant" && message.content == assistant_content);
    let answer_unchanged = source_answer.is_some_and(|answer| {
        latest.messages.iter().any(|message| {
            message.id == answer.id
                && message.role == "assistant"
                && message.content == assistant_content
        })
    });
    // Do not compare the whole revision: later turns and unrelated metadata edits
    // are allowed. Edits/rewinds of either source message invalidate this result.
    if latest.id != source.id
        || !user_unchanged
        || !answer_unchanged
        || !is_auto_title(&latest.title, Some(first_user))
    {
        return Err(STALE_AUTO_TITLE.into());
    }
    latest.title = title.to_string();
    Ok(())
}

/// Same request shape as a normal reply on the title model (output budget, first
/// turn replayed as real user/assistant messages), but thinking is `low`: a title
/// does not need deep reasoning. Must stream: some Responses relays only serve
/// stream reliably.
async fn generate_title_with_model(
    settings: &Settings,
    state: &AppState,
    conversation_id: &str,
    session: Option<SessionModel<'_>>,
    user_content: &str,
    assistant_content: &str,
) -> Option<String> {
    let (provider_id, model) = settings.effective_title_summary_model_for_session(session);
    let Some(provider) = settings.get_provider(&provider_id).cloned() else {
        eprintln!("[title] provider 未找到: {provider_id}");
        return None;
    };
    if !provider.authentication_ready() || model.trim().is_empty() {
        eprintln!("[title] provider 未登录或 model 为空: {provider_id} / {model}");
        return None;
    }
    if model_can_generate_images_directly(&provider, &model) {
        eprintln!("[title] 生图模型不用于标题: {model}");
        return None;
    }

    let language = crate::settings::resolve_chat_language(settings);
    let retry_attempts = if settings.retry_enabled {
        settings.retry_attempts as usize
    } else {
        1
    };
    // `low`, or the model's lowest level when it has no `low`. Models without an
    // effort knob get no reasoning field, like a normal reply.
    let capabilities = thinking_capabilities_for_model(Some(&provider), &model);
    let thinking_level = (capabilities.off_mode != ThinkingOffMode::NotApplicable)
        .then(|| {
            capabilities
                .levels
                .iter()
                .find(|level| *level == "low")
                .or_else(|| capabilities.levels.first())
                .cloned()
        })
        .flatten();
    let thinking_enabled = true;
    let request = generate_request_from_openai_messages(
        &model,
        title_summary_messages(user_content, assistant_content, &language),
        None,
        GenerateOptions {
            thinking_enabled,
            thinking_level,
            // Same budget as a normal reply on this model, not a fixed 2048.
            max_tokens: chat_max_output_tokens_on_wire(
                Some(&provider),
                &model,
                settings.chat.max_output_tokens,
            ),
            ..GenerateOptions::default()
        },
        "Chat title summary",
        GenerateRequestContext::new(Some(conversation_id), Some("")),
    );
    let output = crate::chat::agent::planning::generate_via_stream_collect(
        state,
        &provider,
        retry_attempts,
        request,
    )
    .await
    .map_err(|err| eprintln!("[title] 模型请求失败: {err}"))
    .ok()?;
    let raw =
        agent_stop::assistant_content_from_api_message(&output.to_openai_compatible_message());
    sanitize_generated_title(&raw)
}

/// System instruction, then the first turn as real messages, then the ask.
pub(super) fn title_summary_messages(
    user_content: &str,
    assistant_content: &str,
    language: &str,
) -> Vec<serde_json::Value> {
    let user = truncate_chars(user_content.trim(), 1200);
    let assistant = truncate_chars(assistant_content.trim(), 1200);
    let (system, instruction) = if language.starts_with("zh") {
        (
            "你只负责为对话生成简洁标题。只输出标题本身，不要解释。",
            "请为以上对话生成一个概括主题的简洁中文标题。要求：只输出标题本身；不要引号；不要句号；不超过 14 个汉字，最多 20 个字符。",
        )
    } else {
        (
            "You only generate concise conversation titles. Output only the title, with no explanation.",
            "Create a concise English title summarizing the conversation above. Rules: output only the title; no quotes; no period; 3-6 words.",
        )
    };
    let mut messages = vec![serde_json::json!({ "role": "system", "content": system })];
    if assistant.is_empty() {
        messages.push(
            serde_json::json!({ "role": "user", "content": format!("{user}\n\n{instruction}") }),
        );
    } else {
        messages.push(serde_json::json!({ "role": "user", "content": user }));
        messages.push(serde_json::json!({ "role": "assistant", "content": assistant }));
        messages.push(serde_json::json!({ "role": "user", "content": instruction }));
    }
    messages
}

pub(super) fn sanitize_generated_title(raw: &str) -> Option<String> {
    let mut title = raw
        .trim()
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?
        .to_string();

    title = title
        .trim_start_matches(['-', '*', '•', ' '])
        .trim_matches(['"', '\'', '`', '“', '”', '‘', '’', '。', '.', ' '])
        .to_string();
    for prefix in ["标题：", "标题:", "Title:", "Title：", "title:", "title："] {
        if let Some(rest) = title.strip_prefix(prefix) {
            title = rest.trim().to_string();
        }
    }
    title = title
        .trim_matches(['"', '\'', '`', '“', '”', '‘', '’', '。', '.', ' '])
        .to_string();
    if title.is_empty() {
        return None;
    }
    Some(generate_title(&title))
}

const FORK_TITLE_SUFFIX: &str = "（分支）";

/// 创建会话时写入的内部占位。界面不得展示这个字样。
pub(crate) const PLACEHOLDER_CONVERSATION_TITLE: &str = "新对话";

pub(crate) fn is_placeholder_title(title: &str) -> bool {
    let trimmed = title.trim();
    let base = trimmed
        .strip_suffix(FORK_TITLE_SUFFIX)
        .map(str::trim)
        .unwrap_or(trimmed);
    base == PLACEHOLDER_CONVERSATION_TITLE
}

pub(super) fn is_auto_title(title: &str, first_user: Option<&str>) -> bool {
    is_placeholder_title(title) || first_user.is_some_and(|user| title == generate_title(user))
}

/// 首轮用户消息 + 第一条非空助手回复，供标题模型使用。
/// 空对话、或首条用户消息既无正文也无附件时返回 `None`。
pub(super) fn first_turn_title_inputs(messages: &[ChatMessage]) -> Option<(String, String)> {
    let user = messages.iter().find(|message| message.role == "user")?;
    if user.content.trim().is_empty() && user.attachments.is_empty() {
        return None;
    }
    let user_content = title_source_for_user_message(&user.content, &user.attachments);
    let assistant_content = messages
        .iter()
        .filter(|message| message.role == "assistant")
        .map(|message| message.content.trim())
        .find(|content| !content.is_empty())
        .unwrap_or("")
        .to_string();
    Some((user_content, assistant_content))
}

pub(super) fn apply_fork_title_suffix(title: &str, is_fork: bool) -> String {
    if !is_fork || title.ends_with(FORK_TITLE_SUFFIX) {
        return title.to_string();
    }
    format!("{title}{FORK_TITLE_SUFFIX}")
}

/// 按用户请求重新生成对话标题。会覆盖手动重命名；空对话报错。
#[tauri::command]
pub(crate) async fn chat_regenerate_title(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<serde_json::Value, String> {
    let snapshot = crate::chat::repository::repository(&app)
        .get(&app, &conversation_id)
        .await
        .map_err(crate::chat::repository::repository_error)?;
    let (user_content, assistant_content) = first_turn_title_inputs(&snapshot.messages)
        .ok_or_else(|| "对话还没有可用于生成标题的内容".to_string())?;

    let settings = state.settings_read().clone();
    let mut title = resolve_conversation_title(
        &settings,
        state.inner(),
        &snapshot,
        &user_content,
        &assistant_content,
    )
    .await;
    if title.trim().is_empty() {
        title = generate_title(&user_content);
    }
    if title.trim().is_empty() {
        return Err("无法生成标题".to_string());
    }
    title = apply_fork_title_suffix(&title, snapshot.forked_from.is_some());

    let mut conversation = crate::chat::repository::repository(&app)
        .update_metadata(
            &app,
            &conversation_id,
            ConversationMetadataMutation::Title(title),
        )
        .await
        .map_err(crate::chat::repository::repository_error)?;

    super::catalog::strip_transcripts_for_frontend(&mut conversation);
    Ok(serde_json::json!({
        "success": true,
        "conversation": conversation,
    }))
}

/// 生成对话标题（本地兜底截断）
pub(super) fn generate_title(content: &str) -> String {
    let trimmed = content.trim();
    let title = trimmed.chars().take(30).collect::<String>();
    if trimmed.chars().count() > 30 {
        format!("{title}...")
    } else {
        title
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ModelProvider;
    use crate::state::AppState;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    #[test]
    fn placeholder_title_is_the_create_sentinel() {
        assert!(is_placeholder_title("新对话"));
        assert!(is_placeholder_title(" 新对话 "));
        assert!(is_placeholder_title("新对话（分支）"));
        assert!(!is_placeholder_title("Apex 掉帧"));
        assert!(!is_placeholder_title(""));
    }

    #[test]
    fn auto_title_recognizes_only_placeholder_or_local_fallback() {
        assert!(is_auto_title("新对话", Some("天气如何")));
        assert!(is_auto_title("天气如何", Some("天气如何")));
        assert!(!is_auto_title("出行计划", Some("天气如何")));
    }

    fn title_conversation() -> Conversation {
        serde_json::from_value(serde_json::json!({
            "id": "title-test", "revision": 10, "title": "天气如何",
            "provider_id": "p", "model": "m", "created_at": 1, "updated_at": 1,
            "messages": [test_chat_message("user", "天气如何"), test_chat_message("assistant", "今天晴天")]
        })).unwrap()
    }

    // Pause at the model I/O boundary, change the stored snapshot, then exercise the
    // same mutation used by schedule_auto_title_summary. No desktop runtime needed.
    async fn delayed_summary_after_change(
        change: impl FnOnce(&mut Conversation),
    ) -> (Result<(), String>, Conversation, Conversation) {
        let source = title_conversation();
        let stored = Arc::new(tokio::sync::Mutex::new(source.clone()));
        let task_stored = Arc::clone(&stored);
        let (send_title, title_ready) = tokio::sync::oneshot::channel::<String>();
        let task = tokio::spawn(async move {
            let title = title_ready.await.unwrap();
            let mut latest = task_stored.lock().await;
            apply_auto_title(&mut latest, &source, "天气如何", "今天晴天", &title)
        });
        let before = {
            let mut latest = stored.lock().await;
            change(&mut latest);
            latest.revision += 1;
            latest.clone()
        };
        send_title.send("天气查询".into()).unwrap();
        let outcome = task.await.unwrap();
        let after = stored.lock().await.clone();
        (outcome, before, after)
    }

    #[tokio::test]
    async fn delayed_summary_rejects_changed_or_removed_source_without_mutating_snapshot() {
        for change in [
            (|c: &mut Conversation| c.title = "出行计划".into()) as fn(&mut Conversation),
            |c| c.messages[0].content = "怎么修电脑".into(),
            |c| c.messages[0].id = "replacement-user".into(),
            |c| c.messages[1].content = "明天暴雨".into(),
            |c| c.messages[1].id = "regenerated-assistant".into(),
            |c| c.messages.truncate(1),
            |c| c.messages.clear(),
        ] {
            let (outcome, before, after) = delayed_summary_after_change(change).await;
            assert_eq!(outcome, Err(STALE_AUTO_TITLE.into()));
            assert_eq!(
                serde_json::to_value(after).unwrap(),
                serde_json::to_value(before).unwrap()
            );
        }
    }

    #[tokio::test]
    async fn delayed_summary_allows_unrelated_later_turn_and_preserves_it() {
        let (outcome, mut expected, after) = delayed_summary_after_change(|c| {
            let mut next = test_chat_message("user", "那明天呢");
            next.id = "next-user".into();
            c.messages.push(next);
            c.model = "another-model".into();
        })
        .await;
        assert!(outcome.is_ok());
        expected.title = "天气查询".into();
        assert_eq!(
            serde_json::to_value(after).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }

    fn test_app_state() -> AppState {
        let offline_models =
            crate::offline_models::OfflineModelManager::headless(reqwest::Client::new());
        AppState::base(
            Settings::default(),
            std::env::temp_dir().join(format!("kivio-title-test-usage-{}", uuid::Uuid::new_v4())),
            reqwest::Client::new(),
            #[cfg(target_os = "macos")]
            crate::macos_ocr::MacOcrClient::disabled(),
            offline_models.clone(),
            crate::rapidocr::RapidOcrClient::new(offline_models),
        )
    }

    fn test_provider(base_url: &str) -> ModelProvider {
        ModelProvider {
            id: "title-provider".to_string(),
            name: "Title Provider".to_string(),
            api_keys: vec!["test-key".to_string()],
            api_key_legacy: None,
            base_url: base_url.to_string(),
            available_models: Vec::new(),
            enabled_models: Vec::new(),
            enabled: true,
            api_format: "openai_chat".to_string(),
            model_overrides: std::collections::HashMap::new(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        }
    }

    fn start_sse_mock(events: Vec<String>) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let captured = Arc::new(Mutex::new(Vec::new()));
        let captured_thread = Arc::clone(&captured);
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .ok();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            let header_end = loop {
                let Ok(n) = stream.read(&mut chunk) else {
                    return;
                };
                if n == 0 {
                    return;
                }
                buf.extend_from_slice(&chunk[..n]);
                if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break pos + 4;
                }
            };
            let headers = String::from_utf8_lossy(&buf[..header_end]).to_ascii_lowercase();
            let content_length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            while buf.len() < header_end + content_length {
                let Ok(n) = stream.read(&mut chunk) else {
                    break;
                };
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            let body = String::from_utf8_lossy(&buf[header_end..]).into_owned();
            captured_thread
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(body);

            let sse: String = events
                .iter()
                .map(|event| format!("data: {event}\n\n"))
                .collect();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{sse}",
                sse.len()
            );
            let _ = stream.write_all(response.as_bytes());
        });
        (format!("http://{addr}/v1"), captured)
    }

    #[tokio::test]
    async fn title_summary_matches_normal_reply_request() {
        let (base_url, captured) = start_sse_mock(vec![
            r#"{"choices":[{"delta":{"content":"吉林天气查询"}}]}"#.to_string(),
            "[DONE]".to_string(),
        ]);

        let state = test_app_state();
        let mut settings = Settings::default();
        let mut provider = test_provider(&base_url);
        provider.model_overrides.insert(
            "title-model".into(),
            crate::settings::ModelInfo {
                reasoning_efforts: Some(vec!["low".into(), "medium".into(), "high".into()]),
                ..Default::default()
            },
        );
        settings.providers = vec![provider.clone()];
        settings.retry_enabled = false;
        let session = SessionModel {
            provider_id: "title-provider",
            model: "title-model",
        };

        let title = generate_title_with_model(
            &settings,
            &state,
            "conv_title",
            Some(session),
            "今天下雨吗，吉林市",
            "吉林市今天有小雨",
        )
        .await
        .expect("title from streamed model");
        assert_eq!(title, "吉林天气查询");

        let bodies = captured.lock().unwrap_or_else(|e| e.into_inner()).clone();
        assert_eq!(bodies.len(), 1, "exactly one title request");
        let body: serde_json::Value = serde_json::from_str(&bodies[0]).expect("json body");
        assert_eq!(body["stream"], true, "title summary must stream: {body}");
        // Titles always ask for `low`, with the normal reply's output budget.
        assert_eq!(body["reasoning_effort"], "low", "{body}");
        assert_eq!(
            body["max_tokens"],
            chat_max_output_tokens_on_wire(
                Some(&provider),
                "title-model",
                settings.chat.max_output_tokens
            ),
            "{body}"
        );
        let roles: Vec<&str> = body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, ["system", "user", "assistant", "user"], "{body}");
    }

    fn test_chat_message(role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            id: format!("msg_{role}"),
            role: role.to_string(),
            content: content.to_string(),
            attachments: Vec::new(),
            reasoning: None,
            artifacts: Vec::new(),
            tool_calls: Vec::new(),
            segments: Vec::new(),
            agent_plan: None,
            api_messages: Vec::new(),
            model_messages: Vec::new(),
            active_skill_id: None,
            run_entry: None,
            stream_outcome: None,
            usage: None,
            anchor_usage: None,
            cache_pair_input: None,
            cache_pair_read: None,
            group_id: None,
            provider_id: None,
            model: None,
            timestamp: 1,
            degraded: None,
        }
    }

    #[test]
    fn first_turn_title_inputs_requires_a_user_message() {
        assert_eq!(first_turn_title_inputs(&[]), None);
        assert_eq!(
            first_turn_title_inputs(&[test_chat_message("assistant", "hello")]),
            None
        );
        assert_eq!(
            first_turn_title_inputs(&[test_chat_message("user", "")]),
            None
        );
    }

    #[test]
    fn first_turn_title_inputs_uses_first_user_and_first_nonempty_assistant() {
        let messages = vec![
            test_chat_message("user", "今天下雨吗，吉林市"),
            test_chat_message("assistant", ""),
            test_chat_message("assistant", "吉林市今天有小雨"),
        ];
        assert_eq!(
            first_turn_title_inputs(&messages),
            Some((
                "今天下雨吗，吉林市".to_string(),
                "吉林市今天有小雨".to_string()
            ))
        );
    }

    #[test]
    fn apply_fork_title_suffix_appends_once() {
        assert_eq!(apply_fork_title_suffix("吉林天气", false), "吉林天气");
        assert_eq!(
            apply_fork_title_suffix("吉林天气", true),
            "吉林天气（分支）"
        );
        assert_eq!(
            apply_fork_title_suffix("吉林天气（分支）", true),
            "吉林天气（分支）"
        );
    }
}
