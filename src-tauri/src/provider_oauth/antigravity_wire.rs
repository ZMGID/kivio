//! Pure Antigravity provider envelopes; login/refresh remain platform-owned.
use crate::settings::ModelProvider;
use serde_json::{Value, json};
pub const BASE: &str = "https://daily-cloudcode-pa.googleapis.com";
pub fn user_agent() -> String {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => other,
    };
    format!(
        "antigravity/cli/1.1.23 (aidev_client; os_type={}; arch={arch}; cl=974125021; auth_method=consumer)",
        std::env::consts::OS
    )
}

pub fn is_provider(p: &ModelProvider) -> bool {
    p.request
        .oauth
        .as_ref()
        .is_some_and(|a| a.provider == "antigravity")
}

pub(crate) fn model_includes_effort(model: &str) -> bool {
    model.trim().rsplit_once('-').is_some_and(|(_, suffix)| {
        matches!(
            suffix.to_ascii_lowercase().as_str(),
            "low" | "medium" | "high"
        )
    })
}

pub fn unwrap_response(mut value: Value) -> Value {
    if value.get("response").is_some() && value.get("error").is_none() {
        value["response"].take()
    } else {
        value
    }
}

pub fn wrap_request(provider: &ModelProvider, model: &str, mut body: Value) -> Value {
    let custom = model.starts_with("claude-") || model.starts_with("gpt-oss-");
    if let Some(tools) = body["tools"].as_array_mut() {
        tools.retain(|t| t["functionDeclarations"].is_array());
        for tool in tools {
            if let Some(decls) = tool["functionDeclarations"].as_array_mut() {
                for decl in decls {
                    if let Some(schema) = decl.as_object_mut().and_then(|d| d.remove("parameters"))
                    {
                        decl[if custom {
                            "parameters"
                        } else {
                            "parametersJsonSchema"
                        }] = if custom {
                            custom_schema(&schema)
                        } else {
                            schema
                        };
                    }
                }
            }
        }
    }
    if body.get("toolConfig").is_some() || custom {
        body["toolConfig"] = json!({"functionCallingConfig":{"mode":"VALIDATED"}});
    }
    // Runtime catalog IDs already encode reasoning variants. Do not send Gemini's
    // thinkingLevel dialect to the Claude/GPT-OSS bridge.
    if custom {
        if let Some(config) = body["generationConfig"].as_object_mut() {
            config.remove("thinkingConfig");
        }
    } else if model_includes_effort(model) {
        // The catalog model owns its effort. Keep thought output, but discard
        // independent overrides from old conversations or auxiliary requests.
        if let Some(thinking) = body["generationConfig"]["thinkingConfig"].as_object_mut() {
            thinking.remove("thinkingLevel");
            thinking.remove("thinkingBudget");
        }
    }
    if let Some(level) = body["generationConfig"]["thinkingConfig"]["thinkingLevel"].as_str() {
        body["generationConfig"]["thinkingConfig"]["thinkingLevel"] =
            json!(level.to_ascii_uppercase());
    }
    if body.get("systemInstruction").is_some() {
        body["systemInstruction"]["role"] = json!("user");
    }
    if let Some(contents) = body["contents"].as_array_mut() {
        if contents.first().is_some_and(|v| v["role"] == "model") {
            contents.insert(0, json!({"role":"user","parts":[{"text":"Continue."}]}));
        }
    }
    body["sessionId"] = json!(rand::random::<i64>().to_string());
    json!({"project": provider.request.oauth.as_ref().and_then(|a| a.project_id.as_deref()),
        "model": model, "request": body, "requestType":"agent", "userAgent":"antigravity",
        "requestId": format!("agent/{}", uuid::Uuid::new_v4())})
}

pub(super) fn custom_schema(value: &Value) -> Value {
    let Some(map) = value.as_object() else {
        return value.clone();
    };
    let mut out = serde_json::Map::new();
    for (k, v) in map {
        if k == "enum"
            && v.as_array()
                .is_some_and(|a| a.iter().any(|v| !v.is_string()))
        {
            continue;
        }
        if ![
            "type",
            "description",
            "properties",
            "required",
            "items",
            "enum",
        ]
        .contains(&k.as_str())
        {
            continue;
        }
        let next = if k == "properties" {
            Value::Object(
                v.as_object()
                    .map(|m| {
                        m.iter()
                            .map(|(k, v)| (k.clone(), custom_schema(v)))
                            .collect()
                    })
                    .unwrap_or_default(),
            )
        } else if k == "type" && v.is_array() {
            v.as_array()
                .and_then(|a| a.iter().find(|v| v.is_string() && *v != "null"))
                .cloned()
                .unwrap_or(json!("string"))
        } else {
            custom_schema(v)
        };
        out.insert(k.clone(), next);
    }
    Value::Object(out)
}
