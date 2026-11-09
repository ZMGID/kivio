use kivio_agent_sdk::agent::{
    InjectionAdmission, InjectionBatch, InjectionBoundary, InjectionContext, InjectionError,
    InjectionFuture, InjectionSource, SessionEntryKind,
};
use sha2::{Digest, Sha256};

pub(crate) struct ContextNotesReminder {
    threshold: usize,
    path: String,
}

impl ContextNotesReminder {
    pub(crate) fn new(
        config: &super::session_policy::ApplicationRunConfig<'_>,
        host: &dyn super::tool_runtime::ApplicationHost,
    ) -> Self {
        let window =
            super::model_metadata::context_window_for_model(Some(&config.provider), &config.model)
                .0;
        let window = if window == 0 { 200_000 } else { window };
        let threshold = host
            .compaction_threshold(window)
            .unwrap_or_else(|| kivio_agent_sdk::context::auto_compact_budget(window))
            .min(kivio_agent_sdk::context::request_context_budget(
                window,
                config.max_output_tokens,
            ));
        let owner = format!("{:x}", Sha256::digest(config.conversation_id.as_bytes()));
        let branch = config.history.active_leaf().map_or(0, |entry| entry.0);
        Self {
            threshold,
            path: format!(".kivio/context-notes/{owner}-{branch}.md"),
        }
    }

    fn reminder(
        &self,
        context: InjectionContext<'_>,
    ) -> Result<Option<kivio_agent_sdk::agent::host::SessionInput>, InjectionError> {
        if context.boundary != InjectionBoundary::BeforeStep {
            return Ok(None);
        }
        let exposed = |name: &str| {
            context
                .run
                .tools
                .iter()
                .any(|tool| tool.label.source == "native" && tool.label.name == name)
        };
        if !exposed("read") || !exposed("write") {
            return Ok(None);
        }
        let used = context
            .run
            .history
            .iter()
            .map(kivio_agent_sdk::context::estimate_runtime_message_tokens)
            .sum::<usize>()
            .saturating_add(
                context
                    .run
                    .tools
                    .iter()
                    .map(|tool| {
                        super::agent_preparation::estimate_value_tokens(
                            &tool.model.to_openai_tool(),
                        )
                    })
                    .sum(),
            );
        if used < self.threshold.saturating_mul(85) / 100 || used >= self.threshold {
            return Ok(None);
        }
        let boundary = context
            .history
            .active_path()
            .into_iter()
            .rev()
            .find(|entry| {
                matches!(
                    entry.kind,
                    SessionEntryKind::Compaction { .. } | SessionEntryKind::ContextReset { .. }
                )
            })
            .map_or(0, |entry| entry.id.0);
        let marker = format!("<context-notes-reminder window=\"{boundary}\">");
        if context
            .run
            .history
            .iter()
            .any(|message| message.text_content().contains(&marker))
        {
            return Ok(None);
        }
        let input = super::runtime_injections::notice(
            &marker,
            &serde_json::json!({
                "instruction":"Context is nearing compaction. If work remains, use write to save a short continuation note containing decisions, completed work and next steps. Use read to recover it after compaction. Do not copy secrets or large tool output. Skip this if the task is complete.",
                "path":self.path,
            }),
        )?;
        Ok(Some(input))
    }
}

impl InjectionSource for ContextNotesReminder {
    fn collect<'a>(
        &'a mut self,
        context: InjectionContext<'a>,
        admission: &'a mut InjectionAdmission,
    ) -> InjectionFuture<'a, Result<(), InjectionError>> {
        Box::pin(async move {
            if let Some(input) = self.reminder(context)? {
                admission.pending(InjectionBatch::new(vec![input]));
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kivio_agent_sdk::{
        agent::{AgentSession, AgentSessionConfig, InjectionSources},
        model::ModelTool,
        tools::{StaticTool, ToolAuthorization, ToolDefinition, ToolError, ToolFuture, ToolOutput},
    };
    use std::sync::Arc;
    struct FileTool;
    impl StaticTool for FileTool {
        type Args = serde_json::Value;
        type Output = String;
        type Error = &'static str;
        type HostData = Vec<kivio_agent_sdk::model::ModelArtifact>;
        fn execute<'a>(
            &'a self,
            _: Self::Args,
        ) -> ToolFuture<'a, Result<ToolOutput<Self::HostData, String>, Self::Error>> {
            Box::pin(async { panic!("the reminder must not perform file writes itself") })
        }
    }
    struct Allow;
    impl ToolAuthorization for Allow {
        fn authorize<'a>(
            &'a self,
            _: &'a kivio_agent_sdk::agent::execute::ToolProgress,
            _: &'a kivio_agent_sdk::agent::ToolExecutionContext,
            _: &'a ToolDefinition,
            _: &'a serde_json::Value,
        ) -> ToolFuture<'a, Result<(), ToolError>> {
            Box::pin(async { Ok(()) })
        }
    }

    #[tokio::test]
    async fn context_pressure_reminds_once_through_the_actual_model_request() {
        let prompt = "continuation detail ".repeat(300);
        let mut tools = Vec::new();
        for name in ["read", "write"] {
            let mut tool = ToolDefinition::new(ModelTool {
                name: name.into(),
                description: "File access".into(),
                input_schema: serde_json::json!({"type":"object"}),
            });
            tool.label.source = "native".into();
            tools.push(tool);
        }
        let used = kivio_agent_sdk::context::estimate_runtime_message_tokens(
            &kivio_agent_sdk::agent::RuntimeMessage::text(
                kivio_agent_sdk::model::ModelRole::User,
                &prompt,
            ),
        ) + tools
            .iter()
            .map(|tool| {
                crate::chat::agent_preparation::estimate_value_tokens(&tool.model.to_openai_tool())
            })
            .sum::<usize>();
        let model = Arc::new(crate::chat::runtime_injections::tests::CapturingModel::default());
        let mut config = AgentSessionConfig::new(model.clone(), "offline").with_injections(
            InjectionSources::new().with(ContextNotesReminder {
                threshold: used * 110 / 100,
                path: ".kivio/context-notes/test-owner-branch.md".into(),
            }),
        );
        for tool in tools {
            config
                .tools
                .register_static(tool, FileTool, Arc::new(Allow))
                .unwrap();
        }
        let session = AgentSession::new(config);
        session.prompt(prompt.as_str()).await.unwrap();
        session.prompt("continue").await.unwrap();
        let calls = model.0.lock().unwrap();
        assert!(calls[0].contains("test-owner-branch.md"));
        assert_eq!(calls[1].matches("<context-notes-reminder").count(), 1);
    }
}
