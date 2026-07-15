use serde_json::Value;

use crate::chat::model::ModelUsage;
use crate::external_agents::types::{JsonEventParser, StreamFormat, UnifiedAgentEvent};

pub mod claude;
pub mod json_events;

pub fn create_stream_handler(
    format: StreamFormat,
    parser: Option<JsonEventParser>,
) -> StreamHandler {
    match format {
        StreamFormat::ClaudeStreamJson => {
            StreamHandler::Claude(claude::ClaudeStreamState::default())
        }
        StreamFormat::JsonEventStream => StreamHandler::Json(
            json_events::JsonEventStreamState::new(parser.unwrap_or(JsonEventParser::Kimi)),
        ),
        // PiRpc / AcpJsonRpc / CodexAppServer are driven by dedicated session runners in run.rs and
        // never reach this factory.
        StreamFormat::PiRpc | StreamFormat::AcpJsonRpc | StreamFormat::CodexAppServer => {
            unreachable!("{format:?} uses a dedicated session runner, not create_stream_handler")
        }
    }
}

pub enum StreamHandler {
    Claude(claude::ClaudeStreamState),
    Json(json_events::JsonEventStreamState),
}

impl StreamHandler {
    pub fn handle_line(&mut self, line: &str, sink: &mut dyn FnMut(UnifiedAgentEvent)) {
        let value = match serde_json::from_str::<Value>(line.trim()) {
            Ok(v) => v,
            Err(_) => {
                // Not JSON — surface it as a raw line rather than dropping it, so a CLI that
                // prints a plain-text error/notice doesn't leave the run looking empty.
                sink(UnifiedAgentEvent::Raw {
                    line: line.to_string(),
                });
                return;
            }
        };
        match self {
            StreamHandler::Claude(state) => state.handle_value(&value, sink),
            StreamHandler::Json(state) => state.handle_value(&value, sink),
        }
    }
}

pub fn usage_from_numbers(input: u64, output: u64) -> ModelUsage {
    ModelUsage {
        input_tokens: Some(input),
        output_tokens: Some(output),
        total_tokens: Some(input.saturating_add(output)),
        ..Default::default()
    }
}
