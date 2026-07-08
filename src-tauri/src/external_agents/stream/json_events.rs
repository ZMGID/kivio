use std::collections::HashSet;

use serde_json::Value;

use crate::external_agents::stream::usage_from_numbers;
use crate::external_agents::types::{JsonEventParser, UnifiedAgentEvent};

pub struct JsonEventStreamState {
    parser: JsonEventParser,
    cursor_text: String,
    opencode_tool_uses: HashSet<String>,
    codex_tool_uses: HashSet<String>,
}

impl JsonEventStreamState {
    pub fn new(parser: JsonEventParser) -> Self {
        Self {
            parser,
            cursor_text: String::new(),
            opencode_tool_uses: HashSet::new(),
            codex_tool_uses: HashSet::new(),
        }
    }

    pub fn handle_value(&mut self, value: &Value, sink: &mut dyn FnMut(UnifiedAgentEvent)) {
        match self.parser {
            JsonEventParser::Codex => self.handle_codex(value, sink),
            JsonEventParser::CursorAgent => self.handle_cursor(value, sink),
            JsonEventParser::OpenCode => self.handle_opencode(value, sink),
            JsonEventParser::Gemini => self.handle_gemini(value, sink),
            JsonEventParser::Kimi => self.handle_kimi(value, sink),
        }
    }

    fn handle_codex(&mut self, value: &Value, sink: &mut dyn FnMut(UnifiedAgentEvent)) {
        let obj = match value.as_object() {
            Some(o) => o,
            None => return,
        };
        let kind = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match kind {
            "thread.started" => sink(UnifiedAgentEvent::Status {
                label: "initializing".to_string(),
                model: None,
            }),
            "turn.started" => sink(UnifiedAgentEvent::Status {
                label: "running".to_string(),
                model: None,
            }),
            "item.started" => {
                if let Some(item) = obj.get("item").and_then(|v| v.as_object()) {
                    self.emit_codex_command_execution(item, sink, false);
                }
            }
            "item.completed" => {
                if let Some(item) = obj.get("item").and_then(|v| v.as_object()) {
                    match item.get("type").and_then(|v| v.as_str()) {
                        Some("agent_message") => {
                            if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                                sink(UnifiedAgentEvent::TextDelta {
                                    delta: text.to_string(),
                                });
                            }
                        }
                        Some("reasoning") => {
                            if let Some(text) = item
                                .get("text")
                                .and_then(|v| v.as_str())
                                .filter(|t| !t.is_empty())
                            {
                                sink(UnifiedAgentEvent::ThinkingDelta {
                                    delta: text.to_string(),
                                });
                            }
                        }
                        _ => self.emit_codex_command_execution(item, sink, true),
                    }
                }
            }
            "turn.completed" => {
                if let Some(usage) = obj.get("usage").and_then(|v| v.as_object()) {
                    let input = usage
                        .get("input_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let output = usage
                        .get("output_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    sink(UnifiedAgentEvent::Usage {
                        usage: usage_from_numbers(input, output),
                    });
                }
            }
            "error" | "turn.failed" => sink(UnifiedAgentEvent::Error {
                message: value.to_string(),
                code: None,
            }),
            _ => {}
        }
    }

    fn emit_codex_command_execution(
        &mut self,
        item: &serde_json::Map<String, Value>,
        sink: &mut dyn FnMut(UnifiedAgentEvent),
        include_result: bool,
    ) {
        if item.get("type").and_then(|v| v.as_str()) != Some("command_execution") {
            return;
        }
        let id = match item
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            Some(id) => id.to_string(),
            None => return,
        };
        let command = item
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if !self.codex_tool_uses.contains(&id) {
            self.codex_tool_uses.insert(id.clone());
            sink(UnifiedAgentEvent::ToolUse {
                id: id.clone(),
                name: "Bash".to_string(),
                input: serde_json::json!({ "command": command }),
            });
        }
        if !include_result {
            return;
        }
        let content = item
            .get("aggregated_output")
            .map(stringify_json_value)
            .unwrap_or_default();
        let exit_code = item.get("exit_code").and_then(|v| v.as_u64());
        let status_failed = item.get("status").and_then(|v| v.as_str()) == Some("failed");
        let is_error = exit_code.map(|code| code != 0).unwrap_or(status_failed);
        sink(UnifiedAgentEvent::ToolResult {
            tool_use_id: id,
            content,
            is_error,
        });
    }

    fn handle_cursor(&mut self, value: &Value, sink: &mut dyn FnMut(UnifiedAgentEvent)) {
        let obj = match value.as_object() {
            Some(o) => o,
            None => return,
        };
        let kind = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match kind {
            "system" => {
                if obj.get("subtype").and_then(|v| v.as_str()) == Some("init") {
                    sink(UnifiedAgentEvent::Status {
                        label: "initializing".to_string(),
                        model: None,
                    });
                }
            }
            "assistant" => {
                if obj.get("timestamp_ms").is_some() {
                    if let Some(message) = obj.get("message").and_then(|v| v.as_object()) {
                        if let Some(content) = message.get("content").and_then(|v| v.as_array()) {
                            for block in content {
                                if let Some(text) = block
                                    .get("text")
                                    .and_then(|v| v.as_str())
                                    .or_else(|| block.as_str())
                                {
                                    self.cursor_text.push_str(text);
                                    sink(UnifiedAgentEvent::TextDelta {
                                        delta: text.to_string(),
                                    });
                                }
                            }
                        }
                    } else if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                        sink(UnifiedAgentEvent::TextDelta {
                            delta: text.to_string(),
                        });
                    }
                }
            }
            "result" => {
                if let Some(usage) = obj.get("usage").and_then(|v| v.as_object()) {
                    let input = usage
                        .get("input_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let output = usage
                        .get("output_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    sink(UnifiedAgentEvent::Usage {
                        usage: usage_from_numbers(input, output),
                    });
                }
            }
            _ => {}
        }
    }

    fn handle_opencode(&mut self, value: &Value, sink: &mut dyn FnMut(UnifiedAgentEvent)) {
        let obj = match value.as_object() {
            Some(o) => o,
            None => return,
        };
        let kind = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let part = obj.get("part").and_then(|v| v.as_object());

        match kind {
            "step_start" => sink(UnifiedAgentEvent::Status {
                label: "running".to_string(),
                model: None,
            }),
            "text" => {
                if let Some(part) = part {
                    if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                        if !text.is_empty() {
                            sink(UnifiedAgentEvent::TextDelta {
                                delta: text.to_string(),
                            });
                        }
                    }
                }
            }
            "reasoning" => {
                if let Some(part) = part {
                    if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                        if !text.is_empty() {
                            sink(UnifiedAgentEvent::ThinkingDelta {
                                delta: text.to_string(),
                            });
                        }
                    }
                }
            }
            "tool_use" => {
                if let Some(part) = part {
                    let tool = part.get("tool").and_then(|v| v.as_str()).unwrap_or("");
                    let call_id = part.get("callID").and_then(|v| v.as_str()).unwrap_or("");
                    if tool.is_empty() || call_id.is_empty() {
                        return;
                    }
                    let session_id = obj
                        .get("sessionID")
                        .and_then(|v| v.as_str())
                        .unwrap_or("session");
                    let key = format!("{session_id}:{call_id}");
                    if !self.opencode_tool_uses.contains(&key) {
                        self.opencode_tool_uses.insert(key);
                        let state = part.get("state").and_then(|v| v.as_object());
                        let input = state
                            .and_then(|s| s.get("input"))
                            .cloned()
                            .unwrap_or(Value::Null);
                        sink(UnifiedAgentEvent::ToolUse {
                            id: call_id.to_string(),
                            name: tool.to_string(),
                            input,
                        });
                    }
                    if let Some(state) = part.get("state").and_then(|v| v.as_object()) {
                        if state.get("status").and_then(|v| v.as_str()) == Some("completed") {
                            let output = state
                                .get("output")
                                .map(stringify_json_value)
                                .unwrap_or_default();
                            sink(UnifiedAgentEvent::ToolResult {
                                tool_use_id: call_id.to_string(),
                                content: output,
                                is_error: false,
                            });
                        }
                    }
                }
            }
            "step_finish" => {
                if let Some(part) = part {
                    if let Some(tokens) = part.get("tokens").and_then(|v| v.as_object()) {
                        let input = tokens
                            .get("input")
                            .or_else(|| tokens.get("input_tokens"))
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        let output = tokens
                            .get("output")
                            .or_else(|| tokens.get("output_tokens"))
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        if input > 0 || output > 0 {
                            sink(UnifiedAgentEvent::Usage {
                                usage: usage_from_numbers(input, output),
                            });
                        }
                    }
                }
            }
            "error" => {
                let message = obj
                    .get("error")
                    .or_else(|| obj.get("message"))
                    .map(stringify_json_value)
                    .unwrap_or_else(|| "OpenCode error".to_string());
                sink(UnifiedAgentEvent::Error {
                    message,
                    code: None,
                });
            }
            _ => {}
        }
    }

    fn handle_gemini(&self, value: &Value, sink: &mut dyn FnMut(UnifiedAgentEvent)) {
        let obj = match value.as_object() {
            Some(o) => o,
            None => return,
        };
        let kind = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");

        if kind == "init" {
            sink(UnifiedAgentEvent::Status {
                label: "initializing".to_string(),
                model: obj
                    .get("model")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            });
            return;
        }

        if kind == "message" && obj.get("role").and_then(|v| v.as_str()) == Some("assistant") {
            if let Some(text) = obj.get("content").and_then(|v| v.as_str()) {
                if !text.is_empty() {
                    sink(UnifiedAgentEvent::TextDelta {
                        delta: text.to_string(),
                    });
                }
            }
            return;
        }

        if kind == "tool_use" {
            let id = obj
                .get("tool_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let name = obj
                .get("tool_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !id.is_empty() && !name.is_empty() {
                let input = obj.get("parameters").cloned().unwrap_or(Value::Null);
                sink(UnifiedAgentEvent::ToolUse { id, name, input });
            }
        }
    }

    fn handle_kimi(&self, value: &Value, sink: &mut dyn FnMut(UnifiedAgentEvent)) {
        let obj = match value.as_object() {
            Some(o) => o,
            None => return,
        };
        let role = obj.get("role").and_then(|v| v.as_str()).unwrap_or("");

        if role == "assistant" {
            if let Some(calls) = obj.get("tool_calls").and_then(|v| v.as_array()) {
                for raw_call in calls {
                    let call = raw_call.as_object();
                    let func = call
                        .and_then(|c| c.get("function"))
                        .and_then(|v| v.as_object());
                    let id = call
                        .and_then(|c| c.get("id"))
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty());
                    let name = func
                        .and_then(|f| f.get("name"))
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty());
                    if let (Some(id), Some(name)) = (id, name) {
                        let input = func
                            .and_then(|f| f.get("arguments"))
                            .cloned()
                            .unwrap_or(Value::Null);
                        sink(UnifiedAgentEvent::ToolUse {
                            id: id.to_string(),
                            name: name.to_string(),
                            input,
                        });
                    }
                }
                return;
            }
            if let Some(text) = obj.get("content").and_then(|v| v.as_str()) {
                if !text.is_empty() {
                    sink(UnifiedAgentEvent::TextDelta {
                        delta: text.to_string(),
                    });
                }
            }
            return;
        }

        if role == "tool" {
            if let Some(tool_use_id) = obj.get("tool_call_id").and_then(|v| v.as_str()) {
                if !tool_use_id.trim().is_empty() {
                    let content = obj
                        .get("content")
                        .map(stringify_json_value)
                        .unwrap_or_default();
                    sink(UnifiedAgentEvent::ToolResult {
                        tool_use_id: tool_use_id.trim().to_string(),
                        content,
                        is_error: false,
                    });
                }
            }
        }
    }
}

fn stringify_json_value(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        _ => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_command_execution_emits_tool_use_and_result() {
        let started = r#"{"type":"item.started","item":{"type":"command_execution","id":"cmd-1","command":"ls"}}"#;
        let completed = r#"{"type":"item.completed","item":{"type":"command_execution","id":"cmd-1","command":"ls","aggregated_output":"ok\n","exit_code":0,"status":"completed"}}"#;
        let mut state = JsonEventStreamState::new(JsonEventParser::Codex);
        let mut events = Vec::new();
        state.handle_value(&serde_json::from_str(started).unwrap(), &mut |e| {
            events.push(e)
        });
        state.handle_value(&serde_json::from_str(completed).unwrap(), &mut |e| {
            events.push(e)
        });
        assert!(matches!(
            events.first(),
            Some(UnifiedAgentEvent::ToolUse { id, name, .. }) if id == "cmd-1" && name == "Bash"
        ));
        assert!(events.iter().any(|event| matches!(
            event,
            UnifiedAgentEvent::ToolResult { tool_use_id, content, is_error }
                if tool_use_id == "cmd-1" && content.contains("ok") && !*is_error
        )));
    }

    #[test]
    fn codex_agent_message_emits_text_delta() {
        let raw = r#"{"type":"item.completed","item":{"type":"agent_message","text":"hello"}}"#;
        let value: Value = serde_json::from_str(raw).unwrap();
        let mut events = Vec::new();
        JsonEventStreamState::new(JsonEventParser::Codex)
            .handle_value(&value, &mut |e| events.push(e));
        assert!(matches!(
            events.first(),
            Some(UnifiedAgentEvent::TextDelta { delta }) if delta == "hello"
        ));
    }

    #[test]
    fn opencode_text_emits_delta() {
        let raw = r#"{"type":"text","part":{"text":"hello"}}"#;
        let value: Value = serde_json::from_str(raw).unwrap();
        let mut events = Vec::new();
        JsonEventStreamState::new(JsonEventParser::OpenCode)
            .handle_value(&value, &mut |e| events.push(e));
        assert!(matches!(
            events.first(),
            Some(UnifiedAgentEvent::TextDelta { delta }) if delta == "hello"
        ));
    }

    #[test]
    fn codex_reasoning_emits_thinking_delta() {
        let raw =
            r#"{"type":"item.completed","item":{"type":"reasoning","text":"weighing options"}}"#;
        let value: Value = serde_json::from_str(raw).unwrap();
        let mut events = Vec::new();
        JsonEventStreamState::new(JsonEventParser::Codex)
            .handle_value(&value, &mut |e| events.push(e));
        assert!(matches!(
            events.first(),
            Some(UnifiedAgentEvent::ThinkingDelta { delta }) if delta == "weighing options"
        ));
    }

    #[test]
    fn opencode_reasoning_emits_thinking_delta() {
        let raw = r#"{"type":"reasoning","part":{"text":"let me think"}}"#;
        let value: Value = serde_json::from_str(raw).unwrap();
        let mut events = Vec::new();
        JsonEventStreamState::new(JsonEventParser::OpenCode)
            .handle_value(&value, &mut |e| events.push(e));
        assert!(matches!(
            events.first(),
            Some(UnifiedAgentEvent::ThinkingDelta { delta }) if delta == "let me think"
        ));
    }

    #[test]
    fn gemini_assistant_message_emits_delta() {
        let raw = r#"{"type":"message","role":"assistant","content":"hi"}"#;
        let value: Value = serde_json::from_str(raw).unwrap();
        let mut events = Vec::new();
        JsonEventStreamState::new(JsonEventParser::Gemini)
            .handle_value(&value, &mut |e| events.push(e));
        assert!(matches!(
            events.first(),
            Some(UnifiedAgentEvent::TextDelta { delta }) if delta == "hi"
        ));
    }

    #[test]
    fn kimi_assistant_content_emits_delta() {
        let raw = r#"{"role":"assistant","content":"world"}"#;
        let value: Value = serde_json::from_str(raw).unwrap();
        let mut events = Vec::new();
        JsonEventStreamState::new(JsonEventParser::Kimi)
            .handle_value(&value, &mut |e| events.push(e));
        assert!(matches!(
            events.first(),
            Some(UnifiedAgentEvent::TextDelta { delta }) if delta == "world"
        ));
    }
}
