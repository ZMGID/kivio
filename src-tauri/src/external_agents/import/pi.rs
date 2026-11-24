use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{file_mtime_ms, home_dir, paths_match, truncate_title, ImportableSession};
use crate::external_agents::import_history::pi_history_preview;
use crate::external_agents::session::pi_rpc::encode_pi_session_cwd;
use crate::external_agents::types::CapabilitySupport;

pub(super) fn identity(path: &Path) -> Option<(String, String)> {
    if !path.is_file() {
        return None;
    }
    let file = File::open(path).ok()?;
    let header: Value = BufReader::new(file.take(1024 * 1024))
        .lines()
        .map_while(Result::ok)
        .find_map(|line| serde_json::from_str(line.trim_start_matches('\u{feff}')).ok())?;
    if header.get("type")?.as_str()? != "session" {
        return None;
    }
    let id = header.get("id")?.as_str()?;
    let cwd = header.get("cwd")?.as_str()?;
    if id.is_empty() || id != id.trim() || !Path::new(cwd).is_absolute() {
        return None;
    }
    Some((id.to_owned(), cwd.to_owned()))
}

fn resolve_config_path(value: &str, project_root: &Path, home: &Path) -> Option<PathBuf> {
    let path = match value {
        "" => return None,
        "~" => home.to_path_buf(),
        value if value.starts_with("~/") || (cfg!(windows) && value.starts_with("~\\")) => {
            home.join(&value[2..])
        }
        value if value.starts_with("file://") => {
            url::Url::parse(value).ok()?.to_file_path().ok()?
        }
        value => PathBuf::from(value),
    };
    Some(project_root.join(path))
}

fn configured_session_dir(settings_path: &Path) -> Option<String> {
    let raw = fs::read_to_string(settings_path).ok()?;
    let settings = serde_json::from_str::<Value>(raw.trim_start_matches('\u{feff}')).ok()?;
    settings.get("sessionDir")?.as_str().map(str::to_owned)
}

fn session_directories(
    project_root: &Path,
    home: &Path,
    env: impl Fn(&str) -> Option<String>,
) -> Vec<PathBuf> {
    let agent_dir = env("PI_CODING_AGENT_DIR")
        .and_then(|value| resolve_config_path(&value, project_root, home))
        .unwrap_or_else(|| home.join(".pi").join("agent"));
    let mut dirs: Vec<_> = [
        env("PI_CODING_AGENT_SESSION_DIR"),
        configured_session_dir(&project_root.join(".pi").join("settings.json")),
        configured_session_dir(&agent_dir.join("settings.json")),
    ]
    .into_iter()
    .flatten()
    .filter_map(|value| resolve_config_path(&value, project_root, home))
    .collect();

    let stored = agent_dir.join("sessions");
    dirs.push(stored.join(encode_pi_session_cwd(project_root)));
    // A project can have been opened through a junction or symlink with a different encoding.
    if let Ok(project_dirs) = fs::read_dir(stored) {
        let mut stored_dirs: Vec<_> = project_dirs
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        stored_dirs.sort_unstable();
        dirs.extend(stored_dirs);
    }
    let mut visited = HashSet::new();
    dirs.retain(|dir| visited.insert(super::canonical_key(&dir.to_string_lossy())));
    dirs
}

fn configured_directories(project_root: &str) -> Vec<PathBuf> {
    let Some(home) = home_dir() else {
        return Vec::new();
    };
    let overrides = crate::external_agents::overrides::env_for("pi");
    session_directories(Path::new(project_root), &home, |key| {
        overrides
            .iter()
            .find(|(name, _)| name == &key || (cfg!(windows) && name.eq_ignore_ascii_case(key)))
            .map(|(_, value)| value.clone())
            .or_else(|| std::env::var(key).ok())
    })
}

fn session_files(dirs: Vec<PathBuf>) -> impl Iterator<Item = PathBuf> {
    dirs.into_iter()
        .filter_map(|dir| fs::read_dir(dir).ok())
        .flat_map(|entries| {
            let mut paths: Vec<_> = entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl") && path.is_file())
                .collect();
            paths.sort_unstable();
            paths
        })
}

pub(super) fn source_path(project_root: &str, session_id: &str) -> Option<PathBuf> {
    find_session(
        project_root,
        session_id,
        configured_directories(project_root),
    )
}

fn find_session(project_root: &str, session_id: &str, dirs: Vec<PathBuf>) -> Option<PathBuf> {
    session_files(dirs)
        .find(|path| importable_session(path, project_root, Some(session_id)).is_some())
}

pub(super) fn list_sessions(project_root: &str) -> Vec<ImportableSession> {
    list_from_directories(project_root, configured_directories(project_root))
}

fn list_from_directories(project_root: &str, dirs: Vec<PathBuf>) -> Vec<ImportableSession> {
    let mut seen = HashSet::new();
    session_files(dirs)
        .filter_map(|path| importable_session(&path, project_root, None))
        .filter(|session| seen.insert(session.session_id.clone()))
        .collect()
}

fn importable_session(
    path: &Path,
    project_root: &str,
    expected_id: Option<&str>,
) -> Option<ImportableSession> {
    let (session_id, cwd) = identity(path)?;
    if expected_id.is_some_and(|id| session_id != id)
        || !Path::new(&cwd).is_dir()
        || !paths_match(&cwd, project_root)
    {
        return None;
    }
    let preview = pi_history_preview(&fs::read_to_string(path).ok()?)?;
    Some(ImportableSession {
        agent_id: "pi".into(),
        import_history: CapabilitySupport::Native,
        session_id,
        title: preview
            .name
            .or_else(|| preview.first_user_text.as_deref().map(truncate_title)),
        cwd,
        updated_at: file_mtime_ms(path),
        message_count: None,
        already_imported: false,
        bound_conversation_id: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write_session(dir: &Path, filename: &str, id: &str, cwd: &Path, name: &str) -> PathBuf {
        fs::create_dir_all(dir).unwrap();
        let path = dir.join(filename);
        let header = json!({"type":"session","version":3,"id":id,"cwd":cwd});
        let user = json!({"type":"message","id":"user","parentId":null,"message":{
            "role":"user","content":[{"type":"text","text":"Describe the project"}]
        }});
        let name = json!({"type":"session_info","id":"name","parentId":"user","name":name});
        fs::write(&path, format!("{header}\n{user}\n{name}\n")).unwrap();
        path
    }

    #[test]
    fn discovery_binds_by_header_id_and_original_directory() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("Project One");
        let other = root.path().join("Other Project");
        fs::create_dir(&project).unwrap();
        fs::create_dir(&other).unwrap();
        let sessions = root.path().join("shared storage");
        let selected = write_session(
            &sessions,
            "arbitrary.jsonl",
            "native.id",
            &project,
            "Native title",
        );
        write_session(
            &sessions,
            "native.id.jsonl",
            "different-cwd",
            &other,
            "Other title",
        );
        write_session(
            &sessions,
            "unrelated.jsonl",
            "native.id",
            &other,
            "Other title",
        );
        let project_root = project.to_str().unwrap();
        let dirs = vec![sessions];
        let candidates = list_from_directories(project_root, dirs.clone());
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].session_id, "native.id");
        assert_eq!(candidates[0].title.as_deref(), Some("Native title"));
        assert_eq!(candidates[0].cwd, project_root);
        assert_eq!(candidates[0].message_count, None);
        assert_eq!(
            find_session(project_root, "native.id", dirs.clone()),
            Some(selected.clone())
        );
        assert!(find_session(project_root, "native", dirs.clone()).is_none());
        fs::remove_file(&selected).unwrap();
        assert!(find_session(project_root, "native.id", dirs).is_none());
    }

    #[test]
    fn invalid_duplicate_does_not_hide_the_active_session() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let empty_store = root.path().join("empty");
        let corrupt_store = root.path().join("corrupt");
        let active_store = root.path().join("active");
        for dir in [&project, &empty_store, &corrupt_store, &active_store] {
            fs::create_dir(dir).unwrap();
        }
        let header = json!({"type":"session","version":2,"id":"duplicate","cwd":project});
        fs::write(empty_store.join("empty.jsonl"), format!("{header}\n")).unwrap();
        let corrupt = json!({"type":"message","id":"broken","parentId":"missing","message":{
            "role":"user","content":"Corrupt"
        }});
        fs::write(
            corrupt_store.join("corrupt.jsonl"),
            format!("{header}\n{corrupt}\n"),
        )
        .unwrap();
        let selected = active_store.join("actual.jsonl");
        let old = json!({"type":"message","id":"old","parentId":null,"message":{
            "role":"user","content":"Old branch"
        }});
        let active = json!({"type":"message","id":"active","parentId":null,"message":{
            "role":"user","content":"Active branch"
        }});
        fs::write(&selected, format!("{header}\n{old}\n{active}\n")).unwrap();

        let project_root = project.to_str().unwrap();
        let dirs = vec![empty_store, corrupt_store, active_store];
        let candidates = list_from_directories(project_root, dirs.clone());
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].title.as_deref(), Some("Active branch"));
        assert_eq!(
            find_session(project_root, "duplicate", dirs),
            Some(selected)
        );
    }

    #[test]
    fn discovery_lists_environment_settings_and_default_session_storage() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path();
        let project = home.join("workspace");
        let agent = home.join("agent");
        fs::create_dir_all(project.join(".pi")).unwrap();
        fs::create_dir(&agent).unwrap();
        fs::write(
            project.join(".pi/settings.json"),
            r#"{"sessionDir":"./sessions"}"#,
        )
        .unwrap();
        fs::write(
            agent.join("settings.json"),
            r#"{"sessionDir":"~/global sessions"}"#,
        )
        .unwrap();
        let default_dir = agent.join("sessions").join(encode_pi_session_cwd(&project));
        for (dir, id) in [
            (home.join("env sessions"), "env"),
            (project.join("sessions"), "project"),
            (home.join("global sessions"), "global"),
            (default_dir, "default"),
        ] {
            write_session(&dir, "pi-session.jsonl", id, &project, "");
        }
        let dirs = session_directories(&project, home, |key| match key {
            "PI_CODING_AGENT_DIR" => Some("~/agent".into()),
            "PI_CODING_AGENT_SESSION_DIR" => Some("~/env sessions".into()),
            _ => None,
        });
        let candidates = list_from_directories(project.to_str().unwrap(), dirs);
        let ids: Vec<_> = candidates
            .iter()
            .map(|item| item.session_id.as_str())
            .collect();
        assert_eq!(ids, ["env", "project", "global", "default"]);
        assert_eq!(candidates[0].title.as_deref(), Some("Describe the project"));
    }
}
