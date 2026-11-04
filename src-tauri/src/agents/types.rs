//! `AgentDefinition` data model and the built-in agent types.

use serde::Serialize;

/// A subagent persona. An omitted tool list inherits the default toolset;
/// an explicitly empty list grants no tools. `skills` are autoloaded.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub system_prompt: String,
    pub model: Option<String>,
    pub thinking_level: Option<String>,
    pub tools: Option<Vec<String>>,
    pub spawns: Option<String>,
    pub skills: Vec<String>,
    pub source: String,
}

/// Built-in OMP agent personas, with tool names mapped to Kivio's registry.
pub fn builtin_agent_definitions() -> Vec<AgentDefinition> {
    use crate::chat::sub_agent::prompts::{AGENT_REVIEWER, AGENT_SCOUT, AGENT_TASK};

    vec![
        AgentDefinition {
            id: "task".into(),
            name: "task".into(),
            description: "General-purpose subagent with full capabilities for delegated multi-step tasks".into(),
            system_prompt: AGENT_TASK.into(),
            model: Some("@task".into()),
            thinking_level: None,
            tools: None,
            spawns: Some("*".into()),
            skills: Vec::new(),
            source: "builtin".into(),
        },
        AgentDefinition {
            id: "sonic".into(),
            name: "sonic".into(),
            description: "Low-reasoning agent for strictly mechanical updates or data collection only".into(),
            system_prompt: AGENT_TASK.into(),
            model: Some("@smol".into()),
            thinking_level: Some("medium".into()),
            tools: None,
            spawns: None,
            skills: Vec::new(),
            source: "builtin".into(),
        },
        AgentDefinition {
            id: "scout".into(),
            name: "scout".into(),
            description: "MUST be used for exploratory codebase research, rapid code analysis, and broad pattern searches. Fast read-only scout returning compressed context for handoff.".into(),
            system_prompt: AGENT_SCOUT.into(),
            model: Some("@smol".into()),
            thinking_level: Some("medium".into()),
            tools: Some(["read", "glob", "grep", "web_search"].into_iter().map(String::from).collect()),
            spawns: None,
            skills: Vec::new(),
            source: "builtin".into(),
        },
        AgentDefinition {
            id: "reviewer".into(),
            name: "reviewer".into(),
            description: "Code review specialist for quality/security analysis".into(),
            system_prompt: AGENT_REVIEWER.into(),
            model: Some("@slow".into()),
            thinking_level: None,
            tools: Some(["read", "glob", "grep", "bash", "web_search"].into_iter().map(String::from).collect()),
            spawns: Some("scout".into()),
            skills: Vec::new(),
            source: "builtin".into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_expose_omp_personas_and_spawn_policies() {
        let defs = builtin_agent_definitions();
        assert_eq!(defs.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["task", "sonic", "scout", "reviewer"]);
        assert_eq!(defs[0].spawns.as_deref(), Some("*"));
        assert!(defs[0].tools.is_none());
        assert_eq!(defs[1].system_prompt, defs[0].system_prompt);
        assert!(defs[1].spawns.is_none());
        assert_eq!(defs[2].model.as_deref(), Some("@smol"));
        assert_eq!(defs[2].thinking_level.as_deref(), Some("medium"));
        let scout_tools = defs[2].tools.as_ref().unwrap();
        assert!(scout_tools.iter().any(|tool| tool == "glob"));
        assert!(!scout_tools.iter().any(|tool| tool == "write"));
        assert_eq!(defs[3].spawns.as_deref(), Some("scout"));
        assert!(defs[3].tools.as_ref().unwrap().iter().any(|tool| tool == "bash"));
    }
}
