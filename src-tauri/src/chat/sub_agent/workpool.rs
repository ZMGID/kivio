use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::{json, Value};
use tauri::Manager;
use tokio::sync::watch;

use super::{eval::Context, registry::AgentRecord, SubAgentCallCtx, MAIN_AGENT_ID};
use crate::state::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum ItemStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    id: String,
    text: String,
    status: ItemStatus,
    agent_id: Option<String>,
    batch_id: Option<String>,
}

struct Worker {
    record: Option<AgentRecord>,
    queue: Vec<(usize, Context)>,
    busy: bool,
    turns: usize,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Batch {
    id: String,
    agent_id: Option<String>,
    items: Vec<String>,
    status: ItemStatus,
    output: Option<Value>,
}

#[derive(Default)]
struct PoolState {
    closed: bool,
    active: usize,
    cursor: usize,
    items: Vec<Item>,
    workers: Vec<Worker>,
    batches: Vec<Batch>,
    aggregate: Option<String>,
    aggregate_cancel: Option<watch::Receiver<bool>>,
}

impl PoolState {
    fn close(&mut self) -> Vec<String> {
        self.closed = true;
        let mut dropped = Vec::new();
        for item in &mut self.items {
            if item.status == ItemStatus::Queued {
                item.status = ItemStatus::Cancelled;
                dropped.push(item.id.clone());
            }
        }
        for worker in &mut self.workers {
            worker.queue.clear();
        }
        dropped
    }

    fn take_drained_aggregate(&mut self) -> Option<String> {
        if self
            .items
            .iter()
            .any(|item| matches!(item.status, ItemStatus::Queued | ItemStatus::Running))
        {
            return None;
        }
        self.closed = true;
        self.aggregate_cancel = None;
        self.aggregate.take()
    }
}

struct Pool {
    id: String,
    name: String,
    context: Context,
    options: Value,
    fresh: bool,
    state: Mutex<PoolState>,
    changed: watch::Sender<u64>,
}

#[derive(Default)]
pub(crate) struct Pools {
    pools: Mutex<HashMap<String, Arc<Pool>>>,
}

fn validate_origin(context: &Context) -> Result<(), String> {
    let state = context.app.state::<AppState>();
    let conversation = if context.owner == MAIN_AGENT_ID {
        context.root.clone()
    } else {
        let record = state
            .sub_agents
            .registry
            .get(&context.root, &context.owner)
            .ok_or("Originating agent ended")?;
        if record.run_id.as_deref() != Some(&context.run_id) {
            return Err("Originating agent turn ended".into());
        }
        record.child_conversation_id
    };
    if !state
        .chat_runtime()
        .is_generation_active(&conversation, context.generation)
    {
        return Err("Originating agent turn ended".into());
    }
    Ok(())
}

async fn agent(
    context: &Context,
    mut options: Value,
    items: Option<Vec<String>>,
) -> Result<Value, String> {
    validate_origin(context)?;
    let object = options
        .as_object_mut()
        .ok_or("agent() requires an object")?;
    for (source, target) in [
        ("prompt", "task"),
        ("label", "name"),
        ("schema", "outputSchema"),
    ] {
        if let Some(value) = object.remove(source) {
            if object.insert(target.into(), value).is_some() {
                return Err(format!("agent() received both {source} and {target}"));
            }
        }
    }
    let shared = object.remove("context").unwrap_or_else(|| {
        json!("Complete the delegated assignment using the current session's tools and policy.")
    });
    object.entry("solutionSpace").or_insert_with(|| {
        json!("Complete the assigned work using the current session's authorized tools.")
    });
    let arguments = json!({"context":shared,"tasks":[options]});
    let native = context.native();
    let state = context.app.state::<AppState>();
    let call = SubAgentCallCtx {
        app: &context.app,
        state: &state,
        native_ctx: &native,
        arguments: &arguments,
    };
    let pool = items.is_some();
    let result = if let Some(items) = items {
        super::task::dispatch_pool(call, items).await?
    } else {
        super::task::dispatch_eval(call).await?
    };
    let raw = result.structured_content.unwrap_or(result.raw);
    if !pool {
        let handle = raw["agents"]
            .as_array()
            .and_then(|agents| agents.first())
            .ok_or(result.content)?;
        return Ok(
            json!({"id":handle["jobId"],"agent":handle["agent"],"agentId":handle["agentId"]}),
        );
    }
    let Some(item) = raw["results"]
        .as_array()
        .and_then(|items| items.first())
        .cloned()
    else {
        return Err(result.content);
    };
    Ok(
        json!({"id":item["agentId"],"jobId":item["id"],"status":item["status"],"output":item["result"],"data":item["structuredOutput"]["data"],"structuredOutput":item["structuredOutput"],"requests":item["progress"]["requests"]}),
    )
}

impl Pools {
    pub(crate) fn forget_root(&self, root: &str) {
        let owners: std::collections::HashSet<_> = self
            .pools
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .filter(|pool| pool.context.root == root)
            .map(|pool| pool.context.owner.clone())
            .collect();
        for owner in owners {
            self.close_owner(root, &owner);
        }
    }

    fn create(&self, context: Context, mut options: Value) -> Result<Value, String> {
        let object = options
            .as_object_mut()
            .ok_or("workpool() requires an object")?;
        let name = object
            .remove("name")
            .and_then(|name| name.as_str().map(str::to_string))
            .filter(|name| !name.trim().is_empty())
            .ok_or("workpool() requires a name")?;
        if name.len() > 128 || name.chars().any(char::is_control) {
            return Err("Invalid workpool name".into());
        }
        if object.contains_key("task")
            || object.contains_key("taskFile")
            || object.contains_key("isolated")
            || object.contains_key("sourceSession")
        {
            return Err("workpool() accepts worker policy, not an initial task or isolated historical session".into());
        }
        object.entry("agent").or_insert_with(|| json!("task"));
        let state = context.app.state::<AppState>();
        if !state.settings_read().chat_tools.task_async_enabled {
            return Err("workpool() requires async jobs".into());
        }
        let fresh = state
            .settings_read()
            .chat_tools
            .task_eval_workpool_fresh_agents;
        let mut pools = self.pools.lock().unwrap_or_else(|error| error.into_inner());
        if pools.values().any(|pool| {
            pool.context.root == context.root
                && pool.context.owner == context.owner
                && pool.name == name
        }) {
            return Err(format!("Workpool {name} already exists"));
        }
        let id = uuid::Uuid::new_v4().to_string();
        pools.insert(
            id.clone(),
            Arc::new(Pool {
                id: id.clone(),
                name: name.clone(),
                context,
                options,
                fresh,
                state: Mutex::default(),
                changed: watch::channel(0).0,
            }),
        );
        Ok(json!({"id":id,"name":name}))
    }

    fn get(&self, context: &Context, id: &str) -> Result<Arc<Pool>, String> {
        self.pools
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(id)
            .filter(|pool| pool.context.root == context.root && pool.context.owner == context.owner)
            .cloned()
            .ok_or("Workpool not found for this owner".into())
    }

    pub(crate) fn close_owner(&self, root: &str, owner: &str) {
        let pools: Vec<_> = {
            let mut pools = self.pools.lock().unwrap_or_else(|error| error.into_inner());
            let ids: Vec<_> = pools
                .iter()
                .filter(|(_, pool)| pool.context.root == root && pool.context.owner == owner)
                .map(|(id, _)| id.clone())
                .collect();
            ids.into_iter().filter_map(|id| pools.remove(&id)).collect()
        };
        for pool in pools {
            let aggregate = pool
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .aggregate
                .clone();
            if let Some(job) = aggregate {
                pool.context
                    .app
                    .state::<AppState>()
                    .sub_agents
                    .jobs
                    .cancel(root, &job, owner);
            }
            pool.close();
        }
    }
}

impl Pool {
    fn limit(&self) -> usize {
        let limit = self
            .context
            .app
            .state::<AppState>()
            .settings_read()
            .chat_tools
            .task_max_concurrency;
        if limit == 0 {
            usize::MAX
        } else {
            limit
        }
    }

    fn push(self: &Arc<Self>, context: &Context, values: &Value) -> Result<Value, String> {
        let texts = values
            .as_array()
            .ok_or("pool.push requires a list of strings")?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .filter(|text| !text.trim().is_empty())
                    .map(str::to_string)
                    .ok_or("Pool items must be non-empty strings")
            })
            .collect::<Result<Vec<_>, _>>()?;
        let limit = self.limit();
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.closed {
            return Err(format!("Workpool {} is closed", self.name));
        }
        let mut ids = Vec::new();
        for text in texts {
            let index = state.items.len();
            let id = format!("{}#{}", self.name, index + 1);
            state.items.push(Item {
                id: id.clone(),
                text,
                status: ItemStatus::Queued,
                agent_id: None,
                batch_id: None,
            });
            let worker = if self.fresh {
                None
            } else {
                state
                    .workers
                    .iter()
                    .position(|worker| !worker.busy && worker.queue.is_empty())
            };
            let worker = worker.unwrap_or_else(|| {
                if self.fresh || state.workers.len() < limit {
                    let index = state.workers.len();
                    state.workers.push(Worker {
                        record: None,
                        queue: Vec::new(),
                        busy: false,
                        turns: 0,
                    });
                    index
                } else {
                    let index = state.cursor % state.workers.len();
                    state.cursor += 1;
                    index
                }
            });
            // Later eval turns cannot reauthorize work queued by an earlier caller.
            state.workers[worker].queue.push((index, context.clone()));
            ids.push(id);
        }
        let first_push = !ids.is_empty() && state.aggregate.is_none();
        if first_push {
            let app_state = self.context.app.state::<AppState>();
            let (job, cancelled) = app_state.sub_agents.jobs.register(
                &self.context.root,
                &self.context.owner,
                &format!("pool:{}", self.name),
                &format!("Workpool {}", self.name),
            );
            state.aggregate = Some(job);
            state.aggregate_cancel = Some(cancelled);
        }
        drop(state);
        self.changed
            .send_modify(|value| *value = value.wrapping_add(1));
        self.schedule();
        if first_push {
            self.monitor_cancel();
        }
        Ok(json!(ids))
    }

    fn monitor_cancel(self: &Arc<Self>) {
        let pool = Arc::downgrade(self);
        let mut changed = self.changed.subscribe();
        let Some(mut cancelled) = self
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .aggregate_cancel
            .clone()
        else {
            return;
        };
        tokio::spawn(async move {
            loop {
                {
                    let Some(pool) = pool.upgrade() else {
                        return;
                    };
                    if pool
                        .state
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .aggregate
                        .is_none()
                    {
                        return;
                    }
                    if *cancelled.borrow() {
                        pool.close();
                        return;
                    }
                }
                tokio::select! {
                    result = cancelled.changed() => if result.is_err() { return; },
                    result = changed.changed() => if result.is_err() { return; },
                }
            }
        });
    }

    fn schedule(self: &Arc<Self>) {
        loop {
            let next = {
                let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
                if state.active >= self.limit() {
                    return;
                }
                let Some(worker_index) = state
                    .workers
                    .iter()
                    .position(|worker| !worker.busy && !worker.queue.is_empty())
                else {
                    return;
                };
                let worker = &mut state.workers[worker_index];
                worker.busy = true;
                worker.turns += 1;
                let context = worker.queue[0].1.clone();
                let count = worker
                    .queue
                    .iter()
                    .take_while(|(_, queued)| {
                        queued.run_id == context.run_id
                            && queued.generation == context.generation
                            && queued.tool_call_id == context.tool_call_id
                    })
                    .count();
                let indices = worker
                    .queue
                    .drain(..count)
                    .map(|(index, _)| index)
                    .collect::<Vec<_>>();
                let record = worker.record.clone();
                let batch_id = format!("{}-{}-b{}", self.name, worker_index + 1, worker.turns);
                let ids = indices
                    .iter()
                    .map(|index| state.items[*index].id.clone())
                    .collect::<Vec<_>>();
                for index in &indices {
                    state.items[*index].status = ItemStatus::Running;
                    state.items[*index].batch_id = Some(batch_id.clone());
                }
                let batch_index = state.batches.len();
                state.batches.push(Batch {
                    id: batch_id,
                    agent_id: record.as_ref().map(|record| record.id.clone()),
                    items: ids,
                    status: ItemStatus::Running,
                    output: None,
                });
                state.active += 1;
                Some((worker_index, indices, record, batch_index, context))
            };
            let Some((worker_index, indices, record, batch_index, context)) = next else {
                return;
            };
            let pool = self.clone();
            tokio::spawn(async move {
                pool.drive(worker_index, indices, record, batch_index, context)
                    .await;
            });
        }
    }

    async fn drive(
        self: Arc<Self>,
        worker_index: usize,
        indices: Vec<usize>,
        record: Option<AgentRecord>,
        batch_index: usize,
        context: Context,
    ) {
        let (task, schema, ids) = {
            let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            let properties = indices
                .iter()
                .map(|index| (state.items[*index].id.clone(), json!({})))
                .collect::<serde_json::Map<_, _>>();
            let ids = indices
                .iter()
                .map(|index| state.items[*index].id.clone())
                .collect::<Vec<_>>();
            let body = indices
                .iter()
                .enumerate()
                .map(|(key, index)| {
                    format!(
                        "Item {} [{}]\n{}",
                        key + 1,
                        state.items[*index].id,
                        state.items[*index].text
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            (format!("Workpool {}. Process every item below. Submit each item once with yield using its batch-local numeric key and either data or error, for example {{\"key\":1,\"data\":{{\"result\":\"done\"}}}}. The last item ends this turn automatically; do not submit an extra terminal yield.\n\n{body}", self.name), json!({"type":"object","properties":properties,"required":ids,"additionalProperties":false}), ids)
        };
        let completion = super::spawn::CompletionPolicy::Yield {
            schema: Some(schema.clone()),
            strict: true,
        };
        let execution = async {
            validate_origin(&context)?;
            if let Some(record) = record {
                super::executor::run_inline_followup(&context.app, &record, task, completion, ids).await
                    .map(|job| (record, json!({"status":job.status,"output":job.result,"data":job.structured_output.as_ref().and_then(|output| output.get("data")),"structuredOutput":job.structured_output})))
            } else {
                let mut options = self.options.clone();
                options["task"] = json!(task);
                options["outputSchema"] = schema;
                options["schemaMode"] = json!("strict");
                options["name"] = json!(format!("{}-{}", self.name, worker_index + 1));
                agent(&context, options, Some(ids))
                    .await
                    .and_then(|result| {
                        let id = result["id"]
                            .as_str()
                            .ok_or("Pool worker did not return an identity")?;
                        context
                            .app
                            .state::<AppState>()
                            .sub_agents
                            .registry
                            .get(&context.root, id)
                            .map(|record| (record, result))
                            .ok_or("Pool worker is no longer registered".into())
                    })
            }
        };
        let mut cancelled = self
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .aggregate_cancel
            .clone()
            .expect("Running pool has an aggregate cancellation receiver");
        let outcome = tokio::select! {
            biased;
            _ = wait_cancelled(&mut cancelled) => None,
            outcome = execution => Some(outcome),
        };
        let aggregate = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            let (record, result, status) = match outcome {
                Some(Ok((record, result))) => {
                    let status = if result["status"] == "completed" {
                        ItemStatus::Completed
                    } else if result["status"] == "cancelled" {
                        ItemStatus::Cancelled
                    } else {
                        ItemStatus::Failed
                    };
                    (Some(record), result, status)
                }
                Some(Err(error)) => (None, json!({"error":error}), ItemStatus::Failed),
                None => (None, json!({"status":"cancelled"}), ItemStatus::Cancelled),
            };
            let agent_id = record.as_ref().map(|record| record.id.clone());
            for index in indices {
                state.items[index].status = item_status(status, &result, &state.items[index].id);
                state.items[index].agent_id = agent_id.clone();
            }
            state.batches[batch_index].status = status;
            state.batches[batch_index].agent_id = agent_id;
            state.batches[batch_index].output = Some(result);
            state.workers[worker_index].record = record;
            state.workers[worker_index].busy = false;
            state.active = state.active.saturating_sub(1);
            state.take_drained_aggregate()
        };
        self.changed
            .send_modify(|value| *value = value.wrapping_add(1));
        self.settle_aggregate(aggregate);
        self.schedule();
    }

    fn peek(&self) -> Value {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        json!({"name":self.name,"batches":state.batches,"pending":state.items.iter().filter(|item| matches!(item.status,ItemStatus::Queued|ItemStatus::Running)).count()})
    }

    fn status(&self) -> Value {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        json!({"id":self.id,"name":self.name,"closed":state.closed,"limit":json_limit(self.limit()),"freshAgents":self.fresh,"items":state.items,"batches":state.batches.len(),"agents":state.workers.iter().map(|worker| json!({"id":worker.record.as_ref().map(|record| &record.id),"state":if worker.busy {"running"} else {"idle"},"queued":worker.queue.len(),"turns":worker.turns})).collect::<Vec<_>>()})
    }

    fn close(&self) -> Value {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let dropped = state.close();
        let aggregate = state.take_drained_aggregate();
        drop(state);
        self.changed
            .send_modify(|value| *value = value.wrapping_add(1));
        self.settle_aggregate(aggregate);
        json!({"dropped":dropped})
    }

    fn settle_aggregate(&self, job: Option<String>) {
        if let Some(job) = job {
            self.context
                .app
                .state::<AppState>()
                .sub_agents
                .jobs
                .settle(&job, Ok(self.peek().to_string()));
        }
    }

    async fn drain(&self) -> Result<Value, String> {
        let mut changed = self.changed.subscribe();
        loop {
            if self.peek()["pending"] == 0 {
                return Ok(self.peek());
            }
            changed
                .changed()
                .await
                .map_err(|_| "Workpool ended while waiting")?;
        }
    }
}

fn json_limit(limit: usize) -> Value {
    if limit == usize::MAX {
        Value::Null
    } else {
        json!(limit)
    }
}

fn item_status(batch: ItemStatus, result: &Value, id: &str) -> ItemStatus {
    if batch == ItemStatus::Completed && result["data"][id]["error"].is_string() {
        ItemStatus::Failed
    } else {
        batch
    }
}

async fn wait_cancelled(cancelled: &mut watch::Receiver<bool>) {
    let closed = cancelled.wait_for(|value| *value).await.is_err();
    if closed {
        std::future::pending::<()>().await;
    }
}

pub(crate) async fn handle_host(
    context: Context,
    method: &str,
    arguments: Value,
) -> Result<Value, String> {
    validate_origin(&context)?;
    if method == "agent" {
        return agent(&context, arguments, None).await;
    }
    let state = context.app.state::<AppState>();
    if method == "pool.create" {
        return state.sub_agents.pools.create(context.clone(), arguments);
    }
    let id = arguments["id"].as_str().ok_or("Missing pool id")?;
    let pool = state.sub_agents.pools.get(&context, id)?;
    match method {
        "pool.push" => pool.push(&context, &arguments["items"]),
        "pool.peek" => Ok(pool.peek()),
        "pool.status" => Ok(pool.status()),
        "pool.close" => Ok(pool.close()),
        "pool.drain" => pool.drain().await,
        _ => Err("Unknown eval host operation".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::FutureExt;

    fn item(id: &str, status: ItemStatus) -> Item {
        Item {
            id: id.into(),
            text: id.into(),
            status,
            agent_id: None,
            batch_id: None,
        }
    }

    #[test]
    fn closing_a_queued_only_pool_settles_its_aggregate_once() {
        let (_cancel, cancelled) = watch::channel(false);
        let mut state = PoolState {
            items: vec![item("pool#1", ItemStatus::Queued)],
            aggregate: Some("job-1".into()),
            aggregate_cancel: Some(cancelled),
            ..Default::default()
        };
        assert_eq!(state.close(), vec!["pool#1"]);
        assert_eq!(state.items[0].status, ItemStatus::Cancelled);
        assert_eq!(state.take_drained_aggregate().as_deref(), Some("job-1"));
        assert!(state.aggregate_cancel.is_none());
        assert!(state.take_drained_aggregate().is_none());
    }

    #[test]
    fn closing_preserves_active_work_until_it_finishes() {
        let (_cancel, cancelled) = watch::channel(false);
        let mut state = PoolState {
            active: 1,
            items: vec![
                item("pool#1", ItemStatus::Running),
                item("pool#2", ItemStatus::Queued),
            ],
            aggregate: Some("job-1".into()),
            aggregate_cancel: Some(cancelled),
            ..Default::default()
        };
        assert_eq!(state.close(), vec!["pool#2"]);
        assert_eq!(state.items[0].status, ItemStatus::Running);
        assert!(state.take_drained_aggregate().is_none());
        assert!(state.aggregate_cancel.is_some());
        state.items[0].status = ItemStatus::Completed;
        state.active = 0;
        assert_eq!(state.take_drained_aggregate().as_deref(), Some("job-1"));
        assert!(state.aggregate_cancel.is_none());
    }

    #[test]
    fn unbounded_concurrency_uses_json_null() {
        assert_eq!(json_limit(usize::MAX).to_string(), "null");
        assert_eq!(json_limit(3), json!(3));
    }

    #[test]
    fn one_item_error_does_not_fail_other_items_in_the_batch() {
        let result = json!({"data":{"pool#1":{"error":"bad input"},"pool#2":{"value":42},"pool#3":{"error":false}}});
        assert_eq!(
            item_status(ItemStatus::Completed, &result, "pool#1"),
            ItemStatus::Failed
        );
        assert_eq!(
            item_status(ItemStatus::Completed, &result, "pool#2"),
            ItemStatus::Completed
        );
        assert_eq!(
            item_status(ItemStatus::Completed, &result, "pool#3"),
            ItemStatus::Completed
        );
        assert_eq!(
            item_status(ItemStatus::Cancelled, &result, "pool#2"),
            ItemStatus::Cancelled
        );
    }

    #[test]
    fn active_batches_observe_only_explicit_cancellation() {
        let (cancel, mut cancelled) = watch::channel(false);
        assert!(wait_cancelled(&mut cancelled).now_or_never().is_none());
        cancel.send_replace(true);
        assert!(wait_cancelled(&mut cancelled).now_or_never().is_some());
        let (cancel, mut cancelled) = watch::channel(false);
        drop(cancel);
        assert!(wait_cancelled(&mut cancelled).now_or_never().is_none());
    }
}
