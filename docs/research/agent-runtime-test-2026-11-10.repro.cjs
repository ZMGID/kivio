// Offline regressions for F1/F2/F6/F8/F9; F5/F7 still assert known defects.
// No model API, user files, or model-returned tools are executed.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const root = path.resolve(__dirname, '../..');
const probeDirectory = path.join(root, 'target');
fs.mkdirSync(probeDirectory, { recursive: true });
const temporary = fs.mkdtempSync(path.join(probeDirectory, 'kivio-runtime-audit-'));
const slash = value => value.replaceAll('\\', '/');
function run(command, args) {
  const result = spawnSync(command, args, { encoding: 'utf8', cwd: root, maxBuffer: 4 * 1024 * 1024 });
  if (result.stdout) process.stdout.write(result.stdout);
  if (result.stderr) process.stderr.write(result.stderr);
  if (result.error) throw result.error;
  assert.equal(result.status, 0, `${command} exited ${result.status}`);
}
function sourceFunction(source, name) {
  const match = source.match(new RegExp(`^fn ${name}\\([^]*?^\\}`, 'm'));
  assert(match, `Function not found: ${name}`);
  return match[0];
}

// Execute the actual private functions, with small data types for their surrounding modules.
const shell = fs.readFileSync(path.join(root, 'src-tauri/src/native_tools/shell.rs'), 'utf8');
const evidence = fs.readFileSync(path.join(root, 'src-tauri/src/chat/goal/evidence.rs'), 'utf8');
const goal = fs.readFileSync(path.join(root, 'src-tauri/src/chat/goal.rs'), 'utf8');
const patterns = shell.match(/const LONG_RUNNING_DEV_PATTERNS:.*?= &\[[\s\S]*?\n\];/);
assert(patterns, 'Shell command patterns not found');
const isolated = `#![allow(dead_code)]
${patterns[0]}
${['is_long_running_dev_command', 'normalize_run_command', 'parse_leading_cd_prefix', 'find_cd_and_separator', 'strip_shell_quotes'].map(name => sourceFunction(shell, name)).join('\n')}
${sourceFunction(goal, 'is_mutating_tool')}
struct GoalCriterion { evidence: Option<String>, evidence_kind: Option<String>, evidence_ref: Option<String> }
struct ChatToolPayload { id: String, name: String, status: String, error: Option<String> }
struct ChatToolArtifactPayload { id: Option<String>, name: String, path: Option<String> }
struct ChatRunSnapshot { message_id: String }
mod evidence {
 use super::*;
 ${['normalized_path', 'artifact_matches', 'validate'].map(name => sourceFunction(evidence, name)).join('\n')}
 pub fn check() {
  let mut criterion=GoalCriterion{evidence:Some("Coverage is above 90 percent".into()),evidence_kind:Some("artifact".into()),evidence_ref:Some("artifact".into())};
  let artifacts=vec![ChatToolArtifactPayload{id:Some("artifact".into()),name:"export.xlsx".into(),path:None}];
  assert!(validate(&criterion,&[],&artifacts,&[],None).is_none());
  println!("known F5 defect: existing XLSX accepted for a coverage claim");
  criterion.evidence_kind=Some("tool_result".into());criterion.evidence_ref=Some("tested".into());
  let mut tools=vec![ChatToolPayload{id:"tested".into(),name:"bash".into(),status:"success".into(),error:None}];
  tools.push(ChatToolPayload{id:"later".into(),name:"bash".into(),status:"success".into(),error:None});
  assert!(validate(&criterion,&tools,&[],&[],None).is_none());
  println!("known F5 defect: later bash write can pass evidence freshness check");
  tools[1].name="write".into();
  assert!(validate(&criterion,&tools,&[],&[],None).is_some());
  println!("later native write invalidates the same evidence: true");
 }
}
fn main(){
 for command in ["npx vitest run", "npx vitest ls", "vitest run", "vitest"] {
  assert!(!is_long_running_dev_command(command));
  println!("finite test stays in foreground: {command}");
 }
 assert!(!is_long_running_dev_command("node node_modules/vitest/vitest.mjs run"));
 for command in ["vite", "npx vite --port 5173", "cd app && vite", "pnpm exec vite", "pnpm dlx vite", "yarn vite", "bunx vite", "npm exec vite"] {
  assert!(is_long_running_dev_command(command));
 }
 for command in ["vite build", "vite preview", "vite optimize"] {
  assert!(!is_long_running_dev_command(command));
 }
 let command="cd /d/ZMdata/Yuy/android 2>/dev/null && echo ok";
 assert_eq!(normalize_run_command(command,None).unwrap(), (command.to_string(),None));
 println!("cd stderr redirection stays untouched: true");
 evidence::check();
}
`;
const isolatedPath = path.join(temporary, 'source-functions.rs');
const isolatedBinary = path.join(temporary, process.platform === 'win32' ? 'source-functions.exe' : 'source-functions');
fs.writeFileSync(isolatedPath, isolated);
run('rustc', [isolatedPath, '-o', isolatedBinary]);
run(isolatedBinary, []);

// The full public SDK loop is exercised through its existing example host and executor.
fs.mkdirSync(path.join(temporary, 'src'));
fs.writeFileSync(path.join(temporary, 'Cargo.toml'), `[package]
name = "kivio-runtime-audit-repro"
version = "0.1.0"
edition = "2021"
[dependencies]
kivio-agent-sdk = { path = ${JSON.stringify(slash(path.join(root, 'crates/kivio-agent-sdk')))} }
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "net", "io-util"] }
serde_json = "1"
reqwest = "0.13"
`);
fs.writeFileSync(path.join(temporary, 'src/main.rs'), `#![allow(dead_code)]
#[path=${JSON.stringify(slash(path.join(root, 'crates/kivio-agent-sdk/examples/support/mod.rs')))}]
mod support;
use std::{collections::VecDeque,sync::{Arc,Mutex,atomic::{AtomicUsize,Ordering}}};
use kivio_agent_sdk::{
 agent::{run_agent_loop,InjectionSources,session::{NoSessionPolicy,SessionPolicy,RunSnapshot,PolicyFuture,TurnDecision},
  control::AgentRunHandle,recovery::{RecoverySnapshot,RecoveryDecision},
  provider_runtime::{ModelCall,ModelOutputSink,ModelRuntime,ProviderFuture},
  events::{AgentEvent,EventDelivery},types::AgentRunStatus},
 model::{GenerateOutput,ModelError,ModelUsage,PendingToolCall,StreamPart},
};
struct Model { calls:AtomicUsize,outputs:Mutex<VecDeque<GenerateOutput>> }
impl ModelRuntime for Model {
 fn generate<'a>(&'a self,call:ModelCall,sink:&'a mut dyn ModelOutputSink)->ProviderFuture<'a,Result<GenerateOutput,ModelError>> {
  Box::pin(async move {
   sink.request_started(&call);self.calls.fetch_add(1,Ordering::SeqCst);
   self.outputs.lock().unwrap().pop_front().ok_or_else(||"Unexpected model request".into())
  })
 }
}
struct ContinueAfterRecovery;
impl SessionPolicy for ContinueAfterRecovery {
 fn recover(&mut self,_:RecoverySnapshot<'_>,_:&ModelError)->RecoveryDecision {
  RecoveryDecision::Complete("temporary empty response notice".into())
 }
 fn finish_turn<'a>(&'a mut self,run:RunSnapshot<'a>,_:AgentRunHandle)->PolicyFuture<'a,Result<TurnDecision,String>> {
  Box::pin(async move {Ok(if run.history.last().is_some_and(|m|m.text_content()=="temporary empty response notice") {TurnDecision::Continue}else{TurnDecision::Default})})
 }
}
#[tokio::main]
async fn main(){
 {
  use tokio::io::{AsyncReadExt,AsyncWriteExt};
  let first=serde_json::json!({"choices":[{"delta":{"reasoning_content":"Create App"}}]});
  let second=serde_json::json!({"choices":[{"delta":{"reasoning_content":"Container safely."}}]});
  let final_chunk=serde_json::json!({"choices":[{"delta":{"content":"OK"},"finish_reason":"stop"}]});
  let raw=format!("data: {first}\\n\\ndata: {second}\\n\\ndata: {final_chunk}\\n\\ndata: [DONE]\\n\\n");
  let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let address=listener.local_addr().unwrap();
  let server=tokio::spawn(async move {
   let (mut socket,_)=listener.accept().await.unwrap();let mut request=[0u8;8192];let _=socket.read(&mut request).await.unwrap();
   let header=format!("HTTP/1.1 200 OK\\r\\nContent-Type: text/event-stream\\r\\nContent-Length: {}\\r\\nConnection: close\\r\\n\\r\\n",raw.len());
   socket.write_all(header.as_bytes()).await.unwrap();socket.write_all(raw.as_bytes()).await.unwrap();
  });
  let response=reqwest::Client::builder().no_proxy().build().unwrap().get(format!("http://{address}/")).send().await.unwrap();
  let output=kivio_agent_sdk::model::openai::decode_chat_stream(response,&mut |_:StreamPart|Ok(()),"diagnostic").await.unwrap();
  server.await.unwrap();
  assert_eq!(output.text,"OK");
  assert_eq!(output.reasoning.as_deref(),Some("Create App\\n\\nContainer safely."));
  println!("known F7 defect: Chat reasoning delta changed AppContainer into App + two newlines + Container");
 }
 for (label,text,reasoning) in [
  ("empty body","",None),
  ("reasoning only","",Some("I should continue using tools")),
  ("DSML markup only","<|DSML|tool_calls></|DSML|tool_calls>",None),
 ] {
  let model=Model{calls:AtomicUsize::new(0),outputs:Mutex::new(VecDeque::from([
   GenerateOutput::text(text.into(),reasoning.map(str::to_owned)),GenerateOutput::text("recovered on retry".into(),None),
  ]))};
  let control=Arc::new(support::Control::default());
  let result=run_agent_loop(support::config(&model,support::initial_history()),&support::Host,
   Arc::new(support::EchoTool::new(true)),&mut NoSessionPolicy,&mut InjectionSources::new(),
   &|_:&AgentEvent<'_>|EventDelivery::Accepted,support::handle(&control)).await;
  let result=result.unwrap();
  assert_eq!(result.content,"recovered on retry");assert_eq!(model.calls.load(Ordering::SeqCst),2);
  println!("{label}: requests=2, retry answer returned");
 }
 let mut tool=GenerateOutput::text(String::new(),None);
 tool.tool_calls.push(PendingToolCall{id:"echo-1".into(),function_name:"echo".into(),arguments:serde_json::json!({"text":"hello"}),arguments_raw:r#"{"text":"hello"}"#.into(),arguments_parse_error:None,signature:None});
 tool.usage=Some(ModelUsage{input_tokens:Some(10),output_tokens:Some(1),..Default::default()});
 let mut empty=GenerateOutput::text(String::new(),None);
 empty.usage=Some(ModelUsage{input_tokens:Some(20),output_tokens:Some(2),..Default::default()});
 let mut success=GenerateOutput::text("completed normally".into(),None);
 success.usage=Some(ModelUsage{input_tokens:Some(30),output_tokens:Some(3),..Default::default()});
 let model=Model{calls:AtomicUsize::new(0),outputs:Mutex::new(VecDeque::from([
  tool,empty,GenerateOutput::text(String::new(),None),success,
 ]))};
 let control=Arc::new(support::Control::default());
 let result=run_agent_loop(support::config(&model,support::initial_history()),&support::Host,
  Arc::new(support::EchoTool::new(true)),&mut ContinueAfterRecovery,&mut InjectionSources::new(),
  &|_:&AgentEvent<'_>|EventDelivery::Accepted,support::handle(&control)).await.unwrap();
 assert_eq!(result.status,AgentRunStatus::Completed);
 assert!(!result.segments.iter().any(|s|s.text.as_deref().is_some_and(|t|t.contains("temporary empty response notice"))));
 assert!(result.segments.iter().any(|s|s.text.as_deref().is_some_and(|t|t.contains("completed normally"))));
 println!("recovery followed by success: Completed contains final answer without obsolete notice segments");
 assert_eq!(result.usage.as_ref().unwrap().input_tokens,Some(60));
 assert_eq!(result.usage.as_ref().unwrap().output_tokens,Some(6));
 println!("empty response usage retained: recorded input 60 of 60, output 6 of 6");
}
`);
run('cargo', ['run', '--offline', '--quiet', '--manifest-path', path.join(temporary, 'Cargo.toml'),
  '--target-dir', path.join(root, 'crates/kivio-agent-sdk/target')]);
console.log(`Probe files: ${temporary}`);
