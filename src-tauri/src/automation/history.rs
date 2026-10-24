//! Tauri path adapter for the shared run-history store.
use std::{fs,path::PathBuf};
use tauri::AppHandle;
use super::{storage::{automations_dir,validate_id},types::{AutomationRun,AutomationRunSummary},history_core};
fn runs_dir(app:&AppHandle,id:&str)->Result<PathBuf,String>{validate_id(id)?;let dir=automations_dir(app)?.join("runs").join(id);fs::create_dir_all(&dir).map_err(|e|e.to_string())?;Ok(dir)}
pub(crate) fn delete_all(app:&AppHandle,id:&str)->Result<(),String>{let dir=runs_dir(app,id)?;fs::remove_dir_all(dir).map_err(|e|e.to_string())}
pub(crate) fn write_run(app:&AppHandle,run:&AutomationRun)->Result<(),String>{history_core::write_run_in(&runs_dir(app,&run.automation_id)?,run)}
pub(crate) fn list(app:&AppHandle,id:&str)->Result<Vec<AutomationRunSummary>,String>{history_core::list_in(&runs_dir(app,id)?)}
pub(crate) fn get(app:&AppHandle,id:&str,run:&str)->Result<AutomationRun,String>{validate_id(run)?;history_core::read_run_file(&runs_dir(app,id)?.join(format!("{run}.json")))}
