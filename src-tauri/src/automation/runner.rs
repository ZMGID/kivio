//! Tauri entry points for the host-independent graph runner.
use tauri::AppHandle;
use super::types::{AutomationRunStarted, NodeOutput, RunOrigin};
use super::{execution, execution_adapter::host};
pub fn enqueue(app:AppHandle,id:String,origin:RunOrigin,until:Option<String>,input:Option<NodeOutput>)->Result<AutomationRunStarted,String>{execution::enqueue(host(app),id,origin,until,input)}
pub fn test_node(app:AppHandle,id:String,node:String,input:NodeOutput)->Result<AutomationRunStarted,String>{execution::test_node(host(app),id,node,input)}
pub fn cancel(app:&AppHandle,id:&str)->Result<(),String>{execution::cancel(&host(app.clone()),id)}
pub fn cancel_all(app:&AppHandle)->usize{execution::cancel_all(&host(app.clone()))}
pub async fn wait_all_finished(app:&AppHandle){execution::wait_all_finished(&host(app.clone())).await}
