use serde_json::{Map, Value};

use crate::model::{
    GenerateOptions, GenerateRequest, ModelMessage, ModelRole, ModelTool, RequestMetadata,
};

#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeBody {
    System(String),
    Model(ModelMessage),
}

pub type RuntimeSource = Map<String, Value>;

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeMessage {
    pub body: RuntimeBody,
    pub source: RuntimeSource,
}

impl RuntimeMessage {
    pub fn model(message: ModelMessage) -> Self {
        Self {
            body: RuntimeBody::Model(message),
            source: RuntimeSource::default(),
        }
    }

    pub fn text(role: ModelRole, text: impl Into<String>) -> Self {
        Self::model(ModelMessage::text(role, text))
    }

    pub fn system(text: impl Into<String>) -> Self {
        Self {
            body: RuntimeBody::System(text.into()),
            source: RuntimeSource::default(),
        }
    }

    pub fn role(&self) -> Option<ModelRole> {
        match &self.body {
            RuntimeBody::System(_) => None,
            RuntimeBody::Model(message) => Some(message.role),
        }
    }

    pub fn model_message(&self) -> Option<&ModelMessage> {
        match &self.body {
            RuntimeBody::System(_) => None,
            RuntimeBody::Model(message) => Some(message),
        }
    }

    pub fn model_message_mut(&mut self) -> Option<&mut ModelMessage> {
        match &mut self.body {
            RuntimeBody::System(_) => None,
            RuntimeBody::Model(message) => Some(message),
        }
    }

    pub fn text_content(&self) -> String {
        match &self.body {
            RuntimeBody::System(text) => text.clone(),
            RuntimeBody::Model(message) => message.text_content(),
        }
    }

    pub fn from_legacy(value: Value) -> Option<Self> {
        let mut fields = value.as_object()?.clone();
        let body = if fields.get("role")?.as_str()? == "system" {
            RuntimeBody::System(crate::model::openai_message_text_content(&value)?)
        } else {
            let model = crate::model::model_messages_from_openai_messages(vec![value])
                .into_iter()
                .next()?;
            RuntimeBody::Model(model)
        };
        for key in [
            "role",
            "content",
            "reasoning",
            "reasoning_content",
            "tool_calls",
            "tool_call_id",
            "anthropic_content",
            "reasoning_items",
            "is_error",
            "artifacts",
        ] {
            fields.remove(key);
        }
        Some(Self {
            body,
            source: fields,
        })
    }

    pub fn to_legacy(&self) -> Vec<Value> {
        let messages = match &self.body {
            RuntimeBody::System(content) => vec![serde_json::json!({
                "role": "system", "content": content,
            })],
            RuntimeBody::Model(message) => {
                crate::model::openai_messages_from_model_messages(std::slice::from_ref(message))
            }
        };
        messages
            .into_iter()
            .map(|message| {
                let mut fields = self.source.clone();
                if let Value::Object(body) = message {
                    fields.extend(body);
                }
                Value::Object(fields)
            })
            .collect()
    }
}

pub fn import_history(messages: Vec<Value>) -> Vec<RuntimeMessage> {
    messages
        .into_iter()
        .filter_map(RuntimeMessage::from_legacy)
        .collect()
}

pub fn export_history(messages: &[RuntimeMessage]) -> Vec<Value> {
    messages
        .iter()
        .flat_map(RuntimeMessage::to_legacy)
        .collect()
}

pub fn request_from_history(
    history: &[RuntimeMessage],
    model: &str,
    tools: Vec<ModelTool>,
    options: GenerateOptions,
    metadata: RequestMetadata,
) -> GenerateRequest {
    let mut system = Vec::new();
    let mut messages = Vec::new();
    for entry in history {
        match &entry.body {
            RuntimeBody::System(text) => system.push(text.clone()),
            RuntimeBody::Model(message) => messages.push(message.clone()),
        }
    }
    system.retain(|part| !part.trim().is_empty());
    GenerateRequest {
        model: model.into(),
        system: system.join("\n\n"),
        messages,
        tools,
        options,
        metadata,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MessagePart;
    use serde_json::json;

    #[test]
    fn legacy_sources_survive_export_but_do_not_enter_provider_request() {
        let original = json!({
            "role":"developer", "content":"child result", "_ui_message_id":"ui",
            "_subagent_result_id":"child", "_subagent_irc_id":"irc",
            "subagent_parent_persisted":true, "_subagent_advisor":false,
            "future_source":{"version":2},
        });
        let history = import_history(vec![original.clone()]);
        assert_eq!(
            history[0]
                .source
                .get("_subagent_result_id")
                .and_then(Value::as_str),
            Some("child")
        );
        assert_eq!(export_history(&history), vec![original]);
        let request = request_from_history(
            &history,
            "model",
            Vec::new(),
            GenerateOptions::default(),
            RequestMetadata::default(),
        );
        assert_eq!(request.messages[0].role, ModelRole::Developer);
        let wire = serde_json::to_value(request).unwrap();
        assert!(!wire.to_string().contains("future_source"));
        assert!(!wire.to_string().contains("subagent_parent_persisted"));
    }

    #[test]
    fn request_projection_keeps_source_order_and_workbench_prompt_unchanged() {
        let history = vec![
            RuntimeMessage::system("System\n\nCurrent default workbench: C:/workspace"),
            RuntimeMessage::text(ModelRole::User, "question"),
            RuntimeMessage::system("Later notice"),
        ];
        let before = history.clone();
        let request = request_from_history(
            &history,
            "model",
            Vec::new(),
            GenerateOptions::default(),
            RequestMetadata::default(),
        );
        assert_eq!(
            request.system,
            "System\n\nCurrent default workbench: C:/workspace\n\nLater notice"
        );
        assert!(
            matches!(&request.messages[0].content[0], MessagePart::Text { text }
            if text == "question")
        );
        assert_eq!(history, before);
    }

    #[test]
    fn legacy_tool_error_and_artifact_survive_compaction_codec() {
        let original = json!({
            "role":"tool", "tool_call_id":"call", "content":"failed", "is_error":true,
            "artifacts":[{"name":"file", "mimeType":"text/plain", "dataUrl":"",
                "sizeBytes":4, "path":"saved.txt"}],
        });
        let history = import_history(vec![original]);
        let saved = export_history(&history);
        assert_eq!(saved[0]["is_error"], true);
        assert_eq!(saved[0]["artifacts"][0]["path"], "saved.txt");
        assert_eq!(history, import_history(saved));
    }
}
