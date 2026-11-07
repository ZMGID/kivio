pub use kivio_agent_sdk::agent::history::{
    export_history, import_history, RuntimeBody, RuntimeMessage, RuntimeSource,
};

pub fn request_from_history(
    history: &[RuntimeMessage],
    model: &str,
    tools: Vec<crate::chat::model::ModelTool>,
    options: crate::chat::model::GenerateOptions,
    metadata: crate::chat::model::RequestMetadata,
) -> crate::chat::model::GenerateRequest {
    let mut system = Vec::new();
    let mut messages = Vec::new();
    for entry in history {
        match &entry.body {
            RuntimeBody::System(text) => system.push(text.clone()),
            RuntimeBody::Model(message) => messages.push(message.clone()),
        }
    }
    crate::chat::model::generate_request_from_model_messages(
        model, system, messages, tools, options, metadata,
    )
}
