//! Tool selection for OMP subagents and matching for automation tool selections.

use crate::agents::AgentDefinition;
use crate::mcp::ChatToolDefinition;

use super::prepare::{is_native_skill_tool_name, tool_matches_recommended_name};

/// Match an automation tool selection, including its existing MCP prefix syntax.
pub fn entry_matches(tool: &ChatToolDefinition, entry: &str) -> bool {
    let entry = entry.trim();
    if entry.is_empty() {
        return false;
    }
    if entry == "*" {
        return true;
    }
    if let Some(rest) = entry.strip_prefix("mcp__") {
        let server = rest.trim_end_matches('*').trim_end_matches("__");
        if server.is_empty() {
            return tool.source == "mcp";
        }
        if tool.server_id.as_deref() == Some(server) {
            return true;
        }
    }
    if let Some(prefix) = entry.strip_suffix('*') {
        return tool.name.starts_with(prefix)
            || tool.id.starts_with(prefix)
            || tool.openai_tool_name().starts_with(prefix);
    }
    tool.name == entry || tool.id == entry || tool_matches_recommended_name(tool, entry)
}

fn is_native(tool: &ChatToolDefinition, name: &str) -> bool {
    tool.source == "native" && tool.name == name
}

/// Filter the enabled catalog for a child. The catalog must include the task and
/// wait definitions so they can be selected even for an explicit tools list.
pub fn filter_tools_for_agent(
    tools: &mut Vec<ChatToolDefinition>,
    def: &AgentDefinition,
    child_depth: u8,
    max_depth: i32,
) {
    let listed = |tool: &ChatToolDefinition| {
        def.tools.as_ref().is_none_or(|allow| {
            allow.iter().any(|name| {
                let name = name.trim();
                tool.name == name || tool.id == name || tool_matches_recommended_name(tool, name)
            })
        })
    };
    let task_allowed = |tool: &ChatToolDefinition| {
        (max_depth < 0 || i32::from(child_depth) < max_depth)
            && def.spawns.as_deref().is_none_or(|policy| {
                policy.split(',').any(|name| !name.trim().is_empty())
            })
            && (def.spawns.is_some() || listed(tool))
    };
    let has_task = tools.iter().any(|tool| is_native(tool, "task") && task_allowed(tool));
    let has_bash = tools.iter().any(|tool| is_native(tool, "bash") && listed(tool));

    tools.retain(|tool| {
        if tool.source == "native"
            && (tool.name == "ask_user" || tool.name == "todo" || tool.name.starts_with("todo_"))
        {
            return false;
        }
        if is_native(tool, "task") {
            return task_allowed(tool);
        }
        if is_native(tool, "wait") {
            return has_task || has_bash || listed(tool);
        }
        if tool.source == "skill"
            || (tool.source == "native" && is_native_skill_tool_name(&tool.name))
        {
            return true;
        }
        listed(tool)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(source: &str, name: &str) -> ChatToolDefinition {
        ChatToolDefinition {
            id: format!("{source}__{name}"),
            name: name.into(),
            description: String::new(),
            source: source.into(),
            server_id: None,
            server_name: None,
            input_schema: serde_json::json!({}),
            sensitive: false,
            annotations: None,
            output_schema: None,
        }
    }

    fn definition(tools: Option<&[&str]>, spawns: Option<&str>) -> AgentDefinition {
        AgentDefinition {
            id: "worker".into(),
            name: "worker".into(),
            description: String::new(),
            system_prompt: String::new(),
            model: None,
            thinking_level: None,
            tools: tools.map(|items| items.iter().map(|item| (*item).into()).collect()),
            spawns: spawns.map(str::to_string),
            skills: Vec::new(),
            source: "builtin".into(),
        }
    }

    fn catalog() -> Vec<ChatToolDefinition> {
        ["read", "write", "bash", "task", "wait", "ask_user", "todo_write", "todo_read", "skill"]
            .into_iter()
            .map(|name| tool("native", name))
            .chain(std::iter::once(tool("skill", "research")))
            .collect()
    }

    fn names(tools: &[ChatToolDefinition]) -> Vec<&str> {
        tools.iter().map(|tool| tool.name.as_str()).collect()
    }

    #[test]
    fn default_keeps_enabled_tools_but_not_ask_user_or_todos() {
        let mut tools = catalog();
        filter_tools_for_agent(&mut tools, &definition(None, None), 1, 2);
        assert_eq!(names(&tools), ["read", "write", "bash", "task", "wait", "skill", "research"]);
    }

    #[test]
    fn explicit_allowlist_keeps_skill_tools_and_automatically_adds_wait_for_bash() {
        let mut tools = catalog();
        filter_tools_for_agent(&mut tools, &definition(Some(&["read", "bash", "ask_user", "todo_write"]), None), 1, 2);
        assert_eq!(names(&tools), ["read", "bash", "wait", "skill", "research"]);
        let mut tools = catalog();
        filter_tools_for_agent(&mut tools, &definition(Some(&[]), None), 1, 2);
        assert_eq!(names(&tools), ["skill", "research"]);
    }

    #[test]
    fn spawns_adds_task_and_wait_until_depth_limit() {
        let def = definition(Some(&["read"]), Some("scout"));
        let mut tools = catalog();
        filter_tools_for_agent(&mut tools, &def, 1, 2);
        assert_eq!(names(&tools), ["read", "task", "wait", "skill", "research"]);
        let mut tools = catalog();
        filter_tools_for_agent(&mut tools, &def, 2, 2);
        assert_eq!(names(&tools), ["read", "skill", "research"]);
        let mut tools = catalog();
        filter_tools_for_agent(&mut tools, &def, 250, -1);
        assert!(names(&tools).contains(&"task"));
    }

    #[test]
    fn explicitly_requested_task_and_wait_follow_omp_allowlist_rules() {
        let mut tools = catalog();
        filter_tools_for_agent(&mut tools, &definition(Some(&["task", "wait"]), None), 1, 2);
        assert_eq!(names(&tools), ["task", "wait", "skill", "research"]);
        let mut tools = catalog();
        filter_tools_for_agent(&mut tools, &definition(Some(&["task", "wait"]), Some("*")), 1, 2);
        assert_eq!(names(&tools), ["task", "wait", "skill", "research"]);
        let mut tools = catalog();
        filter_tools_for_agent(&mut tools, &definition(Some(&["task", "wait"]), Some("*")), 2, 2);
        assert_eq!(names(&tools), ["wait", "skill", "research"]);
    }

    #[test]
    fn empty_spawn_policy_removes_task_even_when_explicitly_listed() {
        for allowed in [None, Some(&["task"][..])] {
            let mut tools = catalog();
            filter_tools_for_agent(&mut tools, &definition(allowed, Some("")), 1, 2);
            assert!(!names(&tools).contains(&"task"));
        }
    }

    #[test]
    fn automation_prefix_matching_remains_available() {
        let mut mcp = tool("mcp", "search");
        mcp.id = "mcp__server__search".into();
        mcp.server_id = Some("server".into());
        assert!(entry_matches(&mcp, "mcp__server__*"));
        assert!(entry_matches(&mcp, "mcp__*"));
        assert!(!entry_matches(&mcp, "mcp__other"));
    }
}
