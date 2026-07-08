//! 技能市场：拉取远程 JSON 索引、下载并安装技能 zip。货源地址由用户在设置里配置，
//! 本模块不写死任何来源。安装复用 `super::install_skill_zip_bytes` 的解压落盘逻辑。

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use super::{install_skill_zip_bytes, user_skills_dir};
use super::types::SkillImportResult;

const CACHE_TTL: Duration = Duration::from_secs(300);
const MAX_ZIP_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketSkill {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub download_url: String,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub preview_url: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketIndex {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub skills: Vec<MarketSkill>,
}

/// 写入 `{skill_dir}/.market.json`，记录该技能来自市场的哪个版本/索引。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarketMarker {
    id: String,
    version: String,
    index_url: String,
    installed_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketInstalledInfo {
    pub id: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketFetchResult {
    pub success: bool,
    #[serde(default)]
    pub skills: Vec<MarketSkill>,
    /// 本地已安装且带市场标记的技能（供前端算 未装/已装/可更新 三态）。
    #[serde(default)]
    pub installed: Vec<MarketInstalledInfo>,
    pub error: Option<String>,
}

// ponytail: 单条 URL 的内存缓存就够（用户一次只看一个市场）；换 URL 直接失效重拉。
type CacheCell = Mutex<Option<(String, Instant, Vec<MarketSkill>, u32)>>;
fn cache() -> &'static CacheCell {
    static CACHE: OnceLock<CacheCell> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

fn scan_installed(app: &AppHandle) -> Vec<MarketInstalledInfo> {
    let dir = match user_skills_dir(app) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let marker = entry.path().join(".market.json");
        if let Ok(raw) = std::fs::read_to_string(&marker) {
            if let Ok(m) = serde_json::from_str::<MarketMarker>(&raw) {
                out.push(MarketInstalledInfo {
                    id: m.id,
                    version: m.version,
                });
            }
        }
    }
    out
}

#[tauri::command]
pub async fn chat_skills_market_fetch(app: AppHandle, index_url: String) -> MarketFetchResult {
    let url = index_url.trim().to_string();
    if url.is_empty() {
        return MarketFetchResult {
            success: false,
            skills: Vec::new(),
            installed: Vec::new(),
            error: Some("未配置技能市场索引地址".to_string()),
        };
    }

    // 命中缓存直接返回（仍重新扫描本地已装状态，安装后无需等缓存过期就能刷新三态）。
    if let Ok(guard) = cache().lock() {
        if let Some((cached_url, at, skills, _)) = guard.as_ref() {
            if cached_url == &url && at.elapsed() < CACHE_TTL {
                return MarketFetchResult {
                    success: true,
                    skills: skills.clone(),
                    installed: scan_installed(&app),
                    error: None,
                };
            }
        }
    }

    let client = crate::api::build_http_client();
    let resp = match crate::api::with_standard_request_timeout(client.get(&url))
        .send()
        .await
    {
        Ok(r) => r,
        Err(err) => {
            return MarketFetchResult {
                success: false,
                skills: Vec::new(),
                installed: scan_installed(&app),
                error: Some(format!("拉取索引失败：{err}")),
            }
        }
    };
    if !resp.status().is_success() {
        return MarketFetchResult {
            success: false,
            skills: Vec::new(),
            installed: scan_installed(&app),
            error: Some(format!("索引地址返回 {}", resp.status())),
        };
    }
    let text = match resp.text().await {
        Ok(t) => t,
        Err(err) => {
            return MarketFetchResult {
                success: false,
                skills: Vec::new(),
                installed: scan_installed(&app),
                error: Some(format!("读取索引失败：{err}")),
            }
        }
    };
    let index: MarketIndex = match serde_json::from_str(&text) {
        Ok(i) => i,
        Err(err) => {
            return MarketFetchResult {
                success: false,
                skills: Vec::new(),
                installed: scan_installed(&app),
                error: Some(format!("索引 JSON 解析失败：{err}")),
            }
        }
    };

    if let Ok(mut guard) = cache().lock() {
        *guard = Some((url, Instant::now(), index.skills.clone(), index.version));
    }

    MarketFetchResult {
        success: true,
        skills: index.skills,
        installed: scan_installed(&app),
        error: None,
    }
}

#[tauri::command]
pub async fn chat_skills_market_install(
    app: AppHandle,
    skill: MarketSkill,
    index_url: String,
) -> SkillImportResult {
    let fail = |msg: String| SkillImportResult {
        success: false,
        skill: None,
        error: Some(msg),
    };

    let download_url = skill.download_url.trim().to_string();
    if download_url.is_empty() {
        return fail("技能缺少下载地址".to_string());
    }
    let skills_dir = match user_skills_dir(&app) {
        Ok(d) => d,
        Err(err) => return fail(err),
    };

    let client = crate::api::build_http_client();
    let resp = match crate::api::with_standard_request_timeout(client.get(&download_url))
        .send()
        .await
    {
        Ok(r) => r,
        Err(err) => return fail(format!("下载技能失败：{err}")),
    };
    if !resp.status().is_success() {
        return fail(format!("下载地址返回 {}", resp.status()));
    }
    if let Some(len) = resp.content_length() {
        if len > MAX_ZIP_BYTES {
            return fail("技能包过大（>50MB），已拒绝".to_string());
        }
    }
    let bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(err) => return fail(format!("读取技能包失败：{err}")),
    };
    if bytes.len() as u64 > MAX_ZIP_BYTES {
        return fail("技能包过大（>50MB），已拒绝".to_string());
    }

    let meta = match install_skill_zip_bytes(bytes.to_vec(), &skills_dir) {
        Ok(m) => m,
        Err(err) => return fail(err),
    };

    // 写市场标记：用安装解析出的真实 id，避免索引 id 与包内 SKILL.md id 不一致导致更新检测失效。
    let marker = MarketMarker {
        id: meta.id.clone(),
        version: skill.version.clone(),
        index_url: index_url.trim().to_string(),
        installed_at: chrono::Utc::now().to_rfc3339(),
    };
    let marker_path = skills_dir.join(&meta.id).join(".market.json");
    if let Ok(json) = serde_json::to_string_pretty(&marker) {
        let _ = std::fs::write(&marker_path, json);
    }

    SkillImportResult {
        success: true,
        skill: Some(meta),
        error: None,
    }
}
