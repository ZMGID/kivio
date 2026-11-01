use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::settings::{ModelProvider, Settings};
use crate::state::AppState;

const LIVE_INSTRUCTIONS: &str = "You are Kivio's voice assistant. Speak concisely in the user's language. Delegate requests about Kivio conversations, coding agents, projects, and tasks to the client. Never claim an action is complete before the backend confirms it.";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VoiceLiveStart {
    session_id: String,
    answer_sdp: String,
    conversation_id: String,
}

#[derive(Deserialize)]
struct LiveResponse {
    session: LiveSession,
    transport: LiveTransport,
}

#[derive(Deserialize)]
struct LiveSession {
    id: String,
}

#[derive(Deserialize)]
struct LiveTransport {
    sdp: String,
}

impl LiveResponse {
    fn into_start(self, conversation_id: String) -> Result<VoiceLiveStart, String> {
        if self.session.id.trim().is_empty() || self.transport.sdp.trim().is_empty() {
            return Err("语音服务返回的会话 ID 或应答 SDP 为空。".into());
        }
        Ok(VoiceLiveStart {
            session_id: self.session.id,
            answer_sdp: self.transport.sdp,
            conversation_id,
        })
    }
}

fn select_live_provider(settings: &Settings) -> Result<&ModelProvider, String> {
    let explicit = settings
        .voice
        .as_ref()
        .and_then(|voice| voice.live_provider_id.as_deref())
        .map(str::trim)
        .filter(|id| !id.is_empty());
    let provider = if let Some(id) = explicit {
        settings
            .get_provider(id)
            .ok_or("所选语音提供商不存在，请在设置中重新选择。")?
    } else {
        settings
            .providers
            .iter()
            .find(|provider| {
                provider.preferred_api_key().is_some()
                    && reqwest::Url::parse(provider.base_url.trim())
                        .ok()
                        .is_some_and(|url| url.host_str() == Some("api.openai.com"))
            })
            .ok_or(
                "未找到已配置 API 密钥的 OpenAI 语音提供商，请在设置中选择提供商或配置 API 密钥。",
            )?
    };
    if provider.preferred_api_key().is_none() {
        return Err("所选语音提供商未配置 API 密钥。".into());
    }
    Ok(provider)
}

fn live_session_url(base_url: &str) -> Result<reqwest::Url, String> {
    let invalid_url = || "语音提供商的服务地址无效，请检查设置。".to_string();
    let mut url = reqwest::Url::parse(base_url.trim()).map_err(|_| invalid_url())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(invalid_url());
    }
    url.path_segments_mut()
        .map_err(|_| invalid_url())?
        .pop_if_empty()
        .push("live")
        .push("sessions");
    Ok(url)
}

fn brain_selection(
    settings: &Settings,
    provider_id: Option<String>,
    model: Option<String>,
) -> (Option<String>, Option<String>) {
    let configured = settings.voice.as_ref().and_then(|voice| {
        let provider = voice.orchestrator_provider_id.as_deref()?.trim();
        let model = voice.orchestrator_model.as_deref()?.trim();
        (!provider.is_empty() && !model.is_empty()).then_some((provider, model))
    });
    match configured {
        Some((provider, model)) => (Some(provider.to_string()), Some(model.to_string())),
        None => (provider_id, model),
    }
}

fn validate_offer(offer_sdp: &str) -> Result<(), String> {
    if offer_sdp.len() > 64 * 1024 {
        return Err("语音连接的 SDP 超过 64 KiB。".into());
    }
    if offer_sdp.trim().is_empty() {
        return Err("语音连接的 SDP 不能为空。".into());
    }
    Ok(())
}

fn live_request_body(offer_sdp: &str) -> impl Serialize + '_ {
    #[derive(Serialize)]
    struct Body<'a> {
        session: Session,
        transport: Transport<'a>,
    }
    #[derive(Serialize)]
    struct Session {
        model: &'static str,
        instructions: &'static str,
        delegation: Delegation,
    }
    #[derive(Serialize)]
    struct Delegation {
        #[serde(rename = "type")]
        kind: &'static str,
    }
    #[derive(Serialize)]
    struct Transport<'a> {
        #[serde(rename = "type")]
        kind: &'static str,
        sdp: &'a str,
    }
    Body {
        session: Session {
            model: "gpt-live-1",
            instructions: LIVE_INSTRUCTIONS,
            delegation: Delegation { kind: "client" },
        },
        transport: Transport {
            kind: "webrtc",
            sdp: offer_sdp,
        },
    }
}

#[tauri::command]
pub(crate) async fn voice_live_start(
    app: AppHandle,
    offer_sdp: String,
    provider_id: Option<String>,
    model: Option<String>,
) -> Result<VoiceLiveStart, String> {
    validate_offer(&offer_sdp)?;
    let (request, provider_id, model) = {
        let state = app.state::<AppState>();
        let settings = state.settings_read();
        let provider = select_live_provider(&settings)?;
        let url = live_session_url(&provider.base_url)?;
        let key = provider
            .preferred_api_key()
            .ok_or("所选语音提供商未配置 API 密钥。")?;
        let request = crate::api::with_standard_request_timeout(
            state
                .client_for(provider)
                .post(url)
                .bearer_auth(key.trim())
                .json(&live_request_body(&offer_sdp)),
        );
        let (provider_id, model) = brain_selection(&settings, provider_id, model);
        (request, provider_id, model)
    };

    let conversation_id =
        crate::chat::voice_orchestrator::ensure_brain_conversation(&app, provider_id, model)
            .await
            .map_err(|error| format!("准备语音编排对话失败：{error}"))?;
    let response = request.send().await.map_err(|error| {
        if error.is_timeout() {
            "连接语音服务超时，请检查网络和代理设置。"
        } else if error.is_builder() {
            "语音提供商配置无效，请检查服务地址和 API 密钥。"
        } else {
            "无法连接语音服务，请检查服务地址、网络和代理设置。"
        }
        .to_string()
    })?;
    let status = response.status();
    if status != reqwest::StatusCode::CREATED {
        return Err(if matches!(status.as_u16(), 401 | 403) {
            format!(
                "语音服务认证失败，请检查 API 密钥和权限（HTTP {}）。",
                status.as_u16()
            )
        } else {
            format!("创建语音会话失败（HTTP {}）。", status.as_u16())
        });
    }
    response
        .json::<LiveResponse>()
        .await
        .map_err(|_| "语音服务返回的数据无效或缺少会话 ID / 应答 SDP。".to_string())?
        .into_start(conversation_id)
}

#[tauri::command]
pub(crate) async fn voice_delegate(
    app: AppHandle,
    conversation_id: String,
    request: String,
) -> Result<String, String> {
    crate::chat::voice_orchestrator::delegate(&app, &conversation_id, request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::VoiceSettings;
    use serde_json::json;

    fn provider(id: &str, base_url: &str, api_keys: &[&str]) -> ModelProvider {
        serde_json::from_value(json!({
            "id": id,
            "name": id,
            "baseUrl": base_url,
            "apiKeys": api_keys
        }))
        .unwrap()
    }

    #[test]
    fn provider_selection_uses_explicit_choice_or_first_keyed_openai_host() {
        let mut settings = Settings::default();
        settings.providers = vec![
            provider("spoof", "https://api.openai.com.evil.test/v1", &["secret"]),
            provider(
                "userinfo",
                "https://api.openai.com@evil.test/v1",
                &["secret"],
            ),
            provider("missing-key", "https://api.openai.com/v1", &[" "]),
            provider(
                "openai",
                "https://API.OPENAI.COM:443/v1",
                &["first", "second"],
            ),
            provider("later", "https://api.openai.com/v1", &["secret"]),
            provider("custom", "https://example.test/v1", &["secret"]),
        ];
        settings.providers[3].enabled = false;
        settings.providers[3].active_key_index = 1;
        assert_eq!(select_live_provider(&settings).unwrap().id, "openai");
        assert_eq!(
            select_live_provider(&settings).unwrap().preferred_api_key(),
            Some("second")
        );

        settings.providers[5].enabled = false;
        settings.providers[5].api_format = "anthropic_messages".into();
        settings.voice = Some(VoiceSettings {
            live_provider_id: Some("custom".into()),
            ..Default::default()
        });
        assert_eq!(select_live_provider(&settings).unwrap().id, "custom");
        settings.voice.as_mut().unwrap().live_provider_id = Some("missing-key".into());
        assert!(select_live_provider(&settings)
            .unwrap_err()
            .contains("密钥"));
        settings.voice.as_mut().unwrap().live_provider_id = Some("missing-provider".into());
        assert!(select_live_provider(&settings)
            .unwrap_err()
            .contains("不存在"));
        settings.voice = None;
        settings.providers.truncate(3);
        assert!(select_live_provider(&settings).is_err());
    }

    #[test]
    fn live_request_has_exact_client_delegation_and_webrtc_body() {
        let body = serde_json::to_value(live_request_body("v=0\r\n")).unwrap();
        assert_eq!(
            body,
            json!({
                "session": {
                    "model": "gpt-live-1",
                    "instructions": "You are Kivio's voice assistant. Speak concisely in the user's language. Delegate requests about Kivio conversations, coding agents, projects, and tasks to the client. Never claim an action is complete before the backend confirms it.",
                    "delegation": {"type": "client"}
                },
                "transport": {"type": "webrtc", "sdp": "v=0\r\n"}
            })
        );
        assert_eq!(
            live_session_url("https://example.test/v1/")
                .unwrap()
                .as_str(),
            "https://example.test/v1/live/sessions"
        );
        assert!(live_session_url("not a URL").is_err());
    }

    #[test]
    fn brain_override_requires_both_fields() {
        let mut settings = Settings::default();
        settings.voice = Some(VoiceSettings {
            orchestrator_provider_id: Some(" brain ".into()),
            orchestrator_model: Some(" model ".into()),
            ..Default::default()
        });
        let composer = || {
            (
                Some("composer".to_string()),
                Some("composer-model".to_string()),
            )
        };
        let (provider, model) = composer();
        assert_eq!(
            brain_selection(&settings, provider, model),
            (Some("brain".into()), Some("model".into()))
        );
        settings.voice.as_mut().unwrap().orchestrator_model = Some(" ".into());
        let (provider, model) = composer();
        assert_eq!(brain_selection(&settings, provider, model), composer());
    }

    #[test]
    fn offer_and_answer_require_nonempty_values_and_limit_offer_bytes() {
        assert!(validate_offer(" \r\n").is_err());
        assert!(validate_offer(&"a".repeat(64 * 1024)).is_ok());
        assert!(validate_offer(&"中".repeat(21_846)).is_err());
        let response: LiveResponse = serde_json::from_value(json!({
            "session": {"id": "session-1"}, "transport": {"sdp": "answer\r\n"}
        }))
        .unwrap();
        let start = response.into_start("conversation-1".into()).unwrap();
        assert_eq!(
            serde_json::to_value(start).unwrap(),
            json!({
                "sessionId": "session-1", "answerSdp": "answer\r\n", "conversationId": "conversation-1"
            })
        );
        let response: LiveResponse = serde_json::from_value(json!({
            "session": {"id": " "}, "transport": {"sdp": "answer"}
        }))
        .unwrap();
        assert!(response.into_start("conversation-1".into()).is_err());
        let response: LiveResponse = serde_json::from_value(json!({
            "session": {"id": "session-1"}, "transport": {"sdp": "\r\n"}
        }))
        .unwrap();
        assert!(response.into_start("conversation-1".into()).is_err());
        assert!(serde_json::from_value::<LiveResponse>(json!({"session": {"id": "x"}})).is_err());
    }
}
