use std::fs;
use std::path::PathBuf;

use chrono::{SecondsFormat, Utc};
use tauri::{AppHandle, Manager};
use uuid::Uuid;

use super::events;
use super::types::{is_allowed_node_type, Automation, AutomationMeta, SCHEMA_VERSION};

pub(crate) fn automations_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|err| format!("app_data_dir unavailable: {err}"))?
        .join("automations");
    fs::create_dir_all(&dir).map_err(|err| format!("create automations dir failed: {err}"))?;
    Ok(dir)
}

pub(crate) use super::document::validate_id;
#[cfg(test)]
use super::document::validate_graph;

fn file_path(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    validate_id(id)?;
    Ok(automations_dir(app)?.join(format!("{id}.json")))
}

use super::catalog::read_one;

/// 单次目录遍历读全量图。调度器用它避免「list 拿 meta、再逐条 get」的双倍读盘。
pub(crate) fn list_full(app: &AppHandle) -> Result<Vec<Automation>, String> {
    let dir = automations_dir(app)?;
    let (items, warnings) = super::catalog::list_in(&dir)?;
    for warning in warnings { eprintln!("skip automation {warning}"); }
    Ok(items)
}

pub(crate) fn list(app: &AppHandle) -> Result<Vec<AutomationMeta>, String> {
    let mut metas: Vec<AutomationMeta> = list_full(app)?
        .into_iter()
        .map(|automation| automation.meta())
        .collect();
    metas.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(metas)
}

pub(crate) fn get(app: &AppHandle, id: &str) -> Result<Automation, String> {
    read_one(&file_path(app, id)?)
}

pub(crate) fn save(app: &AppHandle, automation: Automation) -> Result<Automation, String> {
    let automation = super::document::save(&automations_dir(app)?, automation)?;
    publish_saved(app, &automation);
    Ok(automation)
}

fn publish_saved(app: &AppHandle, automation: &Automation) {
    events::changed(app, "saved", &automation.id, Some(automation.updated_at.clone()));
    super::schedule::poke();
}

pub(crate) fn delete(app: &AppHandle, id: &str) -> Result<(), String> {
    super::document::delete(&automations_dir(app)?, id)?;
    let _ = super::history::delete_all(app, id);
    super::schedule::forget(app, id);
    events::changed(app, "deleted", id, None);
    super::schedule::poke();
    Ok(())
}

pub(crate) fn set_enabled(app: &AppHandle, id: &str, enabled: bool) -> Result<Automation, String> {
    let mut automation = get(app, id)?;
    automation.enabled = enabled;
    save(app, automation)
}

pub(crate) fn export_to_file(app: &AppHandle, id: &str, dest: &str) -> Result<(), String> {
    super::document::export(&automations_dir(app)?, id, std::path::Path::new(dest))
}

/// 导入永远生成新 id 并强制未启用：既避免 id 撞车覆盖已有图，
/// 也避免「导入即注册」别人文件里的热键/定时触发器。
pub(crate) fn import_from_file(app: &AppHandle, src: &str) -> Result<Automation, String> {
    let automation = super::document::import(&automations_dir(app)?, std::path::Path::new(src))?;
    publish_saved(app, &automation);
    Ok(automation)
}

#[cfg(test)]
mod tests {
    use super::{validate_graph, validate_id};

    #[test]
    fn incomplete_drafts_can_be_saved_but_cannot_be_enabled() {
        let mut automation: super::Automation = serde_json::from_value(serde_json::json!({
            "id": "draft", "name": "Draft", "nodes": [
                {"id": "t", "type": "trigger.manual", "data": {}},
                {"id": "http", "type": "action.http", "data": {"http": {"url": ""}}}
            ], "edges": [{"id": "e", "source": "t", "target": "http"}]
        }))
        .unwrap();
        assert!(validate_graph(&automation).is_ok());
        automation.enabled = true;
        assert!(validate_graph(&automation).is_err());
        automation.nodes[1].data =
            serde_json::json!({"http": {"url": "https://example.com", "method": "GET"}});
        assert!(validate_graph(&automation).is_ok());
    }

    #[test]
    fn rejects_path_ids() {
        assert!(validate_id("ok").is_ok());
        assert!(validate_id("../x").is_err());
        assert!(validate_id("a/b").is_err());
        assert!(validate_id("").is_err());
    }
}
