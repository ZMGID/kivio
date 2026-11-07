use serde_json::Value;

#[cfg(test)]
use crate::model::is_tools_unsupported_error;
use crate::model::{GenerateOutput, MessagePart, ModelMessage, ModelRole, PendingToolCall};

pub fn merge_reasoning(
    planning_parts: &[String],
    final_reasoning: Option<String>,
) -> Option<String> {
    let mut parts = planning_parts
        .iter()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if let Some(reasoning) = final_reasoning
        .as_deref()
        .map(str::trim)
        .filter(|reasoning| !reasoning.is_empty())
    {
        parts.push(reasoning.to_string());
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

pub fn final_response_from_planning_message(
    message: &GenerateOutput,
    planning_reasoning_parts: &[String],
) -> Result<(String, Option<String>), String> {
    final_response_from_planning_message_with_images(message, planning_reasoning_parts, false)
}

/// 同上，但允许「本轮出了图」时正文为空：hosted image generation（Responses
/// `image_generation_call`）的正文本就是空串——图即答案，此时空正文不是失败。
pub fn final_response_from_planning_message_with_images(
    message: &GenerateOutput,
    planning_reasoning_parts: &[String],
    has_images: bool,
) -> Result<(String, Option<String>), String> {
    let response = sanitize_assistant_text_response(&message.text);
    if response.trim().is_empty() && !has_images {
        return Err(empty_assistant_response_error("Chat tools planning"));
    }
    let reasoning = merge_reasoning(planning_reasoning_parts, message.reasoning.clone());
    Ok((response, reasoning))
}

pub fn final_assistant_message(content: &str, reasoning: Option<&str>) -> ModelMessage {
    let mut message = ModelMessage::text(ModelRole::Assistant, content);
    if let Some(reasoning) = reasoning.map(str::trim).filter(|value| !value.is_empty()) {
        message.content.push(MessagePart::Reasoning {
            text: reasoning.to_string(),
        });
    }
    message
}

pub fn pending_tool_calls_from_dsml(content: &str) -> Vec<PendingToolCall> {
    crate::dsml_tools::extract_dsml_tool_calls(content)
        .into_iter()
        .map(|call| {
            let arguments = Value::Object(call.arguments);
            let arguments_raw =
                serde_json::to_string(&arguments).unwrap_or_else(|_| "{}".to_string());
            PendingToolCall {
                id: format!("tool_{}", uuid::Uuid::new_v4()),
                function_name: call.name,
                arguments,
                arguments_raw,
                arguments_parse_error: None,
                signature: None,
            }
        })
        .collect()
}

pub fn sanitize_assistant_text_response(content: &str) -> String {
    let stripped = crate::dsml_tools::strip_dsml_tool_markup(content);
    if stripped.is_empty() && crate::dsml_tools::contains_dsml_tool_markup(content) {
        return String::new();
    }
    stripped
}

pub fn empty_assistant_response_error(scope: &str) -> String {
    format!("{scope} returned an empty assistant response")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_reasoning_keeps_planning_and_final_sections() {
        let reasoning = merge_reasoning(
            &[
                "  planning round one  ".to_string(),
                String::new(),
                "planning round two".to_string(),
            ],
            Some(" final answer reasoning ".to_string()),
        )
        .expect("reasoning should be merged");

        assert_eq!(
            reasoning,
            "planning round one\n\nplanning round two\n\nfinal answer reasoning"
        );
    }

    #[test]
    fn is_tools_unsupported_error_detects_provider_rejection_messages() {
        assert!(is_tools_unsupported_error(
            "Chat tools planning Error: 400 Bad Request - tools not supported (attempt 1/3)"
        ));
        assert!(!is_tools_unsupported_error(
            "Chat tools planning Error: 422 Unprocessable Entity - invalid tool_choice (attempt 1/1)"
        ));
        assert!(is_tools_unsupported_error(
            "Chat tools planning Error: 400 Bad Request - function call is not supported (attempt 1/3)"
        ));
        assert!(is_tools_unsupported_error(
            "Chat tools planning Error: 400 Bad Request - tools[0]: unknown variant `function`, expected `web_search_20250305` or `web_search_20260209` (attempt 1/1)"
        ));
        assert!(!is_tools_unsupported_error(
            "Chat tools planning Error: 429 Too Many Requests - rate limited (attempt 1/3)"
        ));
        assert!(!is_tools_unsupported_error("network timeout"));
    }

    #[test]
    fn malformed_tool_history_is_not_an_unsupported_tools_capability() {
        for error in [
            "Chat tools planning Error: 400 Bad Request - An assistant message with tool_calls must be followed by tool messages",
            "Chat tools planning Error: 400 Bad Request - Missing reasoning_content in assistant tool call message",
            "Chat tools planning Error: 422 - Invalid tools[0].parameters: schema is not valid",
            "Chat tools planning Error: 400 - This model does not support images",
            "Chat tools planning Error: 400 - The `reasoning_text` in the thinking mode must be passed back to the API.",
        ] {
            assert!(crate::model::extract_status_code(error).is_some(), "fixture must exercise HTTP classification");
            assert!(!is_tools_unsupported_error(error), "{error}");
        }
    }

    #[test]
    fn planning_final_uses_display_reasoning_without_mutating_replay() {
        let message = GenerateOutput::text("直接回答".into(), Some("final thought".into()));
        let (response, reasoning) =
            final_response_from_planning_message(&message, &["plan thought".into()]).unwrap();
        assert_eq!(response, "直接回答");
        assert_eq!(reasoning.as_deref(), Some("plan thought\n\nfinal thought"));
        assert_eq!(message.reasoning.as_deref(), Some("final thought"));
    }

    #[test]
    fn planning_final_rejects_empty_text() {
        let message = GenerateOutput::text("<|DSML|tool_calls></|DSML|tool_calls>".into(), None);
        assert_eq!(
            final_response_from_planning_message(&message, &[]).unwrap_err(),
            "Chat tools planning returned an empty assistant response"
        );
    }
}
