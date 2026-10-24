//! Desktop execution seam. The graph runner owns traversal; hosts own I/O and lifetime.
use std::{future::Future, pin::Pin, sync::Arc, path::PathBuf};
use super::types::{AgentNodeRequest, Automation, AutomationRun, AutomationRunEvent, NodeOutput};
pub type AppHandle = Arc<dyn AutomationHost>;
pub type HostFuture<'a,T> = Pin<Box<dyn Future<Output=T> + Send + 'a>>;
pub trait AutomationHost: Send + Sync {
    fn get_automation(&self,id:&str)->Result<Automation,String>;
    fn begin_run(&self,id:&str,run:&str)->Result<(),String>;
    fn finish_run_slot(&self,id:&str,run:&str);
    fn active_automation_ids(&self)->Vec<String>;
    fn mark_run_cancelled(&self,id:&str)->bool;
    fn is_run_cancelled(&self,run:&str)->bool;
    fn cancel_agent_generations(&self,id:&str,nodes:Vec<String>);
    fn write_run(&self,run:&AutomationRun)->Result<(),String>;
    fn event(&self,event:AutomationRunEvent);
    fn settings_language(&self)->String;
    fn working_directory(&self)->String;
    fn http_client(&self)->reqwest::Client;
    fn notify(&self,title:&str,body:&str);
    fn run_agent_node(&self,request:AgentNodeRequest)->HostFuture<'_,Result<NodeOutput,String>>;
    fn run_captured_command<'a>(&'a self,command:&'a str,cwd:PathBuf,timeout:u64)->HostFuture<'a,Result<crate::native_tools::CapturedCommand,String>>;
    fn spawn(&self,future:HostFuture<'static,()>);
}
