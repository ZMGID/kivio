#[cfg(target_os = "macos")]
use std::path::PathBuf;
use std::{fs, io::Write};

use tauri::{AppHandle, Emitter, State};
#[cfg(target_os = "macos")]
use uuid::Uuid;

use crate::api::with_standard_request_timeout;
use crate::state::AppState;

/// 调 GitHub Releases API 检查最新版本
/// 发现新版只返回提示信息，让前端弹"去 GitHub 下载"按钮（不做自动下载安装，避免引入签名密钥那套）
///
/// 双通道：先查 `api.github.com`（能拿到 body/assets，供自动下载）；失败（网络 / 非 2xx /
/// 限流 / 解析）时回退查 `github.com` 的 releases atom feed —— 因为 `api.github.com` 在部分
/// 网络下被单独墙掉或限流（60 次/小时/IP），而 `github.com` 本体仍可访问。两条都失败时返回
/// `checkFailed:true`，让前端明确显示"检查失败"而不是伪装成"已是最新"（会误导用户）。
#[tauri::command]
pub(crate) async fn check_github_latest_release(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    const REPO: &str = "ZMGID/kivio";
    let current = env!("CARGO_PKG_VERSION");

    // 主通道：api.github.com（成功即返回，无论 available 真假）。
    if let Some(json) = try_api_latest(&state, REPO, current).await {
        return Ok(json);
    }
    // 回退通道：github.com atom feed（用户能访问 github.com 但 api.github.com 不通/被限流时）。
    if let Some(json) = try_atom_latest(&state, REPO, current).await {
        return Ok(json);
    }
    // 两条都失败：明确告知检查失败，不伪装成"最新"。
    Ok(serde_json::json!({ "available": false, "checkFailed": true }))
}

/// 主通道：`api.github.com/repos/{REPO}/releases/latest`。
/// 返回 `Some(json)` 当且仅当请求成功且 JSON 解析成功（此时 available 可真可假）；
/// 任何网络 / 非 2xx / 解析失败都返回 `None`，交给 atom 回退。
async fn try_api_latest(state: &AppState, repo: &str, current: &str) -> Option<serde_json::Value> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let response = with_standard_request_timeout(
        state
            .http
            .get(&url)
            // GitHub API 要求显式 User-Agent
            .header("User-Agent", format!("Kivio/{}", env!("CARGO_PKG_VERSION")))
            .header("Accept", "application/vnd.github+json"),
    )
    .send()
    .await
    .ok()?;

    if !response.status().is_success() {
        return None;
    }

    let value: serde_json::Value = response.json().await.ok()?;

    let tag = value.get("tag_name").and_then(|v| v.as_str()).unwrap_or("");
    let html_url = value.get("html_url").and_then(|v| v.as_str()).unwrap_or("");
    let body = value.get("body").and_then(|v| v.as_str()).unwrap_or("");
    let published_at = value
        .get("published_at")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // tag_name 通常是 "v2.5.0"，剥掉前缀 v 再比较
    let latest = tag.trim_start_matches('v');

    Some(serde_json::json!({
      "available": is_newer_version(latest, current),
      "version": latest,
      "tag": tag,
      "htmlUrl": html_url,
      "body": body,
      "publishedAt": published_at,
    }))
}

/// 回退通道：`github.com/{REPO}/releases.atom`（走 github.com 主体，非 api 子域）。
/// 只解析最新 tag —— 没有 assets / body，`htmlUrl` 由 tag 拼出，够前端展示 + "去 GitHub 下载"。
async fn try_atom_latest(state: &AppState, repo: &str, current: &str) -> Option<serde_json::Value> {
    let url = format!("https://github.com/{repo}/releases.atom");
    let response = with_standard_request_timeout(
        state
            .http
            .get(&url)
            .header("User-Agent", format!("Kivio/{}", env!("CARGO_PKG_VERSION")))
            .header("Accept", "application/atom+xml"),
    )
    .send()
    .await
    .ok()?;

    if !response.status().is_success() {
        return None;
    }

    let xml = response.text().await.ok()?;
    let tag = parse_latest_tag_from_atom(&xml)?;
    let latest = tag.trim_start_matches('v');

    Some(serde_json::json!({
      "available": is_newer_version(latest, current),
      "version": latest,
      "tag": tag,
      "htmlUrl": format!("https://github.com/{repo}/releases/tag/{tag}"),
      "body": "",
      "publishedAt": "",
      "viaFallback": true,
    }))
}

/// 从 releases atom feed 里抽取最新 release 的 tag。
/// atom 里首个 `<entry>` 是最新，其 `<link href=".../releases/tag/<TAG>">` 是可靠来源。
/// 只做朴素字符串扫描（避免引 XML 依赖）：定位首个 `/releases/tag/`，读到下一个 `"`/`<`/空白为止。
fn parse_latest_tag_from_atom(xml: &str) -> Option<String> {
    const MARKER: &str = "/releases/tag/";
    let start = xml.find(MARKER)? + MARKER.len();
    let rest = &xml[start..];
    let tag: String = rest
        .chars()
        .take_while(|c| !matches!(c, '"' | '<' | '>' | ' ' | '\t' | '\r' | '\n'))
        .collect();
    let tag = tag.trim();
    if tag.is_empty() {
        None
    } else {
        Some(tag.to_string())
    }
}

/// 朴素 semver 比较：把 "x.y.z" 拆成数字三元组按字典序比较
/// 不处理 prerelease (-beta) / build metadata (+abc)；返回 latest > current
fn is_newer_version(latest: &str, current: &str) -> bool {
    let parse = |s: &str| -> (u32, u32, u32) {
        let mut it = s.split('.').map(|p| {
            // 截断到第一个非数字（兼容 "1.0.0-beta" 这类）
            p.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse::<u32>()
                .unwrap_or(0)
        });
        (
            it.next().unwrap_or(0),
            it.next().unwrap_or(0),
            it.next().unwrap_or(0),
        )
    };
    parse(latest) > parse(current)
}

/// 从 release JSON 的 assets 数组里挑出当前平台 + 架构的安装包。
/// 匹配规则：
///   - macOS aarch64 → `.dmg` 文件名包含 aarch64 / arm64
///   - macOS x86_64  → `.dmg` 包含 x64 / x86_64
///   - Windows       → `-setup.exe` 结尾（NSIS，覆盖升级体验比 MSI 顺）
fn pick_release_asset(assets: &[serde_json::Value]) -> Option<(String, String)> {
    let arch = std::env::consts::ARCH;
    for asset in assets {
        let name = asset.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let url = asset
            .get("browser_download_url")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if name.is_empty() || url.is_empty() {
            continue;
        }
        let lower = name.to_ascii_lowercase();
        let matched = if cfg!(target_os = "macos") {
            lower.ends_with(".dmg")
                && match arch {
                    "aarch64" => lower.contains("aarch64") || lower.contains("arm64"),
                    _ => lower.contains("x64") || lower.contains("x86_64"),
                }
        } else if cfg!(target_os = "windows") {
            lower.ends_with("-setup.exe")
        } else {
            false
        };
        if matched {
            return Some((name.to_string(), url.to_string()));
        }
    }
    None
}

/// 下载新版本安装包到 OS temp dir，边下边 emit "update-download-progress" 事件。
/// 返回本地文件绝对路径。失败 Err 含详细原因（前端显示）。
#[tauri::command]
pub(crate) async fn download_update_asset(
    app: AppHandle,
    state: State<'_, AppState>,
    version: String,
) -> Result<String, String> {
    const REPO: &str = "ZMGID/kivio";
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let resp = with_standard_request_timeout(
        state
            .http
            .get(&url)
            .header("User-Agent", format!("Kivio/{}", env!("CARGO_PKG_VERSION")))
            .header("Accept", "application/vnd.github+json"),
    )
    .send()
    .await
    .map_err(|e| format!("查询 release 失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("GitHub API 返回 {}", resp.status()));
    }
    let value: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 release JSON 失败: {e}"))?;
    let assets = value
        .get("assets")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "release 没有 assets".to_string())?;
    let (name, asset_url) = pick_release_asset(assets).ok_or_else(|| {
        format!(
            "没有匹配当前平台({}/{})的安装包",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;

    // 决定本地文件名：保留原扩展名（.dmg / .exe）便于 install 流程根据扩展名判断行为
    let ext = std::path::Path::new(&name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin");
    let safe_version = version
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '.' || *c == '-')
        .collect::<String>();
    let dest = std::env::temp_dir().join(format!("kivio-update-{safe_version}.{ext}"));

    let mut resp = state
        .http
        .get(&asset_url)
        .header("User-Agent", format!("Kivio/{}", env!("CARGO_PKG_VERSION")))
        .send()
        .await
        .map_err(|e| format!("下载失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("下载返回 {}", resp.status()));
    }
    let total = resp.content_length().unwrap_or(0);
    let mut file = fs::File::create(&dest).map_err(|e| format!("创建文件失败: {e}"))?;
    let mut downloaded: u64 = 0;
    let mut last_emitted_pct: i32 = -1;
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("读取下载流失败: {e}"))?
    {
        file.write_all(&chunk)
            .map_err(|e| format!("写入失败: {e}"))?;
        downloaded += chunk.len() as u64;
        let pct = if total > 0 {
            (downloaded * 100 / total) as i32
        } else {
            0
        };
        // 节流：百分比变化才 emit，避免事件洪水（小 chunk 时容易刷爆）
        if pct != last_emitted_pct {
            last_emitted_pct = pct;
            let _ = app.emit(
                "update-download-progress",
                serde_json::json!({
                  "percent": pct,
                  "downloadedBytes": downloaded,
                  "totalBytes": total,
                }),
            );
        }
    }
    // 收尾再 emit 一次确保 100% 落地
    let _ = app.emit(
        "update-download-progress",
        serde_json::json!({
          "percent": 100,
          "downloadedBytes": downloaded,
          "totalBytes": total.max(downloaded),
        }),
    );
    Ok(dest.to_string_lossy().to_string())
}

/// 启动安装包并退出当前应用。
/// - macOS（.dmg）：hdiutil 挂载 → cp Kivio.app 到 /Applications → 卸载 → open 新版 → app.exit(0)
/// - Windows（.exe）：spawn NSIS installer，立即 exit 让 installer 能写 exe
#[tauri::command]
pub(crate) fn install_update_and_quit(app: AppHandle, path: String) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    if !p.exists() {
        return Err(format!("安装包不存在: {path}"));
    }

    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        // 显式指定挂载点（用 UUID 避免与同名 volume 已挂载时的名字冲突）。比解析 `hdiutil attach` 的
        // 默认表格输出鲁棒很多 —— 那个输出列用空格 padding,VolumeName 含空格(如重复挂载产生的
        // "Kivio 1")会被 split_whitespace 截断。
        let mount_id = Uuid::new_v4().to_string();
        let mount_point = std::env::temp_dir().join(format!("kivio-mount-{mount_id}"));
        fs::create_dir_all(&mount_point).map_err(|e| format!("创建挂载目录失败: {e}"))?;
        let mount_str = mount_point.to_string_lossy().to_string();
        let attach = Command::new("hdiutil")
            .args([
                "attach",
                "-nobrowse",
                "-readonly",
                "-mountpoint",
                &mount_str,
                &path,
            ])
            .output()
            .map_err(|e| format!("hdiutil attach 失败: {e}"))?;
        if !attach.status.success() {
            let _ = fs::remove_dir(&mount_point);
            return Err(format!(
                "挂载 DMG 失败: {}",
                String::from_utf8_lossy(&attach.stderr)
            ));
        }
        // 找挂载点下第一个 .app
        let app_in_dmg = fs::read_dir(&mount_point)
            .map_err(|e| format!("读取挂载点失败: {e}"))?
            .filter_map(|e| e.ok())
            .find(|e| e.path().extension().and_then(|s| s.to_str()) == Some("app"))
            .ok_or_else(|| "DMG 内未找到 .app".to_string())?
            .path();
        let app_name = app_in_dmg
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| "解析 .app 名失败".to_string())?
            .to_string();
        let target = PathBuf::from("/Applications").join(&app_name);
        // 删除旧 app 并 cp 新的（rm -rf 失败也忽略，cp 会用 -R 覆盖）
        let _ = Command::new("rm")
            .args(["-rf", &target.to_string_lossy()])
            .status();
        let cp = Command::new("cp")
            .args([
                "-R",
                &app_in_dmg.to_string_lossy(),
                &target.to_string_lossy(),
            ])
            .status()
            .map_err(|e| format!("cp 失败: {e}"))?;
        if !cp.success() {
            let _ = Command::new("hdiutil")
                .args(["detach", "-force", &mount_str])
                .status();
            let _ = fs::remove_dir(&mount_point);
            return Err("cp 新版本到 /Applications 失败".to_string());
        }
        // 卸载 + 删除空挂载目录
        let _ = Command::new("hdiutil")
            .args(["detach", "-force", &mount_str])
            .status();
        let _ = fs::remove_dir(&mount_point);
        // 剥掉 quarantine 属性 —— DMG 文件本身带 com.apple.quarantine,挂载后 .app 继承这个属性,
        // cp 到 /Applications 后 Gatekeeper 看到 quarantine + 未公证 → 静默拦截启动。
        // xattr -rd 递归剥掉,与 README 里那条手动命令等效。
        let _ = Command::new("xattr")
            .args(["-rd", "com.apple.quarantine", &target.to_string_lossy()])
            .status();
        // open -n 强制开新实例
        let _ = Command::new("open")
            .args(["-n", &target.to_string_lossy()])
            .spawn()
            .map_err(|e| format!("open 新版本失败: {e}"))?;
        app.exit(0);
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        Command::new(&path)
            .spawn()
            .map_err(|e| format!("启动 installer 失败: {e}"))?;
        app.exit(0);
        return Ok(());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = app;
        Err("当前平台不支持自动安装".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_newer_version_handles_basic_semver() {
        assert!(is_newer_version("2.5.0", "2.4.0"));
        assert!(is_newer_version("2.4.1", "2.4.0"));
        assert!(is_newer_version("3.0.0", "2.99.99"));
        assert!(!is_newer_version("2.4.0", "2.4.0"));
        assert!(!is_newer_version("2.3.9", "2.4.0"));
        assert!(!is_newer_version("1.99.99", "2.0.0"));
    }

    #[test]
    fn is_newer_version_strips_prerelease_suffix() {
        // "1.0.0-beta" 截到第一个非数字 → 1.0.0；与 1.0.0 平等
        assert!(!is_newer_version("1.0.0-beta", "1.0.0"));
        assert!(is_newer_version("1.0.1-beta", "1.0.0"));
    }

    #[test]
    fn is_newer_version_handles_missing_patch() {
        // "2.5" 视为 2.5.0
        assert!(is_newer_version("2.5", "2.4.0"));
        assert!(!is_newer_version("2.5", "2.5.0"));
    }

    #[test]
    fn is_newer_version_handles_garbage_input() {
        // 解析失败的部分都视为 0，不 panic
        assert!(!is_newer_version("", "1.0.0"));
        assert!(is_newer_version("1.0.0", ""));
        assert!(!is_newer_version("garbage", "1.0.0"));
    }

    #[test]
    fn parse_latest_tag_from_atom_picks_first_entry() {
        let xml = r#"<?xml version="1.0"?>
<feed>
  <entry>
    <id>tag:github.com,2008:Repository/1/v2.7.5</id>
    <title>v2.7.5</title>
    <link rel="alternate" type="text/html" href="https://github.com/ZMGID/kivio/releases/tag/v2.7.5"/>
  </entry>
  <entry>
    <link rel="alternate" type="text/html" href="https://github.com/ZMGID/kivio/releases/tag/v2.7.4"/>
  </entry>
</feed>"#;
        assert_eq!(parse_latest_tag_from_atom(xml).as_deref(), Some("v2.7.5"));
    }

    #[test]
    fn parse_latest_tag_from_atom_returns_none_when_absent() {
        assert_eq!(parse_latest_tag_from_atom("<feed></feed>"), None);
        assert_eq!(parse_latest_tag_from_atom(""), None);
    }
}
