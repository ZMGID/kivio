//! Per-run visibility is decided only after all capability and policy filters.
use crate::mcp::{
    native_registry::{find_entry, ToolLoadMode},
    ChatToolDefinition,
};
use serde_json::Value;

const ON_DEMAND: &str = "kivioOnDemand";
const PROMPT_START: &str = "\n\n<on_demand_tools>";
const PROMPT_END: &str = "</on_demand_tools>";

pub(crate) fn split_tools(
    tools: &[ChatToolDefinition],
    enabled: bool,
) -> (Vec<&ChatToolDefinition>, Vec<&ChatToolDefinition>) {
    let transport = ["read", "write"].iter().all(|name| {
        tools
            .iter()
            .any(|tool| tool.source == "native" && tool.name == *name)
    });
    tools.iter().partition(|tool| {
        !(enabled
            && transport
            && tool.source == "native"
            && find_entry(&tool.name)
                .is_some_and(|entry| entry.load_mode == ToolLoadMode::OnDemand))
    })
}

pub(crate) fn configure_tools(tools: &mut [ChatToolDefinition], enabled: bool) {
    let (_, on_demand) = split_tools(tools, enabled);
    let ids: Vec<_> = on_demand.iter().map(|tool| tool.id.clone()).collect();
    for tool in tools {
        if tool.source != "native" {
            continue;
        }
        if let Some(object) = tool.annotations.as_mut().and_then(Value::as_object_mut) {
            object.remove(ON_DEMAND);
            if object.is_empty() {
                tool.annotations = None;
            }
        }
        if ids.contains(&tool.id) {
            tool.annotations
                .get_or_insert_with(|| serde_json::json!({}))[ON_DEMAND] = true.into();
        }
    }
}

pub(crate) fn is_on_demand(tool: &ChatToolDefinition) -> bool {
    tool.source == "native"
        && tool
            .annotations
            .as_ref()
            .and_then(|value| value.get(ON_DEMAND))
            .and_then(Value::as_bool)
            == Some(true)
}

fn summary(description: &str) -> String {
    let end = description
        .char_indices()
        .find(|(_, ch)| matches!(ch, '.' | '。' | '!' | '?' | '\n'))
        .map_or(description.len(), |(index, ch)| index + ch.len_utf8());
    description[..end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn catalog(tools: &[ChatToolDefinition]) -> String {
    tools
        .iter()
        .filter(|tool| is_on_demand(tool))
        .map(|tool| format!("{}: {}", tool.name, summary(&tool.description)))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn unavailable(name: &str) -> String {
    super::agent_preparation::disabled_builtin_tool_feedback(name)
        .unwrap_or_else(|| format!("Kivio tool `{name}` is not enabled in the current tool set, so this call was not executed."))
}

pub(crate) fn find<'a>(
    tools: &'a [ChatToolDefinition],
    name: &str,
) -> Option<&'a ChatToolDefinition> {
    let canonical = crate::mcp::types::canonical_tool_name(name);
    tools
        .iter()
        .find(|tool| is_on_demand(tool) && tool.name == canonical)
}

pub(crate) fn read(tools: &[ChatToolDefinition], path: &str) -> Result<String, String> {
    let name = path.strip_prefix("tool://").ok_or("Expected tool:// URL")?;
    if name.is_empty() {
        return Ok(catalog(tools));
    }
    let tool = find(tools, name).ok_or_else(|| unavailable(name))?;
    Ok(format!(
        "{}\n{}\nInput JSON schema:\n{}",
        tool.name, tool.description, tool.input_schema
    ))
}

pub(crate) fn update_prompt(context: &mut String, tools: &[ChatToolDefinition]) {
    if let Some(start) = context.find(PROMPT_START) {
        if let Some(end) = context[start..].find(PROMPT_END) {
            context.replace_range(start..start + end + PROMPT_END.len(), "");
        }
    }
    let catalog = catalog(tools);
    if !catalog.is_empty() {
        context.push_str(&format!("{PROMPT_START}\nThe following tools are available on demand.\nUse read with path=\"tool://\" for the catalog, or path=\"tool://<name>\" for the full description and JSON schema.\nExecute with write: {{\"path\":\"tool://<name>\",\"content\":\"<JSON arguments>\"}}. Calling the tool directly by name also works; the same permissions apply.\n{catalog}\n{PROMPT_END}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::types::{
        native_automation_tool, native_read_file_tool, native_write_file_tool,
    };

    #[test]
    fn final_set_controls_catalog_schema_and_execution_lookup() {
        let mut tools = vec![
            native_read_file_tool(),
            native_write_file_tool(),
            native_automation_tool(),
        ];
        configure_tools(&mut tools, true);
        assert_eq!(split_tools(&tools, true).0.len(), 2);
        assert!(!tools[2].for_run(&Default::default()).declared);
        assert_eq!(read(&tools, "tool://").unwrap().lines().count(), 1);
        assert!(read(&tools, "tool://automation")
            .unwrap()
            .contains(&tools[2].input_schema.to_string()));
        assert_eq!(find(&tools, "automation_list").unwrap().name, "automation");
        assert!(read(&tools, "tool://schedule")
            .unwrap_err()
            .contains("not enabled"));
        tools.retain(|tool| tool.name != "automation");
        assert!(catalog(&tools).is_empty());
        assert!(find(&tools, "automation").is_none());
        assert!(read(&tools, "tool://automation").is_err());
    }

    #[test]
    fn toggle_and_missing_transports_keep_tools_declared_and_non_native_sources_unchanged() {
        for (enabled, missing) in [(false, ""), (true, "read"), (true, "write")] {
            let mut tools = vec![
                native_read_file_tool(),
                native_write_file_tool(),
                native_automation_tool(),
            ];
            configure_tools(&mut tools, true);
            tools.retain(|tool| tool.name != missing);
            configure_tools(&mut tools, enabled);
            assert!(tools
                .iter()
                .all(|tool| tool.for_run(&Default::default()).declared));
            assert!(catalog(&tools).is_empty());
        }
        for source in ["mcp", "mixer", "skill"] {
            let mut tools = vec![
                native_read_file_tool(),
                native_write_file_tool(),
                native_automation_tool(),
            ];
            tools[2].source = source.into();
            tools[2].annotations = Some(serde_json::json!({(ON_DEMAND): true}));
            let original = serde_json::to_value(&tools[2]).unwrap();
            configure_tools(&mut tools, true);
            assert!(catalog(&tools).is_empty());
            assert!(tools[2].for_run(&Default::default()).declared);
            assert_eq!(serde_json::to_value(&tools[2]).unwrap(), original);
        }
    }

    #[test]
    fn prompt_refresh_drops_unavailable_tools_and_preserves_other_context() {
        let mut tools = vec![
            native_read_file_tool(),
            native_write_file_tool(),
            native_automation_tool(),
        ];
        tools[2].description = "First sentence. Second sentence.\nMore details".into();
        configure_tools(&mut tools, true);
        assert_eq!(catalog(&tools), "automation: First sentence.");
        let mut context = "Existing context".to_string();
        update_prompt(&mut context, &tools);
        let once = context.clone();
        update_prompt(&mut context, &tools);
        assert_eq!(context, once);
        assert_eq!(context.matches(PROMPT_START).count(), 1);
        assert!(context.contains("Calling the tool directly by name also works"));
        configure_tools(&mut tools, false);
        update_prompt(&mut context, &tools);
        assert_eq!(context.trim(), "Existing context");
    }

    #[test]
    fn setting_defaults_on_and_false_survives_roundtrip() {
        let default: crate::settings::ChatToolsConfig = serde_json::from_str("{}").unwrap();
        assert!(default.tools_on_demand);
        let disabled: crate::settings::ChatToolsConfig =
            serde_json::from_value(serde_json::json!({"toolsOnDemand":false})).unwrap();
        assert!(!disabled.tools_on_demand);
        assert_eq!(
            serde_json::to_value(disabled).unwrap()["toolsOnDemand"],
            false
        );
    }
}
