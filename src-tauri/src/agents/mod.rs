//! Agent definitions loaded from built-ins, plugins, user and project layers.
//! Later layers override earlier definitions with the same id.

pub mod parse;
pub mod types;

pub use types::{builtin_agent_definitions, AgentDefinition};

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

/// User-level agents directory: `<app_data>/agents`. Created on demand.
pub fn user_agents_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|err| format!("app_data_dir unavailable: {err}"))?
        .join("agents");
    std::fs::create_dir_all(&dir).map_err(|err| format!("create agents dir failed: {err}"))?;
    Ok(dir)
}

/// Load built-ins, plugin, user, and project definitions in override order.
/// Plugin identities remain namespaced independently of running agent IDs.
pub fn load_agent_definitions(
    user_dir: Option<&Path>,
    project_root: Option<&Path>,
    plugins: &[crate::plugins::packages::Resolved],
) -> Vec<AgentDefinition> {
    let mut defs = builtin_agent_definitions();
    for plugin in plugins {
        for path in &plugin.agents {
            if let Err(error) = crate::plugins::packages::component_markdown(path) {
                eprintln!("Invalid plugin agent {}: {error}", path.display());
                continue;
            }
            if let Ok(raw) = std::fs::read_to_string(path) {
                let fallback = path.file_stem().and_then(|s| s.to_str()).unwrap_or("agent");
                if let Some(mut def) = parse::parse_agent_markdown(
                    fallback,
                    &raw,
                    "plugin",
                    Some(path.display().to_string()),
                ) {
                    if def.description.is_empty() {
                        def.description = format!("Run the {fallback} plugin component");
                    }
                    def.id = format!("pkg-{}-{}", plugin.package.id, def.id);
                    def.system_prompt = plugin.expand_body(&def.system_prompt);
                    if let Some(tools) = &mut def.tools {
                        for name in tools {
                            *name = plugin_tool_name(name);
                        }
                    }
                    if def.model.as_deref() == Some("inherit") {
                        def.model = None;
                    }
                    def.name = format!("{}:{}", plugin.package.name, def.name);
                    upsert(&mut defs, def);
                }
            }
        }
    }

    if let Some(dir) = user_dir {
        merge_dir(&mut defs, dir, "user");
    }
    if let Some(root) = project_root {
        merge_dir(&mut defs, &root.join(".kivio").join("agents"), "project");
    }
    defs
}

fn plugin_tool_name(name: &str) -> String {
    match name {
        "Bash" => "bash",
        "Read" => "read",
        "Write" => "write",
        "Edit" => "edit",
        "Glob" => "glob",
        "Grep" => "grep",
        "Agent" | "Task" => "task",
        other => other,
    }
    .into()
}

/// Find a definition by id or (case-insensitive) name.
pub fn find_definition<'a>(
    defs: &'a [AgentDefinition],
    name_or_id: &str,
) -> Option<&'a AgentDefinition> {
    let needle = name_or_id.trim();
    let lower = needle.to_ascii_lowercase();
    defs.iter()
        .find(|d| d.id == needle || d.name == needle)
        .or_else(|| {
            defs.iter()
                .find(|d| d.id.eq_ignore_ascii_case(&lower) || d.name.eq_ignore_ascii_case(&lower))
        })
}

fn merge_dir(defs: &mut Vec<AgentDefinition>, dir: &Path, source: &str) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let fallback_id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("agent")
            .to_string();
        let path_str = path.to_string_lossy().to_string();
        if let Some(def) = parse::parse_agent_markdown(&fallback_id, &raw, source, Some(path_str)) {
            upsert(defs, def);
        }
    }
}

fn upsert(defs: &mut Vec<AgentDefinition>, def: AgentDefinition) {
    if let Some(existing) = defs.iter_mut().find(|d| d.id == def.id) {
        *existing = def;
    } else {
        defs.push(def);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_overrides_user_and_builtin_definitions() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("user");
        let project = root.path().join("project");
        let project_agents = project.join(".kivio").join("agents");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::create_dir_all(&project_agents).unwrap();
        std::fs::write(user.join("scout.md"), "---\nname: scout\n---\nuser").unwrap();
        std::fs::write(project_agents.join("scout.md"), "---\nname: scout\n---\nproject").unwrap();
        let defs = load_agent_definitions(Some(&user), Some(&project), &[]);
        let scout = find_definition(&defs, "scout").unwrap();
        assert_eq!(scout.system_prompt, "project");
        assert_eq!(scout.source, "project");
        assert_eq!(defs.iter().filter(|d| d.id == "scout").count(), 1);
    }

    #[test]
    fn plugin_tool_names_use_current_native_names() {
        assert_eq!(plugin_tool_name("Read"), "read");
        assert_eq!(plugin_tool_name("Task"), "task");
        assert_eq!(plugin_tool_name("custom_tool"), "custom_tool");
    }

    #[test]
    fn plugin_agent_preserves_null_empty_and_unsupported_fields() {
        use crate::plugins::packages::{Package, Resolved};
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("helper.md");
        let plugin = Resolved {
            package: Package {
                id: "example-id".into(), name: "example".into(), description: String::new(),
                version: None, format: "plugin".into(), source: String::new(), revision: None,
                enabled: true, components: Default::default(), diagnostics: Vec::new(), marketplace: None,
            },
            root: root.path().to_path_buf(), skills: Vec::new(), commands: Vec::new(),
            agents: vec![path.clone()], hooks: Vec::new(), servers: Vec::new(),
        };
        std::fs::write(&path, "---\nname: helper\ntools: []\noutput:\n  type: object\n---\nbody").unwrap();
        let defs = load_agent_definitions(None, None, std::slice::from_ref(&plugin));
        let helper = find_definition(&defs, "example:helper").unwrap();
        assert_eq!(helper.tools, Some(vec![]));
        assert_eq!(helper.unsupported, ["output"]);
        std::fs::write(&path, "---\nname: helper\ntools: null\n---\nbody").unwrap();
        let defs = load_agent_definitions(None, None, &[plugin]);
        assert_eq!(find_definition(&defs, "example:helper").unwrap().tools, None);
    }
}
