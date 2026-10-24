use std::sync::Arc;
use super::resources::Catalog;
impl Catalog {
    pub fn discover(
        app: &tauri::AppHandle,
        conversation_id: &str,
        paths: &[String],
        cwd: Option<&std::path::Path>,
    ) -> Result<Self, String> {
        let plugins = crate::plugins::packages::discover_active()?;
        let skills = crate::skills::build_registry_with_plugins(app, paths, cwd, &plugins)?;
        let project_root = super::storage::load_conversation(app, conversation_id)
            .ok()
            .and_then(|conversation| {
                super::storage::resolve_conversation_project(app, &conversation)
                    .ok()
                    .flatten()
            })
            .and_then(|project| project.root_path)
            .map(std::path::PathBuf::from);
        let agents = crate::agents::load_agent_definitions(
            crate::agents::user_agents_dir(app).ok().as_deref(),
            project_root.as_deref(),
            &plugins,
        );
        let hooks = plugins
            .into_iter()
            .flat_map(|plugin| plugin.hooks)
            .collect();
        Ok(Self {
            skills: Arc::new(skills),
            agents,
            hooks,
            ..Default::default()
        })
    }
}
