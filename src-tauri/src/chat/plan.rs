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
    next.status = if executable_plan_text(current).is_some() {
        AgentPlanStatus::Approved
    } else {
        AgentPlanStatus::Empty
    };
    next.updated_at = chrono::Local::now().timestamp();
    next
}

pub fn capture_draft_from_reply(current: &AgentPlanState, content: &str) -> AgentPlanState {
    let plan = content.trim();
    if !is_executable_plan_text(plan) {
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
                "Agent plan mode（内部运行模式）：当前模式是 plan，状态是 {status}。Plan mode 只读：先调研、阅读、搜索、分析，再出计划；不要执行任何会产生副作用的动作，也不要声称已修改文件、运行命令、写入记忆或完成实现，除非 Kivio 返回了实际工具结果。可以提出必要的澄清问题。\n\n调研先行（强制）：写计划之前，必须先用只读工具把现状查清楚，不允许凭空臆测或只看一两个文件就下结论。按以下维度系统调研，每条结论都基于你实际读过的文件/代码并注明来源路径：\n- 现状：相关功能/模块当前怎么实现、入口在哪；\n- 涉及范围：这次改动会 touch 哪些文件/函数，及其上下游调用方；\n- 现有约定：该区域已有的命名、模式、错误处理、测试方式——新代码要对齐；\n- 外部参考（除非纯内部琐碎改动，否则必做）：只要任务涉及外部标准/协议、第三方库/框架 API，或任何架构选型，就必须用 web_search 查官方文档和有代表性的开源项目是怎么做的——什么架构、什么流程、有哪些惯例和坑，再用 web_fetch 读关键页面；不要只凭记忆或已有知识下结论。先确认业界成熟做法再据此规划，不要盲目自己造（仅当 web 搜索工具确实不可用时才跳过，并在发现里说明）；\n- 风险与未知：边界情况、可能被破坏的地方、尚未确认需要进一步查证的点。\n只有当这些维度查得足以支撑一份可落地的计划时，才开始写计划；信息不足就继续调研或向用户提问，不要急着出计划。\n\n最终回复结构：先用「## 调研发现」按上述维度给出关键发现（每条附依据文件路径），紧接着用「## 计划」给出可执行步骤/todo（清晰编号或勾选项），让用户一眼看出这是一份 Plan。背景与风险放计划后面；说明需要用户切到 Act / 执行计划后才会实施。\n\n当前已保存计划：\n{current_plan}"
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
            "Agent plan mode (internal runtime mode): current mode is plan and status is {status}. Plan mode is read-only: research, read, search, and analyze before producing a plan. Do not perform or claim side-effecting work such as editing files, running commands, mutating memory, or implementing changes unless Kivio returned an actual tool result. Ask clarifying questions when needed.\n\nInvestigate first (required): before writing the plan, you MUST use the read-only tools to understand the current state — do not guess or conclude from just one or two files. Investigate systematically along these dimensions, and back every claim with a file/code you actually read (cite the source path):\n- Current state: how the relevant feature/module works today and where its entry points are;\n- Scope: which files/functions this change will touch, plus their upstream/downstream callers;\n- Existing conventions: the naming, patterns, error handling, and testing style already used in this area — new code must align;\n- External references (required unless this is a purely internal, trivial change): whenever the task involves an external standard/protocol, a third-party library/framework API, or any architecture decision, you MUST use web_search to check how official docs and representative open-source projects do it — their architecture, flow, conventions, and pitfalls — then web_fetch to read the key pages. Do not conclude from memory or prior knowledge alone. Confirm the established industry approach before planning rather than building blindly (skip only if the web search tool is genuinely unavailable, and say so in your findings);\n- Risks & unknowns: edge cases, things that could break, and open points that still need confirmation.\nOnly start writing the plan once these dimensions are covered well enough to support an actionable plan; if information is insufficient, keep investigating or ask the user rather than rushing to a plan.\n\nFinal reply structure: first a \"## Findings\" section giving the key findings along the dimensions above (each citing the source file), immediately followed by a \"## Plan\" section with actionable steps/todos (clearly numbered or checkboxed) so the user can immediately tell this is a Plan. Put background and risks after the plan, and make clear that implementation waits for Act / execute plan.\n\nCurrent saved plan:\n{current_plan}"
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

pub fn executable_plan_text(state: &AgentPlanState) -> Option<&str> {
    current_plan_text(state).filter(|plan| is_executable_plan_text(plan))
}

pub fn is_executable_plan_text(content: &str) -> bool {
    let text = content.trim();
    if text.is_empty() {
        return false;
    }

    let meaningful_lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if meaningful_lines.len() < 2 {
        return false;
    }

    let step_lines = meaningful_lines
        .iter()
        .filter(|line| is_step_like_line(line))
        .count();
    if step_lines >= 2 {
        return true;
    }

    has_plan_keyword(text) && step_lines >= 1
}

fn is_step_like_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    starts_with_markdown_step(trimmed)
        || starts_with_chinese_step(trimmed)
        || starts_with_todo_keyword(trimmed)
}

fn starts_with_markdown_step(line: &str) -> bool {
    if line.starts_with("- [ ]")
        || line.starts_with("- [x]")
        || line.starts_with("- [X]")
        || line.starts_with("* [ ]")
        || line.starts_with("* [x]")
        || line.starts_with("* [X]")
        || line.starts_with("- ")
        || line.starts_with("* ")
        || line.starts_with("+ ")
        || line.starts_with("• ")
    {
        return true;
    }

    let bytes = line.as_bytes();
    let mut digit_count = 0;
    while digit_count < bytes.len() && bytes[digit_count].is_ascii_digit() {
        digit_count += 1;
    }
    if digit_count == 0 || digit_count > 3 {
        return false;
    }
    line[digit_count..]
        .chars()
        .next()
        .is_some_and(|ch| matches!(ch, '.' | ')' | '、'))
}

fn starts_with_chinese_step(line: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "第1步",
        "第2步",
        "第3步",
        "第4步",
        "第5步",
        "第6步",
        "第7步",
        "第8步",
        "第9步",
        "第一步",
        "第二步",
        "第三步",
        "第四步",
        "第五步",
        "第六步",
        "第七步",
        "第八步",
        "第九步",
        "步骤1",
        "步骤2",
        "步骤3",
        "步骤4",
        "步骤5",
        "步骤6",
        "步骤7",
        "步骤8",
        "步骤9",
        "一、",
        "二、",
        "三、",
        "四、",
        "五、",
        "六、",
        "七、",
        "八、",
        "九、",
    ];
    PREFIXES.iter().any(|prefix| line.starts_with(prefix))
}

fn starts_with_todo_keyword(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.starts_with("todo:")
        || lower.starts_with("todo ")
        || lower.starts_with("step ")
        || lower.starts_with("步骤：")
        || lower.starts_with("步骤:")
        || lower.starts_with("任务：")
        || lower.starts_with("任务:")
}

fn has_plan_keyword(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("plan")
        || lower.contains("todo")
        || lower.contains("step")
        || text.contains("计划")
        || text.contains("步骤")
        || text.contains("待办")
        || text.contains("任务")
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
    fn capture_draft_ignores_non_plan_fragment() {
        let current = AgentPlanState::default();
        let state = capture_draft_from_reply(&current, "没问题！积萌,");

        assert_eq!(state, current);
    }

    #[test]
    fn executable_plan_requires_real_steps() {
        assert!(is_executable_plan_text(
            "计划：\n1. Read code\n2. Implement fix"
        ));
        assert!(is_executable_plan_text("- [ ] 调研\n- [ ] 修改"));
        assert!(!is_executable_plan_text("没问题！积萌,"));
        assert!(!is_executable_plan_text("计划：我会处理这个问题。"));
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
        state.plan = Some("1. Read code\n2. Edit".to_string());
        state.status = AgentPlanStatus::Draft;
        let approved = approve(&state);
        assert_eq!(approved.mode, AgentPlanMode::Act);
        assert_eq!(approved.status, AgentPlanStatus::Approved);
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
