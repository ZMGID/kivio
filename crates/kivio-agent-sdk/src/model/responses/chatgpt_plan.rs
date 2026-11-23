//! Sign in with ChatGPT's stateless HTTP Responses preview.
use super::{GenerateOutput, ModelError, ModelErrorKind, StreamSink, Value};
use serde_json::json;

pub(super) const LOCAL_NAMESPACE: &str = "kivio";

/// Apply plan restrictions after all host and per-call overrides.
pub fn prepare_body(body: &mut Value) -> Result<(), ModelError> {
    let fields = body.as_object_mut().ok_or_else(|| {
        ModelError::with_kind(
            "Responses request body must be an object",
            ModelErrorKind::InvalidRequest,
        )
    })?;
    for field in [
        "background",
        "conversation",
        "max_output_tokens",
        "max_tool_calls",
        "metadata",
        "moderation",
        "multi_agent",
        "prompt",
        "prompt_cache_options",
        "prompt_cache_retention",
        "safety_identifier",
        "temperature",
        "top_logprobs",
        "top_p",
        "truncation",
        "user",
        "previous_response_id",
    ] {
        fields.remove(field);
    }
    body["stream"] = json!(true);
    body["store"] = json!(false);
    if body["input"].is_string() {
        body["input"] = json!([{"role":"user","content":body["input"].take()}]);
    }
    let input = body["input"].as_array_mut().ok_or_else(|| {
        ModelError::with_kind(
            "ChatGPT input must be an array of items",
            ModelErrorKind::InvalidRequest,
        )
    })?;
    for item in input {
        if item["role"] == "system" {
            item["role"] = json!("developer");
        }
        if matches!(
            item["type"].as_str(),
            Some("function_call" | "custom_tool_call")
        ) && item.get("namespace").is_none()
        {
            item["namespace"] = json!(LOCAL_NAMESPACE);
        }
        if item["content"].as_array().is_some_and(|parts| {
            parts
                .iter()
                .any(|part| matches!(part["type"].as_str(), Some("input_audio" | "input_video")))
        }) {
            return Err(unsupported("audio and video inputs"));
        }
    }
    if let Some(tools) = body.get_mut("tools") {
        let tools = tools
            .as_array_mut()
            .ok_or_else(|| unsupported("a non-array tools field"))?;
        let mut local = Vec::new();
        let mut hosted = Vec::new();
        for tool in std::mem::take(tools) {
            match tool["type"].as_str() {
                Some("function" | "custom") => local.push(tool),
                Some("namespace") if tool["name"] == LOCAL_NAMESPACE => {
                    let tools = tool["tools"]
                        .as_array()
                        .ok_or_else(|| unsupported("an invalid local namespace"))?;
                    for tool in tools {
                        if !matches!(tool["type"].as_str(), Some("function" | "custom")) {
                            return Err(unsupported("hosted tools inside the local namespace"));
                        }
                        local.push(tool.clone());
                    }
                }
                Some("web_search" | "web_search_preview") => hosted.push(tool),
                other => return Err(unsupported(other.unwrap_or("unknown tool"))),
            }
        }
        if !local.is_empty() {
            hosted.insert(
                0,
                json!({"type":"namespace","name":LOCAL_NAMESPACE,
                "description":"Tools executed locally by Kivio.","tools":local}),
            );
        }
        *tools = hosted;
    }
    Ok(())
}

fn unsupported(detail: &str) -> ModelError {
    ModelError::with_kind(
        format!("ChatGPT plan does not support {detail}"),
        ModelErrorKind::InvalidRequest,
    )
}

/// A plan call (including buffered generation) succeeds only on response.completed.
pub async fn decode_stream(
    response: reqwest::Response,
    sink: &mut (dyn StreamSink + Send),
    label: &str,
    model: &str,
) -> Result<GenerateOutput, ModelError> {
    super::decode_stream(response, sink, label, model, true).await
}
