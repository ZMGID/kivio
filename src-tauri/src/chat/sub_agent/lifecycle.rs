use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;
use tauri::{AppHandle, Manager};

use super::{
    executor,
    registry::{self, AgentRecord, AgentStatus},
    session, SubAgentManager,
};
use crate::state::AppState;

const IDLE_TTL: Duration = Duration::from_secs(7 * 60);

fn expected(
    manager: &SubAgentManager,
    root: &str,
    id: &str,
    generation: u64,
) -> Result<AgentRecord, String> {
    manager
        .registry
        .get(root, id)
        .filter(|record| record.generation == generation)
        .ok_or_else(|| "Subagent identity changed; refresh the roster".into())
}

fn require_accepted(manager: &SubAgentManager, record: &AgentRecord) -> Result<(), String> {
    if manager.registry.accepted_lineage(record)? { Ok(()) }
    else { Err("Subagent has not been accepted for execution".into()) }
}

pub(crate) fn tombstone(record: &AgentRecord) -> PathBuf {
    record.session_file.with_extension("jsonl.tombstone")
}

pub(crate) async fn read_worker(record: &AgentRecord) -> Result<session::WorkerSession, String> {
    let metadata = tokio::fs::symlink_metadata(&record.session_file)
        .await
        .map_err(|error| format!("Cannot inspect subagent transcript: {error}"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Subagent transcript must be a regular owned file".into());
    }
    let bytes = tokio::fs::read(&record.session_file)
        .await
        .map_err(|error| format!("Cannot read subagent transcript: {error}"))?;
    session::replay(&bytes, &record.root, &record.id)
}

pub async fn ensure_live(
    app: &AppHandle,
    root: &str,
    id: &str,
    expected_generation: u64,
) -> Result<AgentRecord, String> {
    let state = app.state::<AppState>();
    let manager = &state.sub_agents;
    let mut record = expected(manager, root, id, expected_generation)?;
    require_accepted(manager, &record)?;
    let mut ancestor = record.parent_id.clone();
    let mut depth = 1u8;
    let mut seen = std::collections::HashSet::from([record.id.clone()]);
    while ancestor != super::MAIN_AGENT_ID {
        if !seen.insert(ancestor.clone()) {
            return Err("Subagent parent chain contains a cycle".into());
        }
        let parent = manager
            .registry
            .get(root, &ancestor)
            .ok_or("Subagent parent identity is missing")?;
        ancestor = parent.parent_id;
        depth = depth
            .checked_add(1)
            .ok_or("Subagent parent chain is too deep")?;
    }
    if depth != record.depth {
        return Err("Saved subagent depth does not match its parent chain".into());
    }
    let slot = manager.sessions.slot(&record)?;
    if record.status == AgentStatus::Running {
        return Ok(record);
    }
    let mut guard = slot.worker.lock().await;
    record = expected(manager, root, id, expected_generation)?;
    require_accepted(manager, &record)?;
    if record.status == AgentStatus::Running {
        return Ok(record);
    }
    if guard.is_none() {
        *guard = Some(read_worker(&record).await?);
    }
    if record.status == AgentStatus::Aborted {
        if let Some(worker) = guard.as_mut() {
            worker.prepared = None;
        }
        let operation = slot
            .control
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .operation;
        let preparation = std::sync::Arc::new(session::Slot {
            control: std::sync::Mutex::new(session::Control {
                generation: record.generation,
                ..Default::default()
            }),
            worker: tokio::sync::Mutex::new(None),
        });
        executor::prepare_idle(
            app,
            &record,
            &preparation,
            guard.as_mut().ok_or("Subagent transcript is unavailable")?,
        )
        .await?;
        if slot
            .control
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .operation
            != operation
        {
            return Err("Subagent was killed while being reopened".into());
        }
        match tokio::fs::remove_file(tombstone(&record)).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Cannot reopen killed subagent: {error}")),
        }
        record.status = AgentStatus::Parked;
        record.run_id = None;
        record = manager.registry.register(record);
        session::append_record(
            &record.session_file,
            &serde_json::json!({"type":"identity","generation":record.generation}),
        )
        .await?;
        if let Some(contract) = guard.as_mut().and_then(|worker| worker.contract.as_mut()) {
            contract.generation = record.generation;
        }
        let mut control = slot
            .control
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        control.generation = record.generation;
        control.killed = false;
        control.active_run = None;
        control.cwd = preparation
            .control
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .cwd
            .clone();
    }
    let worker = guard.as_mut().ok_or("Subagent transcript is unavailable")?;
    executor::prepare_idle(app, &record, &slot, worker).await?;
    let record = manager
        .registry
        .update_checked(root, id, record.generation, None, |current| {
            current.status = AgentStatus::Idle
        })
        .ok_or("Subagent changed while being revived")?;
    drop(guard);
    schedule_idle(app, &record);
    Ok(record)
}

pub async fn park(
    app: &AppHandle,
    root: &str,
    id: &str,
    expected_generation: u64,
) -> Result<AgentRecord, String> {
    let state = app.state::<AppState>();
    let manager = &state.sub_agents;
    let record = expected(manager, root, id, expected_generation)?;
    if record.status == AgentStatus::Aborted {
        return Err("Killed subagents cannot be parked".into());
    }
    if record.status == AgentStatus::Running {
        return Err("Running subagents cannot be parked".into());
    }
    let slot = manager.sessions.slot(&record)?;
    let mut worker = slot.worker.lock().await;
    {
        let control = slot
            .control
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if control.active_run.is_some() || control.wake_reserved || manager.irc.has_pending(root, id) {
            return Err("Subagent has active or queued work".into());
        }
    }
    let record = manager
        .registry
        .update_checked(root, id, expected_generation, None, |current| {
            current.status = AgentStatus::Parked;
            current.run_id = None;
            current.activity = None;
        })
        .ok_or("Subagent changed while being parked")?;
    manager.pools.close_owner(root, id);
    manager.eval.close_owner(root, id).await;
    *worker = None;
    Ok(record)
}

pub async fn kill(
    app: &AppHandle,
    root: &str,
    id: &str,
    expected_generation: u64,
) -> Result<AgentRecord, String> {
    let state = app.state::<AppState>();
    let manager = &state.sub_agents;
    let record = expected(manager, root, id, expected_generation)?;
    let slot = manager.sessions.slot(&record)?;
    {
        let mut control = slot
            .control
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        control.killed = true;
        control.operation = control.operation.wrapping_add(1);
        control.wake_reserved = false;
    }
    let record = if record.status == AgentStatus::Aborted {
        record
    } else {
        manager
            .registry
            .update_checked(root, id, expected_generation, None, |current| {
                current.status = AgentStatus::Aborted;
                current.run_id = None;
                current.activity = None;
            })
            .ok_or("Subagent changed while being killed")?
    };
    state
        .chat_runtime()
        .cancel_conversation(&record.child_conversation_id);
    state
        .background_commands_handle()
        .kill_for_conversation(&record.child_conversation_id);
    manager.jobs.cancel_agent(root, id);
    manager.irc.agent_stopped(root, id);
    let persisted = tokio::fs::write(tombstone(&record), format!("{}\n", record.generation))
        .await
        .map_err(|error| {
            format!("Subagent stopped, but its kill marker could not be saved: {error}")
        });
    let mut worker = slot.worker.lock().await;
    manager.pools.close_owner(root, id);
    manager.eval.close_owner(root, id).await;
    *worker = None;
    persisted?;
    Ok(record)
}

pub fn schedule_idle(app: &AppHandle, record: &AgentRecord) {
    let app = app.clone();
    let record = record.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(IDLE_TTL).await;
        let state = app.state::<AppState>();
        let current = state.sub_agents.registry.get(&record.root, &record.id);
        if current.is_some_and(|current| {
            current.generation == record.generation
                && current.status == AgentStatus::Idle
                && registry::now_ms().saturating_sub(current.last_activity)
                    >= IDLE_TTL.as_millis() as i64
        }) {
            let _ = park(&app, &record.root, &record.id, record.generation).await;
        }
    });
}

pub fn cwd_for_run(manager: &SubAgentManager, run: &str) -> Option<PathBuf> {
    let (root, id) = manager.registry.find_by_run(run)?;
    let slot = manager.sessions.existing(&root, &id)?;
    let control = slot
        .control
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    (control.active_run.as_deref() == Some(run) && !control.killed)
        .then(|| control.cwd.clone())
        .flatten()
}

pub(crate) fn spawn_policy_for_run(manager: &SubAgentManager, run: &str) -> Option<String> {
    let (root, id) = manager.registry.find_by_run(run)?;
    let slot = manager.sessions.existing(&root, &id)?;
    let control = slot
        .control
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    (control.active_run.as_deref() == Some(run) && !control.killed)
        .then(|| control.spawns.clone())
        .flatten()
}

pub(crate) fn read_summarize_for_run(manager: &SubAgentManager, run: &str) -> Option<bool> {
    let (root, id) = manager.registry.find_by_run(run)?;
    let slot = manager.sessions.existing(&root, &id)?;
    let control = slot
        .control
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    (control.active_run.as_deref() == Some(run) && !control.killed)
        .then(|| control.read_summarize.unwrap_or(true))
}

pub(crate) fn completion_schema_for_run(manager: &SubAgentManager, run: &str) -> Option<serde_json::Value> {
    let (root, id) = manager.registry.find_by_run(run)?;
    let slot = manager.sessions.existing(&root, &id)?;
    let control = slot.control.lock().unwrap_or_else(|error| error.into_inner());
    (control.active_run.as_deref() == Some(run) && !control.killed).then(|| control.completion_schema.clone()).flatten()
}

pub async fn discover(app: &AppHandle, root: &str) -> Result<Vec<AgentRecord>, String> {
    let directory = registry::artifacts_dir(app, root)?;
    discover_in(&directory, &app.state::<AppState>().sub_agents.registry, root).await
}

async fn discover_in(directory: &std::path::Path, registry: &registry::AgentRegistry, root: &str) -> Result<Vec<AgentRecord>, String> {
    let mut entries = match tokio::fs::read_dir(&directory).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Cannot scan subagent roster: {error}")),
    };
    let mut discovered = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| error.to_string())?
    {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("jsonl")
            || !entry
                .file_type()
                .await
                .map_err(|error| error.to_string())?
                .is_file()
        {
            continue;
        }
        let Some(id) = path
            .file_stem()
            .and_then(|id| id.to_str())
            .filter(|id| super::task::valid_agent_id(id))
        else {
            continue;
        };
        if registry.contains(root, id) || !registry::is_accepted(&path, root, id)? {
            continue;
        }
        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|error| error.to_string())?;
        let contract = bytes
            .split(|byte| *byte == b'\n')
            .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
            .find(|entry| entry["type"] == "session_init")
            .and_then(|entry| serde_json::from_value::<session::SessionContract>(entry).ok());
        let Some(mut saved) =
            contract.filter(|saved| saved.root == root && saved.id == id && saved.generation > 0)
        else {
            continue;
        };
        for entry in bytes
            .split(|byte| *byte == b'\n')
            .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
        {
            if entry["type"] == "identity" {
                saved.generation = saved
                    .generation
                    .max(entry["generation"].as_u64().unwrap_or(0));
            }
        }
        let latest_output = bytes
            .split(|byte| *byte == b'\n')
            .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
            .filter(|entry| entry["type"] == "turn_end")
            .filter_map(|entry| entry["outputFile"].as_str().map(PathBuf::from))
            .filter(|path| {
                path.parent() == Some(directory)
                    && path.extension().is_some_and(|extension| extension == "md")
            })
            .last()
            .unwrap_or_else(|| directory.join(format!("{id}.md")));
        let record = AgentRecord {
            id: id.into(),
            root: root.into(),
            parent_id: saved.parent_id,
            generation: saved.generation,
            display_name: id.into(),
            agent_type: saved.agent_definition.name,
            status: AgentStatus::Parked,
            depth: saved.depth,
            child_conversation_id: saved.conversation_id,
            run_id: None,
            session_file: path.clone(),
            output_file: latest_output,
            last_activity: registry::now_ms(),
            activity: None,
            published: true,
        };
        let mut record = record;
        if tokio::fs::try_exists(tombstone(&record))
            .await
            .unwrap_or(true)
        {
            record.status = AgentStatus::Aborted;
        }
        discovered.push(record);
    }
    discovered.sort_by_key(|record| record.depth);
    for record in discovered {
        if registry.accepted_lineage(&record)? { registry.restore(record); }
    }
    let mut accepted = Vec::new();
    for record in registry.list(root) {
        if registry.accepted_lineage(&record)? { accepted.push(record); }
    }
    Ok(accepted)
}

fn retained_inspection(manager: &SubAgentManager, record: &AgentRecord) -> Option<(Option<Value>, Option<String>)> {
    let slot = manager.sessions.existing(&record.root, &record.id)?;
    let control = slot.control.lock().unwrap_or_else(|error| error.into_inner());
    if control.generation != record.generation { return None; }
    let progress = control.last_progress.clone();
    let model = control.model.clone();
    if record.status == AgentStatus::Running || progress.is_some() || model.is_some() {
        return Some((progress, model));
    }
    drop(control);
    let worker = slot.worker.try_lock().ok()?;
    worker.as_ref().map(|worker| (worker.last_progress.clone(), worker.model.clone()))
}

pub async fn inspect(
    app: &AppHandle,
    record: &AgentRecord,
) -> Result<(Option<Value>, Option<String>), String> {
    let state = app.state::<AppState>();
    let retained = retained_inspection(&state.sub_agents, record);
    if record.status == AgentStatus::Running { return Ok(retained.unwrap_or_default()); }
    if let Some(retained) = retained { return Ok(retained); }
    let bytes = tokio::fs::read(&record.session_file)
        .await
        .map_err(|error| error.to_string())?;
    let worker = session::replay_for_consult(&bytes, &record.root, &record.id)?;
    Ok((worker.last_progress, worker.model))
}

pub async fn revival_error(app: &AppHandle, record: &AgentRecord) -> Option<String> {
    let state = app.state::<AppState>();
    let retained = state.sub_agents.sessions.existing(&record.root, &record.id).and_then(|slot| {
        let worker = slot.worker.try_lock().ok()?;
        let worker = worker.as_ref()?;
        worker.contract.clone().filter(|saved| saved.generation == record.generation).map(|saved| (saved, worker.prepared.is_some()))
    });
    let saved = match retained {
        Some((saved, prepared)) => {
            if !saved.cwd.is_absolute() || !saved.cwd.is_dir() {
                return Some("Saved subagent working directory is unavailable".into());
            }
            if !prepared && matches!(saved.options.workspace, super::spawn::WorkspacePolicy::Isolated { .. }) {
                return Some("Isolated worker leases cannot be cold-revived".into());
            }
            saved
        },
        None => match read_worker(record).await {
            Ok(worker) => worker.contract?,
            Err(error) => return Some(error),
        },
    };
    let settings = state.settings_read().clone();
    match super::spawn::resolve_model(
        &settings,
        &saved.options.model,
        &saved.resolved_model.provider,
        &saved.resolved_model.model,
    ) {
        Ok(model)
            if model.provider.id == saved.resolved_model.provider
                && model.model == saved.resolved_model.model =>
        {
            None
        }
        Ok(_) => Some("Saved exact subagent model is no longer authorized".into()),
        Err(error) => Some(error),
    }
}

pub async fn restore_interrupted(app: &AppHandle, root: &str) -> Result<Vec<String>, String> {
    let records = discover(app, root).await?;
    let mut pending = Vec::new();
    for record in records
        .into_iter()
        .filter(|record| !matches!(record.status, AgentStatus::Aborted | AgentStatus::Running))
    {
        match read_worker(&record).await {
            Ok(worker) if worker.interrupted.is_some() => pending.push((record, worker)),
            Ok(_) => {}
            Err(error) => eprintln!("Cannot restore subagent {}: {error}", record.id),
        }
    }
    pending.sort_by_key(|(record, _)| record.depth);
    let state = app.state::<AppState>();
    let mut restored = Vec::new();
    let mut starts = Vec::new();
    for (record, worker) in pending {
        let saved = worker
            .interrupted
            .clone()
            .ok_or("Missing interrupted turn")?;
        let Some((job, cancel)) = state.sub_agents.jobs.register_restored(
            root,
            &saved.owner_id,
            &record.id,
            &saved.job_id,
            "Restored interrupted assignment",
        ) else { continue; };
        let slot = state.sub_agents.sessions.slot(&record)?;
        *slot.worker.lock().await = Some(worker);
        restored.push(job.clone());
        starts.push((record, job, cancel, saved));
    }
    for (record, job, cancel, saved) in starts {
        super::irc::schedule_restored(app, record, job, cancel, saved);
    }
    Ok(restored)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(status: AgentStatus) -> AgentRecord {
        AgentRecord {
            id: "Scout".into(),
            root: "root".into(),
            parent_id: super::super::MAIN_AGENT_ID.into(),
            generation: 0,
            display_name: "Scout".into(),
            agent_type: "scout".into(),
            status,
            depth: 1,
            child_conversation_id: "child".into(),
            run_id: Some("run".into()),
            session_file: PathBuf::new(),
            output_file: PathBuf::new(),
            last_activity: 0,
            activity: None,
            published: true,
        }
    }

    #[tokio::test]
    async fn roster_metadata_remains_available_while_the_worker_is_busy() {
        let manager = SubAgentManager::default();
        let mut record = manager.registry.register(record(AgentStatus::Running));
        let slot = manager.sessions.slot(&record).unwrap();
        let progress = serde_json::json!({"status":"completed","requests":4});
        {
            let mut control = slot.control.lock().unwrap();
            control.last_progress = Some(progress.clone());
            control.model = Some("saved-model".into());
        }
        let _busy = slot.worker.lock().await;
        assert_eq!(retained_inspection(&manager, &record), Some((Some(progress), Some("saved-model".into()))));
        record.generation += 1;
        assert!(retained_inspection(&manager, &record).is_none());
    }

    #[test]
    fn read_summary_policy_only_applies_to_the_active_worker_run() {
        let manager = SubAgentManager::default();
        let record = manager.registry.register(record(AgentStatus::Running));
        let slot = manager.sessions.slot(&record).unwrap();
        assert!(slot.begin(&record, "run"));
        assert_eq!(read_summarize_for_run(&manager, "run"), Some(true));
        slot.control.lock().unwrap().read_summarize = Some(false);
        assert_eq!(read_summarize_for_run(&manager, "run"), Some(false));
        assert_eq!(read_summarize_for_run(&manager, "other"), None);
        slot.control.lock().unwrap().killed = true;
        assert_eq!(read_summarize_for_run(&manager, "run"), None);
    }

    fn interrupted_worker(manager: &SubAgentManager, directory: &std::path::Path, id: &str, parent: &str, depth: u8) -> AgentRecord {
        let mut worker = record(AgentStatus::Running);
        worker.id = id.into();
        worker.parent_id = parent.into();
        worker.depth = depth;
        worker.published = false;
        worker.child_conversation_id = format!("subagent-root-{id}");
        worker.session_file = directory.join(format!("{id}.jsonl"));
        worker.output_file = directory.join(format!("{id}.md"));
        let worker = manager.registry.register(worker);
        let contract = serde_json::json!({"type":"session_init","version":1,"id":id,"root":"root","parentId":parent,"depth":depth,
            "generation":worker.generation,"conversationId":worker.child_conversation_id,"cwd":directory,"systemPrompt":"system","runtimeContext":"context",
            "task":"No external tools. Return a textual answer.","agentDefinition":crate::agents::builtin_agent_definitions()[0],"tools":[],
            "readOnly":true,"resolvedModel":{"provider":"provider","model":"model"},"thinkingEnabled":false,"resolvedThinkingLevel":null,
            "approvalPolicy":"always_confirm","planState":crate::chat::types::AgentPlanState::default()});
        let entries = [serde_json::json!({"type":"turn_start","jobId":id,"ownerId":parent,"runId":"run",
            "parentRunId":"parent-run","parentToolCallId":"task-call"}), contract,
            serde_json::json!({"role":"system","content":"system"}), serde_json::json!({"role":"user","content":"assignment"})];
        let bytes: Vec<_> = entries.into_iter().flat_map(|entry| {
            let mut row = serde_json::to_vec(&entry).unwrap(); row.push(b'\n'); row
        }).collect();
        std::fs::write(&worker.session_file, bytes).unwrap();
        worker
    }

    #[tokio::test]
    async fn fresh_discovery_and_execution_exclude_unaccepted_interrupted_workers() {
        let directory = tempfile::tempdir().unwrap();
        let live = SubAgentManager::default();
        let accepted = interrupted_worker(&live, directory.path(), "Accepted", "Main", 1);
        let unaccepted = interrupted_worker(&live, directory.path(), "Speculative", "Main", 1);
        let child = interrupted_worker(&live, directory.path(), "Speculative.Child", "Speculative", 2);
        let invalid = interrupted_worker(&live, directory.path(), "Invalid", "Main", 1);
        live.registry.publish("root", &accepted.id, accepted.generation).unwrap();
        live.registry.publish("root", &child.id, child.generation).unwrap();
        std::fs::write(invalid.session_file.with_extension("jsonl.accepted"), b"{\"root\":\"other\",\"id\":\"Invalid\"}").unwrap();
        for record in [&accepted, &unaccepted, &child, &invalid] {
            assert!(read_worker(record).await.unwrap().interrupted.is_some(), "every fixture is a genuinely replayable interrupted session");
        }
        assert!(require_accepted(&live, &unaccepted).is_err(), "the execution gate must reject a running hidden identity before returning it");
        assert!(require_accepted(&live, &live.registry.get("root", &child.id).unwrap()).is_err());
        let cold = SubAgentManager::default();
        let discovered = discover_in(directory.path(), &cold.registry, "root").await.unwrap();
        assert_eq!(discovered.iter().map(|record| record.id.as_str()).collect::<Vec<_>>(), ["Accepted"]);
        let turn = read_worker(&discovered[0]).await.unwrap().interrupted.unwrap();
        assert_eq!(turn.parent_run_id, "parent-run");
        assert_eq!(turn.parent_tool_call_id, "task-call");
        assert!(require_accepted(&cold, &discovered[0]).is_ok());
        assert!(!cold.registry.contains("root", &unaccepted.id));
        assert!(!cold.registry.contains("root", &child.id));
        assert!(!cold.registry.contains("root", &invalid.id));
        assert!(tokio::fs::read_to_string(&unaccepted.session_file).await.unwrap().contains("assignment"), "unaccepted history must remain readable");
        live.registry.publish("root", &unaccepted.id, unaccepted.generation).unwrap();
        let discovered = discover_in(directory.path(), &cold.registry, "root").await.unwrap();
        assert_eq!(discovered.len(), 3, "accepting the parent makes its already accepted child discoverable");
    }
}
