//! Compatibility adapter; the Tauri host retains its existing side effects.
use std::{path::PathBuf,sync::Arc};
use tauri::AppHandle;
use super::{application, events, history, notify, storage, types::*, execution_host::{AppHandle as ExecutionHandle,AutomationHost,HostFuture}};
pub fn host(app:AppHandle)->ExecutionHandle{Arc::new(Desktop(app))}
struct Desktop(AppHandle);
impl AutomationHost for Desktop {
    fn get_automation(&self,id:&str)->Result<Automation,String>{storage::get(&self.0,id)}
    fn begin_run(&self,id:&str,run:&str)->Result<(),String>{application::begin_run(&self.0,id,run)}
    fn finish_run_slot(&self,id:&str,run:&str){application::finish_run_slot(&self.0,id,run)}
    fn active_automation_ids(&self)->Vec<String>{application::active_automation_ids(&self.0)}
    fn mark_run_cancelled(&self,id:&str)->bool{application::mark_run_cancelled(&self.0,id)}
    fn is_run_cancelled(&self,run:&str)->bool{application::is_run_cancelled(&self.0,run)}
    fn cancel_agent_generations(&self,id:&str,nodes:Vec<String>){application::cancel_agent_generations(&self.0,id,nodes)}
    fn write_run(&self,run:&AutomationRun)->Result<(),String>{history::write_run(&self.0,run)}
    fn event(&self,event:AutomationRunEvent){events::emit(&self.0,event)}
    fn settings_language(&self)->String{application::settings_language(&self.0)}
    fn working_directory(&self)->String{application::automation_working_directory(&self.0)}
    fn http_client(&self)->reqwest::Client{application::http_client(&self.0)}
    fn notify(&self,title:&str,body:&str){notify::show(&self.0,title,body)}
    fn run_agent_node(&self,request:AgentNodeRequest)->HostFuture<'_,Result<NodeOutput,String>>{Box::pin(application::run_agent_node(&self.0,request))}
    fn run_captured_command<'a>(&'a self,command:&'a str,cwd:PathBuf,timeout:u64)->HostFuture<'a,Result<crate::native_tools::CapturedCommand,String>>{Box::pin(application::run_captured_command(&self.0,command,cwd,timeout))}
    fn spawn(&self,future:HostFuture<'static,()>){tauri::async_runtime::spawn(future);}
}
