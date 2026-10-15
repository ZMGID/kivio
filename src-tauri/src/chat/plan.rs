use crate::chat::types::{AgentPlanMode, AgentPlanState, AgentPlanStatus};

pub fn mode_from_str(value: &str) -> Result<AgentPlanMode, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "act" => Ok(AgentPlanMode::Act),
        "plan" => Ok(AgentPlanMode::Plan),
        "orchestrate" => Ok(AgentPlanMode::Orchestrate),
        other => Err(format!("Unknown agent plan mode: {other}")),
    }
}

pub fn is_plan_mode(state: &AgentPlanState) -> bool {
    state.mode == AgentPlanMode::Plan
}

pub fn is_orchestrate_mode(state: &AgentPlanState) -> bool {
    state.mode == AgentPlanMode::Orchestrate
}

/// Built-in Chat system text shown in Settings as the empty-state preview.
/// Runtime injects none of this: Chat has no identity/contract essay — date,
/// optional extra instructions, and conversation context only. Tools are the
/// request's tool list, not prompt prohibitions.
pub fn chat_runtime_prompt() -> String {
    String::new()
}

/// Same text for all languages — settings preview and runtime share one source.
pub fn chat_runtime_prompt_for_lang(_language: &str) -> String {
    chat_runtime_prompt()
}

pub fn with_mode(current: &AgentPlanState, mode: AgentPlanMode) -> AgentPlanState {
    let mut next = current.clone();
    if next.mode != mode {
        next.mode = mode;
        next.updated_at = chrono::Local::now().timestamp();
    }
    next
}

pub fn format_prompt(state: &AgentPlanState) -> String {
    if state.mode == AgentPlanMode::Act
        && state.document.is_none()
        && current_plan_text(state).is_none()
    {
        return "Current mode: act.".into();
    }
    let status = status_name(&state.status);
    let current_plan = if let Some(document) = &state.document {
        let text = if state.mode == AgentPlanMode::Plan {
            std::fs::read_to_string(&document.path).unwrap_or_else(|_| {
                "Plan file unavailable. Locate or restore it before using it.".into()
            })
        } else {
            state.plan.clone().unwrap_or_else(|| {
                "Read the file when the user asks to use or revise this plan.".into()
            })
        };
        format!(
            "{} (id: {}, path: {})\n{}",
            document.title, document.id, document.path, text
        )
    } else {
        current_plan_text(state)
            .unwrap_or("No current saved plan.")
            .to_string()
    };

    if state.mode == AgentPlanMode::Plan {
        format!(
            "Plan mode: investigate the relevant code and constraints; look up external facts or ask about important choices when needed. Save the proposed plan as a Markdown document with save_plan, then briefly explain the key decisions and link the document. Update the same document when the user revises the plan. Only investigate and edit the plan document in this mode; implementation belongs in Act or Orchestrate when the user asks to begin.\n\nCurrent plan context:\n{current_plan}"
        )
    } else if state.mode == AgentPlanMode::Orchestrate {
        format!(
            "Agent orchestrate mode: proactively delegate useful independent work to sub-agents in parallel while you advance the main task yourself. Keep dispatches concise: goal, necessary context, and scope; avoid duplicate investigation or concurrent edits to the same files. Steer agents as work develops, reuse the relevant agent for follow-ups, and read or wait for the specific results you need. Use their actual outputs as information, distinguish sources and uncertainties, and decide the next steps and final answer yourself. Handle simple or tightly coupled work directly. If a branch encounters a runtime issue, adjust that branch and keep independent work moving. Follow the user's latest request; use the saved plan as context when relevant.\n\nCurrent saved plan:\n{current_plan}"
        )
    } else {
        format!(
            "Current mode: act. Saved plan status: {status}. Use the saved plan as context when relevant, following the user's latest request.\n\nCurrent saved plan:\n{current_plan}"
        )
    }
}

pub fn current_plan_text(state: &AgentPlanState) -> Option<&str> {
    state
        .plan
        .as_deref()
        .map(str::trim)
        .filter(|plan| !plan.is_empty())
}

fn status_name(status: &AgentPlanStatus) -> &'static str {
    match status {
        AgentPlanStatus::Empty => "empty",
        AgentPlanStatus::Draft => "draft",
        AgentPlanStatus::Approved => "approved",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_state_defaults_to_act_empty() {
        let state: AgentPlanState = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(state.mode, AgentPlanMode::Act);
        assert_eq!(state.status, AgentPlanStatus::Empty);
        assert_eq!(state.plan, None);
    }

    #[test]
    fn mode_from_str_accepts_orchestrate() {
        assert_eq!(
            mode_from_str("orchestrate").unwrap(),
            AgentPlanMode::Orchestrate
        );
        assert_eq!(
            mode_from_str("Orchestrate").unwrap(),
            AgentPlanMode::Orchestrate
        );
        assert_eq!(mode_from_str("act").unwrap(), AgentPlanMode::Act);
        assert_eq!(mode_from_str("plan").unwrap(), AgentPlanMode::Plan);
        assert!(mode_from_str("bogus").is_err());
    }

    #[test]
    fn is_orchestrate_mode_detects_mode() {
        let mut state = AgentPlanState::default();
        assert!(!is_orchestrate_mode(&state));
        state.mode = AgentPlanMode::Orchestrate;
        assert!(is_orchestrate_mode(&state));
        assert!(!is_plan_mode(&state));
    }

    #[test]
    fn orchestrate_keeps_saved_plan_as_context_without_plan_mode_restrictions() {
        let draft = AgentPlanState {
            mode: AgentPlanMode::Plan,
            plan: Some("An existing saved plan".into()),
            ..Default::default()
        };
        let state = with_mode(&draft, AgentPlanMode::Orchestrate);
        assert_eq!(state.plan, draft.plan);
        let prompt = format_prompt(&state);
        assert!(prompt.contains(current_plan_text(&draft).unwrap()));
        assert!(prompt.contains("advance the main task yourself"));
        assert!(prompt.contains("reuse the relevant agent"));
        assert!(!prompt.contains("Plan mode is read-only"));
        assert!(!prompt.contains("Required flow"));
        assert!(!prompt.contains("todo_write"));
        assert!(!prompt.contains("must be delegated"));
        assert_eq!(with_mode(&state, AgentPlanMode::Act).plan, draft.plan);
    }

    #[test]
    fn act_plan_context_only_includes_saved_plan_when_present() {
        for plan in [None, Some("   ".into())] {
            let state = AgentPlanState {
                plan,
                ..Default::default()
            };
            assert_eq!(format_prompt(&state), "Current mode: act.");
        }
        let state = AgentPlanState {
            plan: Some("Inspect markdown, then fix rendering.".into()),
            status: AgentPlanStatus::Approved,
            ..Default::default()
        };
        let prompt = format_prompt(&state);
        assert!(prompt.contains("approved"));
        assert!(prompt.contains(state.plan.as_deref().unwrap()));
        assert!(!prompt.contains("reminders"));
    }

    #[test]
    fn chat_runtime_prompt_is_empty() {
        let en = chat_runtime_prompt();
        assert!(en.is_empty(), "{en}");
        assert!(!en.contains("internal runtime mode"));
        assert!(!en.contains("Act / Plan / Orchestrate"));
        assert_eq!(chat_runtime_prompt_for_lang("zh"), en);
    }
}
