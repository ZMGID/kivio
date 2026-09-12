//! Durable child identities and per-execution control. No provider credentials
//! are stored here. The worker still runs through the shared Agent loop.

use futures::FutureExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Mutex,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Running,
    Finishing,
    Stopping,
    Completed,
    Failed,
    Interrupted,
}
impl Status {
    pub fn active(self) -> bool {
        matches!(self, Self::Running | Self::Finishing | Self::Stopping)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub provider_id: String,
    pub model: String,
    pub agent_type: String,
    pub system_prompt: String,
    pub tool_names: Vec<String>,
    pub skill_cwd: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Execution {
    #[serde(default)]
    pub delivered: bool,
    pub id: String,
    pub parent_run: String,
    pub status: Status,
    pub prompt: String,
    pub result: Option<String>,
    pub error: Option<String>,
    pub usage: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub sender: String,
    pub text: String,
    pub consumed_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    #[serde(default)]
    pub preview: String,
    #[serde(default)]
    pub steps: Vec<String>,
    pub version: u32,
    pub id: String,
    pub conversation_id: String,
    pub request_key: String,
    pub name: String,
    pub profile: Profile,
    pub runs: Vec<Execution>,
    pub history: Vec<Value>,
    pub messages: Vec<Message>,
    pub tools: Vec<Value>,
    pub user_stopped: bool,
    pub sequence: u64,
}
impl Record {
    pub fn current(&self) -> &Execution {
        self.runs.last().expect("validated execution")
    }
    fn current_mut(&mut self) -> &mut Execution {
        self.runs.last_mut().expect("validated execution")
    }
}

struct Control {
    live_parents: HashSet<String>,
    storage_errors: HashMap<String, String>,
    deleting: HashSet<String>,
    owners: HashMap<String, (String, String)>,
    parent_runs: HashMap<String, String>,
    stopping: HashSet<String>,
    user_stops: HashSet<String>,
    exiting: bool,
    claimed: HashSet<String>,
    limit: usize,
    active: HashSet<String>,
    sealed: HashSet<String>,
    handles: HashMap<String, tokio::task::JoinHandle<()>>,
}

/// File transactions and admission share a lock. Only active execution metadata
/// is resident; complete histories are loaded on demand and atomically replaced.
pub struct Runtime {
    summaries_dirty: std::sync::atomic::AtomicBool,
    root: PathBuf,
    control: Mutex<Control>,
    events: tokio::sync::watch::Sender<u64>,
}

impl Runtime {
    pub fn open(root: PathBuf) -> Result<Self, String> {
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let runtime = Self {
            summaries_dirty: std::sync::atomic::AtomicBool::new(true),
            root,
            control: Mutex::new(Control {
                live_parents: HashSet::new(),
                storage_errors: HashMap::new(),
                deleting: HashSet::new(),
                owners: HashMap::new(),
                parent_runs: HashMap::new(),
                stopping: HashSet::new(),
                user_stops: HashSet::new(),
                exiting: false,
                claimed: HashSet::new(),
                limit: 12,
                active: HashSet::new(),
                sealed: HashSet::new(),
                handles: HashMap::new(),
            }),
            events: tokio::sync::watch::channel(0).0,
        };
        // Rebuild lightweight summaries one record at a time on startup. Never
        // retain all histories in memory while recovering interrupted workers.
        for entry in std::fs::read_dir(&runtime.root).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or("Invalid record path")?;
            let mut record = runtime.read(id)?;
            if record.current().status.active() {
                record.current_mut().status = Status::Interrupted;
                record.current_mut().error = Some("Process interrupted; explicitly continue after inspecting unknown tool outcomes".into());
                runtime.save(&mut record)?;
            } else {
                runtime.write_summary(&record)?;
            }
        }
        runtime.repair_summaries()?;
        Ok(runtime)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Control> {
        self.control.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn path(&self, id: &str) -> Result<PathBuf, String> {
        uuid::Uuid::parse_str(id).map_err(|_| "Invalid child identifier".to_string())?;
        Ok(self.root.join(format!("{id}.json")))
    }
    fn read(&self, id: &str) -> Result<Record, String> {
        let text = std::fs::read_to_string(self.path(id)?).map_err(|e| e.to_string())?;
        let record: Record = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        if record.version != 1 || record.runs.is_empty() || record.id != id {
            return Err("Unsupported or damaged child record".into());
        }
        Ok(record)
    }
    fn summary_dir(&self, conversation: &str) -> Result<PathBuf, String> {
        if conversation.is_empty()
            || conversation.len() > 128
            || !conversation
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err("Invalid parent conversation identifier".into());
        }
        Ok(self.root.join("index").join(conversation))
    }
    fn write_summary(&self, record: &Record) -> Result<(), String> {
        let directory = self.summary_dir(&record.conversation_id)?;
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let mut summary = record.clone();
        summary.history.clear();
        summary.tools.clear();
        summary.messages.clear();
        summary.profile.system_prompt.clear();
        summary.profile.tool_names.clear();
        for run in &mut summary.runs {
            run.result = None;
            run.prompt = run.prompt.chars().take(500).collect();
        }
        let text = serde_json::to_string(&summary).map_err(|e| e.to_string())?;
        super::super::storage::atomic_write(
            &directory.join(format!("{}.json", record.id)),
            &text,
            "sub-agent summary",
        )
    }
    fn repair_summaries(&self) -> Result<(), String> {
        if !self
            .summaries_dirty
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Ok(());
        }
        for entry in std::fs::read_dir(&self.root).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.extension().and_then(|s| s.to_str()) != Some("pending") {
                continue;
            }
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or("Invalid pending summary")?;
            if self.path(id)?.exists() {
                self.write_summary(&self.read(id)?)?;
            }
            std::fs::remove_file(path).map_err(|e| e.to_string())?;
        }
        self.summaries_dirty
            .store(false, std::sync::atomic::Ordering::Release);
        Ok(())
    }
    fn summaries(&self, conversation: &str) -> Result<Vec<Record>, String> {
        self.repair_summaries()?;
        let directory = self.summary_dir(conversation)?;
        if !directory.exists() {
            return Ok(Vec::new());
        }
        let mut summaries = Vec::new();
        for entry in std::fs::read_dir(directory).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            summaries.push(serde_json::from_str(&text).map_err(|e| e.to_string())?);
        }
        Ok(summaries)
    }
    fn read_all(&self) -> Result<Vec<Record>, String> {
        let index = self.root.join("index");
        if !index.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in std::fs::read_dir(index).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() {
                if let Some(conversation) = path.file_name().and_then(|s| s.to_str()) {
                    out.extend(self.summaries(conversation)?);
                }
            }
        }
        Ok(out)
    }
    fn scoped(&self, conversation: &str, id: &str) -> Result<Record, String> {
        let record = self.read(id)?;
        if record.conversation_id != conversation {
            return Err("Child does not belong to this conversation".into());
        }
        Ok(record)
    }
    fn save(&self, record: &mut Record) -> Result<(), String> {
        record.sequence += 1;
        let content = serde_json::to_string(record).map_err(|e| e.to_string())?;
        let pending = self.root.join(format!("{}.pending", record.id));
        super::super::storage::atomic_write(&pending, "summary pending", "sub-agent transaction")?;
        super::super::storage::atomic_write(&self.path(&record.id)?, &content, "sub-agent")?;
        // The record is the commit point. The pending marker makes a secondary
        // summary failure recoverable without rejecting an already accepted task.
        if self.write_summary(record).is_ok() {
            let _ = std::fs::remove_file(pending);
        } else {
            self.summaries_dirty
                .store(true, std::sync::atomic::Ordering::Release);
        }
        self.events.send_modify(|seq| *seq = seq.wrapping_add(1));
        Ok(())
    }
    pub fn get(&self, conversation: &str, id: &str) -> Result<Record, String> {
        let control = self.lock();
        let mut record = self.scoped(conversation, id)?;
        Self::overlay_status(&control, &mut record);
        Ok(record)
    }
    fn overlay_status(control: &Control, record: &mut Record) {
        let id = record.current().id.clone();
        if record.current().status.active() {
            if control.stopping.contains(&id) {
                record.current_mut().status = Status::Stopping;
            }
            if let Some(error) = control.storage_errors.get(&id) {
                record.current_mut().error = Some(format!(
                    "Waiting for durable storage; cleanup is not complete: {error}"
                ));
            }
        }
    }
    pub fn list(&self, conversation: &str) -> Result<Vec<Record>, String> {
        let control = self.lock();
        let mut records = self.summaries(conversation)?;
        for record in &mut records {
            Self::overlay_status(&control, record);
        }
        Ok(records)
    }
    pub fn set_limit(&self, limit: usize) {
        self.lock().limit = limit.clamp(1, 64);
    }
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.events.subscribe()
    }
    pub fn sequence(&self) -> u64 {
        *self.events.borrow()
    }
    pub fn attach(&self, run: &str, handle: tokio::task::JoinHandle<()>) {
        let mut control = self.lock();
        if control.active.contains(run) {
            control.handles.insert(run.into(), handle);
        }
    }
    pub fn claim_worker(&self, run: &str) -> bool {
        let mut control = self.lock();
        control.active.contains(run) && control.claimed.insert(run.into())
    }

    pub fn spawn_task<F>(self: &std::sync::Arc<Self>, record: &Record, work: F) -> bool
    where
        F: std::future::Future<Output = Result<(String, Option<Value>), String>> + Send + 'static,
    {
        let run = record.current().id.clone();
        if !self.claim_worker(&run) {
            return false;
        }
        let runtime = self.clone();
        let conversation = record.conversation_id.clone();
        let id = record.id.clone();
        let worker_run = run.clone();
        let handle = tokio::spawn(async move {
            let outcome = std::panic::AssertUnwindSafe(work)
                .catch_unwind()
                .await
                .unwrap_or_else(|_| Err("Sub-agent worker panicked".into()));
            loop {
                match runtime.finish(&conversation, &id, &worker_run, outcome.clone()) {
                    Ok(_) => break,
                    Err(error) => {
                        runtime
                            .lock()
                            .storage_errors
                            .insert(worker_run.clone(), error.clone());
                        runtime.events.send_modify(|seq| *seq = seq.wrapping_add(1));
                        eprintln!(
                            "Cannot persist sub-agent completion; retrying storage only: {error}"
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                }
            }
        });
        self.attach(&run, handle);
        true
    }

    pub fn start(
        &self,
        conversation: &str,
        parent_run: &str,
        key: &str,
        name: &str,
        profile: Profile,
        prompt: &str,
    ) -> Result<Record, String> {
        let mut control = self.lock();
        if prompt.trim().is_empty() || key.is_empty() {
            return Err("Task and admission key are required".into());
        }
        if control.deleting.contains(conversation) {
            return Err("Conversation is being deleted".into());
        }
        if control.exiting {
            return Err("Application is stopping".into());
        }
        let request_key = format!("{parent_run}/{key}");
        for summary in self.summaries(conversation)? {
            if summary.request_key == request_key {
                let record = self.scoped(conversation, &summary.id)?;
                if record.runs[0].prompt != prompt {
                    return Err("Admission key was already used for a different task".into());
                }
                return Ok(record);
            }
        }
        if control.sealed.contains(parent_run) {
            return Err("Collaboration was stopped".into());
        }
        if control.active.len() >= control.limit {
            return Err("Sub-agent capacity is full; wait and retry".into());
        }
        let mut history = Vec::new();
        if !profile.system_prompt.is_empty() {
            history.push(json!({"role":"system", "content":profile.system_prompt}));
        }
        history.push(json!({"role":"user", "content":prompt}));
        let mut record = Record {
            preview: String::new(),
            steps: Vec::new(),
            version: 1,
            id: uuid::Uuid::new_v4().to_string(),
            conversation_id: conversation.into(),
            request_key,
            name: name.into(),
            profile,
            runs: vec![execution(parent_run, prompt)],
            history,
            messages: vec![],
            tools: vec![],
            user_stopped: false,
            sequence: 0,
        };
        self.save(&mut record)?;
        control.active.insert(record.current().id.clone());
        control.parent_runs.insert(
            record.current().id.clone(),
            record.current().parent_run.clone(),
        );
        control.owners.insert(
            record.current().id.clone(),
            (record.conversation_id.clone(), record.id.clone()),
        );
        Ok(record)
    }

    pub fn send(
        &self,
        conversation: &str,
        id: &str,
        message_id: &str,
        sender: &str,
        text: &str,
    ) -> Result<Record, String> {
        let _guard = self.lock();
        let mut record = self.scoped(conversation, id)?;
        enqueue(&mut record, message_id, sender, text)?;
        self.save(&mut record)?;
        Ok(record)
    }

    pub fn resume(
        &self,
        conversation: &str,
        id: &str,
        parent_run: &str,
        key: &str,
        sender: &str,
        text: &str,
    ) -> Result<(Record, bool), String> {
        let mut control = self.lock();
        if control.deleting.contains(conversation) {
            return Err("Conversation is being deleted".into());
        }
        let mut record = self.scoped(conversation, id)?;
        if control.sealed.contains(parent_run) {
            return Err("Collaboration was stopped".into());
        }
        if control.exiting {
            return Err("Application is stopping".into());
        }
        if record.user_stopped && !matches!(sender, "user" | "main_agent_user_requested") {
            return Err("User stopped this child; explicit user continuation required".into());
        }
        if record.messages.iter().any(|m| m.id == key) {
            return Ok((record, false));
        }
        if record.current().status == Status::Stopping
            || record.current().status == Status::Finishing
        {
            return Err("Previous execution is still cleaning up; retry after it ends".into());
        }
        let starts = !record.current().status.active();
        if starts && control.active.len() >= control.limit {
            return Err("Sub-agent capacity is full; wait and retry".into());
        }
        enqueue(&mut record, key, sender, text)?;
        if starts {
            let completed: Vec<_> = record
                .tools
                .iter()
                .filter(|t| t["status"] != "unknown" && t["contextRecorded"] != true)
                .cloned()
                .collect();
            if !completed.is_empty() {
                record.history.push(json!({"role":"user", "content":format!("Previously completed tool operations recovered from the durable tool ledger. Use these results; do not replay the completed work: {}", json!(completed))}));
            }
            for tool in &mut record.tools {
                if tool["status"] != "unknown" {
                    tool["contextRecorded"] = json!(true);
                }
            }
            let unknown: Vec<_> = record
                .tools
                .iter()
                .filter(|t| t["status"] == "unknown")
                .cloned()
                .collect();
            if !unknown.is_empty() {
                record.history.push(json!({"role":"user", "content":format!("Previous execution was interrupted. These tool outcomes are unknown; inspect external state before retrying any operation: {}", json!(unknown))}));
            }
            record.runs.push(execution(parent_run, text));
            record.user_stopped = false;
        }
        self.save(&mut record)?;
        if starts {
            control.active.insert(record.current().id.clone());
            control.parent_runs.insert(
                record.current().id.clone(),
                record.current().parent_run.clone(),
            );
            control.owners.insert(
                record.current().id.clone(),
                (record.conversation_id.clone(), record.id.clone()),
            );
        }
        Ok((record, starts))
    }

    /// Append accepted input and its consumption marker in one file transaction.
    /// A final boundary closes admission into this execution under the same lock.
    pub fn checkpoint(
        &self,
        conversation: &str,
        id: &str,
        run: &str,
        history: &[Value],
        finishing: bool,
    ) -> Result<Vec<Value>, String> {
        let _guard = self.lock();
        let mut record = self.scoped(conversation, id)?;
        if record.current().id != run {
            return Err("Stale child execution".into());
        }
        record.history = history.to_vec();
        // Remember incorporation before compaction can discard old tool messages.
        // Tool-call IDs can be reused in later executions, so match this run too.
        for tool in &mut record.tools {
            if tool["executionId"] == run
                && tool["status"] != "unknown"
                && history
                    .iter()
                    .any(|m| m["role"] == "tool" && m["tool_call_id"] == tool["id"])
            {
                tool["contextRecorded"] = json!(true);
            }
        }
        let mut added = Vec::new();
        if record.current().status == Status::Running {
            for message in &mut record.messages {
                if message.consumed_by.is_none() {
                    let value = json!({"role":"user", "content":format!("[Sub-agent message from {}]\n{}", message.sender, message.text), "subagent_message_id":message.id});
                    message.consumed_by = Some(run.into());
                    record.history.push(value.clone());
                    added.push(value);
                }
            }
            if finishing && added.is_empty() {
                record.current_mut().status = Status::Finishing;
            }
        }
        self.save(&mut record)?;
        Ok(added)
    }

    pub fn close_input(&self, conversation: &str, id: &str, run: &str) -> Result<(), String> {
        let _guard = self.lock();
        let mut record = self.scoped(conversation, id)?;
        if record.current().id != run {
            return Err("Stale child execution".into());
        }
        if record.current().status == Status::Running {
            record.current_mut().status = Status::Finishing;
            self.save(&mut record)?;
        }
        Ok(())
    }

    pub fn stop(
        &self,
        conversation: &str,
        id: &str,
        expected: &str,
        user: bool,
    ) -> Result<Record, String> {
        let mut control = self.lock();
        let mut record = self.scoped(conversation, id)?;
        if record.current().id != expected {
            return Err("Stale child execution; refresh before stopping".into());
        }
        if record.current().status.active() {
            control.stopping.insert(expected.into());
            if user {
                control.user_stops.insert(expected.into());
            }
            record.current_mut().status = Status::Stopping;
            record.user_stopped |= user;
            self.save(&mut record)?;
        }
        Ok(record)
    }

    pub fn stop_parent(&self, conversation: &str, parent_run: Option<&str>) -> Result<(), String> {
        let mut control = self.lock();
        if let Some(run) = parent_run {
            control.live_parents.remove(run);
            control.sealed.insert(run.into());
        }
        // Cancellation cannot depend on a successful disk read or write. Mark
        // every admitted worker first, then persist each status independently.
        let targets: Vec<_> = control
            .owners
            .iter()
            .filter(|(run, (owner, _))| {
                owner == conversation
                    && parent_run.map_or(true, |parent| {
                        control.parent_runs.get(*run).is_some_and(|p| p == parent)
                    })
            })
            .map(|(run, (_, id))| (run.clone(), id.clone()))
            .collect();
        for (run, _) in &targets {
            control.stopping.insert(run.clone());
            control.user_stops.insert(run.clone());
            if let Some(parent) = control.parent_runs.get(run).cloned() {
                control.sealed.insert(parent.clone());
                control.live_parents.remove(&parent);
            }
        }
        let mut errors = Vec::new();
        for (_, id) in targets {
            let result = self.scoped(conversation, &id).and_then(|mut record| {
                record.current_mut().status = Status::Stopping;
                record.user_stopped = true;
                self.save(&mut record)
            });
            if let Err(error) = result {
                errors.push(error);
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }

    pub fn running(&self, conversation: &str, id: &str, run: &str) -> bool {
        let control = self.lock();
        control
            .owners
            .get(run)
            .is_some_and(|(owner, agent)| owner == conversation && agent == id)
            && control.active.contains(run)
            && !control.stopping.contains(run)
    }

    pub fn register_parent(&self, run: &str) {
        self.lock().live_parents.insert(run.into());
    }

    pub fn cancel_admission(&self, run: &str) {
        let mut control = self.lock();
        control.stopping.insert(run.into());
        control.user_stops.insert(run.into());
    }

    /// Orphans are recoverable by the next parent, but a live owner receives its
    /// own results even when multiple model arms share one conversation.
    pub fn can_collect(&self, owner: &str, recipient: &str) -> bool {
        owner == recipient || !self.lock().live_parents.contains(owner)
    }

    pub fn finish(
        &self,
        conversation: &str,
        id: &str,
        run: &str,
        result: Result<(String, Option<Value>), String>,
    ) -> Result<Record, String> {
        let mut control = self.lock();
        let mut record = self.scoped(conversation, id)?;
        if record.current().id != run {
            return Err("Stale child execution".into());
        }
        if !record.current().status.active() {
            return Ok(record);
        }
        let stopping =
            record.current().status == Status::Stopping || control.stopping.contains(run);
        record.user_stopped |= control.user_stops.contains(run);
        let current = record.current_mut();
        match result {
            Ok((content, usage)) => {
                current.result = Some(content);
                current.usage = usage;
                current.status = if stopping {
                    Status::Interrupted
                } else {
                    Status::Completed
                };
            }
            Err(error) => {
                current.error = Some(error);
                current.status = if stopping {
                    Status::Interrupted
                } else {
                    Status::Failed
                };
            }
        }
        self.save(&mut record)?;
        control.storage_errors.remove(run);
        control.active.remove(run);
        control.owners.remove(run);
        control.parent_runs.remove(run);
        control.stopping.remove(run);
        control.user_stops.remove(run);
        control.claimed.remove(run);
        control.handles.remove(run);
        Ok(record)
    }

    pub fn tool_record(
        &self,
        conversation: &str,
        id: &str,
        run: &str,
        tool: Value,
    ) -> Result<(), String> {
        let _guard = self.lock();
        let mut record = self.scoped(conversation, id)?;
        if record.current().id != run {
            return Err("Stale child execution".into());
        }
        let key = tool["id"].clone();
        let mut tool = tool;
        tool["executionId"] = json!(run);
        if let Some(old) = record
            .tools
            .iter_mut()
            .find(|t| t["id"] == key && t["executionId"] == run)
        {
            if let (Some(old), Some(update)) = (old.as_object_mut(), tool.as_object()) {
                old.extend(update.clone());
            }
        } else {
            record.tools.push(tool);
        }
        self.save(&mut record)
    }

    pub fn progress(
        &self,
        conversation: &str,
        id: &str,
        run: &str,
        text: String,
        steps: Vec<String>,
    ) -> Result<(), String> {
        let _guard = self.lock();
        let mut record = self.scoped(conversation, id)?;
        if record.current().id != run || !record.current().status.active() {
            return Ok(());
        }
        record.preview = text;
        record.steps = steps;
        self.save(&mut record)
    }

    pub fn acknowledge_result(
        &self,
        conversation: &str,
        id: &str,
        run: &str,
    ) -> Result<(), String> {
        let _guard = self.lock();
        let mut record = self.scoped(conversation, id)?;
        let execution = record
            .runs
            .iter_mut()
            .find(|r| r.id == run)
            .ok_or("Unknown execution")?;
        if execution.status.active() {
            return Err("Cannot acknowledge an unfinished execution".into());
        }
        execution.delivered = true;
        self.save(&mut record)
    }

    pub async fn shutdown(&self) -> Result<(), String> {
        {
            let mut control = self.lock();
            control.exiting = true;
            let active = control.active.clone();
            control.stopping.extend(active);
            let ids: Vec<_> = control.owners.values().map(|(_, id)| id.clone()).collect();
            for id in ids {
                let result = self.read(&id).and_then(|mut record| {
                    record.current_mut().status = Status::Stopping;
                    self.save(&mut record)
                });
                if let Err(error) = result {
                    eprintln!(
                        "Cannot persist shutdown status; worker retains cleanup ownership: {error}"
                    );
                }
            }
        }
        while !self.lock().active.is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        Ok(())
    }

    pub async fn delete_conversation(&self, conversation: &str) -> Result<(), String> {
        self.lock().deleting.insert(conversation.into());
        self.stop_parent(conversation, None)?;
        loop {
            if !self
                .list(conversation)?
                .iter()
                .any(|r| r.current().status.active())
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let _guard = self.lock();
        for record in self
            .read_all()?
            .into_iter()
            .filter(|r| r.conversation_id == conversation)
        {
            std::fs::remove_file(self.path(&record.id)?).map_err(|e| e.to_string())?;
            std::fs::remove_file(
                self.summary_dir(conversation)?
                    .join(format!("{}.json", record.id)),
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

fn execution(parent_run: &str, prompt: &str) -> Execution {
    Execution {
        delivered: false,
        id: uuid::Uuid::new_v4().to_string(),
        parent_run: parent_run.into(),
        status: Status::Running,
        prompt: prompt.into(),
        result: None,
        error: None,
        usage: None,
    }
}
fn enqueue(record: &mut Record, key: &str, sender: &str, text: &str) -> Result<(), String> {
    if key.is_empty() || text.trim().is_empty() || text.len() > 100_000 {
        return Err("Message requires an identifier and 1–100000 bytes of text".into());
    }
    if let Some(message) = record.messages.iter().find(|m| m.id == key) {
        return if message.text == text && message.sender == sender {
            Ok(())
        } else {
            Err("Message identifier was already used for different content".into())
        };
    }
    if record
        .messages
        .iter()
        .filter(|m| m.consumed_by.is_none())
        .count()
        >= 100
    {
        return Err("Child inbox is full".into());
    }
    record.messages.push(Message {
        id: key.into(),
        sender: sender.into(),
        text: text.into(),
        consumed_by: None,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn child(runtime: &Runtime, key: &str) -> Record {
        runtime
            .start(
                "conv_a",
                "parent",
                key,
                "worker",
                Profile::default(),
                "inspect",
            )
            .unwrap()
    }

    #[test]
    fn capacity_is_admission_not_a_hidden_queue() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(dir.path().into()).unwrap();
        runtime.set_limit(1);
        let a = child(&runtime, "a");
        assert!(runtime
            .start(
                "conv_a",
                "parent",
                "b",
                "worker",
                Profile::default(),
                "inspect"
            )
            .unwrap_err()
            .contains("capacity"));
        assert_eq!(runtime.list("conv_a").unwrap().len(), 1);
        runtime
            .finish("conv_a", &a.id, &a.current().id, Ok(("done".into(), None)))
            .unwrap();
        let b = child(&runtime, "b");
        assert_ne!(a.id, b.id);
        assert_eq!(
            runtime
                .get("conv_a", &a.id)
                .unwrap()
                .current()
                .result
                .as_deref(),
            Some("done")
        );
    }

    #[test]
    fn stopping_one_child_keeps_others_running_and_holds_capacity_until_finished() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(dir.path().into()).unwrap();
        runtime.set_limit(2);
        let a = child(&runtime, "a");
        let b = child(&runtime, "b");
        runtime
            .stop("conv_a", &a.id, &a.current().id, true)
            .unwrap();
        assert!(!runtime.running("conv_a", &a.id, &a.current().id));
        assert!(runtime.running("conv_a", &b.id, &b.current().id));
        assert!(runtime
            .start(
                "conv_a",
                "parent",
                "c",
                "worker",
                Profile::default(),
                "inspect"
            )
            .is_err());
        runtime
            .finish("conv_a", &a.id, &a.current().id, Err("cancelled".into()))
            .unwrap();
        assert_eq!(
            runtime.get("conv_a", &a.id).unwrap().current().status,
            Status::Interrupted
        );
        child(&runtime, "c");
    }

    #[test]
    fn messages_are_consumed_once_and_late_messages_remain_idle() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(dir.path().into()).unwrap();
        let a = child(&runtime, "a");
        runtime
            .send("conv_a", &a.id, "m1", "user", "new evidence")
            .unwrap();
        runtime
            .send("conv_a", &a.id, "m1", "user", "new evidence")
            .unwrap();
        let added = runtime
            .checkpoint("conv_a", &a.id, &a.current().id, &a.history, false)
            .unwrap();
        assert_eq!(added.len(), 1);
        let saved = runtime.get("conv_a", &a.id).unwrap();
        assert_eq!(
            saved.messages[0].consumed_by.as_deref(),
            Some(a.current().id.as_str())
        );
        assert_eq!(saved.history.len(), 2);
        assert!(runtime
            .checkpoint("conv_a", &a.id, &a.current().id, &saved.history, true)
            .unwrap()
            .is_empty());
        runtime
            .send("conv_a", &a.id, "late", "user", "later")
            .unwrap();
        runtime
            .finish("conv_a", &a.id, &a.current().id, Ok(("done".into(), None)))
            .unwrap();
        assert!(runtime.get("conv_a", &a.id).unwrap().messages[1]
            .consumed_by
            .is_none());
    }

    #[test]
    fn failed_worker_keeps_late_input_pending_for_explicit_continuation() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(dir.path().into()).unwrap();
        let record = child(&runtime, "a");
        runtime
            .send("conv_a", &record.id, "late", "user", "new requirement")
            .unwrap();
        runtime
            .close_input("conv_a", &record.id, &record.current().id)
            .unwrap();
        assert!(runtime
            .checkpoint(
                "conv_a",
                &record.id,
                &record.current().id,
                &record.history,
                true
            )
            .unwrap()
            .is_empty());
        runtime
            .finish(
                "conv_a",
                &record.id,
                &record.current().id,
                Err("provider failed".into()),
            )
            .unwrap();
        assert!(runtime.get("conv_a", &record.id).unwrap().messages[0]
            .consumed_by
            .is_none());
        let (continued, _) = runtime
            .resume("conv_a", &record.id, "next", "continue", "user", "resume")
            .unwrap();
        let incoming = runtime
            .checkpoint(
                "conv_a",
                &record.id,
                &continued.current().id,
                &continued.history,
                false,
            )
            .unwrap();
        assert!(incoming.iter().any(|m| m["subagent_message_id"] == "late"));
    }

    #[test]
    fn pending_message_at_final_boundary_continues_current_execution() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(dir.path().into()).unwrap();
        let a = child(&runtime, "a");
        runtime
            .send("conv_a", &a.id, "m", "main_agent", "new evidence")
            .unwrap();
        assert_eq!(
            runtime
                .checkpoint("conv_a", &a.id, &a.current().id, &a.history, true)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            runtime.get("conv_a", &a.id).unwrap().current().status,
            Status::Running
        );
    }

    #[test]
    fn user_stop_requires_user_continuation_and_old_stop_cannot_touch_new_run() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(dir.path().into()).unwrap();
        let a = child(&runtime, "a");
        runtime
            .stop("conv_a", &a.id, &a.current().id, true)
            .unwrap();
        runtime
            .finish("conv_a", &a.id, &a.current().id, Err("cancelled".into()))
            .unwrap();
        assert!(runtime
            .resume("conv_a", &a.id, "next", "m", "main_agent", "retry")
            .is_err());
        let (continued, starts) = runtime
            .resume("conv_a", &a.id, "next", "m", "user", "retry")
            .unwrap();
        assert!(starts);
        assert_ne!(continued.current().id, a.current().id);
        assert!(runtime
            .stop("conv_a", &a.id, &a.current().id, true)
            .is_err());
        let (same, starts) = runtime
            .resume("conv_a", &a.id, "next", "m", "user", "retry")
            .unwrap();
        assert!(!starts);
        assert_eq!(same.current().id, continued.current().id);
    }

    #[test]
    fn main_agent_can_relay_a_new_user_continuation_without_impersonation() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(dir.path().into()).unwrap();
        let record = child(&runtime, "a");
        runtime
            .stop("conv_a", &record.id, &record.current().id, true)
            .unwrap();
        runtime
            .finish(
                "conv_a",
                &record.id,
                &record.current().id,
                Err("stopped".into()),
            )
            .unwrap();
        assert!(runtime
            .resume("conv_a", &record.id, "next", "auto", "main_agent", "retry")
            .is_err());
        let (resumed, starts) = runtime
            .resume(
                "conv_a",
                &record.id,
                "next",
                "requested",
                "main_agent_user_requested",
                "user explicitly requested continuation",
            )
            .unwrap();
        assert!(starts);
        assert_eq!(
            resumed.messages.last().unwrap().sender,
            "main_agent_user_requested"
        );
    }

    #[test]
    fn stopping_parent_seals_new_admission_without_affecting_other_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(dir.path().into()).unwrap();
        let a = child(&runtime, "a");
        let other = runtime
            .start(
                "conv_b",
                "other-parent",
                "a",
                "worker",
                Profile::default(),
                "inspect",
            )
            .unwrap();
        runtime.stop_parent("conv_a", Some("parent")).unwrap();
        assert!(!runtime.running("conv_a", &a.id, &a.current().id));
        assert!(runtime
            .start(
                "conv_a",
                "parent",
                "late",
                "worker",
                Profile::default(),
                "inspect"
            )
            .is_err());
        assert!(runtime.running("conv_b", &other.id, &other.current().id));
    }

    #[test]
    fn restart_retains_pending_messages_and_unknown_tools_without_replaying() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(dir.path().into()).unwrap();
        let a = child(&runtime, "a");
        runtime
            .send("conv_a", &a.id, "m", "user", "check again")
            .unwrap();
        runtime
            .tool_record(
                "conv_a",
                &a.id,
                &a.current().id,
                json!({"id":"write","status":"unknown"}),
            )
            .unwrap();
        drop(runtime);
        let runtime = Runtime::open(dir.path().into()).unwrap();
        let saved = runtime.get("conv_a", &a.id).unwrap();
        assert_eq!(saved.current().status, Status::Interrupted);
        assert!(saved.messages[0].consumed_by.is_none());
        let (continued, _) = runtime
            .resume("conv_a", &a.id, "new-parent", "continue", "user", "inspect")
            .unwrap();
        assert!(continued.history.iter().any(|m| m["content"]
            .as_str()
            .unwrap_or("")
            .contains("inspect external state")));
        assert_eq!(continued.tools[0]["status"], "unknown");
    }

    #[test]
    fn terminal_replay_is_idempotent_and_cannot_regress_status() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(dir.path().into()).unwrap();
        let a = child(&runtime, "a");
        runtime
            .finish(
                "conv_a",
                &a.id,
                &a.current().id,
                Ok(("complete result".into(), None)),
            )
            .unwrap();
        let late = runtime
            .finish("conv_a", &a.id, &a.current().id, Err("late failure".into()))
            .unwrap();
        assert_eq!(late.current().status, Status::Completed);
        assert_eq!(late.current().result.as_deref(), Some("complete result"));
    }

    #[test]
    fn history_is_not_evicted_at_the_old_process_record_limit() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(dir.path().into()).unwrap();
        let first = child(&runtime, "first");
        runtime
            .finish(
                "conv_a",
                &first.id,
                &first.current().id,
                Ok(("old result".into(), None)),
            )
            .unwrap();
        for n in 0..130 {
            let a = child(&runtime, &n.to_string());
            runtime
                .finish(
                    "conv_a",
                    &a.id,
                    &a.current().id,
                    Ok(("result".into(), None)),
                )
                .unwrap();
        }
        assert_eq!(
            runtime
                .get("conv_a", &first.id)
                .unwrap()
                .current()
                .result
                .as_deref(),
            Some("old result")
        );
    }

    #[tokio::test]
    async fn event_between_snapshot_and_wait_is_not_lost() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(dir.path().into()).unwrap();
        let mut events = runtime.subscribe();
        child(&runtime, "a");
        tokio::time::timeout(std::time::Duration::from_secs(1), events.changed())
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn real_supervisor_returns_before_work_finishes_and_notifies_independently() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = std::sync::Arc::new(Runtime::open(dir.path().into()).unwrap());
        let a = child(&runtime, "a");
        let b = child(&runtime, "b");
        let (release_a, wait_a) = tokio::sync::oneshot::channel();
        let (release_b, wait_b) = tokio::sync::oneshot::channel();
        assert!(runtime.spawn_task(&a, async move {
            wait_a.await.unwrap();
            Ok(("A result".into(), None))
        }));
        assert!(runtime.spawn_task(&b, async move {
            wait_b.await.unwrap();
            Ok(("B result".into(), None))
        }));
        assert!(!runtime.spawn_task(&a, async { panic!("duplicate must never execute") }));
        assert_eq!(
            runtime.get("conv_a", &a.id).unwrap().current().status,
            Status::Running
        );
        let mut events = runtime.subscribe();
        release_a.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while runtime
                .get("conv_a", &a.id)
                .unwrap()
                .current()
                .status
                .active()
            {
                events.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert_eq!(
            runtime
                .get("conv_a", &a.id)
                .unwrap()
                .current()
                .result
                .as_deref(),
            Some("A result")
        );
        assert_eq!(
            runtime.get("conv_a", &b.id).unwrap().current().status,
            Status::Running
        );
        release_b.send(()).unwrap();
        while runtime
            .get("conv_a", &b.id)
            .unwrap()
            .current()
            .status
            .active()
        {
            events.changed().await.unwrap();
        }
    }

    #[tokio::test]
    async fn worker_panic_becomes_observable_failure_and_releases_capacity() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = std::sync::Arc::new(Runtime::open(dir.path().into()).unwrap());
        runtime.set_limit(1);
        let a = child(&runtime, "a");
        let mut events = runtime.subscribe();
        runtime.spawn_task(&a, async { panic!("simulated worker failure") });
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while runtime
                .get("conv_a", &a.id)
                .unwrap()
                .current()
                .status
                .active()
            {
                events.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert_eq!(
            runtime.get("conv_a", &a.id).unwrap().current().status,
            Status::Failed
        );
        child(&runtime, "b");
    }

    #[test]
    fn accepted_start_is_idempotent_and_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(dir.path().to_path_buf()).unwrap();
        let first = runtime
            .start(
                "conv_a",
                "parent-run",
                "request-1",
                "research",
                Profile::default(),
                "inspect",
            )
            .unwrap();
        let again = runtime
            .start(
                "conv_a",
                "parent-run",
                "request-1",
                "research",
                Profile::default(),
                "inspect",
            )
            .unwrap();
        assert_eq!(first.id, again.id);
        drop(runtime);
        let reopened = Runtime::open(dir.path().to_path_buf()).unwrap();
        let record = reopened.get("conv_a", &first.id).unwrap();
        assert_eq!(record.current().status, Status::Interrupted);
        assert!(reopened.get("conv_b", &first.id).is_err());
        assert_eq!(record.history[0]["content"], "inspect");
    }

    #[test]
    fn scoped_lists_do_not_load_full_history_or_unrelated_conversations() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(dir.path().into()).unwrap();
        let a = child(&runtime, "a");
        let other = runtime
            .start(
                "conv_b",
                "other",
                "b",
                "worker",
                Profile::default(),
                "private history",
            )
            .unwrap();
        std::fs::write(
            dir.path().join(format!("{}.json", other.id)),
            "damaged unrelated history",
        )
        .unwrap();
        let list = runtime.list("conv_a").unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].history.is_empty());
        assert!(list[0].tools.is_empty());
        assert_eq!(
            runtime.get("conv_a", &a.id).unwrap().history[0]["content"],
            "inspect"
        );
    }

    #[test]
    fn accepted_message_survives_secondary_summary_write_failure() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(dir.path().into()).unwrap();
        let a = child(&runtime, "a");
        let index = dir.path().join("index");
        let backup = dir.path().join("index-backup");
        std::fs::rename(&index, &backup).unwrap();
        std::fs::write(&index, "simulate failed summary storage").unwrap();
        runtime
            .send("conv_a", &a.id, "m", "user", "durable input")
            .unwrap();
        assert_eq!(runtime.get("conv_a", &a.id).unwrap().messages.len(), 1);
        std::fs::remove_file(&index).unwrap();
        std::fs::rename(&backup, &index).unwrap();
        assert_eq!(runtime.list("conv_a").unwrap().len(), 1);
        drop(runtime);
        let reopened = Runtime::open(dir.path().into()).unwrap();
        assert_eq!(
            reopened.get("conv_a", &a.id).unwrap().messages[0].text,
            "durable input"
        );
    }

    #[test]
    fn failed_primary_write_does_not_acknowledge_input() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("records");
        let backup = dir.path().join("backup");
        let runtime = Runtime::open(root.clone()).unwrap();
        let a = child(&runtime, "a");
        std::fs::rename(&root, &backup).unwrap();
        std::fs::write(&root, "storage offline").unwrap();
        assert!(runtime
            .send("conv_a", &a.id, "m", "user", "not accepted")
            .is_err());
        std::fs::remove_file(&root).unwrap();
        std::fs::rename(&backup, &root).unwrap();
        assert!(runtime.get("conv_a", &a.id).unwrap().messages.is_empty());
    }

    #[test]
    fn completed_tool_is_reconciled_into_continuation_after_checkpoint_crash() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(dir.path().into()).unwrap();
        let a = child(&runtime, "a");
        runtime
            .tool_record(
                "conv_a",
                &a.id,
                &a.current().id,
                json!({"id":"write","arguments":{"path":"report"},"status":"unknown"}),
            )
            .unwrap();
        runtime
            .tool_record(
                "conv_a",
                &a.id,
                &a.current().id,
                json!({"id":"write","status":"returned","result":"saved"}),
            )
            .unwrap();
        drop(runtime);
        let runtime = Runtime::open(dir.path().into()).unwrap();
        let (continued, _) = runtime
            .resume("conv_a", &a.id, "next", "m", "user", "continue")
            .unwrap();
        assert_eq!(continued.tools[0]["arguments"]["path"], "report");
        assert!(continued.history.iter().any(|m| m["content"]
            .as_str()
            .is_some_and(|s| s.contains("Previously completed") && s.contains("saved"))));
    }

    #[test]
    fn continuation_preserves_compaction_and_does_not_resurrect_old_tools() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(dir.path().into()).unwrap();
        let a = child(&runtime, "a");
        runtime
            .tool_record(
                "conv_a",
                &a.id,
                &a.current().id,
                json!({"id":"write", "status":"returned", "result":"large old result"}),
            )
            .unwrap();
        runtime
            .checkpoint(
                "conv_a",
                &a.id,
                &a.current().id,
                &[json!({"role":"tool", "tool_call_id":"write", "content":"large old result"})],
                false,
            )
            .unwrap();
        let compacted = vec![json!({"role":"user", "content":"Compacted: report saved"})];
        runtime
            .checkpoint("conv_a", &a.id, &a.current().id, &compacted, true)
            .unwrap();
        runtime
            .finish("conv_a", &a.id, &a.current().id, Ok(("done".into(), None)))
            .unwrap();
        drop(runtime);
        let reopened = Runtime::open(dir.path().into()).unwrap();
        let (continued, _) = reopened
            .resume("conv_a", &a.id, "next", "m", "user", "continue")
            .unwrap();
        assert_eq!(continued.history, compacted);
        assert_eq!(continued.tools[0]["result"], "large old result");
    }

    #[test]
    fn live_parent_results_are_not_claimed_by_other_model_arms() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(dir.path().into()).unwrap();
        runtime.register_parent("a");
        runtime.register_parent("b");
        assert!(runtime.can_collect("a", "a"));
        assert!(!runtime.can_collect("a", "b"));
        runtime.stop_parent("conv_a", Some("a")).unwrap();
        assert!(runtime.can_collect("a", "b"));
    }

    #[test]
    fn parent_stop_cancels_every_worker_even_when_storage_is_offline() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("records");
        let backup = dir.path().join("backup");
        let runtime = Runtime::open(root.clone()).unwrap();
        let a = child(&runtime, "a");
        let b = child(&runtime, "b");
        std::fs::rename(&root, &backup).unwrap();
        std::fs::write(&root, "storage offline").unwrap();
        assert!(runtime.stop_parent("conv_a", Some("parent")).is_err());
        assert!(!runtime.running("conv_a", &a.id, &a.current().id));
        assert!(!runtime.running("conv_a", &b.id, &b.current().id));
        std::fs::remove_file(&root).unwrap();
        std::fs::rename(&backup, &root).unwrap();
        for record in [a, b] {
            let ended = runtime
                .finish(
                    "conv_a",
                    &record.id,
                    &record.current().id,
                    Ok(("partial".into(), None)),
                )
                .unwrap();
            assert_eq!(ended.current().status, Status::Interrupted);
            assert!(ended.user_stopped);
        }
        assert!(runtime.lock().active.is_empty());
    }

    #[tokio::test]
    async fn terminal_storage_retry_keeps_result_and_never_reexecutes_work() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = std::sync::Arc::new(Runtime::open(dir.path().into()).unwrap());
        let record = child(&runtime, "a");
        let path = runtime.path(&record.id).unwrap();
        let backup = dir.path().join("record-backup");
        std::fs::rename(&path, &backup).unwrap();
        std::fs::create_dir(&path).unwrap();
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let executed = calls.clone();
        runtime.spawn_task(&record, async move {
            executed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(("durable final report".into(), None))
        });
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while runtime.lock().storage_errors.is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(runtime.lock().active.contains(&record.current().id));
        std::fs::remove_dir(&path).unwrap();
        std::fs::rename(&backup, &path).unwrap();
        let waiting = runtime.get("conv_a", &record.id).unwrap();
        assert!(waiting.current().status.active());
        assert!(waiting
            .current()
            .error
            .as_deref()
            .unwrap()
            .contains("durable storage"));
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while !runtime.lock().active.is_empty() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let done = runtime.get("conv_a", &record.id).unwrap();
        assert_eq!(done.current().status, Status::Completed);
        assert_eq!(
            done.current().result.as_deref(),
            Some("durable final report")
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
