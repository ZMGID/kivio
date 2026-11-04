//! Stream-owned speculative task launches. Unadopted runs are cancelled on drop.

use std::{collections::HashMap, future::Future, sync::{Arc, Mutex}};

use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use tokio::{sync::mpsc, task::JoinHandle};

use crate::{chat::agent::ToolExecutionContext, mcp::registry::NativeToolContext};
use super::task::{self, SpeculativeRun};

#[derive(Clone)]
struct OwnedContext {
    conversation_id: String,
    issuer_conversation_id: String,
    message_id: String,
    provider: crate::settings::ModelProvider,
    model: String,
    run_id: String,
    generation: u64,
    round: u32,
    depth: u8,
    plan_state: crate::chat::types::AgentPlanState,
    mcp_tool_ids: Vec<(String, String)>,
    mcp_server_ids: Vec<String>,
    approval_policy: String,
    interactive_host: bool,
}

impl OwnedContext {
    fn new(ctx: &ToolExecutionContext<'_>) -> Self {
        Self {
            conversation_id: ctx.tool_conversation_id.to_string(),
            issuer_conversation_id: ctx.conversation_id.to_string(),
            message_id: ctx.message_id.to_string(),
            provider: ctx.provider.clone(),
            model: ctx.model.to_string(),
            run_id: ctx.run_id.to_string(),
            generation: ctx.generation,
            round: ctx.round,
            depth: ctx.depth,
            plan_state: ctx.plan_state.clone(),
            mcp_tool_ids: ctx.mcp_tool_ids.to_vec(),
            mcp_server_ids: ctx.mcp_server_ids.to_vec(),
            approval_policy: ctx.approval_policy.to_string(),
            interactive_host: ctx.interactive_host,
        }
    }

    fn native(&self, tool_call_id: &str) -> NativeToolContext<'_> {
        NativeToolContext {
            conversation_id: self.conversation_id.clone(),
            message_id: self.message_id.clone(),
            provider: &self.provider,
            model: &self.model,
            tool_call_id: Some(tool_call_id.to_string()),
            run_id: self.run_id.clone(),
            generation: self.generation,
            round: self.round,
            depth: self.depth,
            plan_state: &self.plan_state,
            mcp_tool_ids: &self.mcp_tool_ids,
            mcp_server_ids: &self.mcp_server_ids,
            approval_policy: &self.approval_policy,
            interactive_host: self.interactive_host,
        }
    }
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct Key {
    root: String,
    run: String,
    round: u32,
    call: String,
}

impl Key {
    fn new(ctx: &NativeToolContext<'_>) -> Self {
        Self {
            root: ctx.conversation_id.clone(),
            run: ctx.run_id.clone(),
            round: ctx.round,
            call: ctx.tool_call_id.clone().unwrap_or_default(),
        }
    }
}

#[derive(Default, Clone)]
pub(crate) struct Sessions {
    calls: Arc<Mutex<HashMap<Key, Arc<Call<SpeculativeRun>>>>>,
}

/// Owns all sessions made by one streamed attempt through its final tool round.
pub(crate) struct Scope {
    app: AppHandle,
    sessions: Sessions,
    context: OwnedContext,
}

impl Scope {
    pub(crate) fn new(app: AppHandle, ctx: &ToolExecutionContext<'_>) -> Self {
        let sessions = app.state::<crate::state::AppState>().sub_agents.speculative.clone();
        Self { app, sessions, context: OwnedContext::new(ctx) }
    }

    pub(crate) fn update(&self, call: &str, raw: &str) {
        let ctx = self.context.native(call);
        let key = Key::new(&ctx);
        let mut calls = self.sessions.calls.lock().unwrap_or_else(|e| e.into_inner());
        let session = calls.entry(key).or_insert_with(|| {
            let app = self.app.clone();
            let context = self.context.clone();
            let call = call.to_string();
            Call::new(move |index, args| {
                let app = app.clone();
                let context = context.clone();
                let call = call.clone();
                async move {
                    if !app.state::<crate::state::AppState>().chat_runtime()
                        .is_generation_active(&context.issuer_conversation_id, context.generation)
                    {
                        return None;
                    }
                    task::start_speculative(&app, &context.native(&call), &args, index).await
                }
            })
        });
        session.update(raw);
    }

    pub(crate) fn reconcile_call(&self, call: &str, args: Option<&Value>) {
        let key = Key::new(&self.context.native(call));
        let mut calls = self.sessions.calls.lock().unwrap_or_else(|e| e.into_inner());
        let discard = calls.get(&key).is_some_and(|session| args.is_none_or(|args| !session.matches(args)));
        let session = if discard { calls.remove(&key) } else { None };
        drop(calls);
        if let Some(session) = session { session.discard(); }
    }

    pub(crate) fn reconcile(&self, final_calls: &[(&str, &Value)]) {
        let mut calls = self.sessions.calls.lock().unwrap_or_else(|e| e.into_inner());
        let mut removed = Vec::new();
        calls.retain(|key, session| {
            let owned = key.root == self.context.conversation_id
                && key.run == self.context.run_id && key.round == self.context.round;
            if owned && !final_calls.iter().any(|(id, args)| {
                *id == key.call && session.matches(args)
            }) {
                removed.push(session.clone());
                false
            } else {
                true
            }
        });
        drop(calls);
        for session in removed { session.discard(); }
    }

    pub(crate) fn discard(&self) { self.reconcile(&[]); }
}

impl Drop for Scope {
    fn drop(&mut self) { self.discard(); }
}

impl Sessions {
    /// Called only after the final batch passes dispatch preflight and authorization.
    pub(crate) async fn adopt(&self, ctx: &NativeToolContext<'_>, args: &Value) -> Vec<Option<SpeculativeRun>> {
        let call = self.calls.lock().unwrap_or_else(|e| e.into_inner()).remove(&Key::new(ctx));
        match call {
            Some(call) => call.adopt(args).await,
            None => Vec::new(),
        }
    }

    pub(crate) fn discard(&self, ctx: &NativeToolContext<'_>) {
        let call = self.calls.lock().unwrap_or_else(|e| e.into_inner()).remove(&Key::new(ctx));
        if let Some(call) = call { call.discard(); }
    }
}

struct CallState<R> {
    scanner: BatchScanner,
    planned: Vec<(String, Value)>,
    runs: Vec<Option<R>>,
    sender: Option<mpsc::UnboundedSender<(usize, Value)>>,
    closed: bool,
}

struct Call<R> {
    state: Arc<Mutex<CallState<R>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl<R> Drop for Call<R> {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.lock().unwrap_or_else(|e| e.into_inner()).take() {
            worker.abort();
        }
        self.state.lock().unwrap_or_else(|e| e.into_inner()).runs.clear();
    }
}

struct AbortWorker(JoinHandle<()>);
impl Drop for AbortWorker {
    fn drop(&mut self) { self.0.abort(); }
}

impl<R: Send + 'static> Call<R> {
    fn new<F, Fut>(start: F) -> Arc<Self>
    where
        F: Fn(usize, Value) -> Fut + Send + 'static,
        Fut: Future<Output = Option<R>> + Send + 'static,
    {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let state = Arc::new(Mutex::new(CallState {
            scanner: BatchScanner::default(), planned: Vec::new(), runs: Vec::new(),
            sender: Some(sender), closed: false,
        }));
        let worker_state = state.clone();
        let worker = tokio::spawn(async move {
            while let Some((index, args)) = receiver.recv().await {
                if {
                    let state = worker_state.lock().unwrap_or_else(|e| e.into_inner());
                    state.closed
                } { break; }
                let run = start(index, args).await;
                let mut state = worker_state.lock().unwrap_or_else(|e| e.into_inner());
                if state.closed { break; }
                match run {
                    Some(run) => state.runs[index] = Some(run),
                    None => {
                        state.closed = true;
                        state.sender.take();
                        state.runs.clear();
                        break;
                    }
                }
            }
        });
        Arc::new(Self { state, worker: Mutex::new(Some(worker)) })
    }

    fn update(&self, raw: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed || !state.scanner.changed(raw) { return; }
        if state.scanner.invalid {
            drop(state);
            self.discard();
            return;
        }
        let Some(prefix) = parse_prefix(raw) else {
            drop(state);
            self.discard();
            return;
        };
        let Some(context) = prefix.context else { return; };
        if state.planned.iter().enumerate().any(|(index, (old_context, old_item))| {
            old_context != &context || prefix.items.get(index).is_none_or(|item| !items_match(old_item, item))
        }) {
            drop(state);
            self.discard();
            return;
        }
        for index in state.planned.len()..prefix.items.len() {
            let item = &prefix.items[index];
            if !item.is_object() {
                drop(state);
                self.discard();
                return;
            }
            let args = json!({"context": context, "tasks": &prefix.items[..=index]});
            state.planned.push((context.clone(), item.clone()));
            state.runs.push(None);
            if state.sender.as_ref().is_none_or(|sender| sender.send((index, args)).is_err()) {
                drop(state);
                self.discard();
                return;
            }
        }
    }

    fn discard(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.closed = true;
        state.sender.take();
        state.runs.clear();
        drop(state);
        if let Some(worker) = self.worker.lock().unwrap_or_else(|e| e.into_inner()).take() {
            worker.abort();
        }
    }

    fn matches(&self, args: &Value) -> bool {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        !state.closed && matches_plan(&state.planned, args)
    }

    async fn adopt(&self, args: &Value) -> Vec<Option<R>> {
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed || !matches_plan(&state.planned, args) {
                drop(state);
                self.discard();
                return Vec::new();
            }
            state.sender.take();
        }
        let worker = self.worker.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(worker) = worker {
            let mut worker = AbortWorker(worker);
            let _ = (&mut worker.0).await;
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.closed = true;
        std::mem::take(&mut state.runs)
    }
}

fn matches_plan(planned: &[(String, Value)], args: &Value) -> bool {
    let Some(context) = args.get("context").and_then(Value::as_str) else { return false; };
    let Some(items) = args.get("tasks").and_then(Value::as_array) else { return false; };
    planned.iter().enumerate().all(|(index, (old_context, old_item))| {
        old_context == context && items.get(index).is_some_and(|item| items_match(old_item, item))
    })
}

fn items_match(old: &Value, new: &Value) -> bool {
    let (Some(old), Some(new)) = (old.as_object(), new.as_object()) else { return false; };
    let agents_match = match (old.get("agent"), new.get("agent")) {
        (None, None) | (Some(Value::String(_)), None) | (None, Some(Value::String(_))) => true,
        (Some(Value::String(a)), Some(Value::String(b))) => a.trim() == b.trim(),
        _ => false,
    };
    agents_match
        && old.iter().filter(|(key, _)| *key != "agent").all(|(key, value)| new.get(key) == Some(value))
        && new.iter().filter(|(key, _)| *key != "agent").all(|(key, value)| old.get(key) == Some(value))
}

/// Incrementally finds JSON boundaries; serde validates and decodes every prefix.
#[derive(Default)]
struct BatchScanner {
    offset: usize,
    depth: usize,
    string: bool,
    escaped: bool,
    unicode_digits: u8,
    invalid: bool,
}

impl BatchScanner {
    fn changed(&mut self, raw: &str) -> bool {
        if raw.len() < self.offset { return false; }
        let mut boundary = false;
        for &byte in &raw.as_bytes()[self.offset..] {
            if self.string {
                if self.unicode_digits > 0 {
                    if byte.is_ascii_hexdigit() { self.unicode_digits -= 1; }
                    else { self.unicode_digits = 0; self.invalid = true; boundary = true; }
                } else if self.escaped {
                    self.escaped = false;
                    if byte == b'u' { self.unicode_digits = 4; }
                    else if !matches!(byte, b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') {
                        self.invalid = true;
                        boundary = true;
                    }
                } else if byte == b'\\' { self.escaped = true; }
                else if byte == b'"' {
                    self.string = false;
                    boundary |= self.depth == 1 || self.depth == 2;
                } else if byte < 0x20 { self.invalid = true; boundary = true; }
            } else {
                match byte {
                    b'"' => self.string = true,
                    b'{' | b'[' => self.depth += 1,
                    b'}' | b']' => {
                        boundary |= (byte == b'}' && self.depth == 3)
                            || (byte == b']' && self.depth == 2);
                        self.depth = self.depth.saturating_sub(1);
                    }
                    b',' if self.depth == 2 => boundary = true,
                    byte if self.depth == 2 && !byte.is_ascii_whitespace() => boundary = true,
                    _ => {}
                }
            }
        }
        self.offset = raw.len();
        boundary
    }
}

#[derive(Default)]
struct Prefix {
    context: Option<String>,
    items: Vec<Value>,
}

fn parse_prefix(raw: &str) -> Option<Prefix> {
    let mut prefix = Prefix::default();
    let mut de = serde_json::Deserializer::from_str(raw);
    let result = Root(&mut prefix).deserialize(&mut de).and_then(|()| de.end());
    match result {
        Ok(()) => Some(prefix),
        Err(error) if error.is_eof() => Some(prefix),
        Err(_) => None,
    }
}

struct Root<'a>(&'a mut Prefix);
impl<'de> DeserializeSeed<'de> for Root<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, de: D) -> Result<(), D::Error> {
        struct RootVisitor<'a>(&'a mut Prefix);
        impl<'de> Visitor<'de> for RootVisitor<'_> {
            type Value = ();
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("a task batch") }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<(), M::Error> {
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "context" => self.0.context = Some(map.next_value()?),
                        "tasks" => { self.0.items.clear(); map.next_value_seed(Tasks(&mut self.0.items))?; }
                        _ => { map.next_value::<IgnoredAny>()?; }
                    }
                }
                Ok(())
            }
        }
        de.deserialize_map(RootVisitor(self.0))
    }
}

struct Tasks<'a>(&'a mut Vec<Value>);
impl<'de> DeserializeSeed<'de> for Tasks<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, de: D) -> Result<(), D::Error> {
        struct TasksVisitor<'a>(&'a mut Vec<Value>);
        impl<'de> Visitor<'de> for TasksVisitor<'_> {
            type Value = ();
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("a task array") }
            fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<(), S::Error> {
                while let Some(item) = seq.next_element()? { self.0.push(item); }
                Ok(())
            }
        }
        de.deserialize_seq(TasksVisitor(self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::oneshot;

    struct Run {
        index: usize,
        dropped: Arc<AtomicUsize>,
    }

    impl Drop for Run {
        fn drop(&mut self) { self.dropped.fetch_add(1, Ordering::SeqCst); }
    }

    fn input() -> (Value, String, usize) {
        let context = "quoted \" and slash \\ with } and 🌍";
        let first = json!({"task": "inspect \\\" } still inside", "solutionSpace": "one fix"});
        let second = json!({"task": "second", "solutionSpace": "several"});
        let first_raw = first.to_string();
        let raw = format!(
            "{{\"context\":{},\"tasks\":[{},{}]}}",
            serde_json::to_string(context).unwrap(), first_raw, second
        );
        let end = raw.find(&first_raw).unwrap() + first_raw.len();
        (json!({"context": context, "tasks": [first, second]}), raw, end)
    }

    #[tokio::test]
    async fn partial_escapes_multiple_items_and_final_adoption() {
        let (args, raw, first_end) = input();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let dropped = Arc::new(AtomicUsize::new(0));
        let launches = dropped.clone();
        let call = Call::new(move |index, prefix| {
            let tx = tx.clone();
            let dropped = launches.clone();
            async move {
                tx.send((index, prefix)).unwrap();
                Some(Run { index, dropped })
            }
        });
        let escape = raw.find("\\\"").unwrap();
        call.update(&raw[..escape + 1]);
        assert!(rx.try_recv().is_err());
        call.update(&raw[..escape + 2]);
        assert!(rx.try_recv().is_err());
        let inside_string = raw.find("still inside").unwrap();
        call.update(&raw[..inside_string]);
        assert!(rx.try_recv().is_err());
        call.update(&raw[..first_end]);
        let (index, prefix) = rx.recv().await.unwrap();
        assert_eq!(index, 0);
        assert_eq!(prefix["context"], args["context"]);
        assert_eq!(prefix["tasks"], json!([args["tasks"][0]]));
        assert!(rx.try_recv().is_err());
        call.update(&raw);
        let (index, prefix) = rx.recv().await.unwrap();
        assert_eq!(index, 1);
        assert_eq!(prefix, args);
        let runs = call.adopt(&args).await;
        assert_eq!(runs.iter().map(|run| run.as_ref().unwrap().index).collect::<Vec<_>>(), vec![0, 1]);
        assert_eq!(dropped.load(Ordering::SeqCst), 0);
        drop(runs);
        assert_eq!(dropped.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn changed_context_or_item_discards_every_run() {
        for change_context in [true, false] {
            let (mut args, raw, first_end) = input();
            let (tx, mut rx) = mpsc::unbounded_channel();
            let dropped = Arc::new(AtomicUsize::new(0));
            let launches = dropped.clone();
            let call = Call::new(move |index, _| {
                let tx = tx.clone();
                let dropped = launches.clone();
                async move {
                    tx.send(()).unwrap();
                    Some(Run { index, dropped })
                }
            });
            call.update(&raw[..first_end]);
            rx.recv().await.unwrap();
            if change_context { args["context"] = json!("changed"); }
            else { args["tasks"][0]["task"] = json!("changed"); }
            assert!(call.adopt(&args).await.is_empty());
            assert_eq!(dropped.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn item_equivalence_normalizes_agent_but_not_assignment() {
        let implicit = json!({"task": "work", "solutionSpace": "one"});
        let explicit = json!({"task": "work", "solutionSpace": "one", "agent": "task"});
        let padded = json!({"task": "work", "solutionSpace": "one", "agent": " task "});
        let changed = json!({"task": "other", "solutionSpace": "one", "agent": "task"});
        assert!(items_match(&implicit, &explicit));
        assert!(items_match(&explicit, &padded));
        assert!(!items_match(&explicit, &changed));
    }

    #[test]
    fn malformed_escape_never_produces_an_item() {
        let bad = r#"{"context":"valid","tasks":[{"task":"bad\q","solutionSpace":"one"}"#;
        assert!(parse_prefix(bad).is_none());
    }

    #[tokio::test]
    async fn refused_preflight_discards_earlier_runs() {
        let (args, raw, _) = input();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let dropped = Arc::new(AtomicUsize::new(0));
        let launches = dropped.clone();
        let call = Call::<Run>::new(move |index, _| {
            let tx = tx.clone();
            let dropped = launches.clone();
            async move {
                tx.send(index).unwrap();
                (index == 0).then(|| Run { index, dropped })
            }
        });
        call.update(&raw);
        assert_eq!(rx.recv().await, Some(0));
        assert_eq!(rx.recv().await, Some(1));
        assert!(call.adopt(&args).await.is_empty());
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn invalid_tail_cancels_launched_runs_immediately() {
        for suffix in [r#",{"task":"bad\q"#, r#",{"task":"bad\u0g"#, r#","not an object""#] {
            let (_, raw, first_end) = input();
            let (tx, mut rx) = mpsc::unbounded_channel();
            let dropped = Arc::new(AtomicUsize::new(0));
            let launches = dropped.clone();
            let call = Call::new(move |index, _| {
                let tx = tx.clone();
                let dropped = launches.clone();
                async move {
                    tx.send(()).unwrap();
                    Some(Run { index, dropped })
                }
            });
            call.update(&raw[..first_end]);
            rx.recv().await.unwrap();
            call.update(&format!("{}{suffix}", &raw[..first_end]));
            assert_eq!(dropped.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn dropping_an_unadopted_call_cancels_its_run() {
        let (_, raw, first_end) = input();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let dropped = Arc::new(AtomicUsize::new(0));
        let launches = dropped.clone();
        let call = Call::new(move |index, _| {
            let tx = tx.clone();
            let dropped = launches.clone();
            async move {
                tx.send(()).unwrap();
                Some(Run { index, dropped })
            }
        });
        call.update(&raw[..first_end]);
        rx.recv().await.unwrap();
        drop(call);
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn discard_cancels_inflight_start_and_completed_runs() {
        struct Started(Option<oneshot::Sender<()>>);
        impl Drop for Started {
            fn drop(&mut self) { if let Some(tx) = self.0.take() { let _ = tx.send(()); } }
        }
        let (_, raw, _) = input();
        let (started_tx, mut started_rx) = mpsc::unbounded_channel();
        let (cancelled_tx, cancelled_rx) = oneshot::channel();
        let cancelled = Arc::new(Mutex::new(Some(cancelled_tx)));
        let dropped = Arc::new(AtomicUsize::new(0));
        let launches = dropped.clone();
        let call = Call::new(move |index, _| {
            let started = started_tx.clone();
            let cancelled = cancelled.clone();
            let dropped = launches.clone();
            async move {
                started.send(index).unwrap();
                if index == 1 {
                    let _guard = Started(cancelled.lock().unwrap().take());
                    std::future::pending::<()>().await;
                }
                Some(Run { index, dropped })
            }
        });
        call.update(&raw);
        assert_eq!(started_rx.recv().await, Some(0));
        assert_eq!(started_rx.recv().await, Some(1));
        call.discard();
        cancelled_rx.await.unwrap();
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cancelling_adoption_aborts_a_pending_launch() {
        struct NotifyOnDrop(Option<oneshot::Sender<()>>);
        impl Drop for NotifyOnDrop {
            fn drop(&mut self) { if let Some(tx) = self.0.take() { let _ = tx.send(()); } }
        }
        let (args, raw, first_end) = input();
        let (started_tx, started_rx) = oneshot::channel();
        let (cancelled_tx, cancelled_rx) = oneshot::channel();
        let started = Arc::new(Mutex::new(Some(started_tx)));
        let cancelled = Arc::new(Mutex::new(Some(cancelled_tx)));
        let call = Call::<Run>::new(move |_, _| {
            let started = started.clone();
            let cancelled = cancelled.clone();
            async move {
                let _guard = NotifyOnDrop(cancelled.lock().unwrap().take());
                started.lock().unwrap().take().unwrap().send(()).unwrap();
                std::future::pending::<Option<Run>>().await
            }
        });
        call.update(&raw[..first_end]);
        started_rx.await.unwrap();
        let adopting = call.clone();
        let task = tokio::spawn(async move { adopting.adopt(&args).await });
        tokio::task::yield_now().await;
        drop(call);
        task.abort();
        cancelled_rx.await.unwrap();
    }
}
