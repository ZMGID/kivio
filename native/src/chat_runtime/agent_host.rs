//! UI-neutral event bridge for the shared agent loop. Replies belong to one run.
use super::ChatEvent;
use kivio_model_runtime::{agent::{AgentHost, AgentHostFuture, ToolExecutionContext}, ask_user::{self, AskUserPromptPayload, AskUserResponseResult}, conversation_data::{ChatMessageSegment, ToolCallRecord}, hooks::HookDispatcher};
use tokio::sync::oneshot;

pub enum AgentEvent {
    Segment(ChatMessageSegment),
    Tool(ToolCallRecord),
    Rollback { text_chars: u32, reasoning_chars: u32, segment_ids: Vec<String>, tool_ids: Vec<String> },
    Approval { title: String, detail: String, session: bool, response: oneshot::Sender<bool> },
    Question { prompt: AskUserPromptPayload, response: oneshot::Sender<AskUserResponseResult> },
}
pub struct NativeAgentHost {
    pub events: async_channel::Sender<ChatEvent>,
    pub hooks: Option<HookDispatcher>,
    pub completed: bool,
    pub workflow: Option<kivio_model_runtime::workflow_hooks::Runtime>,
}
impl NativeAgentHost {
    fn emit(&self, event: AgentEvent) { let _ = self.events.try_send(ChatEvent::Agent(event)); }
    async fn approve(&self, title: String, detail: String, session: bool) -> bool {
        let (response, answer) = oneshot::channel();
        self.emit(AgentEvent::Approval { title, detail, session, response });
        answer.await.unwrap_or(false)
    }
}
impl AgentHost for NativeAgentHost {
    fn emit_stream_delta(&self, _: &str, _: &str, _: &str, delta: &str, reasoning: Option<&str>, segment: Option<&ChatMessageSegment>) {
        if !delta.is_empty() { let _ = self.events.try_send(ChatEvent::Delta(delta.into())); }
        if let Some(delta) = reasoning.filter(|s| !s.is_empty()) { let _ = self.events.try_send(ChatEvent::Reasoning(delta.into())); }
        if let Some(segment) = segment { self.emit(AgentEvent::Segment(segment.clone())); }
    }
    fn emit_tool_record(&self, _: &str, _: &str, _: &str, record: &ToolCallRecord) { self.emit(AgentEvent::Tool(record.clone())); }
    fn discard_stream_attempt(&self, _: &str, text_chars: u32, reasoning_chars: u32, segment_ids: Vec<String>, tool_ids: Vec<String>) {
        self.emit(AgentEvent::Rollback { text_chars, reasoning_chars, segment_ids, tool_ids });
    }
    fn request_tool_approval<'a>(&'a self, _: &'a ToolExecutionContext<'a>, record: &'a ToolCallRecord) -> AgentHostFuture<'a, bool> {
        Box::pin(self.approve(record.name.clone(), record.arguments.clone(), false))
    }
    fn request_session_consent<'a>(&'a self, _: &'a ToolExecutionContext<'a>) -> AgentHostFuture<'a, bool> {
        Box::pin(self.approve("允许此对话使用文件与命令工具？".into(), "授权后，此对话可读取和修改本机文件、执行命令；仍遵循当前设置中的工具权限规则。".into(), true))
    }
    fn request_user_response<'a>(&'a self, _: &'a ToolExecutionContext<'a>, _: &'a ToolCallRecord, prompt: AskUserPromptPayload) -> AgentHostFuture<'a, AskUserResponseResult> {
        Box::pin(async move {
            let (response, answer) = oneshot::channel();
            self.emit(AgentEvent::Question { prompt, response });
            answer.await.unwrap_or_else(|_| ask_user::cancelled_response())
        })
    }
    fn is_generation_active(&self, _: &str, _: u64) -> bool { !self.events.is_closed() }
    fn wait_for_generation_inactive<'a>(&'a self, _: &'a str, _: u64) -> AgentHostFuture<'a, ()> {
        Box::pin(self.events.closed())
    }
    fn workflow_hooks(&self) -> Option<&kivio_model_runtime::workflow_hooks::Runtime> { self.workflow.as_ref() }
    fn hooks(&self) -> Option<&HookDispatcher> { self.hooks.as_ref() }
}
impl Drop for NativeAgentHost {
    fn drop(&mut self) { if !self.completed { if let Some(hooks) = &self.hooks { hooks.cancel(); } } }
}
