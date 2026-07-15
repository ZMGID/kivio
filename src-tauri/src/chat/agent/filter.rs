//! Sub-agent tool-table filtering (P3).
//!
//! Narrows a tool list to an `AgentDefinition`'s allow-list and ALWAYS strips
//! the `agent` spawn tool itself (second recursion guard alongside the depth
//! check). The sub-agent definition is an explicit settings-level policy: it
//! keeps skill-runtime tools while enforcing the agent's configured tool list.

use crate::agents::AgentDefinition;
use crate::mcp::ChatToolDefinition;

use super::prepare::tool_matches_recommended_name;

/// Filter `tools` in place for a sub-agent run. Returns the removed tools (for
/// transparency/logging), mirroring `apply_agent_plan_tool_filter`.
pub fn filter_tools_for_agent(
    tools: &mut Vec<ChatToolDefinition>,
    def: &AgentDefinition,
) -> Vec<ChatToolDefinition> {
    let mut removed = Vec::new();
    let allow = &def.tools;
    tools.retain(|tool| {
        // The `agent` spawn tool is never available inside a sub-agent: a worker
        // must not spawn sibling agents (recursion is top-down only). This is the
        // second recursion guard alongside the depth check.
        if is_sub_agent_control_tool(tool) {
            removed.push(tool.clone());
            return false;
        }
        // Empty allow-list ⇒ no narrowing (all remaining tools available).
        if allow.is_empty() {
            return true;
        }
        // Always keep skill-source tools and skill-runtime meta-tools so the
        // sub-agent can still read/run skills when present.
        if tool.source == "skill" || super::prepare::is_native_skill_tool_name(&tool.name) {
            return true;
        }
        let allowed = allow
            .iter()
            .any(|name| tool_matches_recommended_name(tool, name));
        // Keep Kivio housekeeping built-ins (todo, etc.) that the agent did not
        // explicitly exclude — they are appended separately and are harmless.
        if allowed {
            true
        } else {
            removed.push(tool.clone());
            false
        }
    });
    removed
}

/// Whether `tool` is the `agent` spawn tool. It is stripped from a sub-agent's
/// table: a worker cannot spawn sibling agents.
fn is_sub_agent_control_tool(tool: &ChatToolDefinition) -> bool {
    tool.source == "native" && crate::chat::sub_agent::is_sub_agent_tool_name(&tool.name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::AgentDefinition;

    fn native(name: &str) -> ChatToolDefinition {
        ChatToolDefinition {
            id: format!("native__{name}"),
            name: name.to_string(),
            description: String::new(),
            source: "native".to_string(),
            server_id: None,
            server_name: Some("Kivio".to_string()),
            input_schema: serde_json::json!({}),
            sensitive: false,
            annotations: None,
            output_schema: None,
        }
    }

    fn def(tools: Vec<&str>) -> AgentDefinition {
        AgentDefinition {
            id: "t".to_string(),
            name: "t".to_string(),
            description: String::new(),
            system_prompt: String::new(),
            model: None,
            tools: tools.into_iter().map(String::from).collect(),
            source: "builtin".to_string(),
        }
    }

    #[test]
    fn always_strips_agent_tool_even_with_empty_allow_list() {
        let mut tools = vec![native("agent"), native("read_file")];
        let removed = filter_tools_for_agent(&mut tools, &def(vec![]));
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "read_file");
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].name, "agent");
    }

    #[test]
    fn narrows_to_allow_list_and_strips_agent() {
        let mut tools = vec![
            native("agent"),
            native("read_file"),
            native("write_file"),
            native("web_search"),
        ];
        let removed = filter_tools_for_agent(&mut tools, &def(vec!["read_file", "web_search"]));
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["read_file", "web_search"]);
        // agent + write_file removed
        assert_eq!(removed.len(), 2);
    }

    #[test]
    fn default_subagent_keeps_non_agent_tools() {
        // A default (general-purpose) sub-agent has an empty allow-list, so all
        // tools except `agent` survive — write_file must NOT be stripped.
        let mut tools = vec![native("agent"), native("write_file"), native("read_file")];
        let removed = filter_tools_for_agent(&mut tools, &def(vec![]));
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"write_file"), "write_file must remain");
        assert!(removed.iter().all(|t| t.name != "write_file"));
    }

    #[test]
    fn filtering_is_idempotent() {
        let mut tools = vec![native("agent"), native("read_file"), native("write_file")];
        let d = def(vec!["read_file"]);
        filter_tools_for_agent(&mut tools, &d);
        let after_first: Vec<String> = tools.iter().map(|t| t.name.clone()).collect();
        filter_tools_for_agent(&mut tools, &d);
        let after_second: Vec<String> = tools.iter().map(|t| t.name.clone()).collect();
        assert_eq!(after_first, after_second);
        assert_eq!(after_first, vec!["read_file".to_string()]);
    }
}
