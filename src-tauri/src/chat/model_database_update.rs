//! 模型库远程更新：从远程 URL（GitHub raw / 自建镜像均可，普通 HTTPS GET）
//! 拉取 `modelDatabase.json` 覆盖文件，落到 `<app_data>/modelDatabase.json`。
//!
//! 覆盖合并逻辑在 [`super::model_metadata`]：进程启动时把该文件逐 key 合并进内置基线。
//! 本模块只负责「检查 + 下载安装」，装完重启应用生效（模型库进程内只读一次）。
//!
//! 为什么不做自动拉取：模型库一天最多变一次，手动「点一下更新」足够；且远程地址
//! 由用户填写（默认取作者仓库 ZMGID/kivio 的 raw 地址 / 自己的镜像），不硬编码必走端点，离线零请求。

use std::path::PathBuf;

use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use super::model_metadata::USER_MODEL_DATABASE_FILE;
use crate::state::AppState;

/// 下载体积上限：当前库 ~120KB，留 30 倍余量防误拉大文件。
const MAX_DOWNLOAD_BYTES: usize = 4 * 1024 * 1024;

fn user_db_path() -> Option<PathBuf> {
    crate::app_data::app_data_dir().map(|dir| dir.join(USER_MODEL_DATABASE_FILE))
}

/// 解析并校验一份模型库 JSON：必须是 object、带非空字符串 `_meta.version`、
/// 除 `_meta` 外至少 1 个条目。返回 `(version, entries)`。
fn parse_db(text: &str) -> Result<(String, usize), String> {
    if text.len() > MAX_DOWNLOAD_BYTES {
        return Err(format!(
            "文件超过 {}MB 上限，不是模型库文件",
            MAX_DOWNLOAD_BYTES / (1024 * 1024)
        ));
    }
    let value: Value =
        serde_json::from_str(text).map_err(|e| format!("JSON 解析失败: {e}"))?;
    let Some(map) = value.as_object() else {
        return Err("顶层必须是 JSON object".into());
    };
    let version = map
        .get("_meta")
        .and_then(|meta| meta.get("version"))
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
        .ok_or("缺少 _meta.version 字段（不是 Kivio 模型库文件）")?
        .trim()
        .to_string();
    let entries = map.len().saturating_sub(usize::from(map.contains_key("_meta")));
    if entries == 0 {
        return Err("模型库没有任何模型条目".into());
    }
    Ok((version, entries))
}

/// 当前生效的模型库信息：内置版本 + 用户覆盖版本（若有）。
#[tauri::command]
pub(crate) fn model_database_local_info() -> Value {
    let builtin = super::model_metadata::builtin_database_version();
    let (user_version, user_exists) = user_db_path()
        .filter(|p| p.exists())
        .map(|p| {
            let text = std::fs::read_to_string(&p).unwrap_or_default();
            match parse_db(&text) {
                Ok((v, _)) => (Some(v), true),
                Err(_) => (None, true),
            }
        })
        .unwrap_or((None, false));
    json!({
        "builtinVersion": builtin,
        "userVersion": user_version,
        "userFileExists": user_exists,
        // 生效版本：覆盖层优先（与 model_metadata 的合并顺序一致）
        "effectiveVersion": user_version.clone().unwrap_or(builtin),
        "userFilePath": user_db_path().map(|p| p.display().to_string()),
    })
}

/// 检查远程版本：下载并解析，返回远程版本与条目数，不写盘。
#[tauri::command]
pub(crate) async fn model_database_remote_check(
    app: AppHandle,
    url: String,
) -> Result<Value, String> {
    let (version, entries, bytes) = remote_fetch(&app, &url).await?;
    Ok(json!({ "version": version, "entries": entries, "bytes": bytes }))
}

/// 安装远程模型库：下载、校验、原子写入 `<app_data>/modelDatabase.json`。
/// 重启应用后生效（覆盖合并发生在进程启动时）。
#[tauri::command]
pub(crate) async fn model_database_remote_install(
    app: AppHandle,
    url: String,
) -> Result<Value, String> {
    let text = remote_body(&app, &url).await?;
    let (version, entries) = parse_db(&text)?;
    let Some(dest) = user_db_path() else {
        return Err("无法定位应用数据目录".into());
    };
    // 原子写：先写临时文件，再替换目标。Windows 的 rename 不覆盖已存在文件，先删旧再改名。
    let tmp = dest.with_extension("json.tmp");
    std::fs::write(&tmp, text.as_bytes()).map_err(|e| format!("写临时文件失败: {e}"))?;
    if dest.exists() {
        std::fs::remove_file(&dest).map_err(|e| format!("移除旧文件失败: {e}"))?;
    }
    std::fs::rename(&tmp, &dest).map_err(|e| {
        // 尽力把临时文件挪回来，避免留下孤儿 .tmp
        let _ = std::fs::rename(&tmp, &dest);
        format!("落盘失败: {e}")
    })?;
    Ok(json!({ "version": version, "entries": entries, "path": dest.display().to_string() }))
}

async fn remote_body(app: &AppHandle, url: &str) -> Result<String, String> {
    let url = url.trim();
    if !url.starts_with("https://") {
        return Err("模型库地址必须以 https:// 开头".into());
    }
    let state = app.state::<AppState>();
    let resp = state
        .http
        .get(url)
        .header("User-Agent", "kivio-model-database")
        .send()
        .await
        .map_err(|e| format!("下载失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("远程返回 {}", resp.status()));
    }
    let text = resp
        .text()
        .await
        .map_err(|e| format!("读取响应失败: {e}"))?;
    if text.len() > MAX_DOWNLOAD_BYTES {
        return Err("文件超过大小上限，不是模型库文件".into());
    }
    Ok(text)
}

async fn remote_fetch(app: &AppHandle, url: &str) -> Result<(String, usize, usize), String> {
    let text = remote_body(app, url).await?;
    let (version, entries) = parse_db(&text)?;
    Ok((version, entries, text.len()))
}
