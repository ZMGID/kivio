//! OMP batch task dispatch and conversation-scoped spawn admission.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tauri::Manager;
use tokio::sync::{oneshot, watch};

use crate::agents::{find_definition, AgentDefinition};
use crate::mcp::native_registry::NativeToolFuture;
use crate::mcp::types::{ChatToolDefinition, McpToolCallResult};
use crate::settings::Settings;

use super::{executor, names, prompts, registry, SubAgentCallCtx, MAIN_AGENT_ID, TASK_TOOL_NAME};

#[derive(Default)]
pub(crate) struct Admission {
    roots: Mutex<HashMap<String, Arc<RootAdmission>>>,
}

#[derive(Default)]
struct RootAdmission {
    // Covers identity allocation and registration as one operation per root.
    allocation: Mutex<bool>,
    state: Mutex<AdmissionState>,
}

#[derive(Default)]
struct AdmissionState {
    max: usize,
    active: usize,
    next_ticket: u64,
    retired: bool,
    queue: VecDeque<(u64, oneshot::Sender<Permit>)>,
}

impl Admission {
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
        let mut retired = admission.allocation.lock().unwrap_or_else(|e| e.into_inner());
        *retired = true;
        let mut state = admission.state.lock().unwrap_or_else(|e| e.into_inner());
        state.retired = true;
        state.queue.clear();
    }
}

impl RootAdmission {
    fn enqueue(self: &Arc<Self>, max: usize) -> Ticket {
        let (sender, receiver) = oneshot::channel();
        let (id, grants) = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.max = max;
            state.next_ticket += 1;
            let id = state.next_ticket;
            if !state.retired {
                state.queue.push_back((id, sender));
            }
            (id, state.grants())
        };
        self.send_grants(grants);
        Ticket { root: self.clone(), id, receiver }
    }

    fn send_grants(self: &Arc<Self>, grants: Vec<oneshot::Sender<Permit>>) {
        // Send after unlocking: a closed receiver drops its permit immediately.
        for sender in grants {
            let _ = sender.send(Permit { root: self.clone() });
        }
    }

    fn release(self: &Arc<Self>) {
        let grants = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.active -= 1;
            state.grants()
        };
        self.send_grants(grants);
    }
}

impl AdmissionState {
    fn grants(&mut self) -> Vec<oneshot::Sender<Permit>> {
        let mut grants = Vec::new();
        while self.max == 0 || self.active < self.max {
            let Some((_, sender)) = self.queue.pop_front() else { break };
            if !sender.is_closed() {
                self.active += 1;
                grants.push(sender);
            }
        }
        grants
    }
}

struct Ticket {
    root: Arc<RootAdmission>,
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
        if let Some(index) = state.queue.iter().position(|(id, _)| *id == self.id) {
            state.queue.remove(index);
        }
    }
}

/// One running subagent. Dropping it transfers capacity to the oldest waiter.
pub(crate) struct Permit {
    root: Arc<RootAdmission>,
}

/// A subagent waiting for its children does not count against the running limit.
pub(crate) struct SuspendedPermit {
    root: Arc<RootAdmission>,
}

impl Permit {
    pub(crate) fn release(self) -> SuspendedPermit {
        let root = self.root.clone();
        drop(self);
        SuspendedPermit { root }
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.root.release();
    }
}

impl SuspendedPermit {
    pub(crate) async fn reacquire(self, max: usize, cancel: &mut watch::Receiver<bool>) -> Option<Permit> {
        self.root.enqueue(max).wait(cancel).await
    }
}

struct SpawnPolicy<'a> {
    allowed: Option<Vec<&'a str>>,
}

impl<'a> SpawnPolicy<'a> {
    fn new(spawns: Option<&'a str>) -> Self {
        let spawns = spawns.unwrap_or("*").trim();
        Self {
            allowed: (spawns != "*").then(|| spawns.split(',').map(str::trim).filter(|s| !s.is_empty()).collect()),
        }
    }

    fn default_agent(&self) -> &str {
        self.allowed.as_ref().and_then(|names| names.first().copied()).unwrap_or("task")
    }

    fn allows(&self, definition: &AgentDefinition) -> bool {
        self.allowed.as_ref().is_none_or(|names| names.iter().any(|name| {
            definition.name.eq_ignore_ascii_case(name) || definition.id.eq_ignore_ascii_case(name)
        }))
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
    tool_definition_with_policy(defs, settings, None)
}

pub(crate) fn tool_definition_with_policy(
    defs: &[AgentDefinition],
    _settings: &Settings,
    spawns: Option<&str>,
) -> ChatToolDefinition {
    let policy = SpawnPolicy::new(spawns);
    let available: Vec<_> = defs.iter().filter(|def| policy.allows(def)).collect();
    let agent_list = available.iter()
        .map(|def| {
            let read_only = def.tools.as_ref().is_some_and(|tools| {
                !tools.is_empty() && tools.iter().all(|tool| matches!(tool.as_str(),
                    "read" | "wait" | "grep" | "glob" | "find" | "web_search" | "ast_grep" |
                    "yield" | "ask" | "todo" | "recall" | "reflect" | "retain" | "memory_edit" |
                    "checkpoint" | "rewind"))
            });
            let label = if read_only { " (READ-ONLY; investigation only, no edits)" } else { "" };
            format!("- `{}`{}: {}\n", def.name, label, def.description)
        })
        .collect::<String>();
    let async_guidance = prompts::render(prompts::TASK_ASYNC_CONTRACT, &json!({
        "waitTool": true, "ircEnabled": false
    }));
    let description = prompts::render(prompts::TASK_TOOL, &json!({
        "asyncGuidance": async_guidance,
        "defaultAgent": policy.default_agent(),
        "scoutAvailable": available.iter().any(|def| def.name.eq_ignore_ascii_case("scout")),
        "spawningDisabled": policy.allowed.as_ref().is_some_and(Vec::is_empty),
        "hasModelMentions": false,
        "ircEnabled": false,
        "agentList": agent_list,
    }));
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
                    "properties": {
                        "name": {"type": "string"},
                        "agent": {"type": "string"},
                        "task": {"type": "string", "minLength": 1},
                        "solutionSpace": {"type": "string"}
                    },
                    "required": ["task", "solutionSpace"],
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

#[derive(Debug)]
struct PlannedTask {
    name: Option<String>,
    definition: AgentDefinition,
    task: String,
}

pub(crate) fn valid_agent_id(id: &str) -> bool {
    if id.len() > 249 || id.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*')) {
        return false;
    }
    id.split('.').all(|part| {
        if part.is_empty() || part.ends_with(' ') {
            return false;
        }
        if ["CON", "PRN", "AUX", "NUL", "COM¹", "COM²", "COM³", "LPT¹", "LPT²", "LPT³"]
            .iter().any(|reserved| part.eq_ignore_ascii_case(reserved)) {
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
    max_depth: i32,
) -> Result<(String, Vec<PlannedTask>), String> {
    if (max_depth >= 0 && i32::from(depth) >= max_depth) || depth == u8::MAX {
        return Err(format!("Task recursion depth limit reached (depth {depth}, maximum {max_depth})."));
    }
    let obj = args.as_object().ok_or("Expected an object with `context` and `tasks`.")?;
    if obj.contains_key("agent") {
        return Err("Top-level `agent` is not part of the batch shape. Set `tasks[].agent` instead.".into());
    }
    if obj.contains_key("task") {
        return Err("Top-level `task` is not part of the batch shape. Put the work in `tasks[]` items.".into());
    }
    if let Some(key) = obj.keys().find(|key| !matches!(key.as_str(), "context" | "tasks")) {
        return Err(format!("Unexpected top-level field `{key}`."));
    }
    let context = obj.get("context").and_then(Value::as_str).filter(|s| !s.trim().is_empty())
        .ok_or("Missing `context`. Provide shared background for this batch.")?;
    let tasks = obj.get("tasks").and_then(Value::as_array).filter(|items| !items.is_empty())
        .ok_or("Missing `tasks`. Provide at least one task item.")?;
    let policy = SpawnPolicy::new(spawns);
    let mut seen = HashSet::new();
    let mut planned = Vec::with_capacity(tasks.len());
    for (index, item) in tasks.iter().enumerate() {
        let label = format!("Task {}", index + 1);
        let item = item.as_object().ok_or_else(|| format!("{label} must be an object."))?;
        if let Some(key) = item.keys().find(|key| !matches!(key.as_str(), "name" | "agent" | "task" | "solutionSpace")) {
            return Err(format!("{label} has unexpected field `{key}`."));
        }
        let name = match item.get("name") {
            None => None,
            Some(Value::String(name)) if name.trim().is_empty() => None,
            Some(Value::String(name)) => {
                let name = name.trim();
                if !valid_agent_id(name) {
                    return Err(format!("{label} has an unsafe name `{name}`. Use a valid filename without path separators or reserved device names."));
                }
                if !seen.insert(name.to_lowercase()) {
                    return Err(format!("Duplicate task name `{name}`. Provided names must be unique within a call (case-insensitive)."));
                }
                Some(name.to_string())
            }
            Some(_) => return Err(format!("{label} `name` must be a string.")),
        };
        let task = item.get("task").and_then(Value::as_str).filter(|s| !s.trim().is_empty())
            .ok_or_else(|| format!("{label} is missing non-empty `task` instructions."))?;
        if item.get("solutionSpace").and_then(Value::as_str).is_none() {
            return Err(format!("{label} requires a string `solutionSpace`."));
        }
        let agent = match item.get("agent") {
            None => policy.default_agent(),
            Some(Value::String(agent)) if !agent.trim().is_empty() => agent.trim(),
            Some(_) => return Err(format!("{label} `agent` must be a non-empty string.")),
        };
        let definition = find_definition(defs, agent).ok_or_else(|| {
            let available = defs.iter().filter(|def| policy.allows(def)).map(|def| def.name.as_str()).collect::<Vec<_>>().join(", ");
            format!("{label}: unknown agent `{agent}`. Available agents: {}.", if available.is_empty() { "none" } else { &available })
        })?;
        if !policy.allows(definition) {
            return Err(format!("{label}: agent `{}` is not allowed. Allowed agents: {}.", definition.name, policy.description()));
        }
        planned.push(PlannedTask { name, definition: definition.clone(), task: task.trim().to_string() });
    }
    Ok((context.to_string(), planned))
}

fn allocate_id(parent: &str, name: &str, taken: &mut HashSet<String>) -> String {
    let base = if parent == MAIN_AGENT_ID { name.to_string() } else { format!("{parent}.{name}") };
    let mut id = base.clone();
    let mut suffix = 2;
    while taken.contains(&id.to_lowercase()) {
        id = format!("{base}-{suffix}");
        suffix += 1;
    }
    taken.insert(id.to_lowercase());
    id
}

fn soft_error(content: String) -> McpToolCallResult {
    McpToolCallResult { content, is_error: true, ..Default::default() }
}

pub fn dispatch(ctx: SubAgentCallCtx<'_>) -> NativeToolFuture<'_> {
    Box::pin(async move {
        let root = &ctx.native_ctx.conversation_id;
        let admission = ctx.state.sub_agents.admission.root(root);
        if *admission.allocation.lock().unwrap_or_else(|e| e.into_inner()) {
            return Ok(soft_error("The conversation was deleted.".into()));
        }
        let settings = ctx.state.settings_read().clone();
        let cwd = crate::chat::storage::load_conversation(ctx.app, root).ok().and_then(|conversation| {
            crate::chat::storage::resolve_conversation_working_directory(
                ctx.app, &conversation, &settings.chat_tools.native_tools.working_directory,
            ).ok()
        });
        let catalog = ctx.state.chat_runtime().resource_catalog(
            ctx.app, root, &settings.chat_tools.skill_scan_paths, cwd.as_deref(), false,
        ).await;
        if *admission.allocation.lock().unwrap_or_else(|e| e.into_inner()) {
            return Ok(soft_error("The conversation was deleted.".into()));
        }
        let catalog = catalog?;
        let owner = ctx.state.sub_agents.owner_for_run(&ctx.native_ctx.run_id);
        let parent = if owner == MAIN_AGENT_ID {
            if ctx.native_ctx.depth != 0 {
                return Ok(soft_error("Cannot determine the spawning agent's policy.".into()));
            }
            None
        } else {
            let Some(record) = ctx.state.sub_agents.registry.get(root, &owner) else {
                return Ok(soft_error("The spawning agent is no longer registered.".into()));
            };
            let Some(definition) = find_definition(&catalog.agents, &record.agent_type) else {
                return Ok(soft_error(format!("The spawning agent definition `{}` is unavailable.", record.agent_type)));
            };
            Some(definition)
        };
        let spawns = parent.and_then(|def| def.spawns.as_deref());
        let (context, planned) = match preflight(
            ctx.arguments, &catalog.agents, spawns, ctx.native_ctx.depth,
            settings.chat_tools.task_max_recursion_depth,
        ) {
            Ok(plan) => plan,
            Err(error) => return Ok(soft_error(error)),
        };
        let artifacts = registry::artifacts_dir(ctx.app, root)?;
        let mut started = Vec::with_capacity(planned.len());
        let mut launches = Vec::with_capacity(planned.len());
        {
            let _allocation = admission.allocation.lock().unwrap_or_else(|e| e.into_inner());
            if *_allocation {
                return Ok(soft_error("The conversation was deleted.".into()));
            }
            let mut taken: HashSet<String> = ctx.state.sub_agents.registry.list(root).into_iter()
                .map(|record| record.id.to_lowercase()).collect();
            taken.insert(MAIN_AGENT_ID.to_lowercase());
            taken.insert("all".into());
            let mut identities = Vec::with_capacity(planned.len());
            for item in &planned {
                let name = item.name.clone().unwrap_or_else(names::generate_task_name);
                let id = allocate_id(&owner, &name, &mut taken);
                if id.len() > 249 {
                    return Ok(soft_error("Agent id is too long for an artifact filename.".into()));
                }
                identities.push((name, id));
            }
            std::fs::create_dir_all(&artifacts)
                .map_err(|e| format!("Creating subagent artifact directory failed: {e}"))?;
            for (index, (item, (name, id))) in planned.into_iter().zip(identities).enumerate() {
                let record = ctx.state.sub_agents.registry.register(registry::AgentRecord {
                    id: id.clone(),
                    root: root.clone(),
                    parent_id: owner.clone(),
                    generation: 0,
                    display_name: name.clone(),
                    agent_type: item.definition.name.clone(),
                    status: registry::AgentStatus::Running,
                    depth: ctx.native_ctx.depth + 1,
                    child_conversation_id: format!("subagent-{root}-{id}"),
                    run_id: None,
                    session_file: artifacts.join(format!("{id}.jsonl")),
                    output_file: artifacts.join(format!("{id}.md")),
                    last_activity: 0,
                    activity: None,
                });
                let (job_id, cancel_rx) = ctx.state.sub_agents.jobs.register(root, &owner, &id, &item.task);
                let max = ctx.state.settings_read().chat_tools.task_max_concurrency;
                let ticket = admission.enqueue(max);
                started.push(json!({"agentId": id, "jobId": job_id, "agent": item.definition.name, "name": name}));
                launches.push((record, job_id, cancel_rx, ticket, item.definition, item.task, index));
            }
        }
        let parent_run_id = ctx.native_ctx.run_id.clone();
        let parent_tool_call_id = ctx.native_ctx.tool_call_id.clone().unwrap_or_default();
        let parent_provider_id = ctx.native_ctx.provider.id.clone();
        let parent_model = ctx.native_ctx.model.to_string();
        for (record, job_id, mut cancel_rx, ticket, definition, task, index) in launches {
            emit_progress(ctx.app, &parent_run_id, &parent_tool_call_id, &record, "pending");
            let app = ctx.app.clone();
            let context = context.clone();
            let parent_run_id = parent_run_id.clone();
            let parent_tool_call_id = parent_tool_call_id.clone();
            let parent_provider_id = parent_provider_id.clone();
            let parent_model = parent_model.clone();
            tauri::async_runtime::spawn(async move {
                let Some(permit) = ticket.wait(&mut cancel_rx).await else {
                    let state = app.state::<crate::state::AppState>();
                    state.sub_agents.registry.update(&record.root, &record.id, |current| {
                        if current.generation == record.generation {
                            current.status = registry::AgentStatus::Idle;
                            current.run_id = None;
                            current.activity = None;
                        }
                    });
                    emit_progress(&app, &parent_run_id, &parent_tool_call_id, &record, "aborted");
                    return;
                };
                executor::run(app, executor::Request {
                    record, job_id, index, definition, task, context,
                    parent_run_id, parent_tool_call_id, parent_provider_id, parent_model,
                    cancel_rx, permit,
                }).await;
            });
        }
        let mut labels = Vec::new();
        for item in &started {
            let label = item["agent"].as_str().unwrap_or_default();
            if !labels.contains(&label) { labels.push(label); }
        }
        let content = prompts::render(prompts::TASK_SPAWN_FEEDBACK, &json!({
            "count": started.len(),
            "mixed": false,
            "singular": started.len() == 1,
            "singleCall": started.len() == 1,
            "agentId": started[0]["agentId"],
            "jobId": started[0]["jobId"],
            "agentLabel": labels.join(", "),
            "scheduleFailureSummary": "",
            "showListing": started.len() > 1,
            "started": started,
        })).trim().to_string();
        let structured = json!({"agents": started});
        Ok(McpToolCallResult {
            content, raw: structured.clone(), structured_content: Some(structured), ..Default::default()
        })
    })
}

fn emit_progress(app: &tauri::AppHandle, run_id: &str, tool_call_id: &str, record: &registry::AgentRecord, status: &str) {
    crate::chat::protocol::emit_run_event_or_live(app, &record.root, run_id, crate::chat::protocol::ChatRunEvent::SubagentUpdated {
        parent_tool_call_id: tool_call_id.into(),
        task_id: record.id.clone(),
        name: record.display_name.clone(),
        model: None,
        depth: record.depth,
        status: status.into(),
        preview: None,
        steps: Vec::new(),
    });
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
        let valid = batch(json!([one("Alpha", "task"), one("Beta", "scout")]));
        assert_eq!(preflight(&valid, &defs, None, 0, 2).unwrap().1.len(), 2);
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
            assert!(preflight(&args, &defs, None, 0, 2).is_err(), "accepted {args}");
        }
        assert!(preflight(&batch(json!([one("A", "unknown")])), &defs, None, 0, 2)
            .unwrap_err().contains("Available agents:"));
        let names = batch(json!([one("  测试 Alpha.Beta  ", "task")]));
        assert_eq!(preflight(&names, &defs, None, 0, 2).unwrap().1[0].name.as_deref(), Some("测试 Alpha.Beta"));
        let duplicates = batch(json!([one("ÄBC", "task"), one("äbc", "task")]));
        assert!(preflight(&duplicates, &defs, None, 0, 2).is_err());
    }

    #[test]
    fn policy_defaults_depth_and_tool_description_match() {
        let defs = crate::agents::builtin_agent_definitions();
        let no_agent = batch(json!([{"task": "x", "solutionSpace": "x"}]));
        assert_eq!(preflight(&no_agent, &defs, Some("scout, task"), 1, 2).unwrap().1[0].definition.name, "scout");
        assert!(preflight(&batch(json!([one("A", "task")])), &defs, Some("scout"), 0, 2).is_err());
        assert!(preflight(&no_agent, &defs, Some(""), 0, 2).is_err());
        assert!(preflight(&no_agent, &defs, None, 2, 2).is_err());
        assert!(preflight(&no_agent, &defs, None, 8, -1).is_ok());
        let def = tool_definition_with_policy(&defs, &Settings::default(), Some("scout"));
        assert!(def.description.contains("default (`scout`)"));
        assert!(def.description.contains("- `scout` (READ-ONLY; investigation only, no edits):"));
        assert!(!def.description.contains("- `task`:"));
        assert_eq!(def.input_schema["additionalProperties"], false);
        assert_eq!(def.input_schema["properties"]["tasks"]["items"]["required"], json!(["task", "solutionSpace"]));
    }

    #[test]
    fn identities_are_case_insensitive_unique_and_nested() {
        let mut taken = HashSet::from(["main".to_string(), "all".to_string(), "alpha".to_string(), "alpha-2".to_string(), "alpha.child".to_string()]);
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

    #[tokio::test]
    async fn admission_is_fifo_and_removes_cancelled_waiters() {
        let admission = Admission::default();
        let root = admission.root("root");
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
    async fn admission_resizes_live_and_suspended_permits_rejoin_fifo() {
        let admission = Admission::default();
        let root = admission.root("root");
        let (_tx, mut cancel) = watch::channel(false);
        let first = root.enqueue(1).wait(&mut cancel).await.unwrap();
        let mut second = root.enqueue(1);
        let mut third = root.enqueue(1);
        let mut fourth = root.enqueue(2);
        let second_permit = second.receiver.try_recv().unwrap();
        assert!(third.receiver.try_recv().is_err());
        assert!(fourth.receiver.try_recv().is_err());
        let suspended = first.release();
        let third_permit = third.receiver.try_recv().unwrap();
        let mut unlimited = root.enqueue(0);
        let fourth_permit = fourth.receiver.try_recv().unwrap();
        let unlimited_permit = unlimited.receiver.try_recv().unwrap();
        let mut resumed = root.enqueue(1);
        drop(second_permit);
        drop(third_permit);
        drop(fourth_permit);
        assert!(resumed.receiver.try_recv().is_err());
        drop(unlimited_permit);
        let resumed_permit = resumed.receiver.try_recv().unwrap();
        drop(resumed_permit);
        let reacquired = suspended.reacquire(1, &mut cancel).await.unwrap();
        drop(reacquired);
        assert_eq!(root.state.lock().unwrap().active, 0);
    }

    #[tokio::test]
    async fn suspended_permit_rejoins_behind_existing_waiters() {
        let admission = Admission::default();
        let root = admission.root("root");
        let (_tx, mut cancel) = watch::channel(false);
        let first = root.enqueue(1).wait(&mut cancel).await.unwrap();
        let mut older = root.enqueue(1);
        let suspended = first.release();
        let older_permit = older.receiver.try_recv().unwrap();
        let resume = suspended.reacquire(1, &mut cancel);
        tokio::pin!(resume);
        assert!(futures::poll!(&mut resume).is_pending());
        let mut newer = root.enqueue(1);
        drop(older_permit);
        let resumed_permit = resume.await.unwrap();
        assert!(newer.receiver.try_recv().is_err());
        drop(resumed_permit);
        drop(newer.receiver.try_recv().unwrap());
        assert_eq!(root.state.lock().unwrap().active, 0);
    }

    #[tokio::test]
    async fn deletion_tombstones_root_and_cancels_queued_and_future_waiters() {
        let admission = Admission::default();
        let root = admission.root("root");
        let (_tx, mut cancel) = watch::channel(false);
        let running = root.enqueue(1).wait(&mut cancel).await.unwrap();
        let queued = root.enqueue(1);
        admission.remove_root("root");
        assert!(queued.wait(&mut cancel).await.is_none());
        assert!(*admission.root("root").allocation.lock().unwrap());
        assert!(admission.root("root").enqueue(0).wait(&mut cancel).await.is_none());
        drop(running);
        assert_eq!(root.state.lock().unwrap().active, 0);
    }

    #[tokio::test]
    async fn cancellation_after_grant_releases_capacity() {
        let admission = Admission::default();
        let root = admission.root("root");
        let (tx, mut cancel) = watch::channel(false);
        let ticket = root.enqueue(1);
        tx.send(true).unwrap();
        assert!(ticket.wait(&mut cancel).await.is_none());
        assert_eq!(root.state.lock().unwrap().active, 0);
    }
}
