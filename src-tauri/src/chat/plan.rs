use crate::chat::types::{AgentPlanMode, AgentPlanState, AgentPlanStatus};

pub fn mode_from_str(value: &str) -> Result<AgentPlanMode, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "act" => Ok(AgentPlanMode::Act),
        "plan" => Ok(AgentPlanMode::Plan),
        other => Err(format!("Unknown agent plan mode: {other}")),
    }
}

pub fn is_plan_mode(state: &AgentPlanState) -> bool {
    state.mode == AgentPlanMode::Plan
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

/// `delegation` is the main agent's rendered `# Delegation` section, present only
/// when this turn exposes `task`; Act mode carries it, Plan mode keeps its own text.
pub fn format_prompt(state: &AgentPlanState, delegation: Option<&str>) -> String {
    let task_available = delegation.is_some();
    let act_delegation = delegation
        .map(|section| format!("\n\n{}", section.trim()))
        .unwrap_or_default();
    if state.mode == AgentPlanMode::Act
        && state.document.is_none()
        && current_plan_text(state).is_none()
    {
        return format!("Current mode: act.{act_delegation}");
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
        let delegation = if task_available {
            " You may use task for read-only planning investigations; children report findings and must not edit files or spawn further agents."
        } else {
            ""
        };
        format!(
            "Plan mode: investigate the relevant code and constraints; look up external facts or ask about important choices when needed.{delegation} Save the proposed plan as a Markdown document with save_plan, then briefly explain the key decisions and link the document; if save_plan is unavailable, write the plan in your reply. Update the same document when the user revises the plan. Only investigate and edit the plan document in this mode; implementation starts after the user runs the plan or turns planning off with /plan.\n\nCurrent plan context:\n{current_plan}"
        )
    } else {
        format!(
            "Current mode: act. Saved plan status: {status}. Use the saved plan as context when relevant, following the user's latest request.{act_delegation}\n\nCurrent saved plan:\n{current_plan}"
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

    const SECTION: &str = "# Delegation\nInline first.";

    #[test]
    fn old_state_defaults_to_act_empty() {
        let state: AgentPlanState = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(state.mode, AgentPlanMode::Act);
        assert_eq!(state.status, AgentPlanStatus::Empty);
        assert_eq!(state.plan, None);
    }

    #[test]
    fn removed_orchestrate_mode_is_rejected_and_saved_state_loads_as_act() {
        assert_eq!(mode_from_str("act").unwrap(), AgentPlanMode::Act);
        assert_eq!(mode_from_str("Plan").unwrap(), AgentPlanMode::Plan);
        assert!(mode_from_str("orchestrate").is_err());
        assert!(mode_from_str("bogus").is_err());
        let saved: AgentPlanState =
            serde_json::from_value(serde_json::json!({"mode": "orchestrate", "plan": "p"}))
                .unwrap();
        assert_eq!(saved.mode, AgentPlanMode::Act);
        assert!(!is_plan_mode(&saved));
        assert_eq!(serde_json::to_value(&saved.mode).unwrap(), "act");
    }

    #[test]
    fn act_keeps_saved_plan_as_context_without_plan_mode_restrictions() {
        let draft = AgentPlanState {
            mode: AgentPlanMode::Plan,
            plan: Some("An existing saved plan".into()),
            ..Default::default()
        };
        let state = with_mode(&draft, AgentPlanMode::Act);
        assert_eq!(state.plan, draft.plan);
        let prompt = format_prompt(&state, Some(SECTION));
        assert!(prompt.contains(current_plan_text(&draft).unwrap()));
        assert!(prompt.contains(SECTION));
        assert!(!prompt.contains("Plan mode"));
        assert!(!prompt.contains("orchestrate"));
    }

    #[test]
    fn act_plan_context_only_includes_saved_plan_when_present() {
        for plan in [None, Some("   ".into())] {
            let state = AgentPlanState {
                plan,
                ..Default::default()
            };
            assert_eq!(format_prompt(&state, None), "Current mode: act.");
            assert_eq!(
                format_prompt(&state, Some(SECTION)),
                format!("Current mode: act.\n\n{SECTION}")
            );
        }
        let state = AgentPlanState {
            plan: Some("Inspect markdown, then fix rendering.".into()),
            status: AgentPlanStatus::Approved,
            ..Default::default()
        };
        let prompt = format_prompt(&state, Some(SECTION));
        assert!(prompt.contains("approved"));
        assert!(prompt.contains(state.plan.as_deref().unwrap()));
        assert!(!prompt.contains("reminders"));
    }

    #[test]
    fn delegation_guidance_requires_an_exposed_task_tool() {
        let mut state = AgentPlanState {
            mode: AgentPlanMode::Plan,
            ..Default::default()
        };
        assert!(format_prompt(&state, Some(SECTION)).contains("read-only planning investigations"));
        assert!(!format_prompt(&state, None).contains("use task"));
        assert!(!format_prompt(&state, Some(SECTION)).contains("Inline first"));
        state.mode = AgentPlanMode::Act;
        state.plan = Some("saved".into());
        assert!(format_prompt(&state, Some(SECTION)).contains("Inline first"));
        assert!(!format_prompt(&state, None).contains("# Delegation"));
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
