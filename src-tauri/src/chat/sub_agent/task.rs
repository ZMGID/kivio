//! OMP batch task dispatch and conversation-scoped spawn admission.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tauri::Manager;
use tokio::sync::{oneshot, watch};

use crate::agents::{find_definition, AgentDefinition};
use crate::mcp::native_registry::NativeToolFuture;
use crate::mcp::types::{ChatToolDefinition, McpToolCallResult};
use crate::settings::{ChatToolsConfig, Settings};

use super::{
    executor, names, prompts, registry, spawn, SubAgentCallCtx, MAIN_AGENT_ID, TASK_TOOL_NAME,
};

#[derive(Default)]
pub(crate) struct Admission {
    roots: Mutex<HashMap<String, Arc<RootAdmission>>>,
}

#[derive(Default)]
struct RootAdmission {
    // Covers identity allocation and registration as one operation per root.
    allocation: Mutex<bool>,
    sessions: Mutex<HashMap<String, Arc<SessionAdmission>>>,
}

#[derive(Default)]
struct SessionAdmission {
    state: Mutex<AdmissionState>,
}

#[derive(Default)]
struct AdmissionState {
    max: usize,
    active: usize,
    next_ticket: u64,
    retired: bool,
    queue: VecDeque<(u64, oneshot::Sender<Permit>, Option<tauri::AppHandle>)>,
}

impl Admission {
    pub(crate) async fn acquire_wake(
        &self,
        app: &tauri::AppHandle,
        root: &str,
        owner: &str,
        cancel: &mut watch::Receiver<bool>,
    ) -> Option<Permit> {
        let max = app
            .state::<crate::state::AppState>()
            .settings_read()
            .chat_tools
            .task_max_concurrency;
        self.root(root)
            .session(owner)
            .enqueue_with_app(max, Some(app))
            .wait(cancel)
            .await
    }
    fn root(&self, root: &str) -> Arc<RootAdmission> {
        self.roots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(root.to_string())
            .or_default()
            .clone()
    }

    pub(crate) fn remove_root(&self, root: &str) {
        let admission = self.root(root);
        let mut retired = admission
            .allocation
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *retired = true;
        for session in admission
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
        {
            let mut state = session.state.lock().unwrap_or_else(|e| e.into_inner());
            state.retired = true;
            state.queue.clear();
        }
    }
}

impl RootAdmission {
    fn session(&self, owner: &str) -> Arc<SessionAdmission> {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(owner.to_string())
            .or_default()
            .clone()
    }
}

impl SessionAdmission {
    fn enqueue_with_app(self: &Arc<Self>, max: usize, app: Option<&tauri::AppHandle>) -> Ticket {
        let (sender, receiver) = oneshot::channel();
        let (id, grants) = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.max = max;
            state.next_ticket += 1;
            let id = state.next_ticket;
            if !state.retired {
                state.queue.push_back((id, sender, app.cloned()));
            }
            (id, state.grants())
        };
        self.send_grants(grants);
        Ticket {
            root: self.clone(),
            id,
            receiver,
        }
    }

    #[cfg(test)]
    fn enqueue(self: &Arc<Self>, max: usize) -> Ticket {
        self.enqueue_with_app(max, None)
    }

    fn send_grants(
        self: &Arc<Self>,
        grants: Vec<(oneshot::Sender<Permit>, Option<tauri::AppHandle>)>,
    ) {
        // Send after unlocking: a closed receiver drops its permit immediately.
        for (sender, app) in grants {
            let _ = sender.send(Permit {
                root: self.clone(),
                app,
            });
        }
    }

    fn release(self: &Arc<Self>, max: Option<usize>) {
        let grants = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(max) = max {
                state.max = max;
            }
            state.active -= 1;
            state.grants()
        };
        self.send_grants(grants);
    }
}

impl AdmissionState {
    fn grants(&mut self) -> Vec<(oneshot::Sender<Permit>, Option<tauri::AppHandle>)> {
        let mut grants = Vec::new();
        while self.max == 0 || self.active < self.max {
            let Some((_, sender, app)) = self.queue.pop_front() else {
                break;
            };
            if !sender.is_closed() {
                self.active += 1;
                grants.push((sender, app));
            }
        }
        grants
    }
}

struct Ticket {
    root: Arc<SessionAdmission>,
    id: u64,
    receiver: oneshot::Receiver<Permit>,
}

impl Ticket {
    async fn wait(mut self, cancel: &mut watch::Receiver<bool>) -> Option<Permit> {
        loop {
            if *cancel.borrow() {
                return None;
            }
            tokio::select! {
                biased;
                changed = cancel.changed() => {
                    if changed.is_err() || *cancel.borrow() { return None; }
                }
                permit = &mut self.receiver => {
                    let permit = permit.ok()?;
                    return if *cancel.borrow() { None } else { Some(permit) };
                }
            }
        }
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        let mut state = self.root.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(index) = state.queue.iter().position(|(id, _, _)| *id == self.id) {
            state.queue.remove(index);
        }
    }
}

/// One running subagent. Dropping it transfers capacity to the oldest waiter.
pub(crate) struct Permit {
    root: Arc<SessionAdmission>,
    app: Option<tauri::AppHandle>,
}

impl Drop for Permit {
    fn drop(&mut self) {
        let max = self.app.as_ref().map(|app| {
            app.state::<crate::state::AppState>()
                .settings_read()
                .chat_tools
                .task_max_concurrency
        });
        self.root.release(max);
    }
}

struct SpawnPolicy<'a> {
    allowed: Option<Vec<&'a str>>,
}

impl<'a> SpawnPolicy<'a> {
    fn new(spawns: Option<&'a str>) -> Self {
        let spawns = spawns.unwrap_or("*").trim();
        Self {
            allowed: (spawns != "*").then(|| {
                spawns
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .collect()
            }),
        }
    }

    fn default_agent(&self) -> &str {
        self.allowed
            .as_ref()
            .and_then(|names| names.first().copied())
            .unwrap_or("task")
    }

    fn allows(&self, definition: &AgentDefinition) -> bool {
        self.allowed.as_ref().is_none_or(|names| {
            names.iter().any(|name| {
                definition.name.eq_ignore_ascii_case(name)
                    || definition.id.eq_ignore_ascii_case(name)
            })
        })
    }

    fn available(&self, definition: &AgentDefinition, settings: &ChatToolsConfig) -> bool {
        self.allows(definition)
            && definition.unsupported.is_empty()
            && settings
                .ensure_subagent_role_enabled(&definition.name)
                .is_ok()
    }

    fn description(&self) -> String {
        match &self.allowed {
            None => "*".into(),
            Some(names) if names.is_empty() => "none (spawns disabled for this agent)".into(),
            Some(names) => names.join(", "),
        }
    }
}

pub fn tool_definition(defs: &[AgentDefinition], settings: &Settings) -> ChatToolDefinition {
    tool_definition_with_policy(defs, settings, None, true)
}

pub(crate) fn tool_definition_with_policy(
    defs: &[AgentDefinition],
    settings: &Settings,
    spawns: Option<&str>,
    main_agent: bool,
) -> ChatToolDefinition {
    let policy = SpawnPolicy::new(spawns);
    let available: Vec<_> = defs
        .iter()
        .filter(|def| policy.available(def, &settings.chat_tools))
        .collect();
    let default_agent = find_definition(defs, policy.default_agent())
        .filter(|definition| policy.available(definition, &settings.chat_tools))
        .map(|definition| definition.name.as_str());
    let agent_list = available
        .iter()
        .map(|def| {
            let read_only = def.tools.as_ref().is_some_and(|tools| {
                !tools.is_empty()
                    && tools.iter().all(|tool| {
                        matches!(
                            tool.as_str(),
                            "read"
                                | "wait"
                                | "grep"
                                | "glob"
                                | "find"
                                | "web_search"
                                | "ast_grep"
                                | "yield"
                                | "ask"
                                | "todo"
                                | "recall"
                                | "reflect"
                                | "retain"
                                | "memory_edit"
                                | "checkpoint"
                                | "rewind"
                                | "lsp"
                        )
                    })
            });
            let label = if read_only {
                " (READ-ONLY; investigation only, no edits)"
            } else {
                ""
            };
            let blocking = if def.blocking {
                " (BLOCKING; inline result)"
            } else {
                ""
            };
            format!(
                "- `{}`{}{}: {}\n",
                def.name, label, blocking, def.description
            )
        })
        .collect::<String>();
    let async_guidance = prompts::render(
        prompts::TASK_ASYNC_CONTRACT,
        &json!({
            "waitTool": true, "ircEnabled": true, "mainAgent": main_agent
        }),
    );
    let mut description = prompts::render(
        prompts::TASK_TOOL,
        &json!({
            "asyncGuidance": async_guidance,
            "asyncEnabled": settings.chat_tools.task_async_enabled,
            "hasBlockingAgents": available.iter().any(|def| def.blocking),
            "defaultAgent": default_agent,
            "scoutAvailable": available.iter().any(|def| def.name.eq_ignore_ascii_case("scout")),
            "spawningDisabled": available.is_empty(),
            "hasModelMentions": false,
            "ircEnabled": true,
            "agentList": agent_list,
        }),
    );
    if settings.chat_tools.task_file_enabled {
        description.push_str("\nUse exactly one of task or taskFile. taskFile accepts an absolute path or local:// URL to a regular UTF-8 file up to 4 MiB. File tasks are never launched speculatively.\n");
    }
    if settings.chat_tools.task_enable_effort {
        description.push_str("\nOptional effort lo/med/hi selects the resolved model's supported effort within the configured ceiling.\n");
    }
    if settings.chat_tools.task_per_call_model == crate::settings::TaskPerCallModel::On {
        description.push_str("\nOptional model selects one exact operator-approved provider/model identity. Unavailable or unapproved selections fail without fallback.\n");
    }
    if settings.chat_tools.task_structured || available.iter().any(|def| def.structured) {
        description.push_str("\nStructured roles finish through yield. outputSchema overrides role output, including explicit null to disable schema validation. schemaMode is strict or permissive.\n");
    }
    if settings.chat_tools.task_isolation_enabled {
        description.push_str("\nOptional isolated runs in a separate Git worktree. merge selects patch or branch. apply false retains recovery artifacts without applying changes. Conflicts preserve the isolated files and recovery patch.\n");
    }
    if available.iter().any(|def| super::seance::is_bundled(def)) {
        description.push_str("\nThe bundled seance role requires sourceSession. It forks historical evidence read only and accepts an optional saved-model override.\n");
    }
    let mut properties = json!({
        "name": {"type": "string"}, "agent": {"type": "string"},
        "task": {"type": "string", "minLength": 1}, "solutionSpace": {"type": "string"}
    });
    let properties = properties.as_object_mut().expect("task properties");
    if settings.chat_tools.task_file_enabled {
        properties.insert("taskFile".into(), json!({"type":"string","minLength":1}));
    }
    if settings.chat_tools.task_enable_effort {
        properties.insert("effort".into(), json!({"enum":["lo","med","hi"]}));
    }
    if settings.chat_tools.task_per_call_model == crate::settings::TaskPerCallModel::On
        || available.iter().any(|def| super::seance::is_bundled(def))
    {
        properties.insert("model".into(), json!({"type":"string","minLength":1}));
    }
    if settings.chat_tools.task_structured || available.iter().any(|def| def.structured) {
        properties.insert("outputSchema".into(), json!({}));
        properties.insert("schemaMode".into(), json!({"enum":["strict","permissive"]}));
    }
    if settings.chat_tools.task_isolation_enabled {
        properties.insert("isolated".into(), json!({"type":"boolean"}));
        properties.insert("merge".into(), json!({"enum":["patch","branch"]}));
        properties.insert("apply".into(), json!({"type":"boolean"}));
    }
    properties.insert("tools".into(), json!({"type":"array","items":{"type":"string","minLength":1},"description":"Names defined by this session's retained eval kernels. These are additional custom tools, not a native tool allowlist."}));
    if available.iter().any(|def| super::seance::is_bundled(def)) {
        properties.insert(
            "sourceSession".into(),
            json!({"type":"string","minLength":1}),
        );
    }
    ChatToolDefinition {
        id: "native__task".into(),
        name: TASK_TOOL_NAME.into(),
        description,
        source: "native".into(),
        server_id: None,
        server_name: Some("Kivio".into()),
        input_schema: json!({
            "type": "object",
            "properties": {
                "context": {"type": "string"},
                "tasks": {"type": "array", "minItems": 1, "items": {
                    "type": "object",
                    "properties": properties,
                    "required": ["solutionSpace"],
                    "oneOf": [{"required":["task"]},{"required":["taskFile"]}],
                    "additionalProperties": false
                }}
            },
            "required": ["context", "tasks"],
            "additionalProperties": false
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

#[derive(Debug, Clone)]
struct PlannedTask {
    name: Option<String>,
    definition: AgentDefinition,
    task: String,
    solution_space: String,
    options: spawn::SpawnOptions,
    policy_key: String,
}

pub(crate) fn valid_agent_id(id: &str) -> bool {
    if id.len() > 249
        || id.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*')
        })
    {
        return false;
    }
    id.split('.').all(|part| {
        if part.is_empty() || part.ends_with(' ') {
            return false;
        }
        if [
            "CON", "PRN", "AUX", "NUL", "COM¹", "COM²", "COM³", "LPT¹", "LPT²", "LPT³",
        ]
        .iter()
        .any(|reserved| part.eq_ignore_ascii_case(reserved))
        {
            return false;
        }
        let bytes = part.as_bytes();
        !(bytes.len() == 4
            && (bytes[..3].eq_ignore_ascii_case(b"COM") || bytes[..3].eq_ignore_ascii_case(b"LPT"))
            && (b'1'..=b'9').contains(&bytes[3]))
    })
}

fn preflight(
    args: &Value,
    defs: &[AgentDefinition],
    spawns: Option<&str>,
    depth: u8,
    settings: &ChatToolsConfig,
) -> Result<(String, Vec<PlannedTask>), String> {
    let obj = args
        .as_object()
        .ok_or("Expected an object with `context` and `tasks`.")?;
    if obj.contains_key("agent") {
        return Err(
            "Top-level `agent` is not part of the batch shape. Set `tasks[].agent` instead.".into(),
        );
    }
    if obj.contains_key("task") {
        return Err(
            "Top-level `task` is not part of the batch shape. Put the work in `tasks[]` items."
                .into(),
        );
    }
    if let Some(key) = obj
        .keys()
        .find(|key| !matches!(key.as_str(), "context" | "tasks"))
    {
        return Err(format!("Unexpected top-level field `{key}`."));
    }
    let tasks = obj
        .get("tasks")
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
        .ok_or("Missing `tasks`. Provide at least one task item.")?;
    for (index, item) in tasks.iter().enumerate() {
        let label = format!("Task {}", index + 1);
        let item = item
            .as_object()
            .ok_or_else(|| format!("{label} must be an object."))?;
        if let Some(key) = item.keys().find(|key| {
            !matches!(
                key.as_str(),
                "name"
                    | "agent"
                    | "task"
                    | "taskFile"
                    | "solutionSpace"
                    | "effort"
                    | "model"
                    | "outputSchema"
                    | "schemaMode"
                    | "isolated"
                    | "merge"
                    | "apply"
                    | "sourceSession"
                    | "tools"
            )
        }) {
            return Err(format!("{label} has unexpected field `{key}`."));
        }
        if item.contains_key("task") == item.contains_key("taskFile") {
            return Err(format!(
                "{label} requires exactly one of `task` and `taskFile`."
            ));
        }
        if item
            .get("task")
            .or_else(|| item.get("taskFile"))
            .and_then(Value::as_str)
            .is_none_or(|s| s.trim().is_empty())
        {
            return Err(format!("{label} is missing non-empty task instructions."));
        }
        if item.get("solutionSpace").and_then(Value::as_str).is_none() {
            return Err(format!("{label} requires a string `solutionSpace`."));
        }
        if item
            .get("agent")
            .is_some_and(|value| value.as_str().is_none_or(|s| s.trim().is_empty()))
        {
            return Err(format!("{label} `agent` must be a non-empty string."));
        }
        if let Some(name) = item.get("name") {
            let name = name
                .as_str()
                .ok_or_else(|| format!("{label} `name` must be a string."))?
                .trim();
            if !name.is_empty() {
                if !valid_agent_id(name) {
                    return Err(format!("{label} has an unsafe name `{name}`. Use a valid filename without path separators or reserved device names."));
                }
            }
        }
    }
    let mut seen = HashSet::new();
    for item in tasks {
        if let Some(name) = item
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            if !seen.insert(name.to_lowercase()) {
                return Err(format!("Duplicate task name `{name}`. Provided names must be unique within a call (case-insensitive)."));
            }
        }
    }
    let context = obj
        .get("context")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or("Missing `context`. Provide shared background for this batch.")?;
    let max_depth = settings.task_max_recursion_depth;
    if (max_depth >= 0 && i32::from(depth) >= max_depth) || depth == u8::MAX {
        return Err(format!(
            "Task recursion depth limit reached (depth {depth}, maximum {max_depth})."
        ));
    }
    let policy = SpawnPolicy::new(spawns);
    let mut planned = Vec::with_capacity(tasks.len());
    for (index, item) in tasks.iter().enumerate() {
        let label = format!("Task {}", index + 1);
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string);
        let task = item.get("task").and_then(Value::as_str).unwrap_or_default();
        let solution_space = item["solutionSpace"]
            .as_str()
            .expect("validated solutionSpace");
        let agent = item
            .get("agent")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_else(|| policy.default_agent());
        let definition = find_definition(defs, agent).ok_or_else(|| {
            let available = defs
                .iter()
                .filter(|def| policy.available(def, settings))
                .map(|def| def.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "{label}: unknown agent `{agent}`. Available agents: {}.",
                if available.is_empty() {
                    "none"
                } else {
                    &available
                }
            )
        })?;
        if !policy.allows(definition) {
            return Err(format!(
                "{label}: agent `{}` is not allowed. Allowed agents: {}.",
                definition.name,
                policy.description()
            ));
        }
        settings.ensure_subagent_role_enabled(&definition.name)?;
        if !definition.unsupported.is_empty() {
            return Err(format!(
                "{label}: agent `{}` requires unavailable capabilities: {}.",
                definition.name,
                definition.unsupported.join(", ")
            ));
        }
        planned.push(PlannedTask {
            name,
            definition: definition.clone(),
            task: task.trim().to_string(),
            solution_space: solution_space.to_string(),
            options: spawn::SpawnOptions::default(),
            policy_key: String::new(),
        });
    }
    Ok((context.to_string(), planned))
}

fn allocate_id(parent: &str, name: &str, taken: &mut HashSet<String>) -> String {
    let base = if parent == MAIN_AGENT_ID {
        name.to_string()
    } else {
        format!("{parent}.{name}")
    };
    let mut id = base.clone();
    let mut suffix = 2;
    while id.eq_ignore_ascii_case("@user") || taken.contains(&id.to_lowercase()) {
        id = format!("{base}-{suffix}");
        suffix += 1;
    }
    taken.insert(id.to_lowercase());
    id
}

fn soft_error(content: String) -> McpToolCallResult {
    McpToolCallResult {
        content,
        is_error: true,
        ..Default::default()
    }
}

async fn prepare(
    ctx: &SubAgentCallCtx<'_>,
    invocation: spawn::ModelInvocation,
) -> Result<(Settings, String, String, Vec<PlannedTask>), String> {
    if ctx.native_ctx.depth > 0 && crate::chat::plan::is_plan_mode(ctx.native_ctx.plan_state) {
        return Err("Plan-mode subagents cannot spawn additional agents.".into());
    }
    let root = &ctx.native_ctx.conversation_id;
    let admission = ctx.state.sub_agents.admission.root(root);
    if *admission
        .allocation
        .lock()
        .unwrap_or_else(|e| e.into_inner())
    {
        return Err("The conversation was deleted.".into());
    }
    let settings = ctx.state.settings_read().clone();
    let conversation = crate::chat::storage::load_conversation(ctx.app, root)?;
    let cwd = super::lifecycle::cwd_for_run(&ctx.state.sub_agents, &ctx.native_ctx.run_id)
        .map(Ok)
        .unwrap_or_else(|| {
            crate::chat::storage::resolve_conversation_working_directory(
                ctx.app,
                &conversation,
                &settings.chat_tools.native_tools.working_directory,
            )
        })?;
    let catalog = ctx
        .state
        .chat_runtime()
        .resource_catalog(
            ctx.app,
            root,
            &settings.chat_tools.skill_scan_paths,
            Some(&cwd),
            false,
        )
        .await?;
    let owner = ctx.state.sub_agents.owner_for_run(&ctx.native_ctx.run_id);
    let parent = if owner == MAIN_AGENT_ID {
        if ctx.native_ctx.depth != 0 {
            return Err("Cannot determine the spawning agent's policy.".into());
        }
        None
    } else {
        let record = ctx
            .state
            .sub_agents
            .registry
            .get(root, &owner)
            .ok_or("The spawning agent is no longer registered.")?;
        Some(
            find_definition(&catalog.agents, &record.agent_type).ok_or_else(|| {
                format!(
                    "The spawning agent definition `{}` is unavailable.",
                    record.agent_type
                )
            })?,
        )
    };
    let (context, mut planned) = preflight(
        ctx.arguments,
        &catalog.agents,
        parent.and_then(|def| def.spawns.as_deref()),
        ctx.native_ctx.depth,
        &settings.chat_tools,
    )?;
    if owner != MAIN_AGENT_ID {
        let saved =
            super::lifecycle::spawn_policy_for_run(&ctx.state.sub_agents, &ctx.native_ctx.run_id)
                .ok_or("Spawning authority is unavailable for this run")?;
        let policy = SpawnPolicy::new(Some(&saved));
        if let Some(item) = planned.iter().find(|item| !policy.allows(&item.definition)) {
            return Err(format!(
                "Agent `{}` is outside this session's saved spawn grant",
                item.definition.name
            ));
        }
    }
    for (index, item) in planned.iter_mut().enumerate() {
        let raw = ctx.arguments["tasks"][index]
            .as_object()
            .expect("preflight task object");
        let (task, source) = spawn::read_task_source(
            raw,
            settings.chat_tools.task_file_enabled,
            std::path::Path::new(&cwd),
        )?;
        item.task = task;
        let inherited_schema = super::lifecycle::completion_schema_for_run(
            &ctx.state.sub_agents,
            &ctx.native_ctx.run_id,
        );
        item.options = spawn::options(
            raw,
            &item.definition,
            &settings,
            source,
            crate::chat::plan::is_plan_mode(ctx.native_ctx.plan_state),
            inherited_schema.as_ref(),
            invocation,
        )?;
        let shared = ctx
            .state
            .sub_agents
            .eval
            .shared(root, &owner, &item.options.tools)
            .await?;
        if !shared.is_empty() {
            let mounted = crate::mcp::registry::list_enabled_tool_catalog_for_run(
                ctx.app,
                ctx.state,
                Some(ctx.native_ctx.mcp_server_ids),
            )
            .await;
            if let Some(tool) = shared.iter().find(|tool| {
                mounted
                    .tools
                    .iter()
                    .any(|mounted| mounted.openai_tool_name() == tool.definition.name)
            }) {
                return Err(format!(
                    "Eval tool `{}` conflicts with an enabled tool",
                    tool.definition.name
                ));
            }
        }
        if let Some(source) = &item.options.source_session {
            let selected = super::seance::resolve_source(
                source,
                std::path::Path::new(&cwd),
                &registry::artifacts_dir(ctx.app, root)?,
            )?;
            item.options.source_session = Some(selected.file.to_string_lossy().into_owned());
            if item.options.model.selector.is_none() {
                item.options.model.selector = Some(selected.model_selector);
            }
        }
        if let Some(Some(models)) = settings
            .chat_tools
            .task_agent_model_overrides
            .get(&item.definition.name)
        {
            if item.options.model.selector.is_none() {
                item.options.model.role_models = models.clone();
            }
        }
        let resolved = spawn::resolve_model(
            &settings,
            &item.options.model,
            &ctx.native_ctx.provider.id,
            ctx.native_ctx.model,
        )?;
        item.policy_key = serde_json::to_string(&json!({
            "definition":item.definition,"options":item.options,
            "provider":resolved.provider.id,"model":resolved.model,"thinkingLevel":resolved.thinking_level,
            "maxEffort":settings.chat_tools.task_max_effort,
            "perCallModel":settings.chat_tools.task_per_call_model,"allow":settings.chat_tools.task_per_call_model_allow,
            "evalTools":shared.iter().map(|tool| json!({"kernelId":tool.kernel_id,"descriptor":tool.descriptor})).collect::<Vec<_>>(),
        })).map_err(|error| error.to_string())?;
    }
    Ok((settings, owner, context, planned))
}

fn disk_names(artifacts: &std::path::Path, taken: &mut HashSet<String>) -> Result<(), String> {
    std::fs::create_dir_all(artifacts)
        .map_err(|e| format!("Creating subagent artifact directory failed: {e}"))?;
    for entry in std::fs::read_dir(artifacts)
        .map_err(|e| format!("Reading subagent artifacts failed: {e}"))?
    {
        let path = entry
            .map_err(|e| format!("Reading subagent artifact entry failed: {e}"))?
            .path();
        let accepted = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".jsonl.accepted"));
        let stem = if accepted.is_some() {
            accepted
        } else if matches!(
            path.extension().and_then(|s| s.to_str()),
            Some("jsonl" | "md")
        ) {
            path.file_stem().and_then(|s| s.to_str())
        } else {
            None
        };
        if let Some(stem) = stem {
            taken.insert(stem.to_lowercase());
        }
    }
    Ok(())
}

fn reserve(
    ctx: &SubAgentCallCtx<'_>,
    owner: &str,
    planned: &[PlannedTask],
    adopted: &[Option<SpeculativeRun>],
    published: bool,
) -> Result<Vec<registry::AgentRecord>, String> {
    let root = &ctx.native_ctx.conversation_id;
    let admission = ctx.state.sub_agents.admission.root(root);
    let allocation = admission
        .allocation
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if *allocation {
        return Err("The conversation was deleted.".into());
    }
    let artifacts = registry::artifacts_dir(ctx.app, root)?;
    let mut taken: HashSet<String> = ctx
        .state
        .sub_agents
        .registry
        .list(root)
        .into_iter()
        .map(|record| record.id.to_lowercase())
        .collect();
    taken.extend([MAIN_AGENT_ID.to_lowercase(), "all".into()]);
    disk_names(&artifacts, &mut taken)?;
    let mut records = Vec::with_capacity(planned.len());
    for (index, item) in planned.iter().enumerate() {
        if let Some(run) = adopted.get(index).and_then(Option::as_ref) {
            records.push(run.record.clone());
            continue;
        }
        let name = item.name.clone().unwrap_or_else(names::generate_task_name);
        let id = allocate_id(owner, &name, &mut taken);
        if !valid_agent_id(&id) {
            return Err("Agent id is too long or unsafe for an artifact filename.".into());
        }
        records.push(registry::AgentRecord {
            id: id.clone(),
            root: root.clone(),
            parent_id: owner.to_string(),
            generation: 0,
            display_name: name,
            agent_type: item.definition.name.clone(),
            status: registry::AgentStatus::Running,
            depth: ctx.native_ctx.depth + 1,
            child_conversation_id: format!("subagent-{root}-{id}"),
            run_id: None,
            session_file: artifacts.join(format!("{id}.jsonl")),
            output_file: artifacts.join(format!("{id}.md")),
            last_activity: 0,
            activity: None,
            published,
        });
    }
    for (index, record) in records.iter_mut().enumerate() {
        if adopted.get(index).and_then(Option::as_ref).is_none() {
            *record = ctx.state.sub_agents.registry.register(record.clone());
        }
    }
    Ok(records)
}

/// An unadopted run owns cancellation; dropping it never publishes its result.
pub(crate) struct SpeculativeRun {
    app: tauri::AppHandle,
    record: registry::AgentRecord,
    job_id: String,
    definition_key: String,
    async_enabled: bool,
    handle: Option<tauri::async_runtime::JoinHandle<()>>,
    adopted: bool,
}

impl Drop for SpeculativeRun {
    fn drop(&mut self) {
        if !self.adopted {
            self.app
                .state::<crate::state::AppState>()
                .sub_agents
                .jobs
                .discard_inline(&self.record.root, &self.job_id);
        }
    }
}

fn launch(
    ctx: &SubAgentCallCtx<'_>,
    owner: &str,
    context: &str,
    item: PlannedTask,
    record: registry::AgentRecord,
    index: usize,
    settings: &Settings,
) -> Result<SpeculativeRun, String> {
    let admission = ctx.state.sub_agents.admission.root(&record.root);
    let allocation = admission
        .allocation
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if *allocation {
        return Err("The conversation was deleted.".into());
    }
    let (job_id, mut cancel_rx) =
        ctx.state
            .sub_agents
            .jobs
            .register_inline(&record.root, owner, &record.id, &item.task);
    let ticket = admission.session(owner).enqueue_with_app(
        ctx.state.settings_read().chat_tools.task_max_concurrency,
        Some(ctx.app),
    );
    drop(allocation);
    let app = ctx.app.clone();
    let parent_run_id = ctx.native_ctx.run_id.clone();
    let parent_tool_call_id = ctx.native_ctx.tool_call_id.clone().unwrap_or_default();
    let parent_provider_id = ctx.native_ctx.provider.id.clone();
    let parent_model = ctx.native_ctx.model.to_string();
    let plan_state = ctx.native_ctx.plan_state.clone();
    let approval_policy = ctx.native_ctx.approval_policy.to_string();
    let interactive_host = ctx.native_ctx.interactive_host;
    let mcp_tool_ids = ctx.native_ctx.mcp_tool_ids.to_vec();
    let mcp_server_ids = ctx.native_ctx.mcp_server_ids.to_vec();
    let context = context.to_string();
    let definition_key = if record.published {
        String::new()
    } else {
        item.policy_key.clone()
    };
    let run_record = record.clone();
    let run_job = job_id.clone();
    emit_progress(
        ctx.app,
        &parent_run_id,
        &parent_tool_call_id,
        &record,
        "pending",
    );
    let handle = tauri::async_runtime::spawn(async move {
        let Some(permit) = ticket.wait(&mut cancel_rx).await else {
            let state = app.state::<crate::state::AppState>();
            state.sub_agents.registry.update_checked(
                &run_record.root,
                &run_record.id,
                run_record.generation,
                None,
                |current| {
                    current.status = registry::AgentStatus::Idle;
                    current.run_id = None;
                    current.activity = None;
                },
            );
            state
                .sub_agents
                .jobs
                .settle(&run_job, Err("Task cancelled before execution.".into()));
            if let Err(error) = crate::chat::protocol::emit_run_event_or_live_durable(
                &app,
                &run_record.root,
                &parent_run_id,
                progress_event(&parent_tool_call_id, &run_record, "aborted"),
            )
            .await
            {
                eprintln!("Could not persist cancelled task progress: {error}");
            }
            return;
        };
        executor::run(
            app,
            executor::Request {
                record: run_record,
                job_id: run_job,
                index,
                definition: item.definition,
                task: item.task,
                solution_space: item.solution_space,
                context,
                options: item.options,
                parent_run_id,
                parent_tool_call_id,
                parent_provider_id,
                parent_model,
                plan_state,
                mcp_tool_ids,
                mcp_server_ids,
                approval_policy,
                interactive_host,
                cancel_rx,
                permit,
            },
        )
        .await;
    });
    Ok(SpeculativeRun {
        app: ctx.app.clone(),
        record,
        job_id,
        definition_key,
        async_enabled: settings.chat_tools.task_async_enabled,
        handle: Some(handle),
        adopted: false,
    })
}

pub(crate) async fn start_speculative(
    app: &tauri::AppHandle,
    native_ctx: &crate::mcp::registry::NativeToolContext<'_>,
    prefix_args: &Value,
    index: usize,
) -> Option<SpeculativeRun> {
    if prefix_args
        .get("tasks")
        .and_then(Value::as_array)
        .is_some_and(|items| items.iter().any(|item| item.get("taskFile").is_some()))
    {
        return None;
    }
    let state = app.state::<crate::state::AppState>();
    let ctx = SubAgentCallCtx {
        app,
        state: &state,
        native_ctx,
        arguments: prefix_args,
    };
    let (settings, owner, context, planned) =
        prepare(&ctx, spawn::ModelInvocation::Task).await.ok()?;
    if !settings.chat_tools.task_speculative_launch {
        return None;
    }
    let item = planned.get(index)?.clone();
    let record = reserve(&ctx, &owner, std::slice::from_ref(&item), &[], false)
        .ok()?
        .pop()?;
    launch(&ctx, &owner, &context, item, record, index, &settings).ok()
}

pub fn dispatch(ctx: SubAgentCallCtx<'_>) -> NativeToolFuture<'_> {
    dispatch_with_mode(ctx, spawn::ModelInvocation::Task, Vec::new(), false)
}

pub(crate) fn dispatch_eval(ctx: SubAgentCallCtx<'_>) -> NativeToolFuture<'_> {
    dispatch_with_mode(ctx, spawn::ModelInvocation::Eval, Vec::new(), false)
}

pub(crate) fn dispatch_pool(ctx: SubAgentCallCtx<'_>, items: Vec<String>) -> NativeToolFuture<'_> {
    dispatch_with_mode(ctx, spawn::ModelInvocation::Eval, items, true)
}

fn dispatch_with_mode(
    ctx: SubAgentCallCtx<'_>,
    invocation: spawn::ModelInvocation,
    workpool_items: Vec<String>,
    force_inline: bool,
) -> NativeToolFuture<'_> {
    Box::pin(async move {
        let (settings, owner, context, mut planned) = match prepare(&ctx, invocation).await {
            Ok(prepared) => prepared,
            Err(error) => {
                ctx.state.sub_agents.speculative.discard(ctx.native_ctx);
                return Ok(soft_error(error));
            }
        };
        if !workpool_items.is_empty() {
            if planned.len() != 1 {
                return Err("A workpool batch requires exactly one worker".into());
            }
            planned[0].options.workpool_items = workpool_items;
        }
        if invocation == spawn::ModelInvocation::Eval
            && !force_inline
            && !settings.chat_tools.task_async_enabled
        {
            return Ok(soft_error("agent() requires async jobs".into()));
        }
        let mut adopted = if invocation == spawn::ModelInvocation::Eval {
            Vec::new()
        } else {
            ctx.state
                .sub_agents
                .speculative
                .adopt(ctx.native_ctx, ctx.arguments)
                .await
        };
        if adopted.iter().enumerate().any(|(index, run)| {
            run.as_ref().is_some_and(|run| {
                run.async_enabled != settings.chat_tools.task_async_enabled
                    || planned
                        .get(index)
                        .is_none_or(|item| item.policy_key != run.definition_key)
            })
        }) {
            adopted.clear();
        }
        let records = reserve(&ctx, &owner, &planned, &adopted, false)?;
        let mut all = Vec::with_capacity(planned.len());
        let mut started = Vec::new();
        let mut inline = Vec::new();
        for (index, (item, record)) in planned.into_iter().zip(records).enumerate() {
            let background = if invocation == spawn::ModelInvocation::Eval {
                !force_inline
            } else {
                settings.chat_tools.task_async_enabled && !item.definition.blocking
            };
            let record = {
                let admission = ctx.state.sub_agents.admission.root(&record.root);
                let allocation = admission
                    .allocation
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if *allocation {
                    return Err("The conversation was deleted.".into());
                }
                ctx.state.sub_agents.registry.publish(
                    &record.root,
                    &record.id,
                    record.generation,
                )?
            };
            let mut run = match adopted.get_mut(index).and_then(Option::take) {
                Some(run) => run,
                None => launch(&ctx, &owner, &context, item, record, index, &settings)?,
            };
            if !run.record.published {
                replay_progress(
                    ctx.app,
                    &ctx.native_ctx.run_id,
                    ctx.native_ctx.tool_call_id.as_deref().unwrap_or_default(),
                    &run,
                )
                .await;
            }
            let entry = json!({"agentId": run.record.id, "jobId": run.job_id,
                "agent": run.record.agent_type, "name": run.record.display_name});
            all.push(entry.clone());
            if background {
                ctx.state
                    .sub_agents
                    .jobs
                    .promote_inline(&run.record.root, &run.job_id);
                run.adopted = true;
                started.push(entry);
            } else {
                inline.push(run);
            }
        }
        let mut results = Vec::new();
        for mut run in inline {
            if let Some(handle) = run.handle.take() {
                if let Err(error) = handle.await {
                    ctx.state
                        .sub_agents
                        .jobs
                        .settle(&run.job_id, Err(format!("Task execution failed: {error}")));
                }
            }
            let result = ctx
                .state
                .sub_agents
                .jobs
                .take_inline(&run.record.root, &run.job_id)
                .ok_or_else(|| {
                    format!(
                        "Inline task `{}` ended without a retained result.",
                        run.record.id
                    )
                })?;
            results.push(result);
            run.adopted = true;
        }
        let mut progress = serde_json::Map::new();
        for entry in &all {
            let id = entry["agentId"].as_str().unwrap_or_default();
            let job_id = entry["jobId"].as_str().unwrap_or_default();
            let snapshot = results
                .iter()
                .find(|r| r.id == job_id)
                .cloned()
                .or_else(|| ctx.state.sub_agents.jobs.get(job_id));
            if let Some(snapshot) = snapshot {
                let status = match snapshot.status {
                    super::jobs::JobStatus::Running => "running",
                    super::jobs::JobStatus::Completed => "completed",
                    super::jobs::JobStatus::Failed => "failed",
                    super::jobs::JobStatus::Cancelled => "aborted",
                };
                let mut item = snapshot.progress.unwrap_or_else(|| json!({}));
                item["taskId"] = json!(id);
                item["name"] = entry["name"].clone();
                item["status"] = json!(status);
                if item.get("preview").and_then(Value::as_str).is_none() {
                    if let Some(result) = snapshot.result {
                        item["preview"] = json!(result);
                    }
                }
                progress.insert(id.to_string(), item);
            }
        }
        let mut content = results
            .iter()
            .map(|r| {
                r.result
                    .as_deref()
                    .unwrap_or("Task was cancelled before a result was available.")
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        if !started.is_empty() {
            let labels = started
                .iter()
                .filter_map(|v| v["agent"].as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let receipt = prompts::render(
                prompts::TASK_SPAWN_FEEDBACK,
                &json!({
                    "count": started.len(), "mixed": !results.is_empty(),
                    "singular": started.len() == 1, "singleCall": all.len() == 1,
                    "agentId": started[0]["agentId"], "jobId": started[0]["jobId"],
                    "agentLabel": labels, "scheduleFailureSummary": "",
                    "showListing": started.len() > 1, "started": started,
                }),
            );
            if !content.is_empty() {
                content.push_str("\n\n");
            }
            content.push_str(receipt.trim());
        }
        let failed = results
            .iter()
            .any(|r| r.status != super::jobs::JobStatus::Completed);
        let structured = json!({"agents": all, "results": results,
            "subagentProgressByTask": progress,
            "async": {"state": if started.is_empty() { if failed { "failed" } else { "completed" } } else { "running" }}});
        Ok(McpToolCallResult {
            content,
            is_error: failed,
            raw: structured.clone(),
            structured_content: Some(structured),
            ..Default::default()
        })
    })
}

fn progress_event(
    tool_call_id: &str,
    record: &registry::AgentRecord,
    status: &str,
) -> crate::chat::protocol::ChatRunEvent {
    crate::chat::protocol::ChatRunEvent::SubagentUpdated {
        external_session_id: None,
        parent_tool_call_id: tool_call_id.into(),
        task_id: record.id.clone(),
        name: record.display_name.clone(),
        model: None,
        depth: record.depth,
        status: status.into(),
        preview: (status == "aborted").then(|| "Task cancelled before execution.".into()),
        steps: Vec::new(),
        generation: Some(record.generation),
        child_run_id: record.run_id.clone(),
        progress: None,
    }
}

fn emit_progress(
    app: &tauri::AppHandle,
    run_id: &str,
    tool_call_id: &str,
    record: &registry::AgentRecord,
    status: &str,
) {
    crate::chat::protocol::emit_run_event_or_live(
        app,
        &record.root,
        run_id,
        progress_event(tool_call_id, record, status),
    );
}

async fn replay_progress(
    app: &tauri::AppHandle,
    run_id: &str,
    tool_call_id: &str,
    run: &SpeculativeRun,
) {
    let state = app.state::<crate::state::AppState>();
    let Some(snapshot) = state.sub_agents.jobs.get(&run.job_id) else {
        return;
    };
    let mut record = run.record.clone();
    record.run_id = Some(format!("subrun-{}-{}", record.generation, run.job_id));
    if snapshot.execution_started {
        crate::chat::protocol::record_subagent_started(app, &record, run_id, tool_call_id).await;
    }
    let status = match snapshot.status {
        super::jobs::JobStatus::Running if !snapshot.execution_started => "pending",
        super::jobs::JobStatus::Running => "running",
        super::jobs::JobStatus::Completed => "completed",
        super::jobs::JobStatus::Failed => "failed",
        super::jobs::JobStatus::Cancelled => "aborted",
    };
    let preview = snapshot
        .progress
        .as_ref()
        .and_then(|p| p["preview"].as_str())
        .map(str::to_string)
        .or(snapshot.result)
        .or_else(|| {
            snapshot
                .progress
                .as_ref()
                .and_then(|p| p["recentOutput"].as_array())
                .map(|lines| {
                    lines
                        .iter()
                        .rev()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("\n")
                })
        });
    let mut event = progress_event(tool_call_id, &record, status);
    if let crate::chat::protocol::ChatRunEvent::SubagentUpdated {
        preview: target, ..
    } = &mut event
    {
        *target = preview;
    }
    if let Err(error) =
        crate::chat::protocol::emit_run_event_or_live_durable(app, &run.record.root, run_id, event)
            .await
    {
        eprintln!("Could not persist adopted task progress: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch(items: Value) -> Value {
        json!({"context": "shared", "tasks": items})
    }

    fn one(name: &str, agent: &str) -> Value {
        json!({"name": name, "agent": agent, "task": "do work", "solutionSpace": "one fix"})
    }

    #[test]
    fn validates_the_whole_batch_and_wire_shape() {
        let defs = crate::agents::builtin_agent_definitions();
        let settings = ChatToolsConfig::default();
        let valid = batch(json!([one("Alpha", "task"), one("Beta", "scout")]));
        assert_eq!(
            preflight(&valid, &defs, None, 0, &settings)
                .unwrap()
                .1
                .len(),
            2
        );
        for args in [
            json!({"context": "", "tasks": [one("A", "task")]}),
            batch(json!([])),
            json!({"context": "x", "task": "old", "tasks": [one("A", "task")]}),
            json!({"context": "x", "agent": "task", "tasks": [one("A", "task")]}),
            batch(json!([one("Alpha", "task"), one(" alpha ", "task")])),
            batch(json!([one("A", "task"), one("B", "missing")])),
            batch(json!([{"task": " ", "solutionSpace": "x"}])),
            batch(json!([{"task": "x"}])),
            batch(json!([{"task": "x", "solutionSpace": 3}])),
            batch(json!([{"task": "x", "solutionSpace": "x", "extra": 1}])),
            batch(json!([one("../secret", "task")])),
            batch(json!([one("con", "task")])),
            batch(json!([one("test.CON.txt", "task")])),
            batch(json!([one("A..B", "task")])),
            batch(json!([one("A.", "task")])),
            batch(json!([one("A\\B", "task")])),
            batch(json!([one("A:B", "task")])),
        ] {
            assert!(
                preflight(&args, &defs, None, 0, &settings).is_err(),
                "accepted {args}"
            );
        }
        assert!(preflight(
            &batch(json!([one("A", "unknown")])),
            &defs,
            None,
            0,
            &settings
        )
        .unwrap_err()
        .contains("Available agents:"));
        let names = batch(json!([one("  测试 Alpha.Beta  ", "task")]));
        assert_eq!(
            preflight(&names, &defs, None, 0, &settings).unwrap().1[0]
                .name
                .as_deref(),
            Some("测试 Alpha.Beta")
        );
        let duplicates = batch(json!([one("ÄBC", "task"), one("äbc", "task")]));
        assert!(preflight(&duplicates, &defs, None, 0, &settings).is_err());
    }

    #[test]
    fn policy_enforces_role_and_depth_limits() {
        let defs = crate::agents::builtin_agent_definitions();
        let mut settings = ChatToolsConfig::default();
        let no_agent = batch(json!([{"task": "x", "solutionSpace": "x"}]));
        assert_eq!(
            preflight(&no_agent, &defs, Some("scout, task"), 1, &settings)
                .unwrap()
                .1[0]
                .definition
                .name,
            "scout"
        );
        assert!(preflight(
            &batch(json!([one("A", "task")])),
            &defs,
            Some("scout"),
            0,
            &settings
        )
        .is_err());
        assert!(preflight(&no_agent, &defs, Some(""), 0, &settings).is_err());
        assert!(preflight(&no_agent, &defs, None, 2, &settings).is_err());
        settings.task_max_recursion_depth = -1;
        assert!(preflight(&no_agent, &defs, None, 8, &settings).is_ok());
    }

    #[test]
    fn disabled_role_is_hidden_and_cannot_be_selected_by_alias_or_default() {
        let mut defs = crate::agents::builtin_agent_definitions();
        defs.iter_mut()
            .find(|role| role.name == "scout")
            .unwrap()
            .id = "scout-alias".into();
        let mut settings = Settings::default();
        settings.chat_tools.task_disabled_agents = vec!["task".into(), "scout".into()];
        let no_agent = batch(json!([{"task": "x", "solutionSpace": "x"}]));
        for args in [
            no_agent,
            batch(json!([one("A", "SCOUT"), one("B", "reviewer")])),
            batch(json!([one("A", "scout-alias")])),
        ] {
            assert!(preflight(&args, &defs, None, 0, &settings.chat_tools)
                .unwrap_err()
                .contains("disabled in settings"));
        }
        let description = tool_definition(&defs, &settings).description;
        assert!(!description
            .lines()
            .any(|line| line.starts_with("- `scout`")));
        assert!(description
            .lines()
            .any(|line| line.starts_with("- `reviewer`")));
        settings.chat_tools.task_disabled_agents.clear();
        assert!(preflight(
            &batch(json!([one("A", "scout-alias")])),
            &defs,
            None,
            0,
            &settings.chat_tools
        )
        .is_ok());
    }

    #[test]
    fn identities_are_case_insensitive_unique_and_nested() {
        let mut taken = HashSet::from([
            "main".to_string(),
            "all".to_string(),
            "alpha".to_string(),
            "alpha-2".to_string(),
            "alpha.child".to_string(),
        ]);
        assert_eq!(allocate_id(MAIN_AGENT_ID, "Alpha", &mut taken), "Alpha-3");
        assert_eq!(allocate_id("Alpha", "Child", &mut taken), "Alpha.Child-2");
        assert_eq!(allocate_id("Other", "Child", &mut taken), "Other.Child");
        assert_eq!(allocate_id(MAIN_AGENT_ID, "Main", &mut taken), "Main-2");
        assert_eq!(allocate_id(MAIN_AGENT_ID, "all", &mut taken), "all-2");
        assert!(valid_agent_id(&names::generate_task_name()));
        assert!(valid_agent_id("测试 Agent.child"));
        assert!(!valid_agent_id(&format!("{}.child", "a".repeat(244))));
        assert!(!valid_agent_id("parent.COM¹.txt"));
    }

    #[test]
    fn task_ids_cannot_claim_the_human_irc_identity() {
        let mut taken = HashSet::new();
        assert_eq!(allocate_id(MAIN_AGENT_ID, "@UsEr", &mut taken), "@UsEr-2");
        assert_eq!(allocate_id("Scout", "@user", &mut taken), "Scout.@user");
    }

    #[tokio::test]
    async fn admission_is_fifo_and_removes_cancelled_waiters() {
        let admission = Admission::default();
        let root = admission.root("root").session(MAIN_AGENT_ID);
        let (_tx, mut cancel) = watch::channel(false);
        let first = root.enqueue(1).wait(&mut cancel).await.unwrap();
        let (cancel_tx, mut canceled) = watch::channel(false);
        let canceled_ticket = root.enqueue(1);
        let mut second = root.enqueue(1);
        let mut third = root.enqueue(1);
        cancel_tx.send(true).unwrap();
        assert!(canceled_ticket.wait(&mut canceled).await.is_none());
        assert_eq!(root.state.lock().unwrap().queue.len(), 2);
        assert!(second.receiver.try_recv().is_err());
        drop(first);
        let second_permit = second.receiver.try_recv().unwrap();
        assert!(third.receiver.try_recv().is_err());
        drop(second_permit);
        let third_permit = third.receiver.try_recv().unwrap();
        drop(third_permit);
        assert_eq!(root.state.lock().unwrap().active, 0);
    }

    #[tokio::test]
    async fn admission_resize_preserves_fifo_and_unlimited_semantics() {
        let root = SessionAdmission::default();
        let root = Arc::new(root);
        let (_tx, mut cancel) = watch::channel(false);
        let first = root.enqueue(1).wait(&mut cancel).await.unwrap();
        let mut second = root.enqueue(1);
        let mut third = root.enqueue(2);
        let second_permit = second.receiver.try_recv().unwrap();
        assert!(third.receiver.try_recv().is_err());
        let mut fourth = root.enqueue(1);
        drop(first);
        assert!(
            third.receiver.try_recv().is_err(),
            "shrinking must not grant while at capacity"
        );
        drop(second_permit);
        let third_permit = third.receiver.try_recv().unwrap();
        assert!(fourth.receiver.try_recv().is_err());
        let mut fifth = root.enqueue(0);
        drop(fourth.receiver.try_recv().unwrap());
        drop(fifth.receiver.try_recv().unwrap());
        drop(third_permit);
        assert_eq!(root.state.lock().unwrap().active, 0);
    }

    #[tokio::test]
    async fn descendant_can_run_while_parent_holds_its_session_permit() {
        let admission = Admission::default();
        let root = admission.root("root");
        let main = root.session(MAIN_AGENT_ID);
        let child = root.session("Child");
        let (_tx, mut cancel) = watch::channel(false);
        let parent = main.enqueue(1).wait(&mut cancel).await.unwrap();
        let mut sibling = main.enqueue(1);
        let mut descendant = child.enqueue(1);
        let descendant = descendant
            .receiver
            .try_recv()
            .expect("child session has its own capacity");
        assert!(
            sibling.receiver.try_recv().is_err(),
            "parent retains its permit while waiting"
        );
        drop(descendant);
        assert!(sibling.receiver.try_recv().is_err());
        drop(parent);
        drop(sibling.receiver.try_recv().unwrap());
    }

    #[test]
    fn old_transcript_and_output_names_are_not_reused_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Alpha.jsonl"), "old transcript").unwrap();
        std::fs::write(dir.path().join("Alpha-2.md"), "old output").unwrap();
        std::fs::write(
            dir.path().join("Alpha-3.jsonl.accepted"),
            r#"{"root":"root","id":"Alpha-3"}"#,
        )
        .unwrap();
        let mut taken = HashSet::new();
        disk_names(dir.path(), &mut taken).unwrap();
        assert_eq!(allocate_id(MAIN_AGENT_ID, "alpha", &mut taken), "alpha-4");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("Alpha.jsonl")).unwrap(),
            "old transcript"
        );
    }

    #[tokio::test]
    async fn deletion_tombstones_root_and_cancels_queued_and_future_waiters() {
        let admission = Admission::default();
        let root = admission.root("root").session(MAIN_AGENT_ID);
        let (_tx, mut cancel) = watch::channel(false);
        let running = root.enqueue(1).wait(&mut cancel).await.unwrap();
        let queued = root.enqueue(1);
        admission.remove_root("root");
        assert!(queued.wait(&mut cancel).await.is_none());
        assert!(*admission.root("root").allocation.lock().unwrap());
        assert!(root.enqueue(0).wait(&mut cancel).await.is_none());
        drop(running);
        assert_eq!(root.state.lock().unwrap().active, 0);
    }

    #[tokio::test]
    async fn cancellation_after_grant_releases_capacity() {
        let admission = Admission::default();
        let root = admission.root("root").session(MAIN_AGENT_ID);
        let (tx, mut cancel) = watch::channel(false);
        let ticket = root.enqueue(1);
        tx.send(true).unwrap();
        assert!(ticket.wait(&mut cancel).await.is_none());
        assert_eq!(root.state.lock().unwrap().active, 0);
    }
}
