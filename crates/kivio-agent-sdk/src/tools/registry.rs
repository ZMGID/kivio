use std::sync::Arc;

use serde_json::Value;

use crate::{
    agent::{
        content::{AgentToolCall, AgentToolStatus},
        execute::{PreparedToolCall, ToolPreparation, ToolProgress, ToolResultUpdate},
        host::SpeculativeTaskScope,
        ToolExecutionContext, ToolExecutor, ToolExecutorFuture,
    },
    model::{ModelArtifact, PendingToolCall},
};

use super::{
    call_typed, input_schema, schema::validate_arguments, StaticTool, ToolDefinition, ToolError,
    ToolFuture, ToolTimeout,
};

/// Host policy for static tools. Registration alone never grants execution permission.
pub trait ToolAuthorization: Send + Sync {
    fn authorize<'a>(
        &'a self,
        progress: &'a ToolProgress<'a>,
        ctx: &'a ToolExecutionContext<'a>,
        tool: &'a ToolDefinition,
        arguments: &'a Value,
    ) -> ToolFuture<'a, Result<(), ToolError>>;
}

#[derive(Clone, Default)]
pub struct ToolRegistry {
    entries: Vec<(ToolDefinition, Arc<dyn ToolExecutor>)>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> Self {
        self.clone()
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.entries.iter().map(|(tool, _)| tool.clone()).collect()
    }

    pub fn register_dynamic(
        &mut self,
        definition: ToolDefinition,
        executor: Arc<dyn ToolExecutor>,
    ) -> Result<(), ToolError> {
        let names = identifiers(&definition);
        if names.iter().any(|(name, _)| name.trim().is_empty()) {
            return Err(ToolError::InvalidArguments(
                "Tool identifiers must not be empty".into(),
            ));
        }
        for (registered, _) in &self.entries {
            for (name, alias) in &names {
                if identifiers(registered).iter().any(|(other, other_alias)| {
                    if *alias && *other_alias {
                        name == other
                    } else {
                        name.eq_ignore_ascii_case(other)
                    }
                }) {
                    return Err(ToolError::InvalidArguments(format!(
                        "Tool identifier `{name}` is already registered"
                    )));
                }
            }
        }
        self.entries.push((definition, executor));
        Ok(())
    }

    pub fn register_static<T: StaticTool<HostData = Vec<ModelArtifact>> + 'static>(
        &mut self,
        mut definition: ToolDefinition,
        tool: T,
        authorization: Arc<dyn ToolAuthorization>,
    ) -> Result<(), ToolError> {
        definition.model.input_schema = input_schema::<T>();
        self.register_dynamic(
            definition,
            Arc::new(RegisteredStatic {
                tool,
                authorization,
            }),
        )
    }

    fn entry(&self, key: &str) -> Option<&(ToolDefinition, Arc<dyn ToolExecutor>)> {
        self.entries.iter().find(|(tool, _)| tool.key == key)
    }
}

fn identifiers(tool: &ToolDefinition) -> Vec<(&str, bool)> {
    let mut names = vec![
        (tool.key.as_str(), false),
        (tool.model.name.as_str(), false),
        (tool.label.name.as_str(), false),
    ];
    names.extend(tool.aliases.iter().map(|alias| (alias.as_str(), true)));
    names
}

impl ToolExecutor for ToolRegistry {
    fn before_tool_call<'a>(
        &'a self,
        progress: &'a ToolProgress<'a>,
        ctx: &'a ToolExecutionContext<'a>,
        tool: &'a ToolDefinition,
        call: PendingToolCall,
        record: AgentToolCall,
    ) -> ToolFuture<'a, Result<ToolPreparation, String>> {
        Box::pin(async move {
            match self.entry(&tool.key) {
                Some((definition, executor)) => {
                    executor
                        .before_tool_call(progress, ctx, definition, call, record)
                        .await
                }
                None => Ok(complete(
                    record,
                    AgentToolStatus::Error,
                    format!("Tool `{}` is not registered", tool.key),
                )),
            }
        })
    }

    fn call<'a>(
        &'a self,
        progress: &'a ToolProgress<'a>,
        ctx: &'a ToolExecutionContext<'a>,
        tool: &'a ToolDefinition,
        arguments: Value,
    ) -> ToolExecutorFuture<'a> {
        Box::pin(async move {
            let (definition, executor) = self
                .entry(&tool.key)
                .ok_or_else(|| format!("Tool `{}` is not registered", tool.key))?;
            executor.call(progress, ctx, definition, arguments).await
        })
    }

    fn after_tool_call<'a>(
        &'a self,
        progress: &'a ToolProgress<'a>,
        ctx: &'a ToolExecutionContext<'a>,
        tool: &'a ToolDefinition,
        arguments: Value,
        content: &'a str,
        feedback: Vec<String>,
    ) -> ToolFuture<'a, Result<ToolResultUpdate, String>> {
        Box::pin(async move {
            match self.entry(&tool.key) {
                Some((definition, executor)) => {
                    executor
                        .after_tool_call(progress, ctx, definition, arguments, content, feedback)
                        .await
                }
                None => Ok(ToolResultUpdate::default()),
            }
        })
    }

    fn start_speculation(
        &self,
        ctx: &ToolExecutionContext<'_>,
        tools: &[ToolDefinition],
    ) -> Option<Box<dyn SpeculativeTaskScope>> {
        let mut owner: Option<&Arc<dyn ToolExecutor>> = None;
        for tool in tools.iter().filter(|tool| tool.speculative) {
            let (_, executor) = self.entry(&tool.key)?;
            if owner.is_some_and(|owner| !Arc::ptr_eq(owner, executor)) {
                // shortcut: Multiple speculative owners are disabled until updates include tool names.
                return None;
            }
            owner = Some(executor);
        }
        let owner = owner?;
        let definitions: Vec<_> = tools
            .iter()
            .filter_map(|tool| {
                let (definition, executor) = self.entry(&tool.key)?;
                Arc::ptr_eq(owner, executor).then(|| definition.clone())
            })
            .collect();
        owner.start_speculation(ctx, &definitions)
    }

    fn unknown_feedback(&self, name: &str) -> Option<String> {
        self.entries
            .iter()
            .find_map(|(_, executor)| executor.unknown_feedback(name))
    }

    fn blocked_feedback(&self, tool: &ToolDefinition) -> String {
        match self.entry(&tool.key) {
            Some((definition, executor)) => executor.blocked_feedback(definition),
            None => format!("Tool `{}` is unavailable for this run.", tool.model.name),
        }
    }
}

struct RegisteredStatic<T> {
    tool: T,
    authorization: Arc<dyn ToolAuthorization>,
}

impl<T: StaticTool<HostData = Vec<ModelArtifact>>> ToolExecutor for RegisteredStatic<T> {
    fn before_tool_call<'a>(
        &'a self,
        progress: &'a ToolProgress<'a>,
        ctx: &'a ToolExecutionContext<'a>,
        tool: &'a ToolDefinition,
        call: PendingToolCall,
        mut record: AgentToolCall,
    ) -> ToolFuture<'a, Result<ToolPreparation, String>> {
        Box::pin(async move {
            if let Some(error) = call.arguments_parse_error {
                return Ok(complete(record, AgentToolStatus::Error, error));
            }
            if tool
                .argument_limit
                .is_some_and(|limit| call.arguments_raw.chars().count() > limit)
            {
                return Ok(complete(
                    record,
                    AgentToolStatus::Error,
                    tool.argument_limit_error
                        .clone()
                        .unwrap_or_else(|| format!("{} arguments are too large", tool.model.name)),
                ));
            }
            if let Err(error) = validate_arguments(&tool.model.input_schema, &call.arguments) {
                return Ok(complete(record, AgentToolStatus::Error, error));
            }
            if let Err(error) = self
                .authorization
                .authorize(progress, ctx, tool, &call.arguments)
                .await
            {
                return Ok(complete(
                    record,
                    AgentToolStatus::Skipped,
                    error.to_string(),
                ));
            }
            record.arguments = call.arguments.to_string();
            Ok(ToolPreparation::Ready(PreparedToolCall {
                record,
                timeout_ms: ToolTimeout::Default.milliseconds(60_000, &call.arguments),
                timeout_message: format!("Tool `{}` timed out after 60 seconds", tool.model.name),
                arguments: call.arguments,
                output_limit: None,
                drain_on_cancel: false,
                feedback: Vec::new(),
            }))
        })
    }

    fn call<'a>(
        &'a self,
        _progress: &'a ToolProgress<'a>,
        _ctx: &'a ToolExecutionContext<'a>,
        _tool: &'a ToolDefinition,
        arguments: Value,
    ) -> ToolExecutorFuture<'a> {
        Box::pin(async move {
            call_typed(&self.tool, arguments)
                .await
                .map_err(|error| error.to_string())
        })
    }
}

fn complete(mut record: AgentToolCall, status: AgentToolStatus, error: String) -> ToolPreparation {
    record.status = status;
    record.duration_ms = Some(0);
    record.completed_at = Some(chrono::Local::now().timestamp());
    record.error = Some(error.clone());
    ToolPreparation::Complete(record, error, Vec::new())
}
