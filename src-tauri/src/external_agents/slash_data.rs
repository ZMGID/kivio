use serde_json::Value;
use super::types::ExternalCliSlashCommand;
/// Native CLIs expect their command first. Adapt only a known command from the
/// same catalog used by the picker, keeping the stored/displayed user text intact.
pub(crate) fn inline_command_prompt(
    content: &str,
    commands: &[ExternalCliSlashCommand],
) -> Result<Option<String>, String> {
    let matches: Vec<_> = crate::chat::slash_commands::command_ranges(content)
        .into_iter()
        .filter(|range| {
            commands
                .iter()
                .any(|command| command.slash.eq_ignore_ascii_case(&content[range.clone()]))
        })
        .collect();
    if matches.len() > 1 {
        return Err("一次只能执行一个外部 CLI 斜杠命令，请拆成多条消息发送。".into());
    }
    let Some(range) = matches.first() else {
        return Ok(None);
    };
    let prefix = content[..range.start].trim();
    let command = &content[range.clone()];
    let suffix = content[range.end..].trim();
    if prefix.is_empty() {
        let normalized = if suffix.is_empty() {
            command.to_owned()
        } else {
            format!("{command} {suffix}")
        };
        return Ok((normalized != content.trim()).then_some(normalized));
    }
    // Keep positional arguments immediately after the command, then the context
    // written before it. Never drop or persistently reorder the user's draft.
    Ok(Some(if suffix.is_empty() {
        format!("{command} {prefix}")
    } else {
        format!("{command} {suffix}\n{prefix}")
    }))
}

pub fn parse_slash_commands_from_init(value: &Value) -> Vec<ExternalCliSlashCommand> {
    let Some(items) = value.get("slash_commands").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for item in items {
        let parsed = parse_slash_command_item(item);
        if let Some(cmd) = parsed {
            if seen.insert(cmd.name.clone()) {
                out.push(cmd);
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn parse_slash_command_item(item: &Value) -> Option<ExternalCliSlashCommand> {
    if let Some(name) = item.as_str().filter(|s| !s.trim().is_empty()) {
        return Some(ExternalCliSlashCommand {
            slash: format!("/{name}"),
            name: name.to_string(),
            description: None,
            argument_hint: None,
        });
    }
    let obj = item.as_object()?;
    let name = obj
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())?;
    Some(ExternalCliSlashCommand {
        slash: format!("/{name}"),
        name: name.to_string(),
        description: obj
            .get("description")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        argument_hint: obj
            .get("argument_hint")
            .or_else(|| obj.get("argumentHint"))
            .and_then(|v| v.as_str())
            .map(str::to_string),
    })
}

pub fn is_claude_init(value: &Value) -> bool {
    value.get("type").and_then(|v| v.as_str()) == Some("system")
        && value.get("subtype").and_then(|v| v.as_str()) == Some("init")
}
