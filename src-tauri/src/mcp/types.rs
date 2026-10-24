use serde::{Deserialize, Serialize};

use crate::settings::{ChatMcpServer, ChatNativeToolsConfig};

#[path = "tool_contract.rs"]
mod contract;
#[cfg(test)]
use contract::MAX_WIRE_TOOL_NAME;
pub use contract::*;


#[path = "tool_schema.rs"]
mod schema;
pub use schema::McpTool;

#[path = "tool_definitions.rs"]
mod definitions;
pub use definitions::*;

/// Builtin native tool exposure: iterates the static registry in
/// `mcp/native_registry.rs` (declaration order = model-facing order).
pub fn list_native_builtin_tool_defs(
    native: &ChatNativeToolsConfig,
    web_search_configured: bool,
    memory_enabled: bool,
) -> Vec<ChatToolDefinition> {
    super::native_registry::NATIVE_TOOLS
        .iter()
        .filter(|entry| (entry.enabled)(native, web_search_configured, memory_enabled))
        .map(|entry| (entry.def)())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_tool_detection_covers_common_write_verbs() {
        for name in ["save_file", "uploadAsset", "publish_page", "replace_rows"] {
            assert!(
                looks_sensitive_tool(name),
                "{name} should require confirmation"
            );
        }
        assert!(!looks_sensitive_tool("read_file"));
        assert!(!looks_sensitive_tool("web_search"));
    }

    #[test]
    fn mixer_image_tool_description_names_model_and_shows_calls() {
        let unnamed = mixer_generate_image_tool();
        assert!(unnamed.description.contains("Mixer image model"));
        assert!(unnamed
            .description
            .contains("{\"prompt\":\"a fox in snow\"}"));
        assert!(unnamed.description.contains("aspect_ratio"));
        assert!(unnamed.description.contains("2K"));
        assert!(unnamed.description.contains("paths"));
        assert!(unnamed.description.contains("artifact_ids"));

        let named = mixer_generate_image_tool_for(Some("gpt-image-2"));
        assert!(
            named.description.contains("gpt-image-2"),
            "{}",
            named.description
        );
        assert!(named.description.contains("do not pass a model name"));
        let wire = named.to_openai_tool();
        assert_eq!(wire["function"]["name"], "mixer_generate_image");
        assert!(wire["function"]["description"]
            .as_str()
            .is_some_and(|text| text.contains("gpt-image-2")));
    }

    #[test]
    fn skill_and_native_tools_use_prompt_facing_names() {
        assert_eq!(native_skill_activate_tool().openai_tool_name(), "skill");
        // 保留名规避：native web_search 在 wire/prompt 上声明为别名 search_web。
        assert_eq!(native_web_search_tool().openai_tool_name(), "search_web");
    }

    #[test]
    fn skill_activate_legacy_name_maps_to_skill() {
        assert_eq!(canonical_tool_name("skill_activate"), "skill");
    }

    #[test]
    fn native_skill_tools_expose_single_tool() {
        let tools = native_skill_tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "skill");
    }

    #[test]
    fn builtin_skill_tools_are_not_marked_sensitive() {
        assert!(!native_skill_activate_tool().sensitive);
    }

    #[test]
    fn automation_upsert_tool_description_stays_short() {
        let def = native_automation_upsert_tool();
        assert!(
            def.description.contains("`automation` skill"),
            "{}",
            def.description
        );
        assert!(
            !def.description.contains("Allowed node types"),
            "schema belongs in the skill and validation errors, not the always-on tool"
        );
        assert!(
            !def.description.contains("Minimal example"),
            "{}",
            def.description
        );
        assert!(
            def.description.len() < 500,
            "upsert description grew to {} chars",
            def.description.len()
        );
    }

    #[test]
    fn present_artifacts_tool_tells_model_to_pass_ids_only() {
        let def = native_present_artifacts_tool();
        assert!(
            def.description.contains(r#"{"artifact_ids":["art_…"]}"#),
            "{}",
            def.description
        );
        assert!(def.description.contains("base64"), "{}", def.description);
        assert!(
            def.input_schema["properties"]["artifact_ids"]["description"]
                .as_str()
                .unwrap()
                .contains("`art_…`"),
        );
        assert!(def.input_schema["properties"]["caption"]["description"]
            .as_str()
            .unwrap()
            .contains("plain-text"),);
    }

    #[test]
    fn image_tool_unused_sources_allow_empty_arrays() {
        for tool in [
            native_read_file_tool(),
            native_present_artifacts_tool(),
            mixer_generate_image_tool(),
        ] {
            for field in ["artifact_ids", "paths"] {
                let minimum = tool.input_schema["properties"][field]["minItems"]
                    .as_u64()
                    .unwrap_or(0);
                assert_eq!(
                    minimum, 0,
                    "{}.{field} must not force placeholder entries",
                    tool.name
                );
            }
        }
    }

    #[test]
    fn native_file_and_web_tools_have_expected_sensitivity() {
        assert!(!native_read_file_tool().sensitive);
        assert!(!native_web_fetch_tool().sensitive);
        assert!(!native_memory_read_tool().sensitive);
        assert!(!native_memory_modify_tool().sensitive);
        assert!(native_write_file_tool().sensitive);
        assert!(native_edit_file_tool().sensitive);
        assert!(native_run_command_tool().sensitive);
    }

    #[test]
    fn file_tool_path_descriptions_are_scope_specific() {
        let read_schema = native_read_file_tool().input_schema;
        let grep = native_search_files_tool();
        let find = native_glob_files_tool();

        assert!(read_schema["properties"]["path"]["description"]
            .as_str()
            .unwrap()
            .contains("File path"));
        assert!(read_schema["properties"].get("paths").is_some());
        assert!(read_schema.get("required").is_none());
        // read 现在也列目录，描述里应提到目录
        assert!(native_read_file_tool().description.contains("directory"));
        assert!(grep.description.contains("file or under a directory"));
        assert!(grep.description.contains("pass that file path directly"));
        assert!(grep.input_schema["properties"]["path"]["description"]
            .as_str()
            .unwrap()
            .contains("File or directory path"));
        assert!(find.input_schema["properties"]["path"]["description"]
            .as_str()
            .unwrap()
            .contains("Directory"));
        for tool in [
            native_read_file_tool(),
            grep,
            find,
            native_write_file_tool(),
            native_edit_file_tool(),
            native_run_command_tool(),
        ] {
            let declaration = tool.to_openai_tool().to_string();
            assert!(
                !declaration.contains("when allowed by workspace mode"),
                "{}",
                tool.name
            );
            assert!(
                !declaration.contains("validated as workspace-local"),
                "{}",
                tool.name
            );
            assert!(
                declaration.contains("absolute"),
                "{} must describe explicit paths",
                tool.name
            );
        }
        assert!(!native_skill_activate_tool()
            .description
            .contains("`run_command`"));
    }

    #[test]
    fn write_file_tool_description_discourages_inline_code_requests() {
        let tool = native_write_file_tool();

        assert!(tool.description.contains("necessary supporting files"));
        assert!(tool.description.contains("answer inline"));
        assert!(tool.description.contains("code block"));
        assert!(tool.description.contains("prefer edit"));
        assert!(tool
            .description
            .contains("structured file mutation metadata"));
    }

    #[test]
    fn write_gate_exposes_exactly_whole_file_and_path_tools() {
        // Start from an explicitly write-disabled config so this gate test is
        // independent of the default-on baseline.
        let mut native = crate::settings::ChatNativeToolsConfig::default();
        native.write_file = false;
        native.edit_file = false;
        let defs = list_native_builtin_tool_defs(&native, false, false);
        assert!(defs.is_empty() || !defs.iter().any(|tool| tool.name == "write"));

        native.write_file = true;
        native.edit_file = true;
        let defs = list_native_builtin_tool_defs(&native, false, false);
        let names: Vec<&str> = defs.iter().map(|tool| tool.name.as_str()).collect();
        for expected in ["write", "edit"] {
            assert!(names.contains(&expected), "{expected} should be exposed");
        }
        for removed in [
            "create_dir",
            "delete_path",
            "move_path",
            "copy_path",
            "patch",
            "write_file_chunk",
            "begin_file_write",
            "append_file_write",
            "finish_file_write",
            "abort_file_write",
        ] {
            assert!(!names.contains(&removed), "{removed} must not be exposed");
        }
    }

    #[test]
    fn default_native_config_exposes_file_and_command_tools() {
        // Regression for the "sub-agent stuck on skill_activate" bug: with the
        // agentic default config (all native tools ON), the agent — and its
        // sub-agents, which inherit the same tool table — must actually receive
        // read/ls/grep/find/write/edit/bash. Before the green-light default, only
        // skill tools were exposed, so weak models looped guessing skill names.
        // web_search is intentionally NOT asserted here: it stays gated behind a
        // configured provider key (web_search_configured=false below).
        let native = crate::settings::ChatNativeToolsConfig::default();
        let defs = list_native_builtin_tool_defs(&native, false, false);
        let names: Vec<&str> = defs.iter().map(|tool| tool.name.as_str()).collect();
        for expected in ["read", "grep", "glob", "write", "edit", "bash"] {
            assert!(
                names.contains(&expected),
                "default config must expose `{expected}` (got {names:?})"
            );
        }
    }

    #[test]
    fn mcp_tools_keep_namespaced_openai_names() {
        let server = ChatMcpServer {
            id: "server.one".to_string(),
            name: "Server One".to_string(),
            enabled: true,
            transport: "stdio".to_string(),
            url: String::new(),
            command: "demo".to_string(),
            args: Vec::new(),
            env: std::collections::HashMap::new(),
            headers: std::collections::HashMap::new(),
            cwd: None,
            enabled_tools: Vec::new(),
            connector_id: None,
            auth: None,
        };
        let tool = tool_definition_from_mcp(
            &server,
            McpTool {
                name: "search.web".to_string(),
                description: String::new(),
                input_schema: serde_json::json!({ "type": "object" }),
                output_schema: None,
                annotations: None,
            },
        );

        assert_eq!(tool.openai_tool_name(), "mcp__server_one__search_web");
        assert_ne!(tool.openai_tool_name(), tool.name);
    }

    #[test]
    fn mcp_tool_definition_preserves_metadata_and_read_only_hint() {
        let server = ChatMcpServer {
            id: "demo".to_string(),
            name: "Demo".to_string(),
            enabled: true,
            transport: "stdio".to_string(),
            url: String::new(),
            command: "demo".to_string(),
            args: Vec::new(),
            env: std::collections::HashMap::new(),
            headers: std::collections::HashMap::new(),
            cwd: None,
            enabled_tools: Vec::new(),
            connector_id: None,
            auth: None,
        };
        let annotations = serde_json::json!({ "readOnlyHint": true });
        let output_schema = serde_json::json!({
            "type": "object",
            "properties": { "items": { "type": "array" } }
        });

        let tool = tool_definition_from_mcp(
            &server,
            McpTool {
                name: "search".to_string(),
                description: "Search".to_string(),
                input_schema: serde_json::json!({ "type": "object" }),
                output_schema: Some(output_schema.clone()),
                annotations: Some(annotations.clone()),
            },
        );

        assert_eq!(tool.annotations.as_ref(), Some(&annotations));
        assert_eq!(tool.output_schema.as_ref(), Some(&output_schema));
        assert_eq!(tool.read_only_hint(), Some(true));
        assert!(tool.is_read_only_tool());
        assert!(tool.sensitive);
    }

    #[test]
    fn mcp_destructive_and_open_world_annotations_are_sensitive() {
        let server = ChatMcpServer {
            id: "demo".to_string(),
            name: "Demo".to_string(),
            enabled: true,
            transport: "stdio".to_string(),
            url: String::new(),
            command: "demo".to_string(),
            args: Vec::new(),
            env: std::collections::HashMap::new(),
            headers: std::collections::HashMap::new(),
            cwd: None,
            enabled_tools: Vec::new(),
            connector_id: None,
            auth: None,
        };

        for annotations in [
            serde_json::json!({ "destructiveHint": true }),
            serde_json::json!({ "openWorldHint": true }),
            serde_json::json!({ "readOnlyHint": false }),
        ] {
            let tool = tool_definition_from_mcp(
                &server,
                McpTool {
                    name: "remote_action".to_string(),
                    description: "Remote action".to_string(),
                    input_schema: serde_json::json!({ "type": "object" }),
                    output_schema: None,
                    annotations: Some(annotations),
                },
            );
            assert!(tool.sensitive);
        }
    }

    #[test]
    fn mcp_open_world_annotation_overrides_read_only_hint() {
        let server = ChatMcpServer {
            id: "demo".to_string(),
            name: "Demo".to_string(),
            enabled: true,
            transport: "stdio".to_string(),
            url: String::new(),
            command: "demo".to_string(),
            args: Vec::new(),
            env: std::collections::HashMap::new(),
            headers: std::collections::HashMap::new(),
            cwd: None,
            enabled_tools: Vec::new(),
            connector_id: None,
            auth: None,
        };

        let tool = tool_definition_from_mcp(
            &server,
            McpTool {
                name: "remote_search".to_string(),
                description: "Remote search".to_string(),
                input_schema: serde_json::json!({ "type": "object" }),
                output_schema: None,
                annotations: Some(serde_json::json!({
                    "readOnlyHint": true,
                    "openWorldHint": true
                })),
            },
        );

        assert!(tool.sensitive);
        assert!(!tool.is_read_only_tool());
    }

    #[test]
    fn reserved_wire_alias_maps_web_search_both_ways() {
        // Cursor 系上游把 web_search 当内部保留工具吞掉（实测空响应）——wire 层全局改名。
        assert_eq!(apply_reserved_wire_alias("web_search"), "search_web");
        assert_eq!(apply_reserved_wire_alias("web_fetch"), "web_fetch");
        assert_eq!(resolve_reserved_wire_alias("search_web"), "web_search");
        assert_eq!(resolve_reserved_wire_alias("read"), "read");
    }

    #[test]
    fn native_web_search_tool_declares_alias_on_the_wire() {
        let tool = ChatToolDefinition {
            id: "native__web_search".to_string(),
            name: "web_search".to_string(),
            description: "Search the web".to_string(),
            source: "native".to_string(),
            server_id: None,
            server_name: Some("Kivio".to_string()),
            input_schema: serde_json::json!({ "type": "object" }),
            sensitive: false,
            annotations: None,
            output_schema: None,
        };
        assert_eq!(tool.openai_tool_name(), "search_web");
        let wire = tool.to_openai_tool();
        assert_eq!(wire["function"]["name"], "search_web");
        // MCP 工具不受别名影响（按 id 命名）。
        let mcp_tool = ChatToolDefinition {
            source: "mcp".to_string(),
            id: "mcp__srv__web_search".to_string(),
            ..tool
        };
        assert_eq!(mcp_tool.openai_tool_name(), "mcp__srv__web_search");
    }

    #[test]
    fn canonical_tool_name_maps_legacy_names() {
        // 旧名 → 现名（移除/合并/改名后仍可路由）。
        assert_eq!(canonical_tool_name("read_file"), "read");
        assert_eq!(canonical_tool_name("write_file"), "write");
        assert_eq!(canonical_tool_name("edit_file"), "edit");
        assert_eq!(canonical_tool_name("list_dir"), "read");
        assert_eq!(canonical_tool_name("ls"), "read");
        assert_eq!(canonical_tool_name("search_files"), "grep");
        assert_eq!(canonical_tool_name("glob_files"), "glob");
        assert_eq!(canonical_tool_name("find"), "glob");
        assert_eq!(canonical_tool_name("run_command"), "bash");
        assert_eq!(canonical_tool_name("list_background"), "bash_output");
        assert_eq!(canonical_tool_name("todo_update"), "todo_write");
        // 现名与未知名原样返回。
        assert_eq!(canonical_tool_name("read"), "read");
        assert_eq!(canonical_tool_name("glob"), "glob");
        assert_eq!(canonical_tool_name("bash"), "bash");
        assert_eq!(canonical_tool_name("read_file_extra"), "read_file_extra");
    }

    #[test]
    fn sanitize_keeps_short_names_and_hash_suffixes_long_ones() {
        assert_eq!(sanitize_openai_tool_name("search.web"), "search_web");
        assert_eq!(sanitize_openai_tool_name("read_file"), "read_file");
        let long = "a".repeat(80);
        let wire = sanitize_openai_tool_name(&long);
        assert_eq!(wire.chars().count(), MAX_WIRE_TOOL_NAME);
        assert!(wire.contains('_'), "{wire}");
        assert_ne!(
            sanitize_openai_tool_name(&format!("{}x", "a".repeat(79))),
            wire,
            "distinct inputs must not collapse after the 64-char cap"
        );
    }

    #[test]
    fn uuid_mcp_server_tools_keep_distinct_wire_names() {
        // Issue #33: `mcp__<uuid>__zai-mcp-server-ui_*` all left-truncated to
        // the same `...-ui` and Gemini dropped the duplicates.
        let server = ChatMcpServer {
            id: "mcp-d1ad400e-e2c7-4f34-9cd0-4e4cf1afa728".to_string(),
            name: "zhipu".to_string(),
            enabled: true,
            transport: "stdio".to_string(),
            url: String::new(),
            command: "demo".to_string(),
            args: Vec::new(),
            env: std::collections::HashMap::new(),
            headers: std::collections::HashMap::new(),
            cwd: None,
            enabled_tools: Vec::new(),
            connector_id: None,
            auth: None,
        };
        let names = [
            "zai-mcp-server-ui_to_artifact",
            "zai-mcp-server-ui_diff_check",
            "zai-mcp-server-analyze_data_visualization",
            "zai-mcp-server-analyze_image",
            "zai-mcp-server-analyze_video",
        ];
        let mut wires = std::collections::HashSet::new();
        for name in names {
            let tool = tool_definition_from_mcp(
                &server,
                McpTool {
                    name: name.to_string(),
                    description: String::new(),
                    input_schema: serde_json::json!({ "type": "object" }),
                    output_schema: None,
                    annotations: None,
                },
            );
            let wire = tool.openai_tool_name();
            assert!(
                wire.chars().count() <= MAX_WIRE_TOOL_NAME,
                "{wire} exceeds 64"
            );
            assert!(
                wire.contains("zai-mcp-server"),
                "tool suffix should stay visible: {wire}"
            );
            assert!(wires.insert(wire.clone()), "collided: {wire}");
        }
        assert_eq!(wires.len(), names.len());
    }
}
