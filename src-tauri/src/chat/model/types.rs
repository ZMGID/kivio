use std::{future::Future, pin::Pin};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::mcp::{self, types::ChatToolArtifact, ChatToolDefinition};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelRole {
    User,
    Assistant,
    Tool,
}

impl ModelRole {
    pub fn as_str(self) -> &'static str {
        match self {
            ModelRole::User => "user",
            ModelRole::Assistant => "assistant",
            ModelRole::Tool => "tool",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessagePart {
    Text {
        text: String,
    },
    Image {
        mime_type: String,
        data: String,
    },
    ImageUrl {
        url: String,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: Value,
        arguments_raw: String,
    },
    ToolResult {
        tool_call_id: String,
        content: String,
        is_error: bool,
        artifacts: Vec<ChatToolArtifact>,
    },
    Reasoning {
        text: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMessage {
    pub role: ModelRole,
    pub content: Vec<MessagePart>,
}

impl ModelMessage {
    pub fn text(role: ModelRole, text: impl Into<String>) -> Self {
        Self {
            role,
            content: vec![MessagePart::Text { text: text.into() }],
        }
    }

    pub fn text_content(&self) -> String {
        self.content
            .iter()
            .filter_map(|part| match part {
                MessagePart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelTool {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source: String,
    pub server_id: Option<String>,
    pub server_name: Option<String>,
    pub input_schema: Value,
    pub sensitive: bool,
}

impl ModelTool {
    pub fn openai_tool_name(&self) -> String {
        match self.source.as_str() {
            "native" | "skill" | "mixer" => mcp::types::sanitize_openai_tool_name(&self.name),
            _ => mcp::types::sanitize_openai_tool_name(&self.id),
        }
    }

    pub fn to_openai_tool(&self) -> Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": self.openai_tool_name(),
                "description": self.description,
                "parameters": self.input_schema,
            }
        })
    }
}

impl From<&ChatToolDefinition> for ModelTool {
    fn from(tool: &ChatToolDefinition) -> Self {
        Self {
            id: tool.id.clone(),
            name: tool.name.clone(),
            description: tool.description.clone(),
            source: tool.source.clone(),
            server_id: tool.server_id.clone(),
            server_name: tool.server_name.clone(),
            input_schema: tool.input_schema.clone(),
            sensitive: tool.sensitive,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub tool_calling: bool,
    pub vision: bool,
    pub streaming: bool,
    pub reasoning: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateOptions {
    pub temperature: f32,
    pub max_tokens: u32,
    pub stream: bool,
    pub thinking_enabled: bool,
    #[serde(default)]
    pub provider_options: Value,
}

impl Default for GenerateOptions {
    fn default() -> Self {
        Self {
            temperature: 0.7,
            max_tokens: 8192,
            stream: false,
            thinking_enabled: true,
            provider_options: Value::Object(Default::default()),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RequestMetadata {
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateRequest {
    pub model: String,
    pub system: String,
    pub messages: Vec<ModelMessage>,
    pub tools: Vec<ModelTool>,
    pub options: GenerateOptions,
    pub metadata: RequestMetadata,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingToolCall {
    pub id: String,
    pub function_name: String,
    pub arguments: Value,
    pub arguments_raw: String,
    pub arguments_parse_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateOutput {
    pub text: String,
    pub reasoning: Option<String>,
    pub tool_calls: Vec<PendingToolCall>,
    pub usage: Option<ModelUsage>,
    pub finish_reason: Option<String>,
    pub provider_messages: Vec<Value>,
    pub cancelled: bool,
}

impl GenerateOutput {
    pub fn text(text: String, reasoning: Option<String>, provider_message: Value) -> Self {
        Self {
            text,
            reasoning,
            tool_calls: Vec::new(),
            usage: None,
            finish_reason: None,
            provider_messages: vec![provider_message],
            cancelled: false,
        }
    }

    pub fn cancelled(text: String, reasoning: Option<String>) -> Self {
        Self {
            text,
            reasoning,
            tool_calls: Vec::new(),
            usage: None,
            finish_reason: Some("cancelled".to_string()),
            provider_messages: Vec::new(),
            cancelled: true,
        }
    }

    pub fn to_openai_compatible_message(&self) -> Value {
        if let Some(message) = self.provider_messages.first() {
            return message.clone();
        }
        let mut message = serde_json::json!({
            "role": "assistant",
            "content": if self.text.is_empty() { Value::Null } else { Value::String(self.text.clone()) },
        });
        if let Some(reasoning) = self
            .reasoning
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            message["reasoning_content"] = Value::String(reasoning.to_string());
        }
        if !self.tool_calls.is_empty() {
            message["tool_calls"] = Value::Array(
                self.tool_calls
                    .iter()
                    .map(|call| {
                        serde_json::json!({
                            "id": call.id,
                            "type": "function",
                            "function": {
                                "name": call.function_name,
                                "arguments": call.arguments_raw,
                            }
                        })
                    })
                    .collect(),
            );
        }
        message
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamPart {
    TextDelta {
        delta: String,
    },
    ReasoningDelta {
        delta: String,
    },
    ToolCallStart {
        id: String,
        name: String,
    },
    ToolCallDelta {
        id: String,
        delta: String,
    },
    ToolCallDone {
        call: PendingToolCall,
    },
    ToolResult {
        tool_call_id: String,
        content: String,
    },
    Finish {
        reason: String,
        full: String,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone)]
pub struct ModelError {
    pub message: String,
}

impl ModelError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ModelError {}

impl From<String> for ModelError {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for ModelError {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

pub trait StreamSink: Send {
    fn emit(&mut self, part: StreamPart) -> Result<(), ModelError>;
}

impl<F> StreamSink for F
where
    F: FnMut(StreamPart) -> Result<(), ModelError> + Send,
{
    fn emit(&mut self, part: StreamPart) -> Result<(), ModelError> {
        self(part)
    }
}

pub type ModelFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ModelError>> + Send + 'a>>;

pub trait LanguageModelProvider {
    fn generate<'a>(&'a self, request: GenerateRequest) -> ModelFuture<'a, GenerateOutput>;
    fn stream<'a>(
        &'a self,
        request: GenerateRequest,
        sink: &'a mut (dyn StreamSink + Send),
    ) -> ModelFuture<'a, GenerateOutput>;
    fn capabilities(&self) -> ProviderCapabilities;
}

pub fn parse_tool_arguments(arguments_raw: &str) -> (Value, Option<String>) {
    let raw = if arguments_raw.trim().is_empty() {
        "{}"
    } else {
        arguments_raw
    };
    match serde_json::from_str(raw) {
        Ok(arguments) => (arguments, None),
        Err(err) => (
            Value::Null,
            Some(format!(
                "Tool arguments JSON is invalid or incomplete: {err}"
            )),
        ),
    }
}

pub fn generate_request_from_openai_messages(
    model: &str,
    messages: Vec<Value>,
    tools: Option<&[ChatToolDefinition]>,
    options: GenerateOptions,
    label: &str,
) -> GenerateRequest {
    let mut system_parts = Vec::new();
    let mut model_messages = Vec::new();
    for message in messages {
        let role = message
            .get("role")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if role == "system" {
            if let Some(content) = openai_message_text_content(&message) {
                system_parts.push(content);
            }
            continue;
        }
        if let Some(model_message) = model_message_from_openai_message(&message) {
            model_messages.push(model_message);
        }
    }
    GenerateRequest {
        model: model.to_string(),
        system: system_parts.join("\n\n"),
        messages: model_messages,
        tools: tools
            .unwrap_or_default()
            .iter()
            .map(ModelTool::from)
            .collect(),
        options,
        metadata: RequestMetadata {
            label: label.to_string(),
        },
    }
}

pub fn model_messages_from_openai_messages(messages: Vec<Value>) -> Vec<ModelMessage> {
    messages
        .into_iter()
        .filter_map(|message| model_message_from_openai_message(&message))
        .collect()
}

pub fn openai_messages_from_generate_request(request: &GenerateRequest) -> Vec<Value> {
    let mut messages = Vec::new();
    if !request.system.trim().is_empty() {
        messages.push(serde_json::json!({
            "role": "system",
            "content": request.system,
        }));
    }
    for message in &request.messages {
        messages.extend(openai_messages_from_model_message(message));
    }
    messages
}

pub fn openai_messages_from_model_messages(messages: &[ModelMessage]) -> Vec<Value> {
    messages
        .iter()
        .flat_map(openai_messages_from_model_message)
        .collect()
}

fn openai_message_text_content(message: &Value) -> Option<String> {
    match message.get("content")? {
        Value::String(value) => Some(value.clone()),
        Value::Array(parts) => {
            let texts = parts
                .iter()
                .filter_map(|part| {
                    if part.get("type").and_then(|value| value.as_str()) == Some("text") {
                        part.get("text").and_then(|value| value.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            if texts.is_empty() {
                None
            } else {
                Some(texts.join("\n"))
            }
        }
        _ => None,
    }
}

fn model_message_from_openai_message(message: &Value) -> Option<ModelMessage> {
    let role = match message
        .get("role")
        .and_then(|value| value.as_str())
        .unwrap_or("")
    {
        "assistant" => ModelRole::Assistant,
        "tool" => ModelRole::Tool,
        "user" => ModelRole::User,
        _ => return None,
    };
    let mut parts = Vec::new();
    if role == ModelRole::Tool {
        parts.push(MessagePart::ToolResult {
            tool_call_id: message
                .get("tool_call_id")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string(),
            content: message
                .get("content")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string(),
            is_error: false,
            artifacts: Vec::new(),
        });
        return Some(ModelMessage {
            role,
            content: parts,
        });
    }
    if let Some(content) = message.get("content") {
        match content {
            Value::String(text) if !text.is_empty() => {
                parts.push(MessagePart::Text { text: text.clone() });
            }
            Value::Array(items) => {
                for item in items {
                    match item
                        .get("type")
                        .and_then(|value| value.as_str())
                        .unwrap_or("")
                    {
                        "text" => {
                            if let Some(text) = item.get("text").and_then(|value| value.as_str()) {
                                parts.push(MessagePart::Text {
                                    text: text.to_string(),
                                });
                            }
                        }
                        "image_url" => {
                            let url = item
                                .get("image_url")
                                .and_then(|value| value.get("url"))
                                .and_then(|value| value.as_str())
                                .unwrap_or_default();
                            if let Some((mime_type, data)) = parse_data_image_url(url) {
                                parts.push(MessagePart::Image { mime_type, data });
                            } else if !url.is_empty() {
                                parts.push(MessagePart::ImageUrl {
                                    url: url.to_string(),
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(reasoning) = message
        .get("reasoning_content")
        .or_else(|| message.get("reasoning"))
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(MessagePart::Reasoning {
            text: reasoning.to_string(),
        });
    }
    for call in pending_tool_calls_from_openai_message(message) {
        parts.push(MessagePart::ToolCall {
            id: call.id,
            name: call.function_name,
            arguments: call.arguments,
            arguments_raw: call.arguments_raw,
        });
    }
    Some(ModelMessage {
        role,
        content: parts,
    })
}

pub fn pending_tool_calls_from_openai_message(message: &Value) -> Vec<PendingToolCall> {
    message
        .get("tool_calls")
        .and_then(|value| value.as_array())
        .map(|calls| {
            calls
                .iter()
                .filter_map(|call| {
                    let function = call.get("function")?;
                    let name = function.get("name")?.as_str()?.to_string();
                    let arguments_raw = function
                        .get("arguments")
                        .and_then(|value| value.as_str())
                        .unwrap_or("{}")
                        .to_string();
                    let (arguments, arguments_parse_error) = parse_tool_arguments(&arguments_raw);
                    Some(PendingToolCall {
                        id: call
                            .get("id")
                            .and_then(|value| value.as_str())
                            .map(str::to_string)
                            .unwrap_or_else(|| format!("tool_{}", uuid::Uuid::new_v4())),
                        function_name: name,
                        arguments,
                        arguments_raw,
                        arguments_parse_error,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn openai_messages_from_model_message(message: &ModelMessage) -> Vec<Value> {
    if message.role == ModelRole::Tool {
        return message
            .content
            .iter()
            .filter_map(|part| match part {
                MessagePart::ToolResult {
                    tool_call_id,
                    content,
                    ..
                } => Some(serde_json::json!({
                    "role": "tool",
                    "tool_call_id": tool_call_id,
                    "content": content,
                })),
                _ => None,
            })
            .collect();
    }
    let mut text_parts = Vec::new();
    let mut multimodal_parts = Vec::new();
    let mut tool_calls = Vec::new();
    let mut reasoning: Option<String> = None;
    for part in &message.content {
        match part {
            MessagePart::Text { text } => {
                text_parts.push(text.clone());
                multimodal_parts.push(serde_json::json!({ "type": "text", "text": text }));
            }
            MessagePart::Image { mime_type, data } => {
                multimodal_parts.push(serde_json::json!({
                    "type": "image_url",
                    "image_url": { "url": format!("data:{mime_type};base64,{data}") },
                }));
            }
            MessagePart::ImageUrl { url } => {
                multimodal_parts.push(serde_json::json!({
                    "type": "image_url",
                    "image_url": { "url": url },
                }));
            }
            MessagePart::ToolCall {
                id,
                name,
                arguments_raw,
                ..
            } => {
                tool_calls.push(serde_json::json!({
                    "id": id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": arguments_raw,
                    }
                }));
            }
            MessagePart::Reasoning { text } => {
                reasoning = Some(text.clone());
            }
            MessagePart::ToolResult { .. } => {}
        }
    }
    let content = if multimodal_parts
        .iter()
        .any(|part| part.get("type").and_then(|value| value.as_str()) == Some("image_url"))
    {
        Value::Array(multimodal_parts)
    } else if text_parts.is_empty() && !tool_calls.is_empty() {
        Value::Null
    } else {
        Value::String(text_parts.join("\n"))
    };
    let mut out = serde_json::json!({
        "role": message.role.as_str(),
        "content": content,
    });
    if !tool_calls.is_empty() {
        out["tool_calls"] = Value::Array(tool_calls);
    }
    if let Some(reasoning) = reasoning {
        out["reasoning_content"] = Value::String(reasoning);
    }
    vec![out]
}

fn parse_data_image_url(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("data:")?;
    let (mime, data) = rest.split_once(";base64,")?;
    Some((mime.to_string(), data.to_string()))
}
