//! Shared Skill package download and extraction for both desktop hosts.
use super::{parse::parse_skill_markdown, types::SkillMeta};
use std::{
    fs,
    io::{Cursor, Read},
    path::Path,
};
const MAX_SKILL_DOWNLOAD_BYTES: u64 = 50 * 1024 * 1024;

pub(crate) fn normalize_skill_download_url(url: &str) -> String {
    let trimmed = url.trim();
    if let Ok(parsed) = reqwest::Url::parse(trimmed) {
        if parsed.host_str() == Some("github.com") {
            let segs: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();
            if segs.len() >= 2 {
                let owner = segs[0];
                let repo = segs[1].trim_end_matches(".git");
                let git_ref = if segs.len() >= 4 && segs[2] == "tree" {
                    segs[3]
                } else {
                    "HEAD"
                };
                return format!("https://codeload.github.com/{owner}/{repo}/zip/{git_ref}");
            }
        }
    }
    trimmed.to_string()
}

pub async fn download_skill_zip_into(url: &str, skills_dir: &Path) -> Result<SkillMeta, String> {
    download_skill_zip_into_checked(url, skills_dir, &[]).await
}
pub async fn download_skill_zip_into_checked(
    url: &str,
    skills_dir: &Path,
    expected_ids: &[&str],
) -> Result<SkillMeta, String> {
    let download_url = normalize_skill_download_url(url);
    let client = crate::api::build_http_client();
    let mut response = client
        .get(&download_url)
        .header(reqwest::header::USER_AGENT, "kivio-skill-market")
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|err| format!("Download failed: {err}"))?;
    if !response.status().is_success() {
        return Err(format!("Download failed: HTTP {}", response.status()));
    }
    if let Some(len) = response.content_length() {
        if len > MAX_SKILL_DOWNLOAD_BYTES {
            return Err("Skill package too large (over 50MB)".to_string());
        }
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| format!("Read download failed: {err}"))?
    {
        if bytes.len().saturating_add(chunk.len()) as u64 > MAX_SKILL_DOWNLOAD_BYTES {
            return Err("Skill package too large (over 50MB)".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let directory = skills_dir.to_path_buf();
    let expected = expected_ids
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>();
    tokio::task::spawn_blocking(move || install_checked(bytes, &directory, &expected))
        .await
        .map_err(|error| format!("Skill extraction task failed: {error}"))?
}

pub fn install_skill_zip_bytes(bytes: Vec<u8>, skills_dir: &Path) -> Result<SkillMeta, String> {
    install_checked(bytes, skills_dir, &[])
}
fn install_checked(
    bytes: Vec<u8>,
    skills_dir: &Path,
    expected: &[String],
) -> Result<SkillMeta, String> {
    static INSTALL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = INSTALL
        .lock()
        .map_err(|_| "Skill installation lock unavailable")?;
    let reader = Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(reader).map_err(|err| format!("Open zip failed: {err}"))?;
    let mut total = 0u64;
    for index in 0..archive.len() {
        let file = archive.by_index(index).map_err(|error| error.to_string())?;
        total = total.saturating_add(file.size());
        if total > 200 * 1024 * 1024 {
            return Err("Expanded Skill package exceeds 200MB".into());
        }
    }
    let mut skill_raw = String::new();
    let mut skill_path = String::new();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|err| err.to_string())?;
        let name = file.name().map_err(|err| err.to_string())?.into_owned();
        if Path::new(&name)
            .file_name()
            .is_some_and(|name| name == "SKILL.md")
        {
            if file.size() > 1024 * 1024 {
                return Err("SKILL.md exceeds 1MB".into());
            }
            file.read_to_string(&mut skill_raw)
                .map_err(|err| format!("Read SKILL.md in zip failed: {err}"))?;
            skill_path = name;
            break;
        }
    }
    if skill_raw.trim().is_empty() {
        return Err("Zip does not contain SKILL.md".to_string());
    }
    let parsed = parse_skill_markdown(&skill_raw, "user", None, Vec::new())?;
    if !expected.is_empty() && !expected.contains(&parsed.meta.id) {
        return Err("下载的插件 Skill 与官方目录声明不匹配".into());
    }
    fs::create_dir_all(skills_dir).map_err(|error| error.to_string())?;
    let stage = tempfile::Builder::new()
        .prefix(".skill-install-")
        .tempdir_in(skills_dir)
        .map_err(|error| error.to_string())?;
    let unpacked = stage.path().join("new");
    extract_zip_into(&mut archive, &skill_path, &unpacked)?;
    let dest = skills_dir.join(&parsed.meta.id);
    let backup = stage.path().join("previous");
    let existed = dest.exists();
    if existed {
        fs::rename(&dest, &backup)
            .map_err(|error| format!("Preserve previous Skill failed: {error}"))?;
    }
    if let Err(error) = fs::rename(&unpacked, &dest) {
        if existed {
            if let Err(rollback) = fs::rename(&backup, &dest) {
                let recovery = stage.keep();
                return Err(format!("Install failed: {error}; restore failed: {rollback}. Previous Skill preserved at {}", recovery.join("previous").display()));
            }
        }
        return Err(format!("Install failed; previous Skill preserved: {error}"));
    }
    Ok(SkillMeta {
        path: Some(dest.join("SKILL.md").display().to_string()),
        ..parsed.meta
    })
}

fn extract_zip_into<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    skill_path: &str,
    dest: &Path,
) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|err| format!("create skill dir failed: {err}"))?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|err| err.to_string())?;
        if file.is_dir() {
            continue;
        }
        let name = file.name().map_err(|err| err.to_string())?;
        let relative = if skill_path.contains('/') {
            let prefix = skill_path
                .rsplit_once('/')
                .map(|(prefix, _)| format!("{prefix}/"))
                .unwrap_or_default();
            match name.strip_prefix(&prefix) {
                Some(relative) => relative,
                None => continue,
            }
        } else {
            &name
        };
        let relative = Path::new(relative);
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            return Err("Skill archive contains an unsafe path".into());
        }
        if file
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err("Skill archive contains a symbolic link".into());
        }
        let out = dest.join(relative);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let mut output = fs::File::create(&out).map_err(|err| err.to_string())?;
        std::io::copy(&mut file, &mut output).map_err(|err| err.to_string())?;
    }
    Ok(())
}
