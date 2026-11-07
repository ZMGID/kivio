use crate::model::ModelTool;

#[derive(Debug, Clone, Default)]
pub struct ToolLabel {
    pub name: String,
    pub source: String,
    pub server_id: Option<String>,
    pub sensitive: bool,
}

#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub key: String,
    pub model: ModelTool,
    pub label: ToolLabel,
    pub aliases: Vec<String>,
    pub parallel_safe: bool,
    pub checkpoint_before_dispatch: bool,
    pub speculative: bool,
    pub argument_limit: Option<usize>,
    pub argument_limit_error: Option<String>,
}

impl ToolDefinition {
    pub fn new(model: ModelTool) -> Self {
        Self {
            key: model.name.clone(),
            label: ToolLabel {
                name: model.name.clone(),
                ..Default::default()
            },
            model,
            aliases: Vec::new(),
            parallel_safe: false,
            checkpoint_before_dispatch: false,
            speculative: false,
            argument_limit: None,
            argument_limit_error: None,
        }
    }
}
