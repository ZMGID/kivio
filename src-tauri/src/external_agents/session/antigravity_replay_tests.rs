use super::*;
use crate::external_agents::replay_tests::{fixture, Step};

#[test]
fn offline_replay_antigravity_steps_emit_one_tool_result_text_and_usage() {
    let mut stream = TurnStream::default();
    let mut previous = Some(Usage::default());
    let mut events = Vec::new();
    for step in fixture("antigravity-steps") {
        let Step::Emit { frame } = step else {
            panic!("inbound-only parser record")
        };
        if frame.get("status").is_some() {
            stream
                .finish(&frame, &mut previous, &mut |event| events.push(event))
                .unwrap();
        } else {
            stream.handle(&frame, &mut |event| events.push(event));
        }
    }
    assert_eq!(events.iter().filter(|event| matches!(event, UnifiedAgentEvent::ToolUse { name, .. } if name == "view_file")).count(), 1);
    assert_eq!(events.iter().filter(|event| matches!(event, UnifiedAgentEvent::ToolResult { content, is_error: false, .. } if content == "probe")).count(), 1);
    assert_eq!(
        events
            .iter()
            .filter(
                |event| matches!(event, UnifiedAgentEvent::TextDelta { delta } if delta == "answer")
            )
            .count(),
        1
    );
    assert!(events.iter().any(|event| matches!(event, UnifiedAgentEvent::Usage { usage } if usage.total_tokens == Some(15) && usage.context_tokens == Some(15))));
}
