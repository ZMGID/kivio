use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, Weak,
};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::AppHandle;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::oneshot;

use super::SubAgentCallCtx;
use crate::mcp::{types::McpToolCallResult, ChatToolDefinition};

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Language {
    Js,
    Py,
}

#[derive(Clone)]
pub(crate) struct Context {
    pub app: AppHandle,
    pub root: String,
    pub owner: String,
    pub run_id: String,
    pub generation: u64,
    pub depth: u8,
    pub provider: crate::settings::ModelProvider,
    pub model: String,
    pub plan: crate::chat::types::AgentPlanState,
    pub mcp_tool_ids: Vec<(String, String)>,
    pub mcp_server_ids: Vec<String>,
    pub approval_policy: String,
    pub interactive_host: bool,
    pub tool_call_id: String,
    pub cwd: PathBuf,
}

impl Context {
    pub(crate) fn native(&self) -> crate::mcp::registry::NativeToolContext<'_> {
        crate::mcp::registry::NativeToolContext {
            conversation_id: self.root.clone(),
            message_id: format!("eval-{}", self.tool_call_id),
            provider: &self.provider,
            model: &self.model,
            tool_call_id: Some(self.tool_call_id.clone()),
            run_id: self.run_id.clone(),
            generation: self.generation,
            round: 0,
            depth: self.depth,
            plan_state: &self.plan,
            mcp_tool_ids: &self.mcp_tool_ids,
            mcp_server_ids: &self.mcp_server_ids,
            approval_policy: &self.approval_policy,
            interactive_host: self.interactive_host,
        }
    }
}

struct Pending {
    reply: oneshot::Sender<Result<Value, String>>,
    context: Option<Context>,
    output: String,
    host_calls: usize,
    result: Option<Result<Value, String>>,
}

struct PendingRequest<'a> {
    kernel: &'a Kernel,
    id: String,
}
impl Drop for PendingRequest<'_> {
    fn drop(&mut self) {
        self.kernel
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.id);
    }
}

struct EvalOperation {
    kernel: Arc<Kernel>,
    completed: bool,
}
impl Drop for EvalOperation {
    fn drop(&mut self) {
        if !self.completed {
            let kernel = self.kernel.clone();
            tauri::async_runtime::spawn(async move {
                kernel.stop().await;
            });
        }
    }
}

pub(crate) struct Kernel {
    pub(crate) id: String,
    language: Language,
    cwd: PathBuf,
    stdin: tokio::sync::Mutex<ChildStdin>,
    child: tokio::sync::Mutex<Box<dyn process_wrap::tokio::ChildWrapper>>,
    pending: Mutex<HashMap<String, Pending>>,
    alive: AtomicBool,
    stopped: tokio::sync::watch::Sender<bool>,
}

#[derive(Default, Clone)]
pub(crate) struct Kernels {
    kernels: Arc<tokio::sync::Mutex<HashMap<(String, String, Language), Arc<Kernel>>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Descriptor {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub language: Language,
}

pub(crate) struct SharedTool {
    pub definition: ChatToolDefinition,
    pub descriptor: Descriptor,
    pub kernel: Weak<Kernel>,
    pub kernel_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SavedTool {
    pub descriptor: Descriptor,
    pub kernel_id: String,
}

impl SharedTool {
    pub(crate) fn saved(&self) -> SavedTool {
        SavedTool {
            descriptor: self.descriptor.clone(),
            kernel_id: self.kernel_id.clone(),
        }
    }
}

impl Clone for SharedTool {
    fn clone(&self) -> Self {
        Self {
            definition: self.definition.clone(),
            descriptor: self.descriptor.clone(),
            kernel: self.kernel.clone(),
            kernel_id: self.kernel_id.clone(),
        }
    }
}

fn command(language: Language) -> Command {
    let mut command = match language {
        Language::Js => {
            let mut command = Command::new("node");
            command.args(["-e", include_str!("eval-kernel.cjs")]);
            command
        }
        Language::Py => {
            let mut command = Command::new(if cfg!(windows) { "py" } else { "python3" });
            command.args(["-u", "-c", include_str!("eval-kernel.py")]);
            command
        }
    };
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000 | 0x00000200);
    #[cfg(unix)]
    command.process_group(0);
    command
}

async fn bounded_line(
    reader: &mut BufReader<tokio::process::ChildStdout>,
) -> Result<Option<Vec<u8>>, String> {
    let mut line = Vec::new();
    loop {
        let buffer = reader.fill_buf().await.map_err(|error| error.to_string())?;
        if buffer.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Err("Eval kernel ended during a protocol message".into())
            };
        }
        let count = buffer
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| index + 1)
            .unwrap_or(buffer.len());
        if line.len() + count > 8 * 1024 * 1024 {
            return Err("Eval kernel protocol message exceeds 8 MiB".into());
        }
        line.extend_from_slice(&buffer[..count]);
        reader.consume(count);
        if line.ends_with(b"\n") {
            return Ok(Some(line));
        }
    }
}

impl Kernel {
    fn complete_if_ready(&self, id: &str) {
        let mut calls = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !calls
            .get(id)
            .is_some_and(|call| call.host_calls == 0 && call.result.is_some())
        {
            return;
        }
        let mut call = calls.remove(id).expect("Ready call exists");
        let result = call
            .result
            .take()
            .expect("Ready call has a result")
            .map(|value| json!({"value":value,"output":call.output}));
        let _ = call.reply.send(result);
    }
    async fn start(language: Language, cwd: PathBuf) -> Result<Arc<Self>, String> {
        let mut process = command(language);
        process.current_dir(&cwd);
        let mut wrapped = process_wrap::tokio::CommandWrap::from(process);
        #[cfg(windows)]
        {
            let mut flags = process_wrap::tokio::CreationFlags(Default::default());
            flags.0 .0 = crate::proc::CREATE_NO_WINDOW;
            wrapped.wrap(flags).wrap(process_wrap::tokio::JobObject);
        }
        #[cfg(unix)]
        wrapped.wrap(process_wrap::tokio::ProcessGroup::leader());
        wrapped.wrap(process_wrap::tokio::KillOnDrop);
        let mut child = wrapped.spawn().map_err(|error| {
            format!("Cannot start {language:?} eval kernel. Install its runtime first: {error}")
        })?;
        let stdin = child
            .stdin()
            .take()
            .ok_or("Eval kernel input unavailable")?;
        let stdout = child
            .stdout()
            .take()
            .ok_or("Eval kernel output unavailable")?;
        let kernel = Arc::new(Self {
            id: uuid::Uuid::new_v4().to_string(),
            language,
            cwd,
            stdin: tokio::sync::Mutex::new(stdin),
            child: tokio::sync::Mutex::new(child),
            pending: Mutex::default(),
            alive: AtomicBool::new(true),
            stopped: tokio::sync::watch::channel(false).0,
        });
        let weak = Arc::downgrade(&kernel);
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            loop {
                let message = match bounded_line(&mut reader).await {
                    Ok(Some(line)) => serde_json::from_slice::<Value>(&line)
                        .map_err(|error| format!("Invalid eval kernel response: {error}")),
                    Ok(None) => Err("Eval kernel exited".into()),
                    Err(error) => Err(error),
                };
                let Some(kernel) = weak.upgrade() else {
                    break;
                };
                match message {
                    Ok(message) => kernel.receive(message),
                    Err(error) => {
                        kernel.fail(&error);
                        kernel.stop().await;
                        break;
                    }
                }
            }
        });
        Ok(kernel)
    }

    fn fail(&self, error: &str) {
        self.alive.store(false, Ordering::Release);
        self.stopped.send_replace(true);
        let pending = std::mem::take(
            &mut *self
                .pending
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        );
        for (_, call) in pending {
            let _ = call.reply.send(Err(error.to_string()));
        }
    }

    pub(crate) async fn stop(&self) {
        self.fail("The eval kernel defining this tool has ended");
        let mut child = self.child.lock().await;
        if let Some(pid) = child.id() {
            crate::native_tools::kill_process_group(pid);
        }
        let _ = Box::into_pin(child.kill()).await;
        let _ = child.wait().await;
    }

    async fn write(&self, value: &Value) -> Result<(), String> {
        let mut bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
        bytes.push(b'\n');
        self.stdin
            .lock()
            .await
            .write_all(&bytes)
            .await
            .map_err(|error| error.to_string())
    }

    fn receive(self: &Arc<Self>, message: Value) {
        match message["type"].as_str() {
            Some("result") => {
                let Some(id) = message["id"].as_str() else {
                    return;
                };
                {
                    let mut calls = self
                        .pending
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    if let Some(call) = calls.get_mut(id) {
                        let result = if message["ok"] == true {
                            Ok(message["value"].clone())
                        } else {
                            Err(message["error"]
                                .as_str()
                                .unwrap_or("Eval failed")
                                .to_string())
                        };
                        call.result = Some(result);
                    }
                }
                self.complete_if_ready(id);
            }
            Some("output") => {
                let mut calls = self
                    .pending
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if let Some(call) = message["cell"].as_str().and_then(|id| calls.get_mut(id)) {
                    if let Some(text) = message["text"].as_str() {
                        let remaining = (64 * 1024usize).saturating_sub(call.output.len());
                        let end = text
                            .char_indices()
                            .map(|(index, _)| index)
                            .take_while(|index| *index <= remaining)
                            .last()
                            .unwrap_or(0);
                        call.output.push_str(if text.len() <= remaining {
                            text
                        } else {
                            &text[..end]
                        });
                    }
                }
            }
            Some("host") => {
                let cell = message["cell"].as_str().map(str::to_string);
                let context = {
                    let mut calls = self
                        .pending
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    cell.as_ref()
                        .and_then(|id| calls.get_mut(id))
                        .and_then(|call| {
                            call.host_calls += 1;
                            call.context.clone()
                        })
                };
                let kernel = self.clone();
                tokio::spawn(async move {
                    let mut stopped = kernel.stopped.subscribe();
                    let result = match context {
                        Some(context) if !*stopped.borrow() => tokio::select! {
                            result = super::workpool::handle_host(context, message["method"].as_str().unwrap_or_default(), message["args"].clone()) => result,
                            _ = stopped.changed() => Err("The originating eval kernel ended".into()),
                        },
                        _ => Err("The originating eval cell has ended".into()),
                    };
                    let response = match result {
                        Ok(value) => {
                            json!({"type":"host-result","id":message["id"],"ok":true,"value":value})
                        }
                        Err(error) => {
                            json!({"type":"host-result","id":message["id"],"ok":false,"error":error})
                        }
                    };
                    let _ = kernel.write(&response).await;
                    if let Some(cell) = cell {
                        if let Some(call) = kernel
                            .pending
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .get_mut(&cell)
                        {
                            call.host_calls = call.host_calls.saturating_sub(1);
                        }
                        kernel.complete_if_ready(&cell);
                    }
                });
            }
            _ => self.fail("Unknown eval kernel response"),
        }
    }

    async fn request(
        &self,
        mut request: Value,
        context: Option<Context>,
        timeout: Duration,
    ) -> Result<Value, String> {
        if !self.alive.load(Ordering::Acquire) {
            return Err("The eval kernel defining this tool has ended".into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        request["id"] = json!(id);
        let (reply, receive) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(
                id.clone(),
                Pending {
                    reply,
                    context,
                    output: String::new(),
                    host_calls: 0,
                    result: None,
                },
            );
        let _pending = PendingRequest {
            kernel: self,
            id: id.clone(),
        };
        if let Err(error) = self.write(&request).await {
            self.pending
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .remove(&id);
            return Err(error);
        }
        let result = tokio::time::timeout(timeout, receive).await;
        self.pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&id);
        match result {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("Eval kernel response was interrupted".into()),
            Err(_) => {
                self.stop().await;
                Err("Eval cell timed out; its kernel was stopped".into())
            }
        }
    }
}

impl Kernels {
    pub(crate) fn forget_root(&self, root: &str) {
        let manager = self.clone();
        let root = root.to_string();
        tauri::async_runtime::spawn(async move {
            let removed = {
                let mut kernels = manager.kernels.lock().await;
                let keys: Vec<_> = kernels
                    .keys()
                    .filter(|(r, _, _)| r == &root)
                    .cloned()
                    .collect();
                keys.into_iter()
                    .filter_map(|key| kernels.remove(&key))
                    .collect::<Vec<_>>()
            };
            for kernel in removed {
                kernel.stop().await;
            }
        });
    }
    async fn get(
        &self,
        context: &Context,
        language: Language,
        reset: bool,
    ) -> Result<Arc<Kernel>, String> {
        let key = (context.root.clone(), context.owner.clone(), language);
        let mut kernels = self.kernels.lock().await;
        if reset {
            if let Some(old) = kernels.remove(&key) {
                old.stop().await;
            }
        }
        if let Some(kernel) = kernels
            .get(&key)
            .filter(|kernel| kernel.alive.load(Ordering::Acquire))
        {
            if kernel.cwd != context.cwd {
                return Err(
                    "Eval working directory changed. Reset this kernel before continuing.".into(),
                );
            }
            return Ok(kernel.clone());
        }
        let kernel = Kernel::start(language, context.cwd.clone()).await?;
        kernels.insert(key, kernel.clone());
        Ok(kernel)
    }

    pub(crate) async fn close_owner(&self, root: &str, owner: &str) {
        let removed = {
            let mut kernels = self.kernels.lock().await;
            let keys: Vec<_> = kernels
                .keys()
                .filter(|(r, o, _)| r == root && o == owner)
                .cloned()
                .collect();
            keys.into_iter()
                .filter_map(|key| kernels.remove(&key))
                .collect::<Vec<_>>()
        };
        for kernel in removed {
            kernel.stop().await;
        }
    }

    pub(crate) async fn shared(
        &self,
        root: &str,
        owner: &str,
        names: &[String],
    ) -> Result<Vec<SharedTool>, String> {
        if names.is_empty() {
            return Ok(Vec::new());
        }
        let kernels: Vec<_> = self
            .kernels
            .lock()
            .await
            .iter()
            .filter(|((r, o, _), kernel)| {
                r == root && o == owner && kernel.alive.load(Ordering::Acquire)
            })
            .map(|(_, kernel)| kernel.clone())
            .collect();
        let mut found = HashMap::new();
        for kernel in kernels {
            let response = kernel
                .request(
                    json!({"op":"describe","names":names}),
                    None,
                    Duration::from_secs(10),
                )
                .await?;
            let descriptors: Vec<Descriptor> = serde_json::from_value(response["value"].clone())
                .map_err(|error| format!("Invalid eval tool descriptors: {error}"))?;
            for descriptor in descriptors {
                if found.contains_key(&descriptor.name) {
                    return Err(format!(
                        "Eval tool {} is defined in both kernels",
                        descriptor.name
                    ));
                }
                if descriptor.name.is_empty()
                    || !descriptor
                        .name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
                {
                    return Err("Invalid eval tool name".into());
                }
                super::structured::validate_schema(&descriptor.parameters)?;
                let definition = ChatToolDefinition {
                    id: format!("eval__{}", descriptor.name),
                    name: descriptor.name.clone(),
                    description: descriptor.description.clone(),
                    source: "eval".into(),
                    server_id: None,
                    server_name: Some("Eval".into()),
                    input_schema: descriptor.parameters.clone(),
                    sensitive: true,
                    annotations: None,
                    output_schema: None,
                };
                found.insert(
                    descriptor.name.clone(),
                    SharedTool {
                        definition,
                        descriptor,
                        kernel: Arc::downgrade(&kernel),
                        kernel_id: kernel.id.clone(),
                    },
                );
            }
        }
        names
            .iter()
            .map(|name| {
                found.remove(name).ok_or_else(|| {
                    format!("Unknown eval tool {name}. Define it in an eval cell first.")
                })
            })
            .collect()
    }
}

pub(crate) async fn invoke(
    tool: &SharedTool,
    mut arguments: Value,
) -> Result<McpToolCallResult, String> {
    let kernel = tool
        .kernel
        .upgrade()
        .ok_or("The eval kernel defining this tool has ended")?;
    if tool.descriptor.parameters["properties"].get("i").is_none() {
        if let Some(object) = arguments.as_object_mut() {
            object.remove("i");
        }
    }
    super::structured::validate_result(&tool.descriptor.parameters, &arguments)?;
    let result = kernel
        .request(
            json!({"op":"invoke","name":tool.descriptor.name,"arguments":arguments}),
            None,
            Duration::from_secs(120),
        )
        .await?;
    let value = &result["value"];
    Ok(McpToolCallResult {
        content: value
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| serde_json::to_string_pretty(value).unwrap_or_default()),
        raw: value.clone(),
        ..Default::default()
    })
}

pub fn definition() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__eval".into(), name: "eval".into(), source: "native".into(), server_id: None, server_name: Some("Kivio".into()),
        description: "Execute js or py in a persistent kernel. Variables survive calls. Define shareable tools with tool(fn,{name,description,parameters}) in JS or @tool(parameters=...) in Python. Use await agent({prompt,agent?,label?,schema?,schemaMode?,tools?}) to register a background structured subagent and return its job id immediately; collect it with wait. Use await workpool({name,agent?,context?}) then await pool.push(items), status(), peek(), drain(), close(). Kernels require installed Node.js or Python. reset=true ends tools defined by the old kernel.".into(),
        input_schema: json!({"type":"object","properties":{"language":{"enum":["js","py"]},"code":{"type":"string"},"title":{"type":"string"},"timeout":{"type":"number","minimum":0},"reset":{"type":"boolean"}},"required":["language","code"],"additionalProperties":false}),
        sensitive: true, annotations: None, output_schema: None,
    }
}

pub fn dispatch(ctx: SubAgentCallCtx<'_>) -> crate::mcp::native_registry::NativeToolFuture<'_> {
    Box::pin(async move {
        let language: Language = serde_json::from_value(ctx.arguments["language"].clone())
            .map_err(|_| "Eval language must be js or py")?;
        let code = ctx.arguments["code"]
            .as_str()
            .filter(|code| !code.trim().is_empty())
            .ok_or("Eval requires code")?;
        if code.len() > 1024 * 1024 {
            return Err("Eval cell exceeds 1 MiB".into());
        }
        let root = &ctx.native_ctx.conversation_id;
        let owner = ctx.state.sub_agents.owner_for_run(&ctx.native_ctx.run_id);
        if ctx.native_ctx.depth > 0 && owner == super::MAIN_AGENT_ID {
            return Err("Eval owner is no longer registered".into());
        }
        let settings = ctx.state.settings_read().clone();
        let cwd = super::lifecycle::cwd_for_run(&ctx.state.sub_agents, &ctx.native_ctx.run_id)
            .or_else(|| {
                crate::chat::storage::load_conversation(ctx.app, root)
                    .ok()
                    .and_then(|conversation| {
                        crate::chat::storage::resolve_conversation_working_directory(
                            ctx.app,
                            &conversation,
                            &settings.chat_tools.native_tools.working_directory,
                        )
                        .ok()
                    })
            })
            .ok_or("Cannot resolve eval working directory")?;
        let context = Context {
            app: ctx.app.clone(),
            root: root.clone(),
            owner,
            run_id: ctx.native_ctx.run_id.clone(),
            generation: ctx.native_ctx.generation,
            depth: ctx.native_ctx.depth,
            provider: ctx.native_ctx.provider.clone(),
            model: ctx.native_ctx.model.into(),
            plan: ctx.native_ctx.plan_state.clone(),
            mcp_tool_ids: ctx.native_ctx.mcp_tool_ids.to_vec(),
            mcp_server_ids: ctx.native_ctx.mcp_server_ids.to_vec(),
            approval_policy: ctx.native_ctx.approval_policy.into(),
            interactive_host: ctx.native_ctx.interactive_host,
            tool_call_id: ctx
                .native_ctx
                .tool_call_id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            cwd,
        };
        let kernel = ctx
            .state
            .sub_agents
            .eval
            .get(&context, language, ctx.arguments["reset"] == true)
            .await?;
        let mut operation = EvalOperation {
            kernel: kernel.clone(),
            completed: false,
        };
        let seconds = ctx.arguments["timeout"].as_f64().unwrap_or(120.0);
        if !seconds.is_finite() || seconds < 0.0 {
            return Err("Invalid eval timeout".into());
        }
        let timeout = if seconds == 0.0 {
            Duration::from_secs(365 * 24 * 3600)
        } else {
            Duration::from_secs_f64(seconds.clamp(1.0, 3600.0))
        };
        let runtime_conversation = ctx
            .state
            .sub_agents
            .registry
            .find_by_run(&ctx.native_ctx.run_id)
            .and_then(|(root, id)| ctx.state.sub_agents.registry.get(&root, &id))
            .map_or_else(|| root.clone(), |record| record.child_conversation_id);
        let execution = kernel.request(json!({"op":"eval","code":code}), Some(context), timeout);
        let response = tokio::select! {
            biased;
            _ = ctx.state.chat_runtime().wait_for_generation_inactive(&runtime_conversation, ctx.native_ctx.generation) => { kernel.stop().await; return Err("cancelled".into()); },
            response = execution => response,
        };
        operation.completed = true;
        let response = response?;
        let output = response["output"].as_str().unwrap_or_default();
        let value = &response["value"];
        let text = if value.is_null() {
            output.to_string()
        } else {
            format!(
                "{output}\n{}",
                value
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| serde_json::to_string_pretty(value).unwrap_or_default())
            )
        };
        Ok(McpToolCallResult {
            content: text.trim().to_string(),
            raw: response,
            ..Default::default()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires installed Node.js and Python runtimes"]
    async fn real_kernels_persist_state_and_fail_closed_after_stop() {
        let cwd = tempfile::tempdir().unwrap();
        for (language, first, second) in [
            (Language::Js, "var value = 40; value", "value += 2; value"),
            (Language::Py, "value = 40\nvalue", "value += 2\nvalue"),
        ] {
            let kernel = Kernel::start(language, cwd.path().into()).await.unwrap();
            let timeout = Duration::from_secs(10);
            assert_eq!(
                kernel
                    .request(json!({"op":"eval","code":first}), None, timeout)
                    .await
                    .unwrap()["value"],
                40
            );
            assert_eq!(
                kernel
                    .request(json!({"op":"eval","code":second}), None, timeout)
                    .await
                    .unwrap()["value"],
                42
            );
            assert!(kernel
                .request(json!({"op":"invoke","name":"missing"}), None, timeout)
                .await
                .is_err());
            kernel.stop().await;
            assert!(kernel
                .request(json!({"op":"eval","code":"1"}), None, timeout)
                .await
                .unwrap_err()
                .contains("ended"));
        }
    }

    #[tokio::test]
    #[ignore = "requires installed Node.js runtime"]
    async fn real_kernel_timeout_terminates_its_tool_authority() {
        let cwd = tempfile::tempdir().unwrap();
        let kernel = Kernel::start(Language::Js, cwd.path().into())
            .await
            .unwrap();
        {
            let mut request = Box::pin(kernel.request(
                json!({"op":"eval","code":"await new Promise(() => {})"}),
                None,
                Duration::from_secs(10),
            ));
            assert!(futures::poll!(request.as_mut()).is_pending());
            assert_eq!(kernel.pending.lock().unwrap().len(), 1);
        }
        assert!(
            kernel.pending.lock().unwrap().is_empty(),
            "Dropped requests release their reply slots"
        );
        let error = kernel
            .request(
                json!({"op":"eval","code":"await new Promise(() => {})"}),
                None,
                Duration::from_millis(100),
            )
            .await
            .unwrap_err();
        assert!(error.contains("timed out"));
        assert!(!kernel.alive.load(Ordering::Acquire));
    }

    #[tokio::test]
    #[ignore = "requires installed Node.js runtime"]
    async fn real_kernel_cell_settlement_waits_for_floated_host_calls() {
        let cwd = tempfile::tempdir().unwrap();
        let kernel = Kernel::start(Language::Js, cwd.path().into())
            .await
            .unwrap();
        let (reply, mut receive) = oneshot::channel();
        kernel.pending.lock().unwrap().insert(
            "cell".into(),
            Pending {
                reply,
                context: None,
                output: String::new(),
                host_calls: 1,
                result: None,
            },
        );
        kernel.receive(json!({"type":"result","id":"cell","ok":true,"value":42}));
        assert!(matches!(
            receive.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        kernel
            .pending
            .lock()
            .unwrap()
            .get_mut("cell")
            .unwrap()
            .host_calls = 0;
        kernel.complete_if_ready("cell");
        assert_eq!(receive.await.unwrap().unwrap()["value"], 42);
        kernel.stop().await;
    }
}
