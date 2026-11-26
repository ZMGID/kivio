use serde_json::Value;

use super::{
    resolve_tool_result, switch_role, truncate_tool_result, ImportedImage, ImportedMessage, Pending,
};

pub(in crate::external_agents) fn parse_dsh_history(events: &[Value]) -> Vec<ImportedMessage> {
    let mut messages = Vec::new();
    let mut pending = Pending::default();

    for event in events {
        let data = &event["data"];
        let time = event["time"].as_i64().unwrap_or_default();
        match event["type"].as_str() {
            Some(kind @ ("user/message" | "assistant/message")) => {
                let role = if kind == "user/message" {
                    if let Some(message) = std::mem::take(&mut pending).finish() {
                        messages.push(message);
                    }
                    "user"
                } else {
                    "assistant"
                };
                switch_role(&mut pending, &mut messages, role, time);
                for block in data["content"].as_array().into_iter().flatten() {
                    let text = block["text"].as_str().unwrap_or_default();
                    match block["type"].as_str() {
                        Some("text") => pending.push_text(text),
                        Some("reasoning") => pending.push_reasoning(text),
                        Some("image") => {
                            if let (Some(media_type), Some(data_base64)) =
                                (block["mediaType"].as_str(), block["data"].as_str())
                            {
                                pending.images.push(ImportedImage {
                                    media_type: media_type.to_owned(),
                                    data_base64: data_base64.to_owned(),
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
            Some("tool/call") => {
                if let (Some(id), Some(name)) = (data["callId"].as_str(), data["name"].as_str()) {
                    switch_role(&mut pending, &mut messages, "assistant", time);
                    pending.push_tool(
                        id.to_owned(),
                        name.to_owned(),
                        data["arguments"].as_str().unwrap_or("{}").to_owned(),
                    );
                }
            }
            Some("tool/result") => apply_tool_results(data, &mut pending, &mut messages),
            _ => {}
        }
    }

    if let Some(message) = pending.finish() {
        messages.push(message);
    }
    messages
}

fn apply_tool_results(data: &Value, pending: &mut Pending, messages: &mut [ImportedMessage]) {
    let message = &data["message"];
    let blocks = if message["role"] == "tool" {
        std::slice::from_ref(message)
    } else {
        message["content"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
    };
    for block in blocks {
        let Some(id) = block["toolCallId"].as_str() else {
            continue;
        };
        let output = block["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| match item["type"].as_str() {
                Some("text") => item["text"].as_str(),
                Some("image") => Some("[image attachment]"),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let is_error = block["isError"]
            .as_bool()
            .unwrap_or_else(|| data.get("error").is_some());
        resolve_tool_result(
            pending,
            messages,
            id,
            |_| truncate_tool_result(&output),
            is_error,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn snapshot_keeps_rendered_messages_and_tool_results_without_model_input() {
        let events = [
            json!({"type":"user/message", "time":1000, "data":{"content":[{"type":"text", "text":"Question"}, {"type":"image", "mediaType":"image/png", "data":"aGVsbG8="}]}}),
            json!({"type":"assistant/message", "time":2000, "data":{"content":[{"type":"reasoning", "text":"Consider options"}]}}),
            json!({"type":"tool/call", "time":2001, "data":{"callId":"call-1", "name":"read", "arguments":"{}"}}),
            json!({"type":"tool/result", "time":2200, "data":{"message":{"role":"tool", "toolCallId":"call-1", "isError":true, "content":[{"type":"text", "text":"长".repeat(800)}]}}}),
            json!({"type":"assistant/message", "time":3000, "data":{"content":[{"type":"text", "text":"Answer"}]}}),
        ];

        let messages = parse_dsh_history(&events);
        assert_eq!(messages.len(), 2);
        let [question, answer] = &messages[..] else {
            unreachable!();
        };
        assert_eq!(question.message.content, "Question");
        assert_eq!(question.images[0].media_type, "image/png");
        assert_eq!(answer.message.content, "Answer");
        assert_eq!(answer.message.timestamp, 2);
        assert_eq!(
            answer.message.reasoning.as_deref(),
            Some("Consider options")
        );
        assert!(answer.message.model_messages.is_empty());
        let tool = &answer.message.tool_calls[0];
        assert_eq!(tool.id, "call-1");
        assert!(tool.error.as_deref().unwrap().contains("导入时已截断"));
    }
}
