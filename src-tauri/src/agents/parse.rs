//! Parse OMP agent Markdown and its YAML frontmatter.

use std::collections::BTreeMap;

use serde_yaml::Value;
use crate::skills::slugify;

use super::types::AgentDefinition;

/// Parse one agent `.md`, falling back to the file stem when its name is omitted.
pub fn parse_agent_markdown(
    fallback_id: &str,
    raw: &str,
    source: &str,
    path: Option<String>,
) -> Option<AgentDefinition> {
    let (frontmatter, body) = match split_agent_frontmatter(raw) {
        Ok(parts) => parts,
        Err(error) => {
            eprintln!("Invalid agent {}: {error}", path.as_deref().unwrap_or(fallback_id));
            return None;
        }
    };

    let string = |key: &str| frontmatter.get(key).and_then(Value::as_str)
        .map(str::trim).filter(|s| !s.is_empty());
    let name = string("name").unwrap_or(fallback_id).to_string();
    if name.is_empty() {
        return None;
    }
    let id = slugify(&name);
    let description = string("description").unwrap_or_default().to_string();
    let models = list(frontmatter.get("model"));
    let model = models.first().cloned();
    let thinking_level = string("thinkingLevel").or_else(|| string("thinking-level"))
        .or_else(|| string("thinking")).and_then(normalize_thinking).map(str::to_string);
    let tools = match frontmatter.get("tools") {
        None | Some(Value::Null) => None,
        value => {
            let mut tools = Vec::new();
            for name in list(value) {
                let name = normalize_tool_name(&name);
                if !tools.contains(&name) {
                    tools.push(name);
                }
            }
            Some(tools)
        }
    };
    let spawns = match frontmatter.get("spawns") {
        None | Some(Value::Null) => {
            tools.as_ref().filter(|tools| tools.iter().any(|tool| tool == "task"))
                .map(|_| "*".to_string())
        }
        Some(Value::Bool(true)) => Some("*".into()),
        Some(Value::Bool(false)) => Some(String::new()),
        value => Some(list(value).join(",")),
    };
    let skills = list(frontmatter.get("autoloadSkills").or_else(|| frontmatter.get("autoload-skills"))
        .or_else(|| frontmatter.get("skills")));
    let blocking = frontmatter.get("blocking").and_then(parse_bool).unwrap_or(false);
    let mut unsupported = Vec::new();
    if models.len() > 1 {
        unsupported.push("multiple model fallbacks".into());
    }
    for (key, value) in &frontmatter {
        let unsupported_value = match key.as_str() {
            "name" | "description" | "tools" | "spawns" => false,
            "model" | "autoloadSkills" | "autoload-skills" | "skills" => {
                !matches!(value, Value::Null | Value::String(_) | Value::Sequence(_))
            }
            "thinkingLevel" | "thinking-level" | "thinking" => match value {
                Value::Null => false,
                Value::String(value) if value.trim().is_empty() => false,
                Value::String(value) => normalize_thinking(value.trim()).is_none(),
                _ => true,
            },
            "blocking" => !value.is_null() && parse_bool(value).is_none(),
            // Native read is already verbatim, matching readSummarize: false.
            "readSummarize" | "read-summarize" => {
                !value.is_null() && parse_bool(value) != Some(false)
            }
            "structured" | "prewalk" | "advisor" => {
                !value.is_null() && parse_bool(value) != Some(false)
            }
            "output" | "disallowed_tools" | "disallowed-tools" | "disallowedTools" => !value.is_null(),
            _ => false,
        };
        if unsupported_value {
            unsupported.push(key.clone());
        }
    }
    let system_prompt = body.trim().to_string();

    Some(AgentDefinition {
        id,
        name,
        description,
        system_prompt,
        model,
        thinking_level,
        blocking,
        tools,
        spawns,
        skills,
        source: source.to_string(),
        unsupported,
    })
}

fn split_agent_frontmatter(raw: &str) -> Result<(BTreeMap<String, Value>, &str), String> {
    let trimmed = raw.trim_start();
    let Some(rest) = trimmed.strip_prefix("---") else {
        return Ok((BTreeMap::new(), raw));
    };
    let end = rest.find("\n---").ok_or("Unclosed frontmatter")?;
    let frontmatter = serde_yaml::from_str::<Option<BTreeMap<String, Value>>>(&rest[..end])
        .map_err(|error| error.to_string())?.unwrap_or_default();
    Ok((frontmatter, &rest[end + 4..]))
}

fn list(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(s)) => s.split(',').map(str::trim)
            .filter(|s| !s.is_empty()).map(str::to_string).collect(),
        Some(Value::Sequence(items)) => items.iter().filter_map(Value::as_str)
            .map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect(),
        _ => Vec::new(),
    }
}

fn parse_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        Value::String(value) if value.trim().eq_ignore_ascii_case("true") => Some(true),
        Value::String(value) if value.trim().eq_ignore_ascii_case("false") => Some(false),
        _ => None,
    }
}

fn normalize_thinking(value: &str) -> Option<&'static str> {
    const LEVELS: &[&str] = &["inherit", "off", "minimal", "low", "medium", "high", "xhigh", "max"];
    if value == "auto" {
        return Some("auto");
    }
    if value.len() < 2 {
        return None;
    }
    let mut matches = LEVELS.iter().copied().filter(|level| level.starts_with(value));
    let level = matches.next()?;
    matches.next().is_none().then_some(level)
}

fn normalize_tool_name(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "search" => "grep".into(),
        "read" | "bash" | "edit" | "ast_grep" | "ast_edit" | "ask" | "debug" | "ida"
        | "eval" | "github" | "glob" | "grep" | "find" | "lsp" | "checkpoint"
        | "rewind" | "context_notes" | "new_context" | "security_scan" | "task"
        | "wait" | "todo" | "web_search" | "write" | "memory_edit" | "retain"
        | "recall" | "reflect" | "learn" | "manage_skill" | "yield" | "goal"
        | "think" => lower,
        _ => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_frontmatter_and_body() {
        let raw = "---\nname: research-agent\ndescription: Deep research\ntools: read, web_search, web_fetch\nspawns: [scout, reviewer]\nautoload-skills: [pdf, docx]\nthinking-level: medium\nmodel: gpt-4o\n---\n\nYou are a research specialist.\n";
        let def = parse_agent_markdown("fallback", raw, "user", None).unwrap();
        assert_eq!(def.id, "research-agent");
        assert_eq!(def.name, "research-agent");
        assert_eq!(def.description, "Deep research");
        assert_eq!(def.model.as_deref(), Some("gpt-4o"));
        assert_eq!(def.thinking_level.as_deref(), Some("medium"));
        assert_eq!(def.tools, Some(vec!["read".into(), "web_search".into(), "web_fetch".into()]));
        assert_eq!(def.spawns.as_deref(), Some("scout,reviewer"));
        assert_eq!(def.skills, vec!["pdf", "docx"]);
        assert_eq!(def.system_prompt, "You are a research specialist.");
        assert_eq!(def.source, "user");
    }

    #[test]
    fn distinguishes_missing_and_explicitly_empty_tools() {
        let absent = parse_agent_markdown("fallback", "Just a body.", "project", None).unwrap();
        assert_eq!(absent.name, "fallback");
        assert_eq!(absent.tools, None);
        for tools in ["tools:", "tools: null", "tools: ~"] {
            let raw = format!("---\nname: x\n{tools}\n---\nbody");
            assert_eq!(parse_agent_markdown("x", &raw, "user", None).unwrap().tools, None, "{tools}");
        }
        for tools in ["tools: ''", "tools: []", "tools: false"] {
            let raw = format!("---\nname: x\n{tools}\n---\nbody");
            assert_eq!(parse_agent_markdown("x", &raw, "user", None).unwrap().tools, Some(vec![]), "{tools}");
        }
        let indented = "---\n  name: x\n  tools:\n  spawns: '*'\n---\nbody";
        assert_eq!(parse_agent_markdown("x", indented, "user", None).unwrap().tools, None);
        let nested_only = "---\nname: x\noutput:\n  tools: [write]\n---\nbody";
        assert_eq!(parse_agent_markdown("x", nested_only, "user", None).unwrap().tools, None);
        let nested_override = "---\nname: x\ntools: []\noutput:\n  tools: [write]\n  name: nested\n  spawns: '*'\n---\nbody";
        let def = parse_agent_markdown("x", nested_override, "user", None).unwrap();
        assert_eq!(def.name, "x");
        assert_eq!(def.tools, Some(vec![]));
        assert_eq!(def.unsupported, ["output"]);
        assert_eq!(def.spawns, None);
    }

    #[test]
    fn parses_block_lists_and_aliases() {
        let raw = "---\nname: x\ntools:\n  - read\n  - glob\nspawns:\n  - scout\n  - reviewer\nskills:\n  - pdf\n  - docx\nthinking: high\n---\nbody";
        let def = parse_agent_markdown("x", raw, "user", None).unwrap();
        assert_eq!(def.tools, Some(vec!["read".into(), "glob".into()]));
        assert_eq!(def.spawns.as_deref(), Some("scout,reviewer"));
        assert_eq!(def.skills, vec!["pdf", "docx"]);
        assert_eq!(def.thinking_level.as_deref(), Some("high"));
        let unindented = "---\nname: x\ntools:\n# allowed tools\n- read\n- glob\nspawns:\n- scout\n---\nbody";
        let def = parse_agent_markdown("x", unindented, "user", None).unwrap();
        assert_eq!(def.tools, Some(vec!["read".into(), "glob".into()]));
        assert_eq!(def.spawns.as_deref(), Some("scout"));
        let auto = parse_agent_markdown("x", "---\nthinkingLevel: auto\nautoloadSkills: [pdf, docx]\nblocking: 'true'\n---\nbody", "user", None).unwrap();
        assert_eq!(auto.thinking_level.as_deref(), Some("auto"));
        assert_eq!(auto.skills, ["pdf", "docx"]);
        assert!(auto.blocking);
        for key in ["thinking-level", "thinkingLevel", "thinking"] {
            let raw = format!("---\n{key}: auto\n---\nbody");
            assert_eq!(parse_agent_markdown("x", &raw, "user", None).unwrap().thinking_level.as_deref(), Some("auto"));
        }
        let abbreviated = parse_agent_markdown("x", "---\nthinking: med\n---\nbody", "user", None).unwrap();
        assert_eq!(abbreviated.thinking_level.as_deref(), Some("medium"));
        let invalid = parse_agent_markdown("x", "---\nthinking: nope\n---\nbody", "user", None).unwrap();
        assert_eq!(invalid.thinking_level, None);
        assert_eq!(invalid.unsupported, ["thinking"]);
    }

    #[test]
    fn spawn_policy_preserves_explicit_denials_and_normalizes_tools() {
        for value in ["false", "''", "[]"] {
            let raw = format!("---\ntools: [TASK]\nspawns: {value}\n---\nbody");
            let def = parse_agent_markdown("x", &raw, "user", None).unwrap();
            assert_eq!(def.tools, Some(vec!["task".into()]));
            assert_eq!(def.spawns.as_deref(), Some(""), "{value}");
        }
        let def = parse_agent_markdown("x", "---\ntools: [TASK, search, grep, Read]\n---\nbody", "user", None).unwrap();
        assert_eq!(def.tools, Some(vec!["task".into(), "grep".into(), "read".into()]));
        assert_eq!(def.spawns.as_deref(), Some("*"));
        let def = parse_agent_markdown("x", "---\nspawns: true\n---\nbody", "user", None).unwrap();
        assert_eq!(def.spawns.as_deref(), Some("*"));
    }

    #[test]
    fn unsupported_capabilities_are_reported_but_unknown_metadata_is_ignored() {
        let raw = "---\nmodel: [first, fallback]\nstructured: true\noutput:\n  type: object\nprewalk: true\nadvisor: other\nextraField: value\n---\nbody";
        let def = parse_agent_markdown("x", raw, "user", None).unwrap();
        for field in ["multiple model fallbacks", "structured", "output", "prewalk", "advisor"] {
            assert!(def.unsupported.iter().any(|item| item == field), "{field}");
        }
        assert!(!def.unsupported.iter().any(|item| item == "extraField"));
        let disabled = parse_agent_markdown("x", "---\nstructured: false\nprewalk: false\nadvisor: false\nread-summarize: false\n---\nbody", "user", None).unwrap();
        assert!(disabled.unsupported.is_empty());
    }
}
