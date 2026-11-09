//! Baseline for the model-facing tool list. Mirrors the composition order in
//! `reply.rs` (catalog → mode filter → appended conversation tools) and the
//! sub-agent order in `sub_agent/executor.rs`, without MCP servers.

use super::tooling::{
    append_agent_ask_user_tools, append_agent_todo_tools, append_goal_tools,
    apply_agent_plan_tool_filter, apply_chat_mode_tool_filter, move_mcp_tools_to_end,
};
use crate::chat::model::ModelTool;
use crate::mcp::ChatToolDefinition;
use crate::settings::Settings;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Agent,
    AgentGoal,
    Plan,
    Chat,
}

fn catalog(settings: &Settings) -> Vec<ChatToolDefinition> {
    let mut tools = crate::mcp::types::list_native_builtin_tool_defs(
        &settings.chat_tools.native_tools,
        crate::mcp::registry::web_search_configured(settings),
        crate::settings::chat_memory_tools_enabled(settings),
    );
    tools.extend(crate::mcp::registry::list_skill_tool_defs(settings));
    tools
}

fn scheduled_tools() -> [ChatToolDefinition; 1] {
    [crate::scheduled_tasks::tools::tool_definition()]
}

fn main_tools(settings: &Settings, mode: Mode) -> Vec<ChatToolDefinition> {
    let chat_mode = mode == Mode::Chat;
    let plan_mode = mode == Mode::Plan;
    let mut tools = catalog(settings);
    super::tooling::apply_knowledge_base_tool_filter(&mut tools, &[]);
    if chat_mode {
        apply_chat_mode_tool_filter(&mut tools, true, &settings.chat.chat_mode);
    } else {
        apply_agent_plan_tool_filter(&mut tools, plan_mode);
    }
    crate::chat::plan_document::append_tools(&mut tools, plan_mode);
    append_agent_ask_user_tools(&mut tools);
    if !chat_mode && !plan_mode {
        append_agent_todo_tools(&mut tools);
    }
    if !chat_mode {
        let agents = crate::agents::builtin_agent_definitions();
        crate::chat::sub_agent::append_tool_definitions(&mut tools, settings, &agents);
    }
    if settings.chat_tools.native_tools.scheduled_tasks && !plan_mode {
        tools.extend(scheduled_tools());
    }
    if mode == Mode::AgentGoal {
        tools.extend(crate::chat::goal::tool_definitions());
    } else if !chat_mode && !plan_mode {
        append_goal_tools(&mut tools, None);
    }
    move_mcp_tools_to_end(&mut tools);
    crate::chat::tool_on_demand::configure_tools(&mut tools, settings.chat_tools.tools_on_demand);
    tools
}

fn sub_agent_tools(settings: &Settings, role: &str) -> Vec<ChatToolDefinition> {
    let agents = crate::agents::builtin_agent_definitions();
    let definition = agents.iter().find(|agent| agent.name == role).unwrap();
    let mut tools = catalog(settings);
    super::tooling::apply_knowledge_base_tool_filter(&mut tools, &[]);
    crate::chat::sub_agent::append_tool_definitions(&mut tools, settings, &agents);
    if let Some(tool) = tools
        .iter_mut()
        .find(|tool| tool.name == crate::chat::sub_agent::TASK_TOOL_NAME)
    {
        *tool = crate::chat::sub_agent::task::tool_definition_with_policy(
            &agents,
            settings,
            definition.spawns.as_deref(),
            false,
        );
    }
    crate::chat::tool_filter::filter_tools_for_agent(
        &mut tools,
        definition,
        1,
        settings.chat_tools.task_max_recursion_depth,
    );
    move_mcp_tools_to_end(&mut tools);
    crate::chat::tool_on_demand::configure_tools(&mut tools, settings.chat_tools.tools_on_demand);
    tools
}

fn names(tools: &[ChatToolDefinition]) -> Vec<String> {
    tools
        .iter()
        .map(ChatToolDefinition::openai_tool_name)
        .collect()
}

/// Same arithmetic as the context meter's "Tools" segment.
fn tool_cost(tools: &[ChatToolDefinition]) -> (usize, usize) {
    let model: Vec<ModelTool> = tools.iter().map(ModelTool::from).collect();
    crate::chat::context_measure::measure_prepared_request(&[], &model, None, "")
        .into_iter()
        .find(|segment| segment.id == "tools")
        .map(|segment| (segment.estimated_tokens, segment.chars))
        .unwrap_or_default()
}

fn report(label: &str, tools: &[ChatToolDefinition]) {
    let (tokens, chars) = tool_cost(tools);
    println!(
        "{label}: {} tools, ~{tokens} tokens, {chars} chars",
        tools.len()
    );
    let mut rows: Vec<_> = tools
        .iter()
        .map(|tool| {
            (
                tool_cost(std::slice::from_ref(tool)).0,
                tool.openai_tool_name(),
            )
        })
        .collect();
    rows.sort_by(|a, b| b.0.cmp(&a.0));
    for (tokens, name) in rows {
        println!("  {tokens:>6}  {name}");
    }
}

const AGENT_TOOLS: &[&str] = &[
    "web_fetch",
    "automation",
    "read",
    "grep",
    "glob",
    "write",
    "edit",
    "bash",
    "bash_output",
    "kill_background",
    "present_artifacts",
    "html_render",
    "html_preview",
    "kivio_inspect",
    "kivio_configure",
    "skill",
    "ask_user",
    "todo_write",
    "task",
    "wait",
    "eval",
    "schedule",
];
const GOAL_TOOLS: &[&str] = &[
    "get_goal",
    "initialize_goal_criteria",
    "report_goal_progress",
    "goal_complete",
    "goal_blocked",
    "goal_wait",
];
const PLAN_TOOLS: &[&str] = &[
    "web_fetch",
    "automation",
    "read",
    "grep",
    "glob",
    "bash_output",
    "present_artifacts",
    "html_render",
    "html_preview",
    "kivio_inspect",
    "skill",
    "save_plan",
    "ask_user",
    "task",
    "wait",
    "eval",
];
const CHAT_TOOLS: &[&str] = &[
    "web_fetch",
    "automation",
    "html_render",
    "html_preview",
    "ask_user",
    "schedule",
];
const SUBAGENT_TASK_TOOLS: &[&str] = &[
    "web_fetch",
    "automation",
    "read",
    "grep",
    "glob",
    "write",
    "edit",
    "bash",
    "bash_output",
    "kill_background",
    "present_artifacts",
    "html_render",
    "html_preview",
    "kivio_inspect",
    "kivio_configure",
    "skill",
    "task",
    "wait",
    "eval",
];
const SUBAGENT_SCOUT_TOOLS: &[&str] = &["read", "grep", "glob", "skill"];

/// Default settings, no MCP servers, no search provider, memory off.
/// Run with `--nocapture` to print per-tool cost for before/after comparisons.
#[test]
fn default_tool_list_baseline() {
    for enabled in [true, false] {
        let mut settings = Settings::default();
        settings.chat_tools.tools_on_demand = enabled;
        let agent_goal: Vec<&str> = AGENT_TOOLS.iter().chain(GOAL_TOOLS).copied().collect();
        for (label, tools, expected) in [
            ("agent", main_tools(&settings, Mode::Agent), AGENT_TOOLS),
            (
                "agent+goal",
                main_tools(&settings, Mode::AgentGoal),
                agent_goal.as_slice(),
            ),
            ("plan", main_tools(&settings, Mode::Plan), PLAN_TOOLS),
            ("chat", main_tools(&settings, Mode::Chat), CHAT_TOOLS),
            (
                "subagent:task",
                sub_agent_tools(&settings, "task"),
                SUBAGENT_TASK_TOOLS,
            ),
            (
                "subagent:scout",
                sub_agent_tools(&settings, "scout"),
                SUBAGENT_SCOUT_TOOLS,
            ),
        ] {
            assert_eq!(names(&tools), expected, "{label} executable set changed");
            let listed: Vec<_> = tools
                .iter()
                .filter(|tool| !crate::chat::tool_on_demand::is_on_demand(tool))
                .cloned()
                .collect();
            let expected_listed: Vec<_> = expected
                .iter()
                .copied()
                .filter(|name| {
                    !enabled
                        || !matches!(label, "agent" | "agent+goal" | "subagent:task")
                        || ![
                            "automation",
                            "bash_output",
                            "kill_background",
                            "html_render",
                            "html_preview",
                            "kivio_inspect",
                            "kivio_configure",
                            "eval",
                            "schedule",
                        ]
                        .contains(name)
                })
                .collect();
            report(&format!("{label} (on-demand={enabled})"), &listed);
            let catalog = crate::chat::tool_on_demand::catalog(&tools);
            println!(
                "  on-demand catalog: {} tools, ~{} tokens, {} chars",
                tools.len() - listed.len(),
                crate::chat::agent_preparation::estimate_tokens(&catalog),
                catalog.chars().count()
            );
            assert_eq!(
                names(&listed),
                expected_listed,
                "{label} declared tool list changed"
            );
            if enabled && label == "agent" {
                assert!(tool_cost(&listed).0 < tool_cost(&tools).0 * 3 / 4);
            }
        }
    }
}

#[test]
fn goal_toggle_preserves_stable_tool_prefix() {
    let settings = Settings::default();
    let without = main_tools(&settings, Mode::Agent);
    let with = main_tools(&settings, Mode::AgentGoal);
    let goal_at = with
        .iter()
        .position(|tool| crate::chat::goal::is_goal_tool_name(&tool.name))
        .unwrap();
    assert_eq!(
        serde_json::to_value(&with[..goal_at]).unwrap(),
        serde_json::to_value(&without).unwrap()
    );
    assert_eq!(names(&with[goal_at..]), GOAL_TOOLS);

    let mut scheduled_turn = without.clone();
    scheduled_turn.retain(|tool| tool.name != "schedule");
    assert_eq!(
        serde_json::to_value(&without[..scheduled_turn.len()]).unwrap(),
        serde_json::to_value(&scheduled_turn).unwrap()
    );
}

#[test]
fn mcp_discovery_preserves_non_mcp_prefix_and_tool_set() {
    let settings = Settings::default();
    for mut tools in [
        main_tools(&settings, Mode::AgentGoal),
        sub_agent_tools(&settings, "task"),
    ] {
        let prefix = serde_json::to_value(&tools).unwrap();
        let mut remote = crate::mcp::types::native_web_fetch_tool();
        remote.id = "mcp__test__fetch".into();
        remote.source = "mcp".into();
        tools.insert(1, remote.clone());
        remote.id = "mcp__test__other".into();
        tools.insert(3, remote);
        let before: Vec<_> = tools.iter().map(|tool| tool.id.clone()).collect();
        move_mcp_tools_to_end(&mut tools);
        let mcp_at = tools.iter().position(|tool| tool.source == "mcp").unwrap();
        assert_eq!(serde_json::to_value(&tools[..mcp_at]).unwrap(), prefix);
        assert_eq!(
            tools[mcp_at..]
                .iter()
                .map(|tool| tool.id.as_str())
                .collect::<Vec<_>>(),
            ["mcp__test__fetch", "mcp__test__other"]
        );
        let mut after: Vec<_> = tools.iter().map(|tool| tool.id.clone()).collect();
        let mut before = before;
        before.sort();
        after.sort();
        assert_eq!(before, after);
        let once = serde_json::to_value(&tools).unwrap();
        move_mcp_tools_to_end(&mut tools);
        assert_eq!(serde_json::to_value(&tools).unwrap(), once);
    }
}
