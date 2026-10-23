//! OAuth wire identities; credential storage/refresh remain platform owners.
use crate::settings::ModelProvider;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
pub(super) fn claims(token: &str) -> Value {
    token
        .split('.')
        .nth(1)
        .and_then(|s| URL_SAFE_NO_PAD.decode(s).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null)
}
pub fn is_codex(provider: &ModelProvider) -> bool {
    provider
        .request
        .oauth
        .as_ref()
        .is_some_and(|a| a.provider == "codex")
}

pub fn header_pairs(
    provider: &ModelProvider,
    device_id: impl Fn() -> &'static str,
    antigravity_user_agent: impl Fn() -> String,
    version: &str,
) -> Vec<(String, String)> {
    let Some(auth) = &provider.request.oauth else {
        return vec![];
    };
    let pair = |k: &str, v: &str| (k.to_string(), v.to_string());
    if is_codex(provider) {
        let mut pairs = vec![
            pair("originator", "kivio"),
            pair("User-Agent", &format!("Kivio/{version}")),
            pair("OpenAI-Beta", "responses=experimental"),
        ];
        if let Some(token) = provider.preferred_api_key() {
            if let Some(id) =
                claims(token)["https://api.openai.com/auth"]["chatgpt_account_id"].as_str()
            {
                pairs.push(pair("ChatGPT-Account-Id", id));
            }
        }
        pairs
    } else if auth.provider == "antigravity" {
        let mut pairs = vec![pair("User-Agent", &antigravity_user_agent())];
        if let Some(token) = provider.preferred_api_key() {
            pairs.push(pair("Authorization", &format!("Bearer {token}")));
        }
        pairs
    } else if auth.provider == "kimi" {
        vec![
            pair("User-Agent", "KimiCLI/1.0.0"),
            pair("X-Msh-Platform", "kimi_cli"),
            pair("X-Msh-Version", "1.0.0"),
            pair("X-Msh-Device-Name", "Kivio"),
            pair("X-Msh-Device-Model", std::env::consts::ARCH),
            pair("X-Msh-Os-Version", std::env::consts::OS),
            pair("X-Msh-Device-Id", device_id()),
        ]
    } else {
        vec![]
    }
}

pub fn codex_body(body: &mut Value) {
    body["stream"] = json!(true);
    body["store"] = json!(false);
    if !body["instructions"].is_string() {
        body["instructions"] = json!("");
    }
    if let Some(object) = body.as_object_mut() {
        for key in [
            "max_output_tokens",
            "temperature",
            "top_p",
            "prompt_cache_retention",
            "previous_response_id",
        ] {
            object.remove(key);
        }
    }
}
