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

pub fn with_mode(current: &AgentPlanState, mode: AgentPlanMode) -> AgentPlanState {
    let mut next = current.clone();
    if next.mode != mode {
        next.mode = mode;
        next.updated_at = chrono::Local::now().timestamp();
    }
    next
}

pub fn approve(current: &AgentPlanState) -> AgentPlanState {
    let mut next = current.clone();
    next.mode = AgentPlanMode::Act;
    next.status = if current_plan_text(current).is_some() {
        AgentPlanStatus::Approved
    } else {
        AgentPlanStatus::Empty
    };
    next.updated_at = chrono::Local::now().timestamp();
    next
}

pub fn capture_draft_from_reply(current: &AgentPlanState, content: &str) -> AgentPlanState {
    let plan = content.trim();
    if plan.is_empty() {
        return current.clone();
    }
    AgentPlanState {
        mode: AgentPlanMode::Plan,
        status: AgentPlanStatus::Draft,
        plan: Some(plan.to_string()),
        updated_at: chrono::Local::now().timestamp(),
    }
}

pub fn format_prompt(state: &AgentPlanState, language: &str) -> String {
    let status = status_name(&state.status);
    let current_plan = current_plan_text(state)
        .map(|plan| plan.to_string())
        .unwrap_or_else(|| {
            if language.starts_with("zh") {
                "当前没有已保存计划。".to_string()
            } else {
                "No current saved plan.".to_string()
            }
        });

    if language.starts_with("zh") {
        if state.mode == AgentPlanMode::Plan {
            format!(
                "Agent plan mode（内部运行模式）：当前模式是 plan，状态是 {status}。Plan mode 用于先调研、阅读、搜索、分析和提出计划；不要执行会产生副作用的动作，不要声称已经修改文件、运行命令、写入记忆或完成实现，除非 Kivio 返回了实际工具结果。可以提出必要的澄清问题。最终回复应给出可执行、简洁的计划，并说明需要用户切到 Act / 执行计划后才会实施。\n\n当前已保存计划：\n{current_plan}"
            )
        } else if state.mode == AgentPlanMode::Orchestrate {
            format!(
                "Agent orchestrate mode（内部运行模式）：当前模式是 orchestrate，计划状态是 {status}。你是 orchestrator（编排者），默认行为就是把活拆开**派给子 agent**，而不是自己动手做。这一点是强约束：**只要任务能拆成 2 个或以上相互独立 / 可并行 / 可分主题的部分，你就必须为每个部分各派一个子 agent（用 `agent` 工具 fan-out），不要自己串行把它们全做完。**\n\n典型必须 fan-out 的场景：研究 / 对比 / 调研多个主题、汇总多个来源、跨多个文件的工作。即使最终要汇总成一篇报告或写一个文件，也必须**先把各部分的研究 / 调查分别派给子 agent**，你自己只负责最后的聚合与产出——绝不要一个人把所有部分都查完、写完。\n\n流程（多步任务必须遵循）：①先用 `todo_write` 列出任务计划；②把每个独立子任务委派给子 agent——在对应 todo 上把 `owner` 设为该子 agent 名并标 `in_progress`，再用 `agent` 工具派发（多个独立部分可在同一轮并行派发）；③子 agent 返回后把该 todo 标 `completed`；④最后汇总各子 agent 的结果回复用户。子 agent 各自独立运行、只返回结果；你负责规划、分派、聚合（orchestrator-worker 模型）。\n\n唯一的例外：只有真正无法再拆分的**单一步骤小任务**（如一句翻译、一个简单事实问答）才可以自己直接做。其余一律 fan-out。如果用户要求继续/执行计划，参考下面的已保存计划。\n\n当前已保存计划：\n{current_plan}"
            )
        } else {
            format!(
                "Agent plan context（内部运行状态）：当前模式是 act，计划状态是 {status}。如果用户要求继续/执行计划，优先参考下面的已保存计划；若用户改变需求，以最新用户消息为准并说明计划需要调整。不要把 plan 当作用户可编辑 todo，也不要创建提醒或日历事项。\n\n当前已保存计划：\n{current_plan}"
            )
        }
    } else if state.mode == AgentPlanMode::Plan {
        format!(
            "Agent plan mode (internal runtime mode): current mode is plan and status is {status}. Plan mode is for researching, reading, searching, analyzing, asking clarifying questions, and producing a plan before action. Do not perform or claim side-effecting work such as editing files, running commands, mutating memory, or implementing changes unless Kivio returned an actual tool result. The final reply should be a concise executable plan and should make clear that implementation waits for Act / execute plan.\n\nCurrent saved plan:\n{current_plan}"
        )
    } else if state.mode == AgentPlanMode::Orchestrate {
        format!(
            "Agent orchestrate mode (internal runtime mode): current mode is orchestrate and plan status is {status}. You are the orchestrator, and your default behavior is to break work apart and **delegate it to sub-agents** instead of doing it yourself. This is a hard rule: **whenever a task can be split into 2 or more independent / parallelizable / separable parts, you MUST dispatch one sub-agent per part (fan out with the `agent` tool); do NOT serially do them all yourself.**\n\nScenarios that MUST fan out: researching / comparing / investigating multiple topics, aggregating multiple sources, and work spanning multiple files. Even when the end goal is to combine everything into one report or write a single file, you MUST first delegate the research / investigation of each part to separate sub-agents, and only own the final aggregation and output yourself — never investigate and write every part single-handedly.\n\nFlow (required for multi-step tasks): (1) first use `todo_write` to lay out the task plan; (2) delegate each independent subtask to a sub-agent — set the matching todo's `owner` to that sub-agent name and mark it `in_progress`, then dispatch with the `agent` tool (multiple independent parts can be dispatched in parallel in the same round); (3) when a sub-agent returns, mark its todo `completed`; (4) finally aggregate the sub-agents' results into your reply to the user. Each sub-agent runs in isolation and only returns its result; you own planning, dispatch, and aggregation (orchestrator-worker model).\n\nThe only exception: a genuinely indivisible **single-step small task** (e.g. a one-line translation, a simple factual question) may be done directly yourself. Everything else must fan out. If the user asks to continue or execute the plan, use the saved plan below.\n\nCurrent saved plan:\n{current_plan}"
        )
    } else {
        format!(
            "Agent plan context (internal runtime state): current mode is act and plan status is {status}. If the user asks to continue or execute the plan, use the saved plan below as context; if the user changes requirements, follow the latest user message and note that the plan needs adjustment. Do not treat the plan as a user-editable todo list, and do not create reminders or calendar items.\n\nCurrent saved plan:\n{current_plan}"
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
    fn capture_draft_keeps_plan_mode_and_trims_reply() {
        let state =
            capture_draft_from_reply(&AgentPlanState::default(), "  1. Read code\n2. Edit  ");
        assert_eq!(state.mode, AgentPlanMode::Plan);
        assert_eq!(state.status, AgentPlanStatus::Draft);
        assert_eq!(state.plan.as_deref(), Some("1. Read code\n2. Edit"));
        assert!(state.updated_at > 0);
    }

    #[test]
    fn approve_without_plan_stays_empty_act() {
        let mut state = AgentPlanState::default();
        state.mode = AgentPlanMode::Plan;
        let approved = approve(&state);
        assert_eq!(approved.mode, AgentPlanMode::Act);
        assert_eq!(approved.status, AgentPlanStatus::Empty);
    }

    #[test]
    fn approve_with_plan_marks_approved() {
        let mut state = AgentPlanState::default();
        state.plan = Some("Plan".to_string());
        state.status = AgentPlanStatus::Draft;
        let approved = approve(&state);
        assert_eq!(approved.mode, AgentPlanMode::Act);
        assert_eq!(approved.status, AgentPlanStatus::Approved);
    }

    #[test]
    fn mode_from_str_accepts_orchestrate() {
        assert_eq!(mode_from_str("orchestrate").unwrap(), AgentPlanMode::Orchestrate);
        assert_eq!(mode_from_str("Orchestrate").unwrap(), AgentPlanMode::Orchestrate);
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
    fn format_prompt_emits_orchestrate_section() {
        let mut state = AgentPlanState::default();
        state.mode = AgentPlanMode::Orchestrate;
        let zh = format_prompt(&state, "zh-CN");
        assert!(zh.contains("orchestrate"));
        assert!(zh.contains("子 agent"));
        let en = format_prompt(&state, "en");
        assert!(en.contains("orchestrate mode"));
        assert!(en.contains("sub-agents"));
    }
}
