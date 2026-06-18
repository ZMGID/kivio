use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::chat::model::ModelUsage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamFormat {
    ClaudeStreamJson,
    JsonEventStream,
    PiRpc,
    AcpJsonRpc,
    CodexAppServer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JsonEventParser {
    Codex,
    CursorAgent,
    OpenCode,
    Gemini,
    Kimi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalMcpInjection {
    ClaudeMcpJson,
    OpenCodeEnvContent,
    AcpMerge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptInputFormat {
    Text,
    StreamJson,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelProbeStrategy {
    Acp,
    ClaudeInit,
}

/// How a CLI's `/commands` are discovered for the slash popover. We only advertise commands
/// that the CLI genuinely honors in our (headless) invocation, so most one-shot CLIs are
/// `None` rather than carrying a fabricated list the CLI would ignore.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashStrategy {
    /// Probe the Claude `system/init` event — yields built-ins + the user's custom commands
    /// and skills, exactly as the `claude` CLI resolves them for this cwd.
    ClaudeInit,
    /// Discover commands natively over ACP: run `initialize` → `session/new`, then read
    /// `session/update` notifications for the `available_commands_update` payload the agent
    /// pushes. Works for any ACP-speaking CLI (cursor / gemini / opencode).
    Acp,
    /// No discoverable slash commands for this CLI in headless mode.
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeModelOption {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedAgent {
    pub id: String,
    pub name: String,
    pub available: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub models: Vec<RuntimeModelOption>,
    pub reasoning_options: Vec<RuntimeModelOption>,
    #[serde(default)]
    pub sandbox_options: Vec<RuntimeModelOption>,
    pub auth_status: Option<String>,
    pub external_mcp_injection: Option<ExternalMcpInjection>,
}

#[derive(Debug, Clone)]
pub struct RuntimeBuildOptions {
    pub model: Option<String>,
    pub reasoning: Option<String>,
    /// Sandbox/permission level id (native flag value, e.g. claude "bypassPermissions" / codex "workspace-write").
    pub sandbox: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RuntimeContext {
    pub cwd: Option<String>,
    pub extra_allowed_dirs: Vec<String>,
    pub resume_session_id: Option<String>,
    pub new_session_id: Option<String>,
    pub include_partial_messages: bool,
}

#[derive(Debug, Clone)]
pub struct RuntimeAgentDef {
    pub id: &'static str,
    pub name: &'static str,
    pub bin: &'static str,
    pub fallback_bins: &'static [&'static str],
    pub version_args: &'static [&'static str],
    pub auth_probe_args: Option<&'static [&'static str]>,
    pub fallback_models: &'static [(&'static str, &'static str)],
    pub reasoning_options: &'static [(&'static str, &'static str)],
    pub list_models_args: Option<&'static [&'static str]>,
    pub list_models_timeout_secs: Option<u64>,
    pub models_from_stderr: bool,
    pub model_probe: Option<ModelProbeStrategy>,
    pub model_probe_args: Option<&'static [&'static str]>,
    pub slash_strategy: SlashStrategy,
    pub env: &'static [(&'static str, &'static str)],
    pub max_prompt_arg_bytes: Option<usize>,
    pub prompt_via_stdin: bool,
    pub prompt_input_format: PromptInputFormat,
    pub stream_format: StreamFormat,
    pub json_event_parser: Option<JsonEventParser>,
    pub external_mcp_injection: Option<ExternalMcpInjection>,
    pub resumes_session_via_cli: bool,
    pub build_args: fn(&RuntimeContext, &RuntimeBuildOptions, Option<&str>) -> Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalCliSlashCommand {
    pub name: String,
    pub slash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UnifiedAgentEvent {
    Status {
        label: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
    TextDelta {
        delta: String,
    },
    ThinkingDelta {
        delta: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    Usage {
        usage: ModelUsage,
    },
    TurnEnd {
        stop_reason: String,
    },
    Error {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
    Raw {
        line: String,
    },
    SlashCommands {
        commands: Vec<ExternalCliSlashCommand>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgentSession {
    pub conversation_id: String,
    pub agent_id: String,
    pub session_id: String,
    pub stable_prompt_hash: Option<String>,
}

pub fn default_model_option() -> RuntimeModelOption {
    RuntimeModelOption {
        id: "default".to_string(),
        label: "Default".to_string(),
        context_window_tokens: None,
    }
}

pub fn fallback_models_from_pairs(pairs: &[(&str, &str)]) -> Vec<RuntimeModelOption> {
    let mut out = vec![default_model_option()];
    for (id, label) in pairs {
        if *id == "default" {
            continue;
        }
        out.push(RuntimeModelOption {
            id: (*id).to_string(),
            label: (*label).to_string(),
            context_window_tokens: None,
        });
    }
    out
}

pub fn reasoning_options_from_pairs(pairs: &[(&str, &str)]) -> Vec<RuntimeModelOption> {
    pairs
        .iter()
        .map(|(id, label)| RuntimeModelOption {
            id: (*id).to_string(),
            label: (*label).to_string(),
            context_window_tokens: None,
        })
        .collect()
}
