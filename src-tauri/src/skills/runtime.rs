use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Component, Path, PathBuf},
    process::Stdio,
};

use serde_json::Value;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use super::{
    discover::build_registry,
    types::{SkillFileKind, SkillRecord, SkillRegistry},
};
use tauri::AppHandle;

#[derive(Default)]
pub struct SkillRunCache {
    activated: HashSet<String>,
    read_files: HashMap<(String, String), String>,
}

impl SkillRunCache {
    pub fn activate_with_cache(&mut self, record: &SkillRecord) -> String {
        let key = record.meta.name.clone();
        if self.activated.contains(&key) {
            return format!(
                "Skill \"{}\" is already active in this run.\nSkill directory: {}",
                record.meta.name,
                record.base_dir.display()
            );
        }
        self.activated.insert(key);
        activate_skill(record)
    }

    pub fn read_file_with_cache(
        &mut self,
        record: &SkillRecord,
        relative_path: &str,
    ) -> Result<String, String> {
        let normalized = relative_path.trim().replace('\\', "/");
        let key = (record.meta.name.clone(), normalized.clone());
        if let Some(cached) = self.read_files.get(&key) {
            return Ok(format!("[cached]\n{cached}"));
        }
        let content = read_skill_file(record, &normalized)?;
        self.read_files.insert(key, content.clone());
        Ok(content)
    }
}

pub fn resolve_skill_path(base_dir: &Path, relative_path: &str) -> Result<PathBuf, String> {
    let relative = relative_path.trim().replace('\\', "/");
    if relative.is_empty() {
        return Err("Relative path is empty".to_string());
    }
    if relative.contains("..") {
        return Err("Relative path must not contain ..".to_string());
    }

    let mut candidate = PathBuf::new();
    for component in Path::new(&relative).components() {
        match component {
            Component::Normal(part) => candidate.push(part),
            Component::CurDir => {}
            _ => return Err("Invalid path component".to_string()),
        }
    }

    let joined = base_dir.join(candidate);
    let canonical_base = fs::canonicalize(base_dir)
        .map_err(|err| format!("Resolve skill base dir failed: {err}"))?;
    let canonical_joined = if joined.exists() {
        fs::canonicalize(&joined).map_err(|err| format!("Resolve skill path failed: {err}"))?
    } else {
        let parent = joined
            .parent()
            .ok_or_else(|| "Invalid skill path".to_string())?;
        let canonical_parent = fs::canonicalize(parent)
            .map_err(|err| format!("Resolve skill parent failed: {err}"))?;
        let file_name = joined
            .file_name()
            .ok_or_else(|| "Invalid skill path".to_string())?;
        canonical_parent.join(file_name)
    };

    if !canonical_joined.starts_with(&canonical_base) {
        return Err("Skill path escapes skill directory".to_string());
    }
    Ok(canonical_joined)
}

pub fn activate_skill(record: &SkillRecord) -> String {
    let mut out = format!(
        "<skill_content name=\"{}\">\n",
        xml_escape(&record.meta.name)
    );
    out.push_str(&record.body);
    out.push_str("\n\nSkill directory: ");
    out.push_str(&record.base_dir.display().to_string());
    out.push_str("\nRelative paths in this skill are relative to the skill directory.\n");

    if !record.meta.files.is_empty() {
        out.push_str("\n<skill_resources>\n");
        for file in &record.meta.files {
            out.push_str(&format!(
                "  <file kind=\"{}\">{}</file>\n",
                skill_file_kind_label(file.kind),
                xml_escape(&file.relative_path)
            ));
        }
        out.push_str("</skill_resources>\n");
    }
    out.push_str("</skill_content>");
    out
}

pub fn read_skill_file(record: &SkillRecord, relative_path: &str) -> Result<String, String> {
    let path = resolve_skill_path(&record.base_dir, relative_path)?;
    if !path.is_file() {
        return Err(format!("Skill file not found: {relative_path}"));
    }
    let bytes = fs::read(&path).map_err(|err| format!("Read skill file failed: {err}"))?;
    let cap = crate::native_tools::MAX_READ_FILE_BYTES as usize;
    if bytes.len() <= cap {
        return Ok(String::from_utf8_lossy(&bytes).into_owned());
    }
    // Decode lossily, then keep the head up to a UTF-8 char boundary at or below
    // the byte cap, and append a marker pointing the model at skill_run_script.
    let text = String::from_utf8_lossy(&bytes);
    let mut head_len = cap.min(text.len());
    while head_len > 0 && !text.is_char_boundary(head_len) {
        head_len -= 1;
    }
    let head = &text[..head_len];
    Ok(format!(
        "{head}\n\n[Skill file truncated: original {} bytes, showing first {} bytes (max {cap}). Use skill_run_script to process the full file.]",
        bytes.len(),
        head_len
    ))
}

pub async fn run_skill_script(
    record: &SkillRecord,
    relative_path: &str,
    args: &[String],
    timeout_ms: u64,
    allowlist: &[String],
) -> Result<String, String> {
    let normalized = relative_path.trim().replace('\\', "/");
    if !normalized.starts_with("scripts/") {
        return Err("skill_run_script only allows files under scripts/".to_string());
    }

    let path = resolve_skill_path(&record.base_dir, &normalized)?;
    if !path.is_file() {
        return Err(format!("Script not found: {normalized}"));
    }

    let (program, script_args) = build_script_command(&path, args, allowlist)?;
    let mut command = Command::new(&program);
    command
        .args(script_args)
        .current_dir(&record.base_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        command.kill_on_drop(true);
    }

    let child = command
        .spawn()
        .map_err(|err| format!("Script execution failed: {err}"))?;
    let output = match timeout(Duration::from_millis(timeout_ms), child.wait_with_output()).await {
        Ok(result) => result.map_err(|err| format!("Script execution failed: {err}"))?,
        Err(_) => {
            return Err(format!("Script execution timed out after {timeout_ms}ms"));
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut content = String::new();
    if !stdout.trim().is_empty() {
        content.push_str("stdout:\n");
        content.push_str(stdout.trim());
    }
    if !stderr.trim().is_empty() {
        if !content.is_empty() {
            content.push('\n');
        }
        content.push_str("stderr:\n");
        content.push_str(stderr.trim());
    }
    if content.is_empty() {
        content = format!("Script exited with status {}", output.status);
    }
    if !output.status.success() {
        return Err(content);
    }
    Ok(content)
}

fn build_script_command(
    path: &Path,
    args: &[String],
    allowlist: &[String],
) -> Result<(String, Vec<String>), String> {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let program = match ext.as_str() {
        "py" => "python3",
        "js" | "mjs" | "cjs" => "node",
        "sh" => "bash",
        "" => "bash",
        _ if path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.ends_with(".sh"))
            .unwrap_or(false) =>
        {
            "bash"
        }
        _ => return Err(format!("Unsupported script extension: {ext}")),
    };

    if !allowlist.iter().any(|item| item == program) {
        return Err(format!("Script interpreter {program} is not allowed"));
    }

    let mut script_args = vec![path.display().to_string()];
    script_args.extend(args.iter().cloned());
    Ok((program.to_string(), script_args))
}

pub fn lookup_skill<'a>(registry: &'a SkillRegistry, name: &str) -> Option<&'a SkillRecord> {
    registry.find(name)
}

pub fn build_registry_for_app(
    app: &AppHandle,
    scan_paths: &[String],
) -> Result<SkillRegistry, String> {
    build_registry(app, scan_paths)
}

pub fn extract_skill_name(arguments: &Value) -> Result<String, String> {
    arguments
        .get("name")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| "Skill name is required".to_string())
}

pub fn extract_relative_path(arguments: &Value) -> Result<String, String> {
    arguments
        .get("relative_path")
        .or_else(|| arguments.get("relativePath"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| "relative_path is required".to_string())
}

pub fn extract_script_args(arguments: &Value) -> Vec<String> {
    arguments
        .get("args")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::trim).filter(|s| !s.is_empty()))
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn skill_file_kind_label(kind: SkillFileKind) -> &'static str {
    match kind {
        SkillFileKind::SkillMd => "skillmd",
        SkillFileKind::Reference => "reference",
        SkillFileKind::Script => "script",
        SkillFileKind::Asset => "asset",
        SkillFileKind::Other => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::{SkillMeta, SkillRecord};
    use super::*;
    use std::{fs, path::PathBuf};

    fn demo_meta() -> SkillMeta {
        SkillMeta {
            id: "demo".to_string(),
            name: "demo".to_string(),
            description: "Demo".to_string(),
            source: "user".to_string(),
            path: None,
            recommended_tools: vec![],
            disable_model_invocation: false,
            files: vec![],
        }
    }

    #[test]
    fn resolve_skill_path_rejects_parent_traversal() {
        let dir = std::env::temp_dir().join(format!("kivio-skill-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let err = resolve_skill_path(&dir, "../secret.txt").expect_err("should reject");
        assert!(err.contains(".."));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn resolve_skill_path_allows_nested_file() {
        let dir = std::env::temp_dir().join(format!("kivio-skill-test-{}", uuid::Uuid::new_v4()));
        let nested = dir.join("references");
        fs::create_dir_all(&nested).unwrap();
        let file = nested.join("guide.md");
        fs::write(&file, "hello").unwrap();
        let resolved = resolve_skill_path(&dir, "references/guide.md").unwrap();
        assert_eq!(resolved, fs::canonicalize(&file).unwrap());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn skill_run_cache_deduplicates_activate() {
        let record = SkillRecord {
            meta: SkillMeta {
                id: "demo".to_string(),
                name: "demo".to_string(),
                description: "Demo".to_string(),
                source: "user".to_string(),
                path: None,
                recommended_tools: vec![],
                disable_model_invocation: false,
                files: vec![],
            },
            location: PathBuf::from("/skills/demo/SKILL.md"),
            base_dir: PathBuf::from("/skills/demo"),
            body: "Skill body".to_string(),
            allowed_tools: vec![],
        };
        let mut cache = SkillRunCache::default();
        let first = cache.activate_with_cache(&record);
        assert!(first.contains("Skill body"));
        let second = cache.activate_with_cache(&record);
        assert!(second.contains("already active"));
        assert!(!second.contains("Skill body"));
    }

    #[test]
    fn skill_run_cache_deduplicates_read_file() {
        let dir = std::env::temp_dir().join(format!("kivio-skill-cache-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("guide.md");
        fs::write(&file, "cached content").unwrap();
        let record = SkillRecord {
            meta: SkillMeta {
                id: "demo".to_string(),
                name: "demo".to_string(),
                description: "Demo".to_string(),
                source: "user".to_string(),
                path: None,
                recommended_tools: vec![],
                disable_model_invocation: false,
                files: vec![],
            },
            location: dir.join("SKILL.md"),
            base_dir: dir.clone(),
            body: String::new(),
            allowed_tools: vec![],
        };
        let mut cache = SkillRunCache::default();
        let first = cache.read_file_with_cache(&record, "guide.md").unwrap();
        assert_eq!(first, "cached content");
        let second = cache.read_file_with_cache(&record, "guide.md").unwrap();
        assert!(second.starts_with("[cached]"));
        assert!(second.contains("cached content"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn skill_run_script_rejects_paths_outside_scripts_dir() {
        let dir = std::env::temp_dir().join(format!("kivio-skill-script-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let record = SkillRecord {
            meta: SkillMeta {
                id: "demo".to_string(),
                name: "demo".to_string(),
                description: "Demo".to_string(),
                source: "user".to_string(),
                path: None,
                recommended_tools: vec![],
                disable_model_invocation: false,
                files: vec![],
            },
            location: dir.join("SKILL.md"),
            base_dir: dir.clone(),
            body: String::new(),
            allowed_tools: vec![],
        };
        let err = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(run_skill_script(
                &record,
                "references/guide.md",
                &[],
                1_000,
                &["python3".to_string()],
            ))
            .expect_err("should reject non-scripts path");
        assert!(err.contains("scripts/"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn skill_run_script_reports_timeout() {
        let dir =
            std::env::temp_dir().join(format!("kivio-skill-timeout-{}", uuid::Uuid::new_v4()));
        let scripts_dir = dir.join("scripts");
        fs::create_dir_all(&scripts_dir).unwrap();
        fs::write(scripts_dir.join("slow.py"), "import time\ntime.sleep(2)\n").unwrap();
        let record = SkillRecord {
            meta: SkillMeta {
                id: "demo".to_string(),
                name: "demo".to_string(),
                description: "Demo".to_string(),
                source: "user".to_string(),
                path: None,
                recommended_tools: vec![],
                disable_model_invocation: false,
                files: vec![],
            },
            location: dir.join("SKILL.md"),
            base_dir: dir.clone(),
            body: String::new(),
            allowed_tools: vec![],
        };
        let err = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(run_skill_script(
                &record,
                "scripts/slow.py",
                &[],
                100,
                &["python3".to_string()],
            ))
            .expect_err("should time out");
        assert!(err.contains("timed out after 100ms"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn read_skill_file_returns_full_when_small() {
        let dir = std::env::temp_dir().join(format!("kivio-skill-read-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("note.md"), "hello world").unwrap();
        let record = SkillRecord {
            meta: demo_meta(),
            location: dir.join("SKILL.md"),
            base_dir: dir.clone(),
            body: String::new(),
            allowed_tools: vec![],
        };
        let content = read_skill_file(&record, "note.md").unwrap();
        assert_eq!(content, "hello world");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn read_skill_file_caps_oversize() {
        let dir = std::env::temp_dir().join(format!("kivio-skill-cap-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let cap = crate::native_tools::MAX_READ_FILE_BYTES as usize;
        let big = "a".repeat(cap + 1024);
        fs::write(dir.join("big.txt"), &big).unwrap();
        let record = SkillRecord {
            meta: demo_meta(),
            location: dir.join("SKILL.md"),
            base_dir: dir.clone(),
            body: String::new(),
            allowed_tools: vec![],
        };
        let content = read_skill_file(&record, "big.txt").unwrap();
        assert!(content.contains("Skill file truncated"));
        assert!(content.contains("skill_run_script"));
        assert!(content.len() < big.len());
        let _ = fs::remove_dir_all(dir);
    }
}
