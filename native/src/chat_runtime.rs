//! Native task lifecycle around the same Rust provider adapters as Kivio.
//! No GPUI entity, window or Tauri handle crosses the network task boundary.
use crate::{
    conversation_data::{ChatMessage, ModelMessage, ModelUsage},
    provider_settings::ModelProvider,
};
use kivio_model_runtime::{
    GenerateOptions, GenerateRequest, ModelError, ModelRole, RequestMetadata, StreamPart,
};
use std::sync::OnceLock;

pub enum ChatEvent {
    Delta(String),
    Reasoning(String),
    Finished(ChatCompletion),
    Failed(String),
}
pub struct ChatCompletion {
    pub text: String,
    pub reasoning: Option<String>,
    pub provider_messages: Vec<ModelMessage>,
    pub usage: Option<ModelUsage>,
    pub finish_reason: Option<String>,
}
pub struct ChatRun {
    pub events: async_channel::Receiver<ChatEvent>,
    cancel: Option<tokio::sync::oneshot::Sender<()>>,
}
impl Drop for ChatRun {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
    }
}

pub(crate) fn runtime() -> Result<&'static tokio::runtime::Runtime, String> {
    static RUNTIME: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_name("kivio-model")
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// `history` includes the newly committed user message. Hidden canonical provider
/// messages are kept intact when continuing a saved conversation.
pub fn start(
    provider: ModelProvider,
    model: String,
    history: Vec<ChatMessage>,
    conversation_id: String,
    data_root: std::path::PathBuf,
) -> Result<ChatRun, String> {
    if !provider.enabled {
        return Err("请先启用所选供应商。".into());
    }
    if model.trim().is_empty() {
        return Err("请先选择聊天模型。".into());
    }
    let (events_tx, events) = async_channel::unbounded();
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    runtime()?.spawn(async move {
        let mut sink = |part| {
            let event = match part {
                StreamPart::TextDelta { delta } => Some(ChatEvent::Delta(delta)),
                StreamPart::ReasoningDelta { delta } => Some(ChatEvent::Reasoning(delta)),
                _ => None,
            };
            if let Some(event) = event {
                events_tx
                    .try_send(event)
                    .map_err(|_| ModelError::new("cancelled"))?;
            }
            Ok(())
        };
        let call = async {
            let catalog_dir = data_root
                .parent()
                .ok_or_else(|| ModelError::new("缺少对话数据目录"))?
                .to_path_buf();
            let prepare_provider = provider.clone();
            let (request, attempts) = tokio::task::spawn_blocking(move || {
                let root = catalog_dir.parent().ok_or("缺少设置目录")?;
                let settings = kivio_model_runtime::native_preferences::load(root)?;
                let attempts = if settings.retry_enabled {
                    settings.retry_attempts.clamp(1, 11) as usize
                } else {
                    1
                };
                let mut request = prepare_request(
                    prepare_provider,
                    model,
                    history,
                    conversation_id,
                    catalog_dir,
                )?;
                request.options.thinking_enabled = settings.chat.thinking_enabled;
                request.system = settings.chat.system_prompt;
                Ok::<_, String>((request, attempts))
            })
            .await
            .map_err(|error| ModelError::new(error.to_string()))?
            .map_err(ModelError::new)?;
            let provider = kivio_model_runtime::provider_oauth::resolve_provider(&provider)
                .await
                .map_err(ModelError::new)?;
            let host = kivio_model_runtime::RuntimeHost::shared(&provider, &data_root);
            kivio_model_runtime::dispatch::stream_resolved(
                &*host, &provider, attempts, request, &mut sink,
            )
            .await
        };
        let result = tokio::select! { _=cancel_rx => return, result=call=>result };
        let event = match result {
            Ok(output) => {
                let provider_messages = if let Some(canonical) = output.canonical_message.clone() {
                    vec![canonical]
                } else {
                    kivio_model_runtime::model_messages_from_openai_messages(vec![
                        output.to_openai_compatible_message(),
                    ])
                };
                ChatEvent::Finished(ChatCompletion {
                    text: output.text,
                    reasoning: output.reasoning,
                    provider_messages,
                    usage: output.usage,
                    finish_reason: output.finish_reason,
                })
            }
            Err(error) => ChatEvent::Failed(error.message),
        };
        let _ = events_tx.send(event).await;
    });
    Ok(ChatRun {
        events,
        cancel: Some(cancel_tx),
    })
}

fn prepare_request(
    provider: ModelProvider,
    model: String,
    history: Vec<ChatMessage>,
    conversation_id: String,
    catalog_dir: std::path::PathBuf,
) -> Result<GenerateRequest, String> {
    let message_id = history.last().map(|m| m.id.clone());
    let directory = catalog_dir.join(format!("{conversation_id}_attachments"));
    let mut video_budget = 14 * 1024 * 1024;
    let mut messages = Vec::new();
    for message in history {
        if message.role == "assistant" && !message.model_messages.is_empty() {
            messages.extend(message.model_messages);
        } else if message.role == "assistant" && !message.api_messages.is_empty() {
            messages.extend(kivio_model_runtime::model_messages_from_openai_messages(
                message.api_messages,
            ));
        } else if message.role == "user" && !message.attachments.is_empty() {
            let parts = kivio_model_runtime::attachment_files::user_message_parts(
                &message.content,
                &message.attachments,
                &directory,
                &mut video_budget,
            )?;
            messages.extend(kivio_model_runtime::model_messages_from_openai_messages(
                vec![serde_json::json!({"role":"user","content":parts})],
            ));
        } else if message.role == "user" || message.role == "assistant" {
            let role = if message.role == "user" {
                ModelRole::User
            } else {
                ModelRole::Assistant
            };
            messages.push(ModelMessage::text(role, message.content));
        }
    }
    Ok(GenerateRequest {
        options: GenerateOptions {
            max_tokens: kivio_model_runtime::request_max_tokens(&provider, &model),
            ..GenerateOptions::default()
        },
        model,
        system: String::new(),
        messages,
        tools: Vec::new(),
        metadata: RequestMetadata {
            label: "Chat".into(),
            usage_source: Some("chat".into()),
            usage_operation: Some("chat".into()),
            conversation_id: Some(conversation_id),
            message_id,
        },
    })
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use serde_json::{Value, json};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        time::Duration,
    };

    pub fn message(role: &str, content: &str) -> ChatMessage {
        serde_json::from_value(json!({"id":uuid::Uuid::new_v4().to_string(),"role":role,"content":content,"timestamp":1})).unwrap()
    }
    pub fn response(text: &str) -> String {
        format!(
            "data: {}\n\ndata: {}\n\ndata: {}\n\n",
            json!({"type":"response.reasoning_summary_text.delta","delta":"checking"}),
            json!({"type":"response.output_text.delta","delta":text}),
            json!({"type":"response.completed","response":{"status":"completed","output":[{"type":"reasoning","id":"rs_saved","summary":[{"type":"summary_text","text":"checking"}],"encrypted_content":"opaque-history"},{"type":"message","role":"assistant","content":[{"type":"output_text","text":text}]}],"usage":{"input_tokens":4,"output_tokens":2,"total_tokens":6}}})
        )
    }
    pub fn serve(
        responses: Vec<String>,
        hold_last: bool,
    ) -> (ModelProvider, std::thread::JoinHandle<Vec<Value>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let provider:ModelProvider=serde_json::from_value(json!({"id":"native-test","name":"Local fixture","baseUrl":base,"apiKeys":[],"apiFormat":"openai_responses","enabled":true,"availableModels":["test-model"],"enabledModels":["test-model"],"request":{"useSystemProxy":false}})).unwrap();
        let server = std::thread::spawn(move || {
            let count = responses.len();
            let mut requests = Vec::new();
            for (index, response) in responses.into_iter().enumerate() {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0u8; 4096];
                let header_end = loop {
                    let n = socket.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                        break end + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&bytes[..header_end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap();
                while bytes.len() < header_end + length {
                    let n = socket.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                }
                requests
                    .push(serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap());
                let hold = hold_last && index + 1 == count;
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len()+usize::from(hold),response).unwrap();
                socket.flush().unwrap();
                if hold {
                    assert_eq!(
                        socket
                            .read(&mut buffer)
                            .expect("dropping ChatRun must close the pending HTTP body"),
                        0
                    );
                }
            }
            requests
        });
        (provider, server)
    }
    /// A real local endpoint for native view tests. One request, no production mock mode.
    pub fn provider() -> ModelProvider {
        serve(vec![response("本地流式回答")], false).0
    }
    pub async fn completion(run: &ChatRun) -> ChatCompletion {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match run.events.recv().await.unwrap() {
                    ChatEvent::Finished(result) => return result,
                    ChatEvent::Failed(error) => panic!("local request failed: {error}"),
                    _ => {}
                }
            }
        })
        .await
        .expect("model stream did not terminate")
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn native_stream_persists_canonical_history_and_redacted_telemetry() {
        let data_root = tempfile::tempdir().unwrap();
        let (mut provider, server) = serve(vec![response("第一答"), response("第二答")], false);
        provider.api_keys = vec!["native-secret-never-persist".into()];
        let host =
            kivio_model_runtime::RuntimeHost::for_directory(&data_root.path().join("runtime"));
        let user = message("user", "first question");
        let run = start(
            provider.clone(),
            "test-model".into(),
            vec![user.clone()],
            "conv_native".into(),
            data_root.path().join("runtime"),
        )
        .unwrap();
        let first = completion(&run).await;
        assert_eq!(first.text, "第一答");
        assert_eq!(first.usage.as_ref().unwrap().total_tokens, Some(6));
        assert!(host.debug_snapshot().is_empty());
        assert!(
            !data_root
                .path()
                .join("runtime/request_debug/records.jsonl")
                .exists()
        );
        host.set_request_debug_enabled(true);
        let mut assistant = message("assistant", &first.text);
        assistant.model_messages = first.provider_messages;
        let saved =
            serde_json::to_vec(&vec![user, assistant, message("user", "second question")]).unwrap();
        let restored: Vec<ChatMessage> = serde_json::from_slice(&saved).unwrap();
        let second = start(
            provider,
            "test-model".into(),
            restored,
            "conv_native".into(),
            data_root.path().join("runtime"),
        )
        .unwrap();
        assert_eq!(completion(&second).await.text, "第二答");
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 2);
        let input = requests[1]["input"].as_array().unwrap();
        assert!(
            input
                .iter()
                .any(|v| v["type"] == "reasoning" && v["encrypted_content"] == "opaque-history")
        );
        assert!(requests[1].to_string().contains("first question"));
        assert!(requests[1].to_string().contains("second question"));
        let stats = host
            .usage_stats(kivio_model_runtime::usage_ledger::UsageStatsQuery {
                range: "all".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(stats.summary.total_requests, 2);
        assert_eq!(stats.summary.total_tokens, 12);
        assert_eq!(stats.logs.len(), 2);
        let records = host.debug_snapshot();
        assert_eq!(records.len(), 1);
        let mirror =
            std::fs::read_to_string(data_root.path().join("runtime/request_debug/records.jsonl"))
                .unwrap();
        assert_eq!(mirror.lines().count(), 1);
        assert!(!mirror.contains("native-secret-never-persist"));
        let persisted: kivio_model_runtime::request_debug::RequestDebugRecord =
            serde_json::from_str(mirror.lines().next().unwrap()).unwrap();
        assert_eq!(
            persisted
                .request
                .headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case("authorization"))
                .map(|(_, value)| value.as_str()),
            Some("Bearer nati…")
        );
        host.clear_debug();
        assert!(
            !data_root
                .path()
                .join("runtime/request_debug/records.jsonl")
                .exists()
        );
    }

    #[tokio::test]
    async fn dropping_run_closes_inflight_body_and_stops_events() {
        let frame = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n";
        let data_root = tempfile::tempdir().unwrap();
        let (provider, server) = serve(vec![frame.into()], true);
        let run = start(
            provider,
            "test-model".into(),
            vec![message("user", "wait")],
            "cancel".into(),
            data_root.path().join("runtime"),
        )
        .unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(5), run.events.recv())
                .await
                .unwrap()
                .unwrap(),
            ChatEvent::Delta(_)
        ));
        let receiver = run.events.clone();
        drop(run);
        assert!(
            tokio::time::timeout(Duration::from_secs(5), receiver.recv())
                .await
                .unwrap()
                .is_err()
        );
        server.join().unwrap();
    }

    #[tokio::test]
    async fn interrupted_stream_reports_failure_instead_of_completed_history() {
        let data_root = tempfile::tempdir().unwrap();
        let (provider, server) = serve(
            vec![
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n".into(),
            ],
            false,
        );
        let run = start(
            provider,
            "test-model".into(),
            vec![message("user", "test")],
            "broken".into(),
            data_root.path().join("runtime"),
        )
        .unwrap();
        let mut partial = false;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match run.events.recv().await.unwrap() {
                    ChatEvent::Delta(_) => partial = true,
                    ChatEvent::Failed(error) => {
                        assert!(!error.is_empty());
                        break;
                    }
                    ChatEvent::Finished(_) => panic!("unterminated stream was committed"),
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
        assert!(partial);
        server.join().unwrap();
    }

    #[tokio::test]
    #[ignore = "Explicit opt-in: uses the authorized isolated official DeepSeek account"]
    async fn authorized_deepseek_two_turns_and_cancel() {
        let data_root = tempfile::tempdir().unwrap();
        let root = kivio_model_runtime::app_data::app_data_dir()
            .unwrap()
            .join("conversations");
        let config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("providers.json")).unwrap()).unwrap();
        let provider: ModelProvider = serde_json::from_value(
            config["providers"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| {
                    v["baseUrl"]
                        .as_str()
                        .is_some_and(|url| url.contains("api.deepseek.com"))
                })
                .unwrap()
                .clone(),
        )
        .unwrap();
        let first_user = message("user", "记住测试代号 NATIVE-42，只回复 OK。");
        let first_run = start(
            provider.clone(),
            "deepseek-flash".into(),
            vec![first_user.clone()],
            "native-authorized-smoke".into(),
            data_root.path().join("runtime"),
        )
        .unwrap();
        async fn live_result(run: &ChatRun) -> ChatCompletion {
            tokio::time::timeout(Duration::from_secs(120), async {
                loop {
                    match run.events.recv().await.unwrap() {
                        ChatEvent::Finished(v) => break v,
                        ChatEvent::Failed(_) => {
                            panic!("authorized model request failed (details withheld)")
                        }
                        _ => {}
                    }
                }
            })
            .await
            .expect("authorized request timed out")
        }
        let first = live_result(&first_run).await;
        assert!(!first.text.is_empty());
        assert!(!first.provider_messages.is_empty());
        let mut assistant = message("assistant", &first.text);
        assistant.model_messages = first.provider_messages;
        assistant.usage = first.usage;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");
        crate::file_io::atomic_write(
            &path,
            &serde_json::to_string(&vec![first_user, assistant]).unwrap(),
            "test history",
        )
        .unwrap();
        let mut restored: Vec<ChatMessage> =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        restored.push(message("user", "刚才的测试代号是什么？只回复代号。"));
        let second_run = start(
            provider.clone(),
            "deepseek-flash".into(),
            restored,
            "native-authorized-smoke".into(),
            data_root.path().join("runtime"),
        )
        .unwrap();
        let second = live_result(&second_run).await;
        assert!(
            second.text.contains("NATIVE-42"),
            "saved history was not recalled"
        );
        let cancel_run = start(
            provider,
            "deepseek-flash".into(),
            vec![message("user", "从1开始逐项列出1000个数字。")],
            "native-authorized-cancel".into(),
            data_root.path().join("runtime"),
        )
        .unwrap();
        tokio::time::timeout(Duration::from_secs(120), cancel_run.events.recv())
            .await
            .unwrap()
            .unwrap();
        let events = cancel_run.events.clone();
        drop(cancel_run);
        tokio::time::timeout(Duration::from_secs(5), async {
            while events.recv().await.is_ok() {}
        })
        .await
        .expect("cancelled request task did not release");
    }
}
