use super::{json, HashMap, UnifiedAgentEvent, Value};

/// Hermes sends the clarify arguments in a tool-start content block, then omits its tool
/// ID from elicitation/create. Only a unique pending call with the same form can own it.
#[derive(Default)]
pub(super) struct PendingClarifications(HashMap<String, Clarification>);

struct Clarification {
    questions: Vec<Value>,
    claimed: bool,
}

impl PendingClarifications {
    pub fn observe(&mut self, event: &UnifiedAgentEvent) {
        match event {
            UnifiedAgentEvent::ToolUse { id, name, input }
                if (name == "clarify" || name.starts_with("clarify: "))
                    && input["sessionUpdate"] == "tool_call" =>
            {
                let args = input
                    .get("rawInput")
                    .filter(|args| args.is_object())
                    .cloned()
                    .or_else(|| {
                        serde_json::from_str(input.pointer("/content/0/content/text")?.as_str()?)
                            .ok()
                    });
                if let Some(questions) = args.as_ref().and_then(|args| args["questions"].as_array())
                {
                    self.0.entry(id.clone()).or_insert_with(|| Clarification {
                        questions: questions.clone(),
                        claimed: false,
                    });
                }
            }
            UnifiedAgentEvent::ToolResult { tool_use_id, .. } => {
                self.0.remove(tool_use_id);
            }
            _ => {}
        }
    }

    pub fn bind(&mut self, params: &mut Value) {
        if params["mode"] != "form"
            || ["toolCallId", "elicitationId", "elicitation_id"]
                .iter()
                .any(|key| params.get(*key).is_some())
        {
            return;
        }
        let mut matches = self.0.iter_mut().filter(|(_, call)| {
            call.questions
                .iter()
                .any(|question| matches_form(question, params))
        });
        let Some((id, call)) = matches.next() else {
            return;
        };
        if !call.claimed && matches.next().is_none() {
            call.claimed = true;
            params["toolCallId"] = Value::String(id.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start(id: &str, choices: &[&str]) -> UnifiedAgentEvent {
        let args = json!({"questions":[{"question":"Pick a color","choices":choices}]});
        UnifiedAgentEvent::ToolUse {
            id: id.into(),
            name: "clarify: Pick a color".into(),
            input: json!({"sessionUpdate":"tool_call", "kind":"other",
            "content":[{"type":"content","content":{"type":"text","text":args.to_string()}}]}),
        }
    }

    fn color_form() -> Value {
        json!({"sessionId":"one","mode":"form","message":"Pick a color","requestedSchema":{
        "type":"object","required":["answer"],"properties":{"answer":{
            "type":"string","enum":["blue (Recommended)","green"]
        }}}})
    }

    #[test]
    fn hermes_question_uses_unique_open_clarify_call_and_keeps_explicit_ids() {
        let mut pending = PendingClarifications::default();
        pending.observe(&start("wire-color-test", &["blue", "green"]));
        pending.observe(&start("different-options", &["green", "orange"]));
        let mut unrelated = color_form();
        unrelated["message"] = json!("Different question");
        pending.bind(&mut unrelated);
        assert!(unrelated.get("toolCallId").is_none());
        let mut explicit = color_form();
        explicit["toolCallId"] = json!("given");
        pending.bind(&mut explicit);
        assert_eq!(explicit["toolCallId"], "given");
        let mut form = color_form();
        pending.bind(&mut form);
        assert_eq!(form["toolCallId"], "wire-color-test");
        let mut repeated = color_form();
        pending.bind(&mut repeated);
        assert!(repeated.get("toolCallId").is_none());
    }

    #[test]
    fn hermes_question_does_not_guess_between_identical_parallel_calls_or_use_closed_calls() {
        let mut pending = PendingClarifications::default();
        for id in ["one", "two"] {
            pending.observe(&start(id, &["blue", "green"]));
        }
        let mut form = color_form();
        pending.bind(&mut form);
        assert!(form.get("toolCallId").is_none());
        for id in ["one", "two"] {
            pending.observe(&UnifiedAgentEvent::ToolResult {
                tool_use_id: id.into(),
                content: String::new(),
                is_error: false,
            });
        }
        pending.bind(&mut form);
        assert!(form.get("toolCallId").is_none());
    }
}

fn matches_form(question: &Value, params: &Value) -> bool {
    let Some(text) = question["question"].as_str() else {
        return false;
    };
    if params["message"].as_str() != Some(text.trim()) {
        return false;
    }
    let mut field = json!({"type":"string"});
    if let Some(choices) = question["choices"]
        .as_array()
        .filter(|choices| !choices.is_empty())
    {
        let Some(mut choices) = choices
            .iter()
            .map(|c| Some(c.as_str()?.trim().to_owned()))
            .collect::<Option<Vec<_>>>()
        else {
            return false;
        };
        // Hermes marks the first of several choices before publishing the form.
        if choices.len() > 1 && !choices[0].to_ascii_lowercase().ends_with("(recommended)") {
            choices[0].push_str(" (Recommended)");
        }
        field["enum"] = json!(choices);
        if question["multi_select"] == true {
            field = json!({"type":"array","items":field});
        }
    }
    params["requestedSchema"]
        == json!({"type":"object","properties":{"answer":field},"required":["answer"]})
}
