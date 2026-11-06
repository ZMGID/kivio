use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::agents::AgentDefinition;
use crate::settings::{ModelProvider, Settings, TaskPerCallModel};

pub(crate) const MAX_TASK_FILE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum TaskSource {
    #[default]
    Inline,
    File { path: PathBuf, sha256: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum CompletionPolicy {
    #[default]
    Text,
    Yield { schema: Option<Value>, strict: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Effort { Lo, Med, Hi }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelPolicy {
    pub selector: Option<String>,
    pub role_models: Vec<String>,
    pub effort: Option<Effort>,
    pub thinking_level: Option<String>,
    pub invocation: ModelInvocation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ModelInvocation { #[default] Role, Task, Eval, Seance }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub(crate) enum MergeMode { #[default] Patch, Branch }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum WorkspacePolicy {
    #[default]
    Shared,
    Isolated { mode: MergeMode, apply: bool },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SpawnOptions {
    pub source: TaskSource,
    pub completion: CompletionPolicy,
    pub model: ModelPolicy,
    pub workspace: WorkspacePolicy,
    pub source_session: Option<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub workpool_items: Vec<String>,
}

pub(crate) fn read_task_source(item: &Map<String, Value>, enabled: bool, cwd: &Path) -> Result<(String, TaskSource), String> {
    match (item.get("task"), item.get("taskFile")) {
        (Some(_), Some(_)) | (None, None) => Err("Provide exactly one of `task` and `taskFile`.".into()),
        (Some(task), None) => task.as_str().map(str::trim).filter(|s| !s.is_empty())
            .map(|task| (task.to_string(), TaskSource::Inline))
            .ok_or_else(|| "`task` must be a non-empty string.".into()),
        (None, Some(file)) => {
            if !enabled { return Err("File task instructions are disabled.".into()); }
            let raw = file.as_str().filter(|s| !s.trim().is_empty()).ok_or("`taskFile` must be a non-empty string.")?;
            let path = task_file_path(raw, cwd)?;
            let path = std::fs::canonicalize(&path).map_err(|e| format!("Cannot resolve task file {}: {e}", path.display()))?;
            let mut open = std::fs::OpenOptions::new();
            open.read(true);
            #[cfg(unix)]
            { use std::os::unix::fs::OpenOptionsExt; open.custom_flags(libc::O_NONBLOCK); }
            let mut file = open.open(&path).map_err(|e| format!("Cannot open task file {}: {e}", path.display()))?;
            let metadata = file.metadata().map_err(|e| format!("Cannot inspect task file: {e}"))?;
            if !metadata.is_file() { return Err("`taskFile` must refer to a regular file.".into()); }
            if metadata.len() > MAX_TASK_FILE_BYTES { return Err("Task file exceeds the 4 MiB limit.".into()); }
            let mut bytes = Vec::new();
            Read::by_ref(&mut file).take(MAX_TASK_FILE_BYTES + 1).read_to_end(&mut bytes)
                .map_err(|e| format!("Cannot read task file: {e}"))?;
            if bytes.len() as u64 > MAX_TASK_FILE_BYTES { return Err("Task file exceeds the 4 MiB limit.".into()); }
            let sha256 = format!("{:x}", Sha256::digest(&bytes));
            let text = String::from_utf8(bytes).map_err(|_| "Task file must contain valid UTF-8.")?;
            if text.trim().is_empty() { return Err("Task file has no instructions.".into()); }
            Ok((text.trim().to_string(), TaskSource::File { path, sha256 }))
        }
    }
}

fn task_file_path(raw: &str, cwd: &Path) -> Result<PathBuf, String> {
    if let Some(local) = raw.strip_prefix("local://") {
        #[cfg(windows)]
        let absolute = local.trim_start_matches('/').as_bytes().get(1) == Some(&b':');
        #[cfg(not(windows))]
        let absolute = local.starts_with('/');
        let decoded = if absolute {
            url::Url::parse(&format!("file:///{}", local.trim_start_matches('/')))
        } else {
            if local.split(['/', '\\']).any(|part| part == "..") { return Err("Local task URL cannot escape its working directory.".into()); }
            url::Url::from_directory_path(cwd).map_err(|_| "Invalid local task directory.")?.join(local)
        }.map_err(|e| format!("Invalid local task URL: {e}"))?;
        if decoded.query().is_some() || decoded.fragment().is_some() { return Err("Task URL cannot contain a query or fragment.".into()); }
        return decoded.to_file_path().map_err(|_| "Invalid local task URL path.".into());
    }
    let path = PathBuf::from(raw);
    if !path.is_absolute() { return Err("`taskFile` must be an absolute path or local:// URL.".into()); }
    Ok(path)
}

pub(crate) fn options(
    item: &Map<String, Value>, definition: &AgentDefinition, settings: &Settings,
    source: TaskSource, plan: bool, session_schema: Option<&Value>,
    invocation: ModelInvocation,
) -> Result<SpawnOptions, String> {
    let cfg = &settings.chat_tools;
    let structured = cfg.task_structured || definition.structured || invocation == ModelInvocation::Eval;
    if !structured && (item.contains_key("outputSchema") || item.contains_key("schemaMode")) {
        return Err("Structured task results are disabled for this role.".into());
    }
    let strict = match item.get("schemaMode").and_then(Value::as_str) {
        None if !item.contains_key("schemaMode") => false,
        Some("strict") => true,
        Some("permissive") => false,
        _ => return Err("`schemaMode` must be strict or permissive.".into()),
    };
    let completion = if structured {
        let schema = if item.contains_key("outputSchema") { item.get("outputSchema") }
            else { definition.output.as_ref().or(session_schema) };
        let schema = schema.filter(|schema| !schema.is_null()).map(super::structured::normalize_schema).transpose()?;
        CompletionPolicy::Yield { schema, strict }
    } else { CompletionPolicy::Text };
    let effort = match item.get("effort") {
        None => None,
        Some(_) if !cfg.task_enable_effort => return Err("Per-task effort is disabled.".into()),
        Some(Value::String(value)) => Some(match value.as_str() {
            "lo" => Effort::Lo, "med" => Effort::Med, "hi" => Effort::Hi,
            _ => return Err("`effort` must be lo, med or hi.".into()),
        }),
        Some(_) => return Err("`effort` must be lo, med or hi.".into()),
    };
    let selector = item.get("model").map(|model| model.as_str().map(str::trim).filter(|s| !s.is_empty())
        .map(str::to_string).ok_or("`model` must be one exact provider/model selector.")).transpose()?;
    if selector.is_some() && !super::seance::is_bundled(definition) && !(cfg.task_per_call_model == TaskPerCallModel::On || (cfg.task_per_call_model == TaskPerCallModel::Eval && invocation == ModelInvocation::Eval)) {
        return Err("Per-call models are disabled for task calls.".into());
    }
    let isolated = match item.get("isolated") {
        None => false, Some(Value::Bool(value)) => *value,
        Some(_) => return Err("`isolated` must be a boolean.".into()),
    };
    if isolated && (!cfg.task_isolation_enabled || plan) { return Err("Task isolation is unavailable in this mode.".into()); }
    if !isolated && (item.contains_key("merge") || item.contains_key("apply")) { return Err("`merge` and `apply` require isolation.".into()); }
    let mode = match item.get("merge").and_then(Value::as_str) {
        None if !item.contains_key("merge") => MergeMode::Patch,
        Some("patch") => MergeMode::Patch, Some("branch") => MergeMode::Branch,
        _ => return Err("`merge` must be patch or branch.".into()),
    };
    let apply = match item.get("apply") {
        None => true, Some(Value::Bool(value)) => *value,
        Some(_) => return Err("`apply` must be a boolean.".into()),
    };
    let source_session = item.get("sourceSession").map(|source| source.as_str().map(str::trim).filter(|s| !s.is_empty())
        .map(str::to_string).ok_or("`sourceSession` must be a non-empty string.")).transpose()?;
    if super::seance::is_bundled(definition) != source_session.is_some() {
        return Err("Only the bundled seance role requires and accepts `sourceSession`.".into());
    }
    if source_session.is_some() && (plan || isolated) { return Err("Historical consultation cannot use plan mode or isolation.".into()); }
    let tools = match item.get("tools") {
        None => Vec::new(),
        Some(Value::Array(names)) => names.iter().map(|name| name.as_str().filter(|name| !name.trim().is_empty()).map(str::to_string).ok_or("Shared eval tool names must be non-empty strings.")).collect::<Result<Vec<_>, _>>()?,
        _ => return Err("`tools` must be a list of eval-defined tool names.".into()),
    };
    if !tools.is_empty() && (plan || source_session.is_some()) { return Err("Plan and seance workers cannot share eval tools.".into()); }
    Ok(SpawnOptions {
        source, completion,
        model: ModelPolicy { invocation: if super::seance::is_bundled(definition) { ModelInvocation::Seance } else if selector.is_some() { invocation } else { ModelInvocation::Role }, selector, role_models: if definition.models.is_empty() { definition.model.iter().cloned().collect() } else { definition.models.clone() }, effort, thinking_level: definition.thinking_level.clone() },
        workspace: if isolated { WorkspacePolicy::Isolated { mode, apply } } else { WorkspacePolicy::Shared },
        source_session,
        tools,
        workpool_items: Vec::new(),
    })
}

#[derive(Debug)]
pub(crate) struct ResolvedModel {
    pub provider: ModelProvider,
    pub model: String,
    pub thinking_level: Option<String>,
}

fn exact<'a>(selector: &'a str, settings: &'a Settings) -> Result<(&'a ModelProvider, &'a str, Option<&'a str>), String> {
    let (identity, level) = selector.rsplit_once(':').filter(|(_, level)| matches!(*level, "inherit" | "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"))
        .map_or((selector, None), |(identity, level)| (identity, Some(level)));
    let (provider, model) = identity.split_once('/').filter(|(p, m)| !p.is_empty() && !m.is_empty())
        .ok_or("Use one exact provider/model selector.")?;
    if identity.contains(['*', '?']) || model.contains('@') { return Err("Patterns and upstream pins are not supported by this provider runtime.".into()); }
    let provider = settings.get_provider(provider).filter(|provider| provider.enabled && provider.authentication_ready())
        .ok_or_else(|| format!("Model provider `{provider}` is unavailable."))?;
    if !provider.enabled_models.iter().any(|enabled| enabled == model) { return Err(format!("Model `{identity}` is not enabled.")); }
    Ok((provider, model, level))
}

pub(crate) fn resolve_model(settings: &Settings, policy: &ModelPolicy, parent_provider: &str, parent_model: &str) -> Result<ResolvedModel, String> {
    let cfg = &settings.chat_tools;
    let (provider, model, level) = if let Some(selector) = &policy.selector {
        if policy.invocation != ModelInvocation::Seance && !(cfg.task_per_call_model == TaskPerCallModel::On || (cfg.task_per_call_model == TaskPerCallModel::Eval && policy.invocation == ModelInvocation::Eval)) { return Err("Per-call model permission was revoked.".into()); }
        let (provider, model, level) = exact(selector, settings)?;
        let identity = format!("{}/{model}", provider.id);
        let (default_provider, default_model) = settings.effective_chat_model();
        let granted = (provider.id == parent_provider && model == parent_model)
            || (provider.id == default_provider && model == default_model)
            || cfg.sub_agent_models.values().any(|selection| selection.provider_id == provider.id && selection.model == model)
            || cfg.task_agent_model_overrides.values().flatten().flatten().any(|selector| selector == &identity || selector.strip_prefix(&identity).is_some_and(|suffix| suffix.starts_with(':')));
        if policy.invocation != ModelInvocation::Seance && (!granted || (!cfg.task_per_call_model_allow.is_empty() && !cfg.task_per_call_model_allow.contains(&identity))) {
            return Err(format!("Model `{identity}` has no current operator grant."));
        }
        (provider.clone(), model.to_string(), level.map(str::to_string))
    } else {
        let mut resolved = None;
        for selector in &policy.role_models {
            if selector == "inherit" { break; }
            if let Some(role) = selector.strip_prefix('@') {
                if let Some(selection) = cfg.sub_agent_models.get(role).or_else(|| cfg.sub_agent_models.get("task")) {
                    if let Ok((provider, model, _)) = exact(&format!("{}/{}", selection.provider_id, selection.model), settings) {
                        resolved = Some((provider.clone(), model.to_string(), selection.thinking_level.clone()));
                        break;
                    }
                }
            } else {
                let selector = if selector.contains('/') { selector.clone() } else { format!("{parent_provider}/{selector}") };
                if let Ok((provider, model, level)) = exact(&selector, settings) {
                    resolved = Some((provider.clone(), model.to_string(), level.map(str::to_string)));
                    break;
                }
            }
        }
        match resolved {
            Some(resolved) => resolved,
            None => {
                let provider = settings.get_provider(parent_provider).filter(|p| p.enabled && p.authentication_ready())
                    .ok_or("Parent provider is unavailable.")?;
                if !provider.enabled_models.iter().any(|model| model == parent_model) { return Err("Parent model is no longer enabled.".into()); }
                (provider.clone(), parent_model.to_string(), None)
            }
        }
    };
    let requested = level.as_deref().or(policy.thinking_level.as_deref());
    let thinking_level = resolve_effort(&provider, &model, policy.effort, requested, cfg.task_max_effort.as_deref())?;
    Ok(ResolvedModel { provider, model, thinking_level })
}

fn rank(level: &str) -> Option<usize> {
    ["off", "minimal", "low", "medium", "high", "xhigh", "max"].iter().position(|candidate| *candidate == level)
}

fn resolve_effort(provider: &ModelProvider, model: &str, effort: Option<Effort>, requested: Option<&str>, cap: Option<&str>) -> Result<Option<String>, String> {
    if effort.is_none() && requested.is_none_or(|level| matches!(level, "auto" | "inherit" | "off")) { return Ok(requested.map(str::to_string)); }
    let mut levels = crate::chat::model_metadata::reasoning_efforts_for_model(Some(provider), model);
    levels.sort_by_key(|level| rank(level).unwrap_or(usize::MAX));
    if let Some(cap) = cap {
        let ceiling = rank(cap).ok_or("Invalid task maximum effort.")?;
        levels.retain(|level| rank(level).is_some_and(|value| value <= ceiling));
    }
    if levels.is_empty() { return Err(format!("Model `{model}` has no supported effort within the configured ceiling.")); }
    let chosen = match effort {
        Some(Effort::Lo) => levels.first(), Some(Effort::Hi) => levels.last(),
        Some(Effort::Med) => levels.get((levels.len() - 1) / 2),
        None => {
            let desired = requested.and_then(rank).ok_or("Invalid requested thinking level.")?;
            levels.iter().rev().find(|level| rank(level).is_some_and(|value| value <= desired)).or_else(|| levels.first())
        }
    };
    Ok(chosen.cloned())
}

pub(crate) fn cap_thinking(provider: &ModelProvider, model: &str, requested: &str, cap: Option<&str>) -> Result<Option<String>, String> {
    resolve_effort(provider, model, None, Some(requested), cap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn file_source_is_strict_bounded_and_hashes_original_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("task instructions.txt");
        let bytes = "  do work\r\n";
        std::fs::write(&path, bytes).unwrap();
        let item = json!({"taskFile":path});
        let (task, source) = read_task_source(item.as_object().unwrap(), true, dir.path()).unwrap();
        assert_eq!(task, "do work");
        assert_eq!(source, TaskSource::File { path:std::fs::canonicalize(&path).unwrap(),sha256:format!("{:x}",Sha256::digest(bytes.as_bytes())) });
        assert!(read_task_source(item.as_object().unwrap(), false, dir.path()).is_err());
        let local = json!({"taskFile":"local://task%20instructions.txt"});
        assert_eq!(read_task_source(local.as_object().unwrap(), true, dir.path()).unwrap().0, "do work");
        for invalid in [json!({}), json!({"task":"a","taskFile":path}), json!({"taskFile":"relative.txt"}), json!({"taskFile":dir.path()})] {
            assert!(read_task_source(invalid.as_object().unwrap(), true, dir.path()).is_err());
        }
        std::fs::write(&path, [255, 254]).unwrap();
        assert!(read_task_source(item.as_object().unwrap(), true, dir.path()).unwrap_err().contains("UTF-8"));
        std::fs::write(&path, vec![b'x'; MAX_TASK_FILE_BYTES as usize + 1]).unwrap();
        assert!(read_task_source(item.as_object().unwrap(), true, dir.path()).unwrap_err().contains("4 MiB"));
    }

    #[test]
    fn explicit_null_caller_schema_suppresses_role_and_session_schema() {
        let mut role = crate::agents::builtin_agent_definitions().remove(0);
        role.structured = true;
        role.output = Some(json!({"type":"string"}));
        let item = json!({"outputSchema":null,"schemaMode":"strict"});
        let policy = options(item.as_object().unwrap(), &role, &Settings::default(), TaskSource::Inline, false, Some(&json!({"type":"integer"})), ModelInvocation::Task).unwrap();
        assert_eq!(policy.completion, CompletionPolicy::Yield { schema:None,strict:true });
        let item = json!({});
        let policy = options(item.as_object().unwrap(), &role, &Settings::default(), TaskSource::Inline, false, None, ModelInvocation::Task).unwrap();
        assert_eq!(policy.completion, CompletionPolicy::Yield { schema:role.output,strict:false });
    }

    fn model_settings() -> Settings {
        let mut settings = Settings::default();
        let mut provider: ModelProvider = serde_json::from_value(json!({"id":"test","name":"Test","baseUrl":"http://127.0.0.1"})).unwrap();
        provider.id = "test".into(); provider.enabled = true;
        provider.api_keys = vec!["test-key".into()];
        provider.enabled_models = vec!["first".into(),"second".into()];
        provider.available_models = provider.enabled_models.clone();
        settings.providers = vec![provider];
        settings.chat_tools.task_per_call_model = TaskPerCallModel::On;
        settings.chat_tools.sub_agent_models.insert("task".into(), crate::settings::SubAgentModelSelection {
            provider_id:"test".into(),model:"first".into(),thinking_level:None,
        });
        settings
    }

    #[test]
    fn role_fallbacks_are_ordered_and_explicit_models_never_fall_back() {
        let settings = model_settings();
        let policy = ModelPolicy { role_models:vec!["missing/nope".into(),"test/second".into(),"test/first".into()], ..Default::default() };
        assert_eq!(resolve_model(&settings, &policy, "test", "first").unwrap().model,"second");
        let explicit = ModelPolicy { selector:Some("test/second".into()), invocation:ModelInvocation::Task, ..Default::default() };
        assert!(resolve_model(&settings, &explicit, "test", "first").unwrap_err().contains("grant"));
        let explicit = ModelPolicy { selector:Some("test/first".into()), invocation:ModelInvocation::Task, ..Default::default() };
        assert_eq!(resolve_model(&settings, &explicit, "test", "first").unwrap().model,"first");
        let explicit = ModelPolicy { selector:Some("test/missing".into()), invocation:ModelInvocation::Task, ..Default::default() };
        assert!(resolve_model(&settings, &explicit, "test", "first").is_err());
    }

    #[test]
    fn grants_allowlist_revocation_and_effort_ceiling_are_live() {
        let mut settings = model_settings();
        let policy = ModelPolicy { selector:Some("test/first:high".into()), invocation:ModelInvocation::Task, ..Default::default() };
        settings.chat_tools.task_max_effort = Some("medium".into());
        assert_eq!(resolve_model(&settings, &policy, "test", "first").unwrap().thinking_level.as_deref(), Some("medium"));
        settings.chat_tools.task_per_call_model_allow = vec!["test/second".into()];
        assert!(resolve_model(&settings, &policy, "test", "first").is_err());
        settings.chat_tools.task_per_call_model_allow.clear();
        settings.chat_tools.task_per_call_model = TaskPerCallModel::Off;
        assert!(resolve_model(&settings, &policy, "test", "first").is_err());
    }
}
