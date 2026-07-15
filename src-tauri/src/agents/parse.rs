//! Parse an `AgentDefinition` from a Markdown file with YAML-ish frontmatter,
//! reusing the Skill frontmatter parser so the two systems stay consistent.
//!
//! ```text
//! ---
//! name: research-agent
//! description: Deep-dive research and source synthesis
//! tools: read_file, web_search, web_fetch
//! model: gpt-4o
//! ---
//!
//! You are specialized in deep fact-checking research...
//! ```

use crate::skills::parse::{parse_list_value, split_frontmatter};
use crate::skills::slugify;

use super::types::AgentDefinition;

/// Parse one agent `.md`. Returns `None` only when there is no usable name
/// (every other field has a sensible default), so a partially-specified file
/// still loads. `name`/`description` fall back to the file stem.
pub fn parse_agent_markdown(
    fallback_id: &str,
    raw: &str,
    source: &str,
    path: Option<String>,
) -> Option<AgentDefinition> {
    let (frontmatter, body) = split_frontmatter(raw);

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
    let model = frontmatter
        .get("model")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let tools = parse_list_value(frontmatter.get("tools"));
    let system_prompt = body.trim().to_string();

    let _ = path; // reserved for future "open definition" UX; kept for symmetry with skills
    Some(AgentDefinition {
        id,
        name,
        description,
        system_prompt,
        model,
        tools,
        source: source.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_frontmatter_and_body() {
        let raw = "---\nname: research-agent\ndescription: Deep research\ntools: read_file, web_search, web_fetch\nmodel: gpt-4o\n---\n\nYou are a research specialist.\n";
        let def = parse_agent_markdown("fallback", raw, "user", None).unwrap();
        assert_eq!(def.id, "research-agent");
        assert_eq!(def.name, "research-agent");
        assert_eq!(def.description, "Deep research");
        assert_eq!(def.model.as_deref(), Some("gpt-4o"));
        assert_eq!(
            def.tools,
            vec!["read_file", "web_search", "web_fetch"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        );
        assert_eq!(def.system_prompt, "You are a research specialist.");
        assert_eq!(def.source, "user");
    }

    #[test]
    fn falls_back_to_file_stem_when_name_missing() {
        let raw = "Just a body, no frontmatter.";
        let def = parse_agent_markdown("my-agent", raw, "project", None).unwrap();
        assert_eq!(def.id, "my-agent");
        assert_eq!(def.name, "my-agent");
        assert!(def.tools.is_empty());
        assert_eq!(def.system_prompt, "Just a body, no frontmatter.");
    }

    #[test]
    fn parses_bracket_tool_list() {
        let raw = "---\nname: x\ntools: [read_file, edit_file]\n---\nbody";
        let def = parse_agent_markdown("x", raw, "user", None).unwrap();
        assert_eq!(
            def.tools,
            vec!["read_file".to_string(), "edit_file".to_string()]
        );
    }
}
