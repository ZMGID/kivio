//! Parse an `AgentDefinition` from a Markdown file with YAML-ish frontmatter,
//! reusing the Skill frontmatter parser so the two systems stay consistent.
//!
//! ```text
//! ---
//! name: research-agent
//! description: Deep-dive research and source synthesis
//! tools: read, web_search, web_fetch
//! spawns: scout
//! autoload-skills: pdf
//! thinking-level: medium
//! model: gpt-4o
//! ---
//!
//! You are specialized in deep fact-checking research...
//! ```

use crate::skills::parse::{parse_list_value, split_frontmatter};
use crate::skills::slugify;

use super::types::AgentDefinition;

/// Parse one agent `.md`, falling back to the file stem when its name is omitted.
pub fn parse_agent_markdown(
    fallback_id: &str,
    raw: &str,
    source: &str,
    path: Option<String>,
) -> Option<AgentDefinition> {
    let (frontmatter, body, declares_tools) = split_agent_frontmatter(raw);

    let name = frontmatter
        .get("name")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| fallback_id.to_string());
    if name.is_empty() {
        return None;
    }
    let id = slugify(&name);
    let description = frontmatter
        .get("description")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let optional = |key: &str| {
        frontmatter.get(key).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    };
    let model = optional("model");
    let thinking_level = optional("thinking-level").or_else(|| optional("thinking"))
        .filter(|value| value != "auto");
    let tools = declares_tools.then(|| parse_list_value(frontmatter.get("tools")));
    let spawns = frontmatter.get("spawns").map(|value| parse_list_value(Some(value)).join(","))
        .filter(|value| !value.is_empty());
    let skills = parse_list_value(frontmatter.get("autoload-skills").or_else(|| frontmatter.get("skills")));
    let system_prompt = body.trim().to_string();

    let _ = path; // reserved for future "open definition" UX; kept for symmetry with skills
    Some(AgentDefinition {
        id,
        name,
        description,
        system_prompt,
        model,
        thinking_level,
        tools,
        spawns,
        skills,
        source: source.to_string(),
    })
}

// The shared parser flattens nested keys and drops empty scalars. Preserve the
// top-level field boundary and an explicitly empty tools list.
fn split_agent_frontmatter(raw: &str) -> (std::collections::HashMap<String, String>, &str, bool) {
    let (_, body) = split_frontmatter(raw);
    let Some((front, _)) = raw.trim_start().strip_prefix("---").and_then(|s| s.split_once("\n---")) else {
        return (Default::default(), body, false);
    };
    let indent = front.lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            !trimmed.is_empty() && !trimmed.starts_with('#') && !trimmed.starts_with('-')
        })
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);
    let mut top_level = String::from("---\n");
    let mut declares_tools = false;
    let mut in_list = false;
    for line in front.lines() {
        let trimmed = line.trim_start();
        let depth = line.len() - trimmed.len();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if depth >= indent && in_list && trimmed.starts_with("- ") {
            top_level.push_str(line);
            top_level.push('\n');
        } else if depth == indent {
            if let Some((key, _)) = trimmed.split_once(':') {
                let key = key.trim();
                declares_tools |= key == "tools";
                in_list = matches!(key, "tools" | "spawns" | "autoload-skills" | "skills");
                top_level.push_str(line);
                top_level.push('\n');
            }
        }
    }
    top_level.push_str("---\n");
    let (frontmatter, _) = split_frontmatter(&top_level);
    (frontmatter, body, declares_tools)
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
        for tools in ["tools:", "tools: ''", "tools: []"] {
            let raw = format!("---\nname: x\n{tools}\nspawns: '*'\n---\nbody");
            let def = parse_agent_markdown("x", &raw, "user", None).unwrap();
            assert_eq!(def.tools, Some(vec![]), "{tools}");
            assert_eq!(def.spawns.as_deref(), Some("*"));
        }
        let indented = "---\n  name: x\n  tools:\n  spawns: '*'\n---\nbody";
        assert_eq!(parse_agent_markdown("x", indented, "user", None).unwrap().tools, Some(vec![]));
        let nested_only = "---\nname: x\noutput:\n  tools: [write]\n---\nbody";
        assert_eq!(parse_agent_markdown("x", nested_only, "user", None).unwrap().tools, None);
        let nested_override = "---\nname: x\ntools: []\noutput:\n  tools: [write]\n  name: nested\n  spawns: '*'\n---\nbody";
        let def = parse_agent_markdown("x", nested_override, "user", None).unwrap();
        assert_eq!(def.name, "x");
        assert_eq!(def.tools, Some(vec![]));
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
        let auto = parse_agent_markdown("x", "---\nthinking-level: auto\n---\nbody", "user", None).unwrap();
        assert_eq!(auto.thinking_level, None);
    }
}
