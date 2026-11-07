use std::{fmt, future::Future, pin::Pin};

use schemars::JsonSchema;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;

use super::model::ModelMessage;

pub mod definition;
pub mod registry;
pub mod schema;
pub use definition::{ToolDefinition, ToolLabel};
pub use registry::{ToolAuthorization, ToolRegistry};
#[cfg(test)]
mod tests;

pub type ToolFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, Default)]
pub struct ToolOutput<H = (), S = Value> {
    pub content: String,
    pub structured_content: Option<S>,
    pub host_data: H,
    pub follow_up_messages: Vec<ModelMessage>,
    pub is_error: bool,
}

impl<H: Default, S> ToolOutput<H, S> {
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            structured_content: None,
            host_data: H::default(),
            follow_up_messages: Vec::new(),
            is_error: false,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ToolError {
    InvalidArguments(String),
    Execution(String),
    OutputEncoding(String),
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidArguments(message) => write!(f, "Invalid tool arguments: {message}"),
            Self::Execution(message) => f.write_str(message),
            Self::OutputEncoding(message) => write!(f, "Serialize tool output failed: {message}"),
        }
    }
}

impl std::error::Error for ToolError {}

pub trait StaticTool: Send + Sync {
    type Args: DeserializeOwned + JsonSchema + Send;
    type Output: Serialize + Send;
    type Error: fmt::Display + Send;
    type HostData: Send;

    fn execute<'a>(
        &'a self,
        args: Self::Args,
    ) -> ToolFuture<'a, Result<ToolOutput<Self::HostData, Self::Output>, Self::Error>>;
}

pub fn input_schema<T: StaticTool>() -> Value {
    schemars::schema_for!(T::Args).to_value()
}

pub async fn call_typed<T: StaticTool>(
    tool: &T,
    arguments: Value,
) -> Result<ToolOutput<T::HostData>, ToolError> {
    let args = serde_json::from_value(arguments)
        .map_err(|error| ToolError::InvalidArguments(error.to_string()))?;
    let output = tool
        .execute(args)
        .await
        .map_err(|error| ToolError::Execution(error.to_string()))?;
    Ok(ToolOutput {
        content: output.content,
        structured_content: output
            .structured_content
            .map(serde_json::to_value)
            .transpose()
            .map_err(|error| ToolError::OutputEncoding(error.to_string()))?,
        host_data: output.host_data,
        follow_up_messages: output.follow_up_messages,
        is_error: output.is_error,
    })
}

#[derive(Clone, Copy)]
pub enum ToolTimeout {
    Default,
    Managed,
    AtLeast(u64),
    Arguments(fn(u64, &Value) -> u64),
}

impl ToolTimeout {
    pub fn milliseconds(self, default_ms: u64, arguments: &Value) -> Option<u64> {
        match self {
            Self::Default => Some(default_ms),
            Self::Managed => None,
            Self::AtLeast(minimum) => Some(default_ms.max(minimum)),
            Self::Arguments(resolve) => Some(resolve(default_ms, arguments)),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ArgumentPolicy {
    Schema,
    ToolAfterApproval,
}
