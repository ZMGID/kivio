use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncWriteExt;

use super::{executor::PreparedWorker, registry::AgentRecord};

pub(crate) fn policy_tool_record(record: &crate::chat::types::ToolCallRecord) -> Option<Value> {
    if record.source != "native"
        || !matches!(record.name.as_str(), "edit" | "write" | "todo_write")
        || super::policies::PolicyToolResult::from_record(record).is_none()
    {
        return None;
    }
    let path = serde_json::from_str::<Value>(&record.arguments)
        .ok()
        .and_then(|args| args.get("path").cloned());
    let details = record.structured_content.as_ref();
    Some(serde_json::json!({
        "id":record.id, "name":record.name, "source":record.source, "status":record.status,
        "round":record.round, "trace_id":record.trace_id,
        "arguments":serde_json::json!({"path":path}).to_string(),
        "structured_content":{"xdev":details.and_then(|details| details.get("xdev")),
            "todoState":details.and_then(|details| details.get("todoState"))},
    }))
}

fn policy_result_from_message(
    message: &Value,
) -> Result<Option<crate::chat::types::ToolCallRecord>, String> {
    let Some(saved) = message.get("_subagent_policy_tool") else {
        return Ok(None);
    };
    let record: crate::chat::types::ToolCallRecord = serde_json::from_value(saved.clone())
        .map_err(|error| format!("Invalid saved policy tool result: {error}"))?;
    if message["role"] != "tool"
        || message["tool_call_id"] != record.id
        || policy_tool_record(&record).is_none()
    {
        return Err("Saved policy tool result does not match its call".into());
    }
    Ok(Some(record))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResolvedModel {
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionContract {
    pub version: u32,
    pub id: String,
    pub root: String,
    pub parent_id: String,
    pub depth: u8,
    pub generation: u64,
    pub conversation_id: String,
    pub cwd: PathBuf,
    pub system_prompt: String,
    pub runtime_context: String,
    pub task: String,
    #[serde(default)]
    pub solution_space: String,
    #[serde(default)]
    pub context: String,
    pub agent_definition: crate::agents::AgentDefinition,
    pub tools: Vec<String>,
    #[serde(default)]
    pub mcp_tool_ids: Vec<(String, String)>,
    #[serde(default)]
    pub mcp_server_ids: Vec<String>,
    #[serde(default)]
    pub eval_tools: Vec<super::eval::SavedTool>,
    #[serde(default)]
    pub policies: Option<super::policies::SessionPolicies>,
    pub read_only: bool,
    #[serde(default, deserialize_with = "null_spawn_policy")]
    pub spawns: String,
    pub resolved_model: ResolvedModel,
    pub thinking_enabled: bool,
    pub resolved_thinking_level: Option<String>,
    pub approval_policy: String,
    pub plan_state: crate::chat::types::AgentPlanState,
    #[serde(default)]
    pub options: super::spawn::SpawnOptions,
}

fn null_spawn_policy<'de, D: serde::Deserializer<'de>>(decoder: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(decoder)?.unwrap_or_default())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavedTurn {
    pub job_id: String,
    pub owner_id: String,
    pub run_id: String,
    #[serde(default)]
    pub parent_run_id: String,
    #[serde(default)]
    pub parent_tool_call_id: String,
}

pub(crate) struct WorkerSession {
    pub contract: Option<SessionContract>,
    pub prepared: Option<PreparedWorker>,
    pub history: Vec<Value>,
    pub interrupted: Option<SavedTurn>,
    pub last_progress: Option<Value>,
    pub model: Option<String>,
    pub policies: Arc<tokio::sync::Mutex<super::policies::SessionPolicies>>,
}

impl WorkerSession {
    pub fn fresh() -> Self {
        Self {
            contract: None,
            prepared: None,
            history: Vec::new(),
            interrupted: None,
            last_progress: None,
            model: None,
            policies: Arc::new(tokio::sync::Mutex::new(Default::default())),
        }
    }
}

#[derive(Default)]
pub(crate) struct Control {
    pub generation: u64,
    pub active_run: Option<String>,
    pub wake_reserved: bool,
    pub killed: bool,
    pub cwd: Option<PathBuf>,
    pub last_progress: Option<Value>,
    pub model: Option<String>,
    pub spawns: Option<String>,
    pub read_summarize: Option<bool>,
    pub completion_schema: Option<Value>,
    pub operation: u64,
}

pub(crate) struct Slot {
    pub control: Mutex<Control>,
    pub worker: tokio::sync::Mutex<Option<WorkerSession>>,
}

impl Slot {
    pub fn begin(&self, record: &AgentRecord, run: &str) -> bool {
        let mut control = self.control.lock().unwrap_or_else(|e| e.into_inner());
        if control.killed || control.generation != record.generation || control.active_run.is_some()
        {
            return false;
        }
        control.active_run = Some(run.into());
        true
    }

    pub fn owns(&self, generation: u64, run: &str) -> bool {
        let control = self.control.lock().unwrap_or_else(|e| e.into_inner());
        !control.killed
            && control.generation == generation
            && control.active_run.as_deref() == Some(run)
    }

    pub fn finish(&self, generation: u64, run: &str) -> bool {
        let mut control = self.control.lock().unwrap_or_else(|e| e.into_inner());
        if control.killed
            || control.generation != generation
            || control.active_run.as_deref() != Some(run)
        {
            return false;
        }
        control.active_run = None;
        true
    }
}

#[derive(Default)]
struct Slots {
    slots: HashMap<(String, String), Arc<Slot>>,
    retired: HashSet<String>,
}

#[derive(Default)]
pub(crate) struct Sessions(Mutex<Slots>);

impl Sessions {
    pub fn slot(&self, record: &AgentRecord) -> Result<Arc<Slot>, String> {
        let mut slots = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if slots.retired.contains(&record.root) {
            return Err("The conversation was deleted".into());
        }
        Ok(slots
            .slots
            .entry((record.root.clone(), record.id.clone()))
            .or_insert_with(|| {
                Arc::new(Slot {
                    control: Mutex::new(Control {
                        generation: record.generation,
                        ..Default::default()
                    }),
                    worker: tokio::sync::Mutex::new(None),
                })
            })
            .clone())
    }

    pub fn existing(&self, root: &str, id: &str) -> Option<Arc<Slot>> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .slots
            .get(&(root.into(), id.into()))
            .cloned()
    }

    pub fn forget(&self, root: &str) {
        let mut slots = self.0.lock().unwrap_or_else(|e| e.into_inner());
        slots.retired.insert(root.into());
        slots.slots.retain(|(owner, _), slot| {
            if owner != root {
                return true;
            }
            slot.control
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .killed = true;
            false
        });
    }
}

pub(crate) fn replay(bytes: &[u8], root: &str, id: &str) -> Result<WorkerSession, String> {
    replay_checked(bytes, root, id, true)
}

pub(crate) fn replay_for_consult(
    bytes: &[u8],
    root: &str,
    id: &str,
) -> Result<WorkerSession, String> {
    replay_checked(bytes, root, id, false)
}

fn replay_checked(
    bytes: &[u8],
    root: &str,
    id: &str,
    execution: bool,
) -> Result<WorkerSession, String> {
    if !bytes.ends_with(b"\n") {
        return Err(
            "Subagent transcript has an incomplete record; execution restore is unsafe".into(),
        );
    }
    let mut worker = WorkerSession::fresh();
    let mut transcript_tail = std::collections::VecDeque::new();
    let mut pending_policy_tools = Vec::<crate::chat::types::ToolCallRecord>::new();
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let entry: Value = serde_json::from_slice(line)
            .map_err(|error| format!("Corrupt subagent transcript: {error}"))?;
        match entry["type"].as_str() {
            Some("session_init") => {
                if worker.contract.is_some() {
                    return Err("Duplicate subagent session contract".into());
                }
                let mut contract: SessionContract = serde_json::from_value(entry)
                    .map_err(|error| format!("Invalid subagent session contract: {error}"))?;
                if contract.options.model.role_models.is_empty()
                    && contract.options.model.selector.is_none()
                {
                    contract.options.model.role_models =
                        if contract.agent_definition.models.is_empty() {
                            contract.agent_definition.model.iter().cloned().collect()
                        } else {
                            contract.agent_definition.models.clone()
                        };
                }
                if contract.version != 1
                    || contract.root != root
                    || contract.id != id
                    || !super::task::valid_agent_id(id)
                    || contract.system_prompt.trim().is_empty()
                    || contract.resolved_model.provider.is_empty()
                    || contract.resolved_model.model.is_empty()
                    || contract.parent_id == id
                    || contract.generation == 0
                    || contract.conversation_id != format!("subagent-{root}-{id}")
                {
                    return Err("Subagent session contract does not match its identity".into());
                }
                if let Some(policies) = &contract.policies {
                    worker.policies = Arc::new(tokio::sync::Mutex::new(policies.clone()));
                }
                worker.contract = Some(contract);
            }
            Some("policy_state") => {
                if worker.contract.is_none() {
                    return Err("Policy update precedes session contract".into());
                }
                let policies = serde_json::from_value(entry["policies"].clone())
                    .map_err(|error| format!("Invalid subagent policy state: {error}"))?;
                let messages = entry["messages"]
                    .as_array()
                    .ok_or("Invalid subagent policy messages")?;
                for message in messages {
                    validate_message(message)?;
                }
                worker.policies = Arc::new(tokio::sync::Mutex::new(policies));
                pending_policy_tools.clear();
                if !messages.is_empty() {
                    transcript_tail.clear();
                }
                worker.history.extend(messages.iter().cloned());
            }
            Some("compaction") => {
                let history = entry["messages"]
                    .as_array()
                    .ok_or("Invalid subagent compaction record")?
                    .clone();
                let start = entry["appendStart"]
                    .as_u64()
                    .map(|start| start as usize)
                    .unwrap_or_else(|| super::executor::history_delta(&worker.history, &history).1);
                if start > history.len() {
                    return Err("Invalid compaction transcript cursor".into());
                }
                for message in &history[start..] {
                    if let Some(record) = policy_result_from_message(message)? {
                        pending_policy_tools.push(record);
                    }
                }
                transcript_tail = history[start..].iter().cloned().collect();
                worker.history = history;
            }
            Some("turn_start") => {
                worker.interrupted = Some(
                    serde_json::from_value(entry)
                        .map_err(|error| format!("Invalid saved turn: {error}"))?,
                )
            }
            Some("turn_end") => {
                worker.interrupted = None;
                worker.last_progress = entry.get("progress").cloned();
                worker.model = entry["model"].as_str().map(str::to_string);
            }
            Some("identity") => {
                let generation = entry["generation"]
                    .as_u64()
                    .ok_or("Invalid identity generation")?;
                let contract = worker
                    .contract
                    .as_mut()
                    .ok_or("Identity update precedes session contract")?;
                if generation <= contract.generation {
                    return Err("Identity generation did not advance".into());
                }
                contract.generation = generation;
            }
            Some("irc") => {}
            Some(_) => return Err("Unknown subagent journal record".into()),
            None => {
                validate_message(&entry)?;
                if transcript_tail.front().is_some_and(|expected| {
                    if expected == &entry {
                        return true;
                    }
                    let mut annotated = entry.clone();
                    if let Some(object) = annotated.as_object_mut() {
                        object.remove("is_error");
                        object.remove("_subagent_policy_tool");
                    }
                    expected == &annotated
                }) {
                    transcript_tail.pop_front();
                    continue;
                }
                transcript_tail.clear();
                if let Some(record) = policy_result_from_message(&entry)? {
                    pending_policy_tools.push(record);
                }
                worker.history.push(entry);
            }
        }
    }
    let contract = worker
        .contract
        .as_ref()
        .ok_or("Subagent transcript has no revival contract")?;
    if worker.history.len() < 2
        || !worker
            .history
            .iter()
            .any(|message| message["role"] == "user")
    {
        return Err("Subagent transcript has no conversational history".into());
    }
    for message in &worker.history {
        validate_message(message)?;
        policy_result_from_message(message)?;
    }
    if worker
        .history
        .first()
        .is_none_or(|message| message["role"] != "system")
    {
        return Err("Subagent context has no system message".into());
    }
    if execution {
        if !contract.cwd.is_absolute() || !contract.cwd.is_dir() {
            return Err("Saved subagent working directory is unavailable".into());
        }
        if matches!(
            contract.options.workspace,
            super::spawn::WorkspacePolicy::Isolated { .. }
        ) {
            return Err("Isolated worker leases cannot be cold-revived".into());
        }
        validate_tool_pairs(&worker.history)?;
    }
    let todo_available = contract
        .tools
        .iter()
        .any(|tool| tool == "native__todo_write");
    let mut policies = worker
        .policies
        .try_lock()
        .expect("replayed policy state is not shared");
    for turn in pending_policy_tools
        .chunk_by(|left, right| left.trace_id == right.trace_id && left.round == right.round)
    {
        if policies.observe_tools(turn, todo_available).nudge.is_some() {
            // Replay recovers effects; the live checkpoint persists any missing nudge.
            policies.prewalk.plan_nudged = false;
        }
    }
    drop(policies);
    Ok(worker)
}

pub(crate) async fn append_context(
    path: &std::path::Path,
    history: &[Value],
) -> Result<(), String> {
    append_record(
        path,
        &serde_json::json!({"type":"compaction","messages":history,"appendStart":history.len()}),
    )
    .await
}

pub(crate) async fn append_record(path: &std::path::Path, record: &Value) -> Result<(), String> {
    let mut data = serde_json::to_vec(record).map_err(|error| error.to_string())?;
    data.push(b'\n');
    let mut file = tokio::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .await
        .map_err(|error| format!("Cannot open subagent transcript: {error}"))?;
    file.write_all(&data)
        .await
        .map_err(|error| format!("Cannot save subagent context: {error}"))?;
    file.sync_data()
        .await
        .map_err(|error| format!("Cannot commit subagent context: {error}"))
}

fn validate_message(message: &Value) -> Result<(), String> {
    if !matches!(
        message["role"].as_str(),
        Some("system" | "developer" | "user" | "assistant" | "tool")
    ) {
        return Err("Invalid subagent history role".into());
    }
    Ok(())
}

fn validate_tool_pairs(history: &[Value]) -> Result<(), String> {
    let mut pending = HashSet::new();
    for message in history {
        if let Some(calls) = message["tool_calls"].as_array() {
            for call in calls {
                pending.insert(
                    call["id"]
                        .as_str()
                        .ok_or("Saved tool call has no identity")?
                        .to_string(),
                );
            }
        }
        if message["role"] == "tool" {
            pending.remove(
                message["tool_call_id"]
                    .as_str()
                    .ok_or("Saved tool result has no identity")?,
            );
        }
    }
    if !pending.is_empty() {
        return Err(
            "Subagent has an uncertain pending tool effect; automatic replay was refused".into(),
        );
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn test_contract(cwd: &std::path::Path) -> Value {
    serde_json::json!({"type":"session_init","version":1,"id":"A","root":"r","parentId":"Main","depth":1,
            "generation":1,"conversationId":"subagent-r-A","cwd":cwd,"systemPrompt":"system","runtimeContext":"context",
            "task":"task","agentDefinition":crate::agents::builtin_agent_definitions()[0],"tools":[],"mcpToolIds":[],"mcpServerIds":[],
            "readOnly":true,"resolvedModel":{"provider":"provider","model":"model"},"thinkingEnabled":false,"resolvedThinkingLevel":null,
            "approvalPolicy":"always_confirm","planState":crate::chat::types::AgentPlanState::default()})
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use super::test_contract as contract;

    fn bytes(entries: &[Value]) -> Vec<u8> {
        entries
            .iter()
            .flat_map(|entry| {
                let mut row = serde_json::to_vec(entry).unwrap();
                row.push(b'\n');
                row
            })
            .collect()
    }

    #[test]
    fn replay_replaces_compacted_context_without_duplicating_journal_tail() {
        let cwd = tempfile::tempdir().unwrap();
        let system = json!({"role":"system","content":"system"});
        let initial = json!({"role":"user","content":"old task"});
        let summary = json!({"role":"user","content":"summary"});
        let answer = json!({"role":"assistant","content":"answer"});
        let entries = vec![
            contract(cwd.path()),
            system.clone(),
            initial,
            json!({"type":"compaction","messages":[system.clone(),summary.clone(),answer.clone()],"appendStart":2}),
            answer.clone(),
            json!({"role":"user","content":"follow-up"}),
        ];
        let worker = replay(&bytes(&entries), "r", "A").unwrap();
        assert_eq!(
            worker.history,
            vec![
                system,
                summary,
                answer,
                json!({"role":"user","content":"follow-up"})
            ]
        );
        assert_eq!(
            worker.contract.unwrap().spawns,
            "",
            "missing saved spawns never grants wildcard"
        );
    }

    #[test]
    fn replay_rejects_corruption_missing_history_wrong_identity_and_truncation() {
        let cwd = tempfile::tempdir().unwrap();
        let init = contract(cwd.path());
        assert!(replay(&bytes(&[init.clone()]), "r", "A").is_err());
        let entries = vec![
            init,
            json!({"role":"system","content":"system"}),
            json!({"role":"user","content":"task"}),
        ];
        let mut encoded = bytes(&entries);
        assert!(replay(&encoded, "other", "A").is_err());
        encoded.pop();
        assert!(replay(&encoded, "r", "A")
            .err()
            .unwrap()
            .contains("incomplete"));
        assert!(replay(b"{broken}\n", "r", "A").is_err());
    }

    #[test]
    fn turn_lease_rejects_overlap_and_late_completion_in_same_generation() {
        let slot = Slot {
            control: Mutex::new(Control {
                generation: 1,
                ..Default::default()
            }),
            worker: tokio::sync::Mutex::new(None),
        };
        let record: AgentRecord = serde_json::from_value(json!({"id":"A","root":"r","parentId":"Main","generation":1,
            "displayName":"A","agentType":"task","status":"idle","depth":1,"childConversationId":"subagent-r-A",
            "runId":null,"sessionFile":"A.jsonl","outputFile":"A.md","lastActivity":0,"activity":null,"published":true})).unwrap();
        assert!(slot.begin(&record, "first"));
        assert!(!slot.begin(&record, "overlap"));
        assert!(slot.finish(1, "first"));
        assert!(slot.begin(&record, "second"));
        assert!(!slot.finish(1, "first"));
        assert!(slot.owns(1, "second"));
        slot.control.lock().unwrap().killed = true;
        assert!(!slot.finish(1, "second"));
        assert!(!slot.begin(&record, "after-kill"));
    }

    #[test]
    fn pending_effects_fail_closed_and_completed_calls_can_resume() {
        let mut history =
            vec![serde_json::json!({"role":"assistant","tool_calls":[{"id":"write-1"}]})];
        assert!(validate_tool_pairs(&history)
            .unwrap_err()
            .contains("uncertain"));
        history.push(serde_json::json!({"role":"tool","tool_call_id":"write-1","content":"done"}));
        validate_tool_pairs(&history).unwrap();
    }

    #[tokio::test]
    async fn replay_restores_policy_handoff_advisor_and_injected_messages() {
        use super::super::policies::{AdvisorState, PolicyModel, PrewalkState, SessionPolicies};
        let cwd = tempfile::tempdir().unwrap();
        let model = PolicyModel {
            provider_id: "provider".into(),
            model: "implementation".into(),
            thinking_level: Some("low".into()),
        };
        let mut initial = SessionPolicies::default();
        initial.prewalk = PrewalkState {
            target: Some(model.clone()),
            ..Default::default()
        };
        initial.read_summarize = Some(false);
        let mut init = contract(cwd.path());
        init["policies"] = serde_json::to_value(&initial).unwrap();
        let mut advanced = initial.clone();
        let records = vec![
            serde_json::from_value(json!({"id":"todo","name":"todo_write","source":"native","status":"success","trace_id":"run"})).unwrap(),
            serde_json::from_value(json!({"id":"edit","name":"edit","source":"native","status":"error","arguments":"{\"path\":\"code.rs\"}","trace_id":"run"})).unwrap(),
        ];
        assert!(advanced.observe_tools(&[], true).nudge.is_some());
        assert_eq!(
            advanced.observe_tools(&records, true).handoff,
            Some(model.clone())
        );
        advanced.advisor = Some(AdvisorState {
            model: model.clone(),
            messages: vec![json!({"role":"assistant","content":"Check errors."})],
            last_history_digest: Some("checkpoint".into()),
        });
        let nudge = json!({"role":"user","content":"Plan before edits.","_subagent_prewalk":true});
        let entries = vec![
            init,
            json!({"role":"system","content":"system"}),
            json!({"role":"user","content":"task"}),
            json!({"type":"policy_state","policies":advanced,"messages":[nudge.clone()]}),
        ];
        let worker = replay(&bytes(&entries), "r", "A").unwrap();
        assert_eq!(worker.history.len(), 3);
        assert_eq!(worker.history.last(), Some(&nudge));
        assert!(worker.contract.as_ref().unwrap().eval_tools.is_empty());
        let mut restored = worker.policies.lock().await;
        assert_eq!(restored.handoff_model(), Some(&model));
        assert_eq!(restored.read_summarize, Some(false));
        assert_eq!(
            restored
                .advisor
                .as_ref()
                .unwrap()
                .last_history_digest
                .as_deref(),
            Some("checkpoint")
        );
        assert!(restored.observe_tools(&records, true).handoff.is_none());
    }

    #[test]
    fn replay_rejects_invalid_or_orphaned_policy_updates() {
        let cwd = tempfile::tempdir().unwrap();
        let update = json!({"type":"policy_state","policies":{},"messages":[]});
        assert!(replay(&bytes(&[update]), "r", "A")
            .err()
            .unwrap()
            .contains("precedes"));
        for update in [
            json!({"type":"policy_state","policies":null,"messages":[]}),
            json!({"type":"policy_state","policies":{},"messages":[{"role":"forged"}]}),
        ] {
            let entries = vec![
                contract(cwd.path()),
                json!({"role":"system","content":"system"}),
                json!({"role":"user","content":"task"}),
                update,
            ];
            assert!(replay(&bytes(&entries), "r", "A").is_err());
        }
    }

    #[tokio::test]
    async fn replay_recovers_effects_journaled_before_the_policy_checkpoint() {
        use super::super::policies::{PolicyModel, PrewalkState, SessionPolicies};
        let cwd = tempfile::tempdir().unwrap();
        let mut policies = SessionPolicies::default();
        policies.prewalk = PrewalkState {
            target: Some(PolicyModel {
                provider_id: "provider".into(),
                model: "implementation".into(),
                thinking_level: None,
            }),
            ..Default::default()
        };
        let mut init = contract(cwd.path());
        init["policies"] = serde_json::to_value(&policies).unwrap();
        init["tools"] = json!(["native__todo_write"]);
        let mut entries = vec![
            init,
            json!({"role":"system","content":"system"}),
            json!({"role":"user","content":"task"}),
        ];
        let todo = crate::chat::todo::apply_todo_write(
            json!({"todos":[{"content":"Verify the effect","status":"in_progress"}]}),
        )
        .unwrap();
        for (round, name, status, arguments) in [
            (
                1,
                "edit",
                "error",
                json!({"path":"code.rs","content":"source contents are not duplicated"}),
            ),
            (2, "todo_write", "success", json!({"todos":[]})),
        ] {
            let mut record: crate::chat::types::ToolCallRecord = serde_json::from_value(json!({
                "id":format!("call-{round}"), "name":name, "source":"native", "status":status,
                "arguments":arguments.to_string(), "round":round, "trace_id":"run",
            }))
            .unwrap();
            if name == "todo_write" {
                record.structured_content =
                    crate::chat::todo::tool_result(&todo).structured_content;
            }
            let compact = policy_tool_record(&record).unwrap();
            assert!(!compact
                .to_string()
                .contains("source contents are not duplicated"));
            entries.push(json!({"role":"assistant","tool_calls":[{"id":record.id}]}));
            entries.push(json!({"role":"tool","tool_call_id":record.id,"content":"completed","_subagent_policy_tool":compact}));
        }
        let worker = replay(&bytes(&entries), "r", "A").unwrap();
        let restored = worker.policies.lock().await;
        assert!(restored.prewalk.todo_seen);
        assert!(
            restored.handoff_model().is_none(),
            "an earlier edit cannot use a later round's todo gate"
        );
        assert!(
            !restored.prewalk.plan_nudged,
            "a missing live nudge must remain due"
        );
        assert_eq!(restored.todo_state.items.len(), 1);
        drop(restored);

        let edit: crate::chat::types::ToolCallRecord =
            serde_json::from_value(json!({"id":"call-3","name":"write","source":"native",
            "status":"error","arguments":"{\"path\":\"code.rs\"}","round":3,"trace_id":"run"}))
            .unwrap();
        entries.push(json!({"role":"assistant","tool_calls":[{"id":edit.id}]}));
        entries.push(json!({"role":"tool","tool_call_id":edit.id,"content":"failed","_subagent_policy_tool":policy_tool_record(&edit)}));
        let worker = replay(&bytes(&entries), "r", "A").unwrap();
        assert_eq!(
            worker.policies.lock().await.handoff_model().unwrap().model,
            "implementation"
        );
        let mut checkpoint = worker.policies.lock().await.clone();
        checkpoint.todo_state = Default::default();
        entries.push(json!({"type":"policy_state","policies":checkpoint,"messages":[]}));
        entries.push(json!({"type":"compaction","messages":worker.history,"appendStart":worker.history.len()}));
        let worker = replay(&bytes(&entries), "r", "A").unwrap();
        assert!(
            worker.policies.lock().await.todo_state.items.is_empty(),
            "older transcript annotations must not override a newer policy snapshot"
        );
    }
}
