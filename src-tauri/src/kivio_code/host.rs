//! `CliAgentHost` — an `AgentHost` implementation for the headless `kivio-code`
//! terminal agent in **print mode** (`-p`).
//!
//! Modeled on `chat::sub_agent::SubAgentHost`, but instead of forwarding stream
//! deltas to a Tauri event it writes the assistant answer to **stdout** (the
//! load-bearing output) and tool-call status lines to **stderr** (so stdout
//! stays clean for piping). Print mode is non-interactive: there is no user to
//! prompt, so approval/consent default to a documented policy and `ask_user`
//! resolves as cancelled.

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::chat::agent::execute::ToolExecutionContext;
use crate::chat::agent::host::{AgentHost, AgentHostFuture};
use crate::chat::ask_user::{AskUserPromptPayload, AskUserResponseResult};
use crate::chat::types::{ChatMessageSegment, ToolCallRecord, ToolCallStatus};

/// Print-mode host. Owns a single generation flag (no parent cascade) used as
/// the loop's cancel token. The flag starts active and is only flipped by
/// `cancel()` (wired to Ctrl-C by the bin if desired).
pub struct CliAgentHost {
    /// Stream reasoning deltas to stderr when true (`--verbose`).
    verbose: bool,
    /// When false, sensitive tools (write/edit/bash) are denied approval and the
    /// session-consent gate is refused, leaving only read-only tools usable.
    approve_sensitive: bool,
    /// Active generation; the loop polls `is_generation_active`.
    active: AtomicBool,
    /// Tracks whether we've emitted any assistant text, so `emit_stream_done`
    /// knows whether a trailing newline is warranted.
    wrote_any: AtomicBool,
    generation: AtomicU64,
}

impl CliAgentHost {
    pub fn new(verbose: bool, approve_sensitive: bool) -> Self {
        Self {
            verbose,
            approve_sensitive,
            active: AtomicBool::new(true),
            wrote_any: AtomicBool::new(false),
            generation: AtomicU64::new(1),
        }
    }

    /// The generation this host considers live (always 1 for a single run).
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Cancel the in-flight run (e.g. on SIGINT). The loop stops at its next
    /// generation check.
    pub fn cancel(&self) {
        self.active.store(false, Ordering::SeqCst);
    }

    fn status_symbol(status: &ToolCallStatus) -> &'static str {
        match status {
            ToolCallStatus::Pending => "·",
            ToolCallStatus::Running => "▶",
            ToolCallStatus::Success => "✓",
            ToolCallStatus::Error => "✗",
            ToolCallStatus::Skipped => "⊘",
            ToolCallStatus::Cancelled => "⊗",
        }
    }
}

impl AgentHost for CliAgentHost {
    fn emit_stream_delta(
        &self,
        _conversation_id: &str,
        _run_id: &str,
        _message_id: &str,
        delta: &str,
        reasoning_delta: Option<&str>,
        _segment: Option<&ChatMessageSegment>,
    ) {
        if !delta.is_empty() {
            self.wrote_any.store(true, Ordering::SeqCst);
            let stdout = std::io::stdout();
            let mut lock = stdout.lock();
            let _ = lock.write_all(delta.as_bytes());
            let _ = lock.flush();
        }
        if self.verbose {
            if let Some(reasoning) = reasoning_delta {
                if !reasoning.is_empty() {
                    let stderr = std::io::stderr();
                    let mut lock = stderr.lock();
                    let _ = lock.write_all(reasoning.as_bytes());
                    let _ = lock.flush();
                }
            }
        }
    }

    fn emit_stream_done(
        &self,
        _conversation_id: &str,
        _run_id: &str,
        _message_id: &str,
        _reason: &str,
        _full: &str,
    ) {
        // Ensure stdout ends with a single trailing newline if we wrote anything.
        if self.wrote_any.swap(false, Ordering::SeqCst) {
            let stdout = std::io::stdout();
            let mut lock = stdout.lock();
            let _ = lock.write_all(b"\n");
            let _ = lock.flush();
        }
    }

    fn emit_tool_record(
        &self,
        _conversation_id: &str,
        _run_id: &str,
        _message_id: &str,
        record: &ToolCallRecord,
    ) {
        // Only print on terminal states to avoid spamming Pending→Running→Done
        // for one call; stderr keeps stdout clean for the answer.
        match record.status {
            ToolCallStatus::Pending | ToolCallStatus::Running => return,
            _ => {}
        }
        let symbol = Self::status_symbol(&record.status);
        let detail = record
            .error
            .as_deref()
            .or(record.result_preview.as_deref())
            .map(|text| {
                let clipped: String = text.chars().take(120).collect();
                format!(" — {}", clipped.replace('\n', " "))
            })
            .unwrap_or_default();
        let stderr = std::io::stderr();
        let mut lock = stderr.lock();
        let _ = writeln!(lock, "[tool] {symbol} {}{detail}", record.name);
    }

    fn request_tool_approval<'a>(
        &'a self,
        _ctx: &'a ToolExecutionContext<'a>,
        record: &'a ToolCallRecord,
    ) -> AgentHostFuture<'a, bool> {
        // Print mode cannot prompt. Approve unless the tool is sensitive and the
        // user did not pass the opt-in. `--no-approve` (approve_sensitive=false)
        // restricts the agent to read-only file/search tools.
        let approve = self.approve_sensitive || !record.sensitive;
        Box::pin(async move { approve })
    }

    fn request_session_consent<'a>(
        &'a self,
        _ctx: &'a ToolExecutionContext<'a>,
    ) -> AgentHostFuture<'a, bool> {
        // Running the CLI on one's own machine is the consent. `--no-approve`
        // still grants the session-consent gate (read-only tools need it); the
        // per-call sensitivity check in `request_tool_approval` is what restricts
        // writes under `--no-approve`.
        Box::pin(async move { true })
    }

    fn request_user_response<'a>(
        &'a self,
        _ctx: &'a ToolExecutionContext<'a>,
        _record: &'a ToolCallRecord,
        _prompt: AskUserPromptPayload,
    ) -> AgentHostFuture<'a, AskUserResponseResult> {
        // No interactive prompt in print mode → resolve as cancelled, mirroring
        // SubAgentHost.
        Box::pin(async move {
            AskUserResponseResult {
                phase: "cancelled".to_string(),
                answers: std::collections::HashMap::new(),
            }
        })
    }

    fn is_generation_active(&self, _conversation_id: &str, _generation: u64) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    fn wait_for_generation_inactive<'a>(
        &'a self,
        _conversation_id: &'a str,
        _generation: u64,
    ) -> AgentHostFuture<'a, ()> {
        Box::pin(async move {
            loop {
                if !self.active.load(Ordering::SeqCst) {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ToolExecutionContext<'static> {
        ToolExecutionContext {
            conversation_id: "kivio-code",
            run_id: "run",
            message_id: "msg",
            generation: 1,
            round: 1,
            depth: 0,
            tool_conversation_id: "kivio-code",
            tool_call_id: "call",
        }
    }

    fn record(name: &str, sensitive: bool) -> ToolCallRecord {
        ToolCallRecord {
            id: "call".to_string(),
            name: name.to_string(),
            source: "native".to_string(),
            server_id: None,
            arguments: "{}".to_string(),
            status: ToolCallStatus::Pending,
            result_preview: None,
            error: None,
            duration_ms: None,
            started_at: None,
            completed_at: None,
            round: 1,
            sensitive,
            artifacts: Vec::new(),
            trace_id: None,
            span_id: None,
            structured_content: None,
        }
    }

    #[tokio::test]
    async fn default_host_approves_everything_and_consents() {
        let host = CliAgentHost::new(false, true);
        let c = ctx();
        assert!(host.request_session_consent(&c).await);
        assert!(host.request_tool_approval(&c, &record("write", true)).await);
        assert!(host.request_tool_approval(&c, &record("read", false)).await);
    }

    #[tokio::test]
    async fn no_approve_denies_sensitive_but_keeps_readonly() {
        let host = CliAgentHost::new(false, false);
        let c = ctx();
        // Session consent still granted (read-only tools are consent-gated).
        assert!(host.request_session_consent(&c).await);
        // Sensitive write denied; read-only allowed.
        assert!(!host.request_tool_approval(&c, &record("write", true)).await);
        assert!(host.request_tool_approval(&c, &record("read", false)).await);
    }

    #[tokio::test]
    async fn ask_user_resolves_cancelled() {
        let host = CliAgentHost::new(false, true);
        let c = ctx();
        let response = host
            .request_user_response(
                &c,
                &record("ask_user", false),
                AskUserPromptPayload {
                    title: None,
                    questions: Vec::new(),
                },
            )
            .await;
        assert_eq!(response.phase, "cancelled");
    }

    #[test]
    fn cancel_flips_generation_active() {
        let host = CliAgentHost::new(false, true);
        assert!(host.is_generation_active("kivio-code", host.generation()));
        host.cancel();
        assert!(!host.is_generation_active("kivio-code", host.generation()));
    }
}
