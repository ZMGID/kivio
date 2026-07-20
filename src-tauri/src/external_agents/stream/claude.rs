use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::external_agents::slash::parse_slash_commands_from_init;
use crate::external_agents::stream::usage_from_numbers;
use crate::external_agents::types::UnifiedAgentEvent;

struct PendingContentBlock {
    block_type: String,
    id: Option<String>,
    name: Option<String>,
    input_json: String,
    input_value: Option<Value>,
}

#[derive(Default)]
pub struct ClaudeStreamState {
    text_streamed: bool,
    current_message_id: Option<String>,
    blocks: HashMap<String, PendingContentBlock>,
    streamed_tool_use_ids: HashSet<String>,
}

impl ClaudeStreamState {
    pub fn handle_value(&mut self, value: &Value, sink: &mut dyn FnMut(UnifiedAgentEvent)) {
        let obj = match value.as_object() {
            Some(o) => o,
            None => return,
        };
        let kind = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match kind {
            "system" => {
                if obj.get("subtype").and_then(|v| v.as_str()) == Some("init") {
                    let commands = parse_slash_commands_from_init(value);
                    if !commands.is_empty() {
                        sink(UnifiedAgentEvent::SlashCommands { commands });
                    }
                }
            }
            "stream_event" => {
                if let Some(event) = obj.get("event").and_then(|v| v.as_object()) {
                    self.handle_stream_event(event, sink);
                }
            }
            "assistant" => {
                if let Some(message) = obj.get("message").and_then(|v| v.as_object()) {
                    if let Some(content) = message.get("content").and_then(|v| v.as_array()) {
                        for block in content {
                            let block = match block.as_object() {
                                Some(b) => b,
                                None => continue,
                            };
                            match block.get("type").and_then(|v| v.as_str()) {
                                Some("text") => {
                                    if !self.text_streamed {
                                        if let Some(text) =
                                            block.get("text").and_then(|v| v.as_str())
                                        {
                                            sink(UnifiedAgentEvent::TextDelta {
                                                delta: text.to_string(),
                                            });
                                        }
                                    }
                                }
                                Some("tool_use") => {
                                    let id = block
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("tool")
                                        .to_string();
                                    if self.streamed_tool_use_ids.contains(&id) {
                                        continue;
                                    }
                                    self.streamed_tool_use_ids.insert(id.clone());
                                    sink(UnifiedAgentEvent::ToolUse {
                                        id,
                                        name: block
                                            .get("name")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("tool")
                                            .to_string(),
                                        input: block.get("input").cloned().unwrap_or(Value::Null),
                                    });
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            "user" => {
                if let Some(message) = obj.get("message").and_then(|v| v.as_object()) {
                    if let Some(content) = message.get("content").and_then(|v| v.as_array()) {
                        for block in content {
                            let block = match block.as_object() {
                                Some(b) => b,
                                None => continue,
                            };
                            if block.get("type").and_then(|v| v.as_str()) == Some("tool_result") {
                                sink(UnifiedAgentEvent::ToolResult {
                                    tool_use_id: block
                                        .get("tool_use_id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string(),
                                    content: block
                                        .get("content")
                                        .map(|v| {
                                            if let Some(s) = v.as_str() {
                                                s.to_string()
                                            } else {
                                                v.to_string()
                                            }
                                        })
                                        .unwrap_or_default(),
                                    is_error: block
                                        .get("is_error")
                                        .and_then(|v| v.as_bool())
                                        .unwrap_or(false),
                                });
                            }
                        }
                    }
                }
            }
            "result" => {
                let usage = obj.get("usage").and_then(|u| u.as_object());
                let input = usage
                    .and_then(|u| u.get("input_tokens"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let output = usage
                    .and_then(|u| u.get("output_tokens"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                sink(UnifiedAgentEvent::Usage {
                    usage: usage_from_numbers(input, output),
                });
            }
            "error" => {
                sink(UnifiedAgentEvent::Error {
                    message: obj
                        .get("error")
                        .and_then(|v| v.as_str())
                        .or_else(|| obj.get("message").and_then(|v| v.as_str()))
                        .unwrap_or("unknown error")
                        .to_string(),
                });
            }
            _ => {}
        }
    }

    fn block_key(&self, index: &Value) -> String {
        format!(
            "{}:{}",
            self.current_message_id.as_deref().unwrap_or("anon"),
            index.as_u64().unwrap_or(0)
        )
    }

    fn handle_stream_event(
        &mut self,
        event: &serde_json::Map<String, Value>,
        sink: &mut dyn FnMut(UnifiedAgentEvent),
    ) {
        let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match event_type {
            "message_start" => {
                // 新 assistant 消息开始:复位 `text_streamed`(N7)。否则上一条消息经 delta
                // 流式发出后此标志一直为真,导致后续整块交付的 assistant 消息正文被永久跳过。
                self.text_streamed = false;
                self.current_message_id = event
                    .get("message")
                    .and_then(|v| v.get("id"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
            }
            "content_block_start" => {
                let Some(block) = event.get("content_block").and_then(|v| v.as_object()) else {
                    return;
                };
                let block_type = block
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let key = self.block_key(event.get("index").unwrap_or(&Value::Null));
                self.blocks.insert(
                    key,
                    PendingContentBlock {
                        block_type,
                        id: block.get("id").and_then(|v| v.as_str()).map(str::to_string),
                        name: block
                            .get("name")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        input_json: String::new(),
                        input_value: block.get("input").cloned(),
                    },
                );
            }
            "content_block_delta" => {
                if let Some(delta) = event.get("delta").and_then(|v| v.as_object()) {
                    match delta.get("type").and_then(|v| v.as_str()) {
                        Some("text_delta") => {
                            if let Some(text) = delta.get("text").and_then(|v| v.as_str()) {
                                self.text_streamed = true;
                                sink(UnifiedAgentEvent::TextDelta {
                                    delta: text.to_string(),
                                });
                            }
                        }
                        Some("thinking_delta") => {
                            if let Some(text) = delta.get("thinking").and_then(|v| v.as_str()) {
                                sink(UnifiedAgentEvent::ThinkingDelta {
                                    delta: text.to_string(),
                                });
                            }
                        }
                        Some("input_json_delta") => {
                            let key = self.block_key(event.get("index").unwrap_or(&Value::Null));
                            if let Some(state) = self.blocks.get_mut(&key) {
                                if let Some(partial) =
                                    delta.get("partial_json").and_then(|v| v.as_str())
                                {
                                    state.input_json.push_str(partial);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            "content_block_stop" => {
                let key = self.block_key(event.get("index").unwrap_or(&Value::Null));
                let Some(state) = self.blocks.remove(&key) else {
                    return;
                };
                if state.block_type != "tool_use" {
                    return;
                }
                let id = state.id.unwrap_or_else(|| "tool".to_string());
                if self.streamed_tool_use_ids.contains(&id) {
                    return;
                }
                let name = state.name.unwrap_or_else(|| "tool".to_string());
                let input = if !state.input_json.trim().is_empty() {
                    serde_json::from_str(&state.input_json)
                        .unwrap_or_else(|_| Value::String(state.input_json.clone()))
                } else {
                    state.input_value.unwrap_or(Value::Null)
                };
                self.streamed_tool_use_ids.insert(id.clone());
                sink(UnifiedAgentEvent::ToolUse { id, name, input });
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_text_delta_from_stream_event() {
        let raw = r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"hi"}}}"#;
        let value: Value = serde_json::from_str(raw).unwrap();
        let mut events = Vec::new();
        ClaudeStreamState::default().handle_value(&value, &mut |e| events.push(e));
        assert!(matches!(
            events.first(),
            Some(UnifiedAgentEvent::TextDelta { delta }) if delta == "hi"
        ));
    }

    #[test]
    fn parses_streamed_tool_use_from_content_blocks() {
        let chunks = [
            r#"{"type":"stream_event","event":{"type":"message_start","message":{"id":"msg-1"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu-1","name":"Write"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"file_path\":\"page.html\"}"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_stop","index":0}}"#,
        ];
        let mut state = ClaudeStreamState::default();
        let mut events = Vec::new();
        for raw in chunks {
            let value: Value = serde_json::from_str(raw).unwrap();
            state.handle_value(&value, &mut |e| events.push(e));
        }
        assert!(events.iter().any(|event| matches!(
            event,
            UnifiedAgentEvent::ToolUse { id, name, .. }
                if id == "toolu-1" && name == "Write"
        )));
    }

    #[test]
    fn text_streamed_resets_per_message() {
        // msg1 经 delta 流式发出;msg2 只以整块 assistant 帧交付。复位后两条正文都应发出。
        let chunks = [
            r#"{"type":"stream_event","event":{"type":"message_start","message":{"id":"msg-1"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"first"}}}"#,
            r#"{"type":"assistant","message":{"id":"msg-1","content":[{"type":"text","text":"first"}]}}"#,
            r#"{"type":"stream_event","event":{"type":"message_start","message":{"id":"msg-2"}}}"#,
            r#"{"type":"assistant","message":{"id":"msg-2","content":[{"type":"text","text":"second"}]}}"#,
        ];
        let mut state = ClaudeStreamState::default();
        let mut events = Vec::new();
        for raw in chunks {
            let value: Value = serde_json::from_str(raw).unwrap();
            state.handle_value(&value, &mut |e| events.push(e));
        }
        let texts: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                UnifiedAgentEvent::TextDelta { delta } => Some(delta.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["first", "second"]);
    }

    #[test]
    fn parses_slash_commands_from_init() {
        let raw = r#"{"type":"system","subtype":"init","slash_commands":["compact","clear"]}"#;
        let value: Value = serde_json::from_str(raw).unwrap();
        let mut events = Vec::new();
        ClaudeStreamState::default().handle_value(&value, &mut |e| events.push(e));
        assert!(events.iter().any(|event| matches!(
            event,
            UnifiedAgentEvent::SlashCommands { commands }
                if commands.len() == 2 && commands.iter().any(|c| c.slash == "/compact")
        )));
    }
}
