use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::agents::AgentDefinition;
use serde_json::Value;

pub(crate) fn is_bundled(definition: &AgentDefinition) -> bool {
    definition.id == "seance" && definition.name == "seance" && definition.source == "builtin"
}

pub(crate) struct SeanceSource {
    pub file: PathBuf,
    pub id: String,
    pub model_selector: String,
    pub entries: Vec<Value>,
    pub history: Vec<Value>,
    nested_directory: PathBuf,
}

pub(crate) fn resolve_source(
    selector: &str,
    cwd: &Path,
    artifacts: &Path,
) -> Result<SeanceSource, String> {
    let selector = selector.trim();
    if selector.is_empty() {
        return Err("A source session is required.".into());
    }
    let candidate = Path::new(selector);
    let file = if candidate.is_absolute()
        || selector.contains(['/', '\\'])
        || selector.ends_with(".jsonl")
        || selector.ends_with(".json")
    {
        if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            cwd.join(candidate)
        }
    } else {
        let mut matches = Vec::new();
        for entry in read_directory_if_present(artifacts)? {
            let path = entry
                .map_err(|e| format!("Cannot list historical sessions: {e}"))?
                .path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("jsonl") {
                continue;
            }
            if path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|id| id.to_lowercase().starts_with(&selector.to_lowercase()))
            {
                matches.push(path);
            }
        }
        let exact: Vec<_> = matches
            .iter()
            .filter(|path| {
                path.file_stem()
                    .and_then(|stem| stem.to_str())
                    .is_some_and(|id| id.eq_ignore_ascii_case(selector))
            })
            .cloned()
            .collect();
        if !exact.is_empty() {
            matches = exact;
        }
        if matches.is_empty() {
            matches = conversation_sources(selector, artifacts)?;
        }
        if matches.len() != 1 {
            return Err(if matches.is_empty() {
                "Historical session was not found."
            } else {
                "Historical session selector is ambiguous."
            }
            .into());
        }
        matches.pop().expect("one source")
    };
    read_source(&file)
}

pub(crate) fn read_source(file: &Path) -> Result<SeanceSource, String> {
    let file =
        fs::canonicalize(file).map_err(|e| format!("Cannot resolve historical session: {e}"))?;
    if !file.is_file() {
        return Err("Historical source is not a regular file.".into());
    }
    let raw =
        fs::read_to_string(&file).map_err(|e| format!("Cannot read historical session: {e}"))?;
    if file.extension().and_then(|extension| extension.to_str()) == Some("json") {
        let conversation: crate::chat::types::Conversation = serde_json::from_str(&raw)
            .map_err(|e| format!("Historical conversation is corrupt: {e}"))?;
        crate::chat::storage::validate_conversation_id(&conversation.id)?;
        if conversation.provider_id.is_empty() || conversation.model.is_empty() {
            return Err("Historical conversation has no saved model.".into());
        }
        let history = crate::chat::commands::context::build_chat_api_messages_with_video(
            None,
            "",
            &conversation,
            None,
            None,
            &[],
            false,
        )?;
        let model_selector = format!("{}/{}", conversation.provider_id, conversation.model);
        let nested_directory = file
            .parent()
            .filter(|parent| {
                parent.file_name().and_then(|name| name.to_str()) == Some("conversations")
            })
            .and_then(Path::parent)
            .map(|appdata| appdata.join("subagents").join(&conversation.id))
            .unwrap_or_else(|| file.with_extension("subagents"));
        return Ok(SeanceSource {
            file,
            id: conversation.id,
            model_selector,
            entries: vec![serde_json::from_str(&raw).map_err(|e| e.to_string())?],
            history,
            nested_directory,
        });
    }
    let entries: Vec<Value> = raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
        .map(|(index, line)| {
            serde_json::from_str(line)
                .map_err(|e| format!("Historical session line {} is corrupt: {e}", index + 1))
        })
        .collect::<Result<_, _>>()?;
    let init = entries
        .iter()
        .find(|entry| entry["type"] == "session_init")
        .ok_or("Historical source has no session contract.")?;
    let id = init["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("Historical source has no identity.")?
        .to_string();
    let provider = init["resolvedModel"]["provider"]
        .as_str()
        .filter(|provider| !provider.is_empty())
        .ok_or("Historical source has no saved provider.")?;
    let model = init["resolvedModel"]["model"]
        .as_str()
        .filter(|model| !model.is_empty())
        .ok_or("Historical source has no saved model.")?;
    let model_selector = format!("{provider}/{model}");
    let root = init["root"]
        .as_str()
        .filter(|root| !root.is_empty())
        .ok_or("Historical source has no root identity.")?;
    let history = super::session::replay_for_consult(raw.as_bytes(), root, &id)?.history;
    let nested_directory = file
        .parent()
        .ok_or("Historical source has no artifact directory.")?
        .to_path_buf();
    Ok(SeanceSource {
        file,
        id,
        model_selector,
        entries,
        history,
        nested_directory,
    })
}

fn conversation_sources(selector: &str, artifacts: &Path) -> Result<Vec<PathBuf>, String> {
    let Some(appdata) = artifacts
        .parent()
        .filter(|parent| parent.file_name().and_then(|name| name.to_str()) == Some("subagents"))
        .and_then(Path::parent)
    else {
        return Ok(Vec::new());
    };
    let directory = appdata.join("conversations");
    let mut matches = Vec::new();
    for entry in
        fs::read_dir(directory).map_err(|e| format!("Cannot list historical conversations: {e}"))?
    {
        let path = entry
            .map_err(|e| format!("Cannot list historical conversations: {e}"))?
            .path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        if !path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(|id| id.to_lowercase().starts_with(&selector.to_lowercase()))
        {
            continue;
        }
        let candidate: crate::chat::types::Conversation =
            serde_json::from_slice(&fs::read(&path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("Historical conversation is corrupt: {e}"))?;
        crate::chat::storage::validate_conversation_id(&candidate.id)?;
        if path.file_stem().and_then(|stem| stem.to_str()) != Some(candidate.id.as_str()) {
            return Err("Historical conversation identity does not match its filename.".into());
        }
        matches.push(path);
    }
    let exact: Vec<_> = matches
        .iter()
        .filter(|path| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|id| id.eq_ignore_ascii_case(selector))
        })
        .cloned()
        .collect();
    Ok(if exact.is_empty() { matches } else { exact })
}

fn read_directory_if_present(
    directory: &Path,
) -> Result<Vec<Result<fs::DirEntry, std::io::Error>>, String> {
    match fs::read_dir(directory) {
        Ok(entries) => Ok(entries.collect()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(format!("Cannot list historical sources: {error}")),
    }
}

pub(crate) fn fork_source(
    source: &SeanceSource,
    destination: &Path,
) -> Result<Vec<PathBuf>, String> {
    if destination.exists() {
        return Err("Historical fork destination already exists.".into());
    }
    fs::create_dir(destination).map_err(|e| format!("Cannot create historical fork: {e}"))?;
    let result = (|| {
        let mut files = vec![source.file.clone()];
        for entry in read_directory_if_present(&source.nested_directory)? {
            let path = entry
                .map_err(|e| format!("Cannot read nested historical sessions: {e}"))?
                .path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("jsonl")
                || path == source.file
            {
                continue;
            }
            let main = source
                .file
                .extension()
                .and_then(|extension| extension.to_str())
                == Some("json");
            if !main
                && !path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .is_some_and(|id| id.starts_with(&format!("{}.", source.id)))
            {
                continue;
            }
            let nested = read_source(&path)?;
            if main || nested.id.starts_with(&format!("{}.", source.id)) {
                files.push(path);
            }
        }
        let mut copied = Vec::new();
        for path in files {
            let target = destination.join(
                path.file_name()
                    .ok_or("Historical source has no filename.")?,
            );
            let bytes =
                fs::read(&path).map_err(|e| format!("Cannot read historical evidence: {e}"))?;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
                .map_err(|e| format!("Cannot create historical evidence: {e}"))?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|e| format!("Cannot save historical evidence: {e}"))?;
            copied.push(target);
        }
        Ok(copied)
    })();
    if let Err(error) = &result {
        return Err(format!(
            "{error} Partial fork retained at {}.",
            destination.display()
        ));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn journal(dir: &Path, id: &str) -> PathBuf {
        let path = dir.join(format!("{id}.jsonl"));
        let init = json!({"type":"session_init","version":1,"id":id,"root":"conv_root","parentId":"Main","depth":1,"generation":1,
            "conversationId":format!("subagent-conv_root-{id}"),"cwd":dir,"systemPrompt":"system","runtimeContext":"","task":"question",
            "agentDefinition":crate::agents::builtin_agent_definitions().remove(0),"tools":[],"readOnly":true,
            "resolvedModel":{"provider":"test","model":"model"},"thinkingEnabled":false,"resolvedThinkingLevel":null,
            "approvalPolicy":"yolo","planState":crate::chat::types::AgentPlanState::default()});
        let entries = [
            init,
            json!({"role":"system","content":"system"}),
            json!({"role":"user","content":"question"}),
            json!({"role":"assistant","content":"old answer"}),
        ];
        let bytes: Vec<u8> = entries
            .iter()
            .flat_map(|entry| {
                let mut bytes = serde_json::to_vec(entry).unwrap();
                bytes.push(b'\n');
                bytes
            })
            .collect();
        fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn selection_rejects_ambiguity_fork_copies_nested_without_touching_sources() {
        let dir = tempfile::tempdir().unwrap();
        let path = journal(dir.path(), "Alpha");
        journal(dir.path(), "Alpha.Child");
        journal(dir.path(), "Alpine");
        let before = fs::read(&path).unwrap();
        assert!(resolve_source("Al", dir.path(), dir.path()).is_err());
        let source = resolve_source("Alpha", dir.path(), dir.path()).unwrap();
        assert_eq!(source.model_selector, "test/model");
        assert_eq!(source.entries.len(), 4);
        assert_eq!(source.history.len(), 3);
        let copied = fork_source(&source, &dir.path().join("fork")).unwrap();
        assert_eq!(copied.len(), 2);
        assert_eq!(fs::read(path).unwrap(), before);
        assert!(copied.iter().all(|path| path.is_file()));
    }

    #[test]
    fn main_conversation_uses_shared_history_builder_and_copies_its_children() {
        let dir = tempfile::tempdir().unwrap();
        let conversations = dir.path().join("conversations");
        let artifacts = dir.path().join("subagents/conv_source");
        fs::create_dir(&conversations).unwrap();
        fs::create_dir_all(&artifacts).unwrap();
        let path = conversations.join("conv_source.json");
        let value = json!({"id":"conv_source","title":"Source","provider_id":"test","model":"model","created_at":1,"updated_at":2,
            "messages":[{"id":"u1","role":"user","content":"original question","timestamp":1},{"id":"a1","role":"assistant","content":"saved answer","timestamp":2}]});
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        journal(&artifacts, "Child");
        let source = resolve_source("conv_source", dir.path(), &artifacts).unwrap();
        assert_eq!(source.id, "conv_source");
        assert!(source
            .history
            .iter()
            .any(|message| message["content"] == "saved answer"));
        let before = fs::read(&path).unwrap();
        let copied = fork_source(&source, &dir.path().join("fork")).unwrap();
        assert_eq!(copied.len(), 2);
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn corrupt_source_is_rejected_and_custom_seance_is_ordinary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.jsonl");
        fs::write(&path, "not json\n").unwrap();
        assert!(read_source(&path).is_err());
        let custom =
            crate::agents::parse::parse_agent_markdown("seance", "body", "project", None).unwrap();
        assert!(!is_bundled(&custom));
    }
}
