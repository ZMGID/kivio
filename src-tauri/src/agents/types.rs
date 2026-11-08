//! `AgentDefinition` data model and the built-in agent types.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AgentModelPolicy {
    Enabled(bool),
    Model(String),
}

/// A subagent persona. An omitted builtin tool list selects defaults; an
/// explicitly empty list selects no builtin tools. `skills` are autoloaded.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub system_prompt: String,
    pub model: Option<String>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub structured: bool,
    pub output: Option<serde_json::Value>,
    #[serde(default)]
    pub prewalk: Option<AgentModelPolicy>,
    #[serde(default)]
    pub advisor: Option<AgentModelPolicy>,
    #[serde(default)]
    pub read_summarize: Option<bool>,
    #[serde(default)]
    pub metadata: std::collections::BTreeMap<String, serde_json::Value>,
    pub thinking_level: Option<String>,
    pub blocking: bool,
    pub tools: Option<Vec<String>>,
    pub spawns: Option<String>,
    pub skills: Vec<String>,
    pub source: String,
    /// Features requested by this role that the runtime cannot execute.
    pub unsupported: Vec<String>,
}

/// Built-in OMP agent personas, with tool names mapped to Kivio's registry.
pub fn builtin_agent_definitions() -> Vec<AgentDefinition> {
    use crate::chat::sub_agent::prompts::{
        AGENT_REVIEWER, AGENT_SCOUT, AGENT_SEANCE, AGENT_SECURITY_REVIEWER, AGENT_TASK,
    };

    vec![
        AgentDefinition {
            id: "task".into(),
            name: "task".into(),
            description: "General-purpose subagent with full capabilities for delegated multi-step tasks".into(),
            system_prompt: AGENT_TASK.into(),
            model: Some("@task".into()),
            models: Vec::new(), structured: false, output: None,
            prewalk: None, advisor: None, read_summarize: None, metadata: Default::default(),
            thinking_level: Some("auto".into()),
            blocking: false,
            tools: None,
            spawns: Some("*".into()),
            skills: Vec::new(),
            source: "builtin".into(),
            unsupported: Vec::new(),
        },
        AgentDefinition {
            id: "sonic".into(),
            name: "sonic".into(),
            description: "Low-reasoning agent for strictly mechanical updates or data collection only".into(),
            system_prompt: AGENT_TASK.into(),
            model: Some("@smol".into()),
            models: Vec::new(), structured: false, output: None,
            prewalk: None, advisor: None, read_summarize: None, metadata: Default::default(),
            thinking_level: Some("medium".into()),
            blocking: false,
            tools: None,
            spawns: None,
            skills: Vec::new(),
            source: "builtin".into(),
            unsupported: Vec::new(),
        },
        AgentDefinition {
            id: "scout".into(),
            name: "scout".into(),
            description: "MUST be used for exploratory codebase research, rapid code analysis, and broad pattern searches. Fast read-only scout returning compressed context for handoff.".into(),
            system_prompt: AGENT_SCOUT.into(),
            model: Some("@smol".into()),
            models: Vec::new(), structured: false, output: Some(serde_json::from_str(include_str!("../chat/sub_agent/prompts/agents/scout.schema.json")).expect("builtin OMP output schema")),
            prewalk: None, advisor: None, read_summarize: None, metadata: Default::default(),
            thinking_level: Some("medium".into()),
            blocking: false,
            tools: Some(["read", "find", "grep", "glob", "ast_grep", "web_search"].into_iter().map(String::from).collect()),
            spawns: None,
            skills: Vec::new(),
            source: "builtin".into(),
            unsupported: Vec::new(),
        },
        AgentDefinition {
            id: "reviewer".into(),
            name: "reviewer".into(),
            description: "Code review specialist for quality/security analysis".into(),
            system_prompt: AGENT_REVIEWER.into(),
            model: Some("@slow".into()),
            models: Vec::new(), structured: false, output: Some(serde_json::from_str(include_str!("../chat/sub_agent/prompts/agents/reviewer.schema.json")).expect("builtin OMP output schema")),
            prewalk: None, advisor: None, read_summarize: None, metadata: Default::default(),
            thinking_level: None,
            blocking: false,
            tools: Some(["read", "find", "grep", "glob", "bash", "lsp", "web_search", "ast_grep"].into_iter().map(String::from).collect()),
            spawns: Some("scout".into()),
            skills: Vec::new(),
            source: "builtin".into(),
            unsupported: Vec::new(),
        },
        AgentDefinition {
            id: "security-reviewer".into(),
            name: "security-reviewer".into(),
            description: "Read-only security specialist for evidence-backed repository vulnerability discovery".into(),
            system_prompt: AGENT_SECURITY_REVIEWER.into(),
            model: None,
            models: Vec::new(), structured: false, output: Some(serde_json::from_str(include_str!("../chat/sub_agent/prompts/agents/security-reviewer.schema.json")).expect("builtin OMP output schema")),
            prewalk: None, advisor: None, read_summarize: None, metadata: Default::default(),
            thinking_level: None,
            blocking: false,
            tools: Some(["read", "find", "grep", "glob", "lsp", "ast_grep"].into_iter().map(String::from).collect()),
            spawns: None,
            skills: Vec::new(),
            source: "builtin".into(),
            unsupported: Vec::new(),
        },
        AgentDefinition {
            id: "seance".into(),
            name: "seance".into(),
            description: "Read a historical session and nested transcripts to answer a consult.".into(),
            system_prompt: AGENT_SEANCE.into(),
            model: None,
            models: Vec::new(), structured: false, output: None,
            prewalk: None, advisor: None, read_summarize: None, metadata: Default::default(),
            thinking_level: None,
            blocking: false,
            tools: Some(["read", "grep", "glob"].into_iter().map(String::from).collect()),
            spawns: None,
            skills: Vec::new(),
            source: "builtin".into(),
            unsupported: Vec::new(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_structured_roles_require_their_report_fields() {
        for definition in builtin_agent_definitions()
            .into_iter()
            .filter(|definition| {
                matches!(
                    definition.id.as_str(),
                    "scout" | "reviewer" | "security-reviewer"
                )
            })
        {
            let schema = definition.output.as_ref().expect("builtin report schema");
            crate::chat::sub_agent::structured::validate_schema(schema).unwrap();
            assert!(
                crate::chat::sub_agent::structured::validate_result(
                    schema,
                    &serde_json::json!({}),
                )
                .is_err(),
                "{} must reject a report missing its required fields",
                definition.id,
            );
        }
    }

    #[test]
    fn builtin_report_schemas_validate_real_reports_and_nested_constraints() {
        use serde_json::json;
        let definitions = builtin_agent_definitions();
        for (role, valid, invalid) in [
            (
                "scout",
                json!({"summary":"Found call sites","files":[{"path":"src/lib.rs:1-5","description":"Entry"}],"architecture":"Single entry","report":"Full report"}),
                json!({"summary":"Found call sites","files":[{"path":"src/lib.rs:1-5"}],"architecture":"Single entry"}),
            ),
            (
                "reviewer",
                json!({"overall_correctness":"incorrect","explanation":"A bug","confidence":0.75,"findings":[{"title":"Fix result","body":"Trigger and impact","priority":2,"confidence":0.85,"file_path":"src/lib.rs","line_start":1,"line_end":2}]}),
                json!({"overall_correctness":"correct","explanation":"A verdict","confidence":"high"}),
            ),
            (
                "security-reviewer",
                json!({"coverage_summary":"Reviewed inputs","findings":[{"rule_id":"R1","title":"Validate input","summary":"Untrusted input","severity":"high","confidence":"medium","category":"validation","locations":[{"path":"src/lib.rs","start_line":1,"end_line":2}],"cwe":["CWE-20"],"evidence":[{"label":"Input","explanation":"Reaches sink"}]}],"reviewed_paths":["src/lib.rs"],"deferred":[{"reason":"Out of scope","paths":["vendor"]}]}),
                json!({"coverage_summary":"Reviewed inputs","findings":[{"rule_id":"R1","title":"Validate input","summary":"Untrusted input","severity":"urgent","confidence":"medium","category":"validation","locations":[],"cwe":[],"evidence":[]}]}),
            ),
        ] {
            let definition = definitions
                .iter()
                .find(|definition| definition.id == role)
                .unwrap();
            assert!(!definition.structured);
            let schema = definition.output.as_ref().unwrap();
            crate::chat::sub_agent::structured::validate_result(schema, &valid).unwrap();
            assert!(
                crate::chat::sub_agent::structured::validate_result(schema, &invalid).is_err(),
                "{role}"
            );
            let mut extra = valid;
            extra["unrecognized"] = json!(true);
            assert!(
                crate::chat::sub_agent::structured::validate_result(schema, &extra).is_err(),
                "{role}"
            );
        }
    }

    #[test]
    fn builtins_expose_omp_personas_and_spawn_policies() {
        let defs = builtin_agent_definitions();
        assert_eq!(
            defs.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
            [
                "task",
                "sonic",
                "scout",
                "reviewer",
                "security-reviewer",
                "seance"
            ]
        );
        assert_eq!(defs[0].spawns.as_deref(), Some("*"));
        assert!(defs[0].tools.is_none());
        assert_eq!(defs[0].thinking_level.as_deref(), Some("auto"));
        assert_eq!(defs[1].system_prompt, defs[0].system_prompt);
        assert!(defs[1].spawns.is_none());
        assert_eq!(defs[2].model.as_deref(), Some("@smol"));
        assert_eq!(defs[2].thinking_level.as_deref(), Some("medium"));
        let scout_tools = defs[2].tools.as_ref().unwrap();
        assert!(scout_tools.iter().any(|tool| tool == "glob"));
        assert!(!scout_tools.iter().any(|tool| tool == "write"));
        assert_eq!(defs[3].spawns.as_deref(), Some("scout"));
        assert!(defs[3]
            .tools
            .as_ref()
            .unwrap()
            .iter()
            .any(|tool| tool == "bash"));
        assert!(defs[4].system_prompt.contains("attacker-controlled"));
        assert_eq!(defs[5].name, "seance");
        assert!(defs[5].unsupported.is_empty());
    }
}
