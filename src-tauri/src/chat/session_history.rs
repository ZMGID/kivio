use std::collections::{HashMap, HashSet};

use kivio_agent_sdk::agent::session::CompactionPlan;
use kivio_agent_sdk::agent::{
    EntryId, EntryRange, SessionEntryKind, SessionHistory, SessionSnapshot,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::agent::history::RuntimeMessage;
use super::commands::context::{
    active_summary, group_answer_excluded_from_context, summary_message,
};
use super::model::{MessagePart, ModelRole};
use super::types::{ChatMessage, Conversation, ConversationContextState};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuiltinHistory {
    pub snapshot: SessionSnapshot,
    pub active_branch_id: String,
    pub bindings: HashMap<String, Option<EntryRange>>,
    #[serde(default)]
    pub archived_branches: Vec<ArchivedBuiltinBranch>,
    pub migration: BuiltinHistoryMigration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuiltinHistoryMigration {
    #[serde(deserialize_with = "migration_version")]
    pub version: u32,
    pub source_revision: u64,
    pub ui_fallback_message_ids: Vec<String>,
}

fn migration_version<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    let version = u32::deserialize(deserializer)?;
    if version != 1 {
        return Err(serde::de::Error::custom(format!(
            "Unsupported built-in history migration version {version}"
        )));
    }
    Ok(version)
}

/// Only explicit navigation archives a UI view; model facts remain in the SDK tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchivedBuiltinBranch {
    pub id: String,
    pub leaf: Option<EntryId>,
    pub messages: Vec<ChatMessage>,
    pub group_selections: HashMap<String, String>,
    pub bindings: HashMap<String, Option<EntryRange>>,
    pub context_state: ConversationContextState,
}

pub(crate) fn history(conversation: &Conversation) -> Result<SessionHistory, String> {
    conversation
        .builtin_history
        .as_ref()
        .map(|history| history.snapshot.clone().into_history())
        .ok_or_else(|| "Built-in session history has not been migrated".into())
}

fn tag(mut message: RuntimeMessage, ui_id: &str) -> RuntimeMessage {
    message
        .source
        .insert("_ui_message_id".into(), Value::String(ui_id.into()));
    message
}

/// The compatibility decoder is permissive. Migration must reject a payload it would drop.
pub(crate) fn import_legacy(value: &Value) -> Result<RuntimeMessage, String> {
    let role = value
        .get("role")
        .and_then(Value::as_str)
        .ok_or("Missing legacy message role")?;
    if !matches!(role, "system" | "user" | "assistant" | "developer" | "tool") {
        return Err(format!("Unsupported legacy message role: {role}"));
    }
    if let Some(content) = value.get("anthropic_content") {
        serde_json::from_value::<Vec<MessagePart>>(content.clone()).map_err(|e| e.to_string())?;
    }
    if let Some(content) = value.get("content") {
        match content {
            Value::Null | Value::String(_) => {}
            Value::Array(parts) if role != "tool" && role != "system" => {
                for part in parts {
                    let probe = serde_json::json!({"role":"user", "content":[part]});
                    let parsed =
                        RuntimeMessage::from_legacy(probe).ok_or("Invalid legacy content")?;
                    if parsed.model_message().is_none_or(|m| m.content.len() != 1) {
                        return Err(format!(
                            "Unsupported legacy content: {}",
                            part.get("type").unwrap_or(&Value::Null)
                        ));
                    }
                }
            }
            _ => return Err("Unsupported legacy message content".into()),
        }
    }
    if role == "tool"
        && (value["tool_call_id"].as_str().is_none_or(str::is_empty)
            || !value["content"].is_string())
    {
        return Err("Invalid legacy tool result".into());
    }
    if let Some(artifacts) = value.get("artifacts") {
        serde_json::from_value::<Vec<super::model::ModelArtifact>>(artifacts.clone())
            .map_err(|e| e.to_string())?;
    }
    if let Some(calls) = value.get("tool_calls") {
        for call in calls.as_array().ok_or("Invalid legacy tool calls")? {
            if call["id"].as_str().is_none_or(str::is_empty)
                || call["function"]["name"].as_str().is_none_or(str::is_empty)
                || call["function"].get("arguments").is_none()
            {
                return Err("Invalid legacy tool call".into());
            }
        }
    }
    if let Some(items) = value.get("reasoning_items") {
        for item in items.as_array().ok_or("Invalid legacy reasoning items")? {
            if !item["model"].is_string() || item.get("item").is_none_or(Value::is_null) {
                return Err("Invalid legacy reasoning item".into());
            }
        }
    }
    RuntimeMessage::from_legacy(value.clone()).ok_or_else(|| "Invalid legacy message".into())
}

fn message_facts(message: &ChatMessage) -> Result<Vec<RuntimeMessage>, String> {
    let facts = if !message.model_messages.is_empty() {
        message
            .model_messages
            .iter()
            .enumerate()
            .map(|(index, model)| {
                let mut fact = RuntimeMessage::model(model.clone());
                if let Some(raw) = message
                    .api_messages
                    .get(index)
                    .and_then(|raw| RuntimeMessage::from_legacy(raw.clone()))
                {
                    if message.model_messages.len() == message.api_messages.len()
                        && raw.role() == Some(model.role)
                    {
                        fact.source = raw.source;
                    }
                }
                fact
            })
            .collect()
    } else if !message.api_messages.is_empty() {
        message
            .api_messages
            .iter()
            .map(import_legacy)
            .collect::<Result<_, _>>()?
    } else if let Some(incoming) = &message.irc_message {
        if incoming.observation {
            Vec::new()
        } else {
            vec![import_legacy(&super::sub_agent::view::model_input(
                incoming,
            ))?]
        }
    } else if message.role == "assistant" && message.id.starts_with("subagent-result-") {
        vec![import_legacy(&super::sub_agent::delivery::report_input(
            &message.content,
        ))?]
    } else {
        let role = match message.role.as_str() {
            "user" => ModelRole::User,
            "assistant" => ModelRole::Assistant,
            other => return Err(format!("Unsupported UI message role: {other}")),
        };
        let mut fact = RuntimeMessage::text(role, &message.content);
        if role == ModelRole::User {
            for attachment in &message.attachments {
                let part = if attachment.attachment_type == "image" {
                    MessagePart::Image {
                        mime_type: super::vision::image_mime_for_path(std::path::Path::new(
                            &attachment.name,
                        ))
                        .into(),
                        data: String::new(),
                        path: Some(attachment.path.clone()),
                        detail: None,
                    }
                } else if let Some(mime) = super::video::mime_for_name(&attachment.name) {
                    MessagePart::Video {
                        mime_type: mime.into(),
                        data: String::new(),
                        path: Some(attachment.path.clone()),
                    }
                } else {
                    MessagePart::Text {
                        text: format!(
                            "\n[Attachment {}: {}]\n{}",
                            attachment.name,
                            attachment.path,
                            attachment.content.as_deref().unwrap_or("")
                        ),
                    }
                };
                fact.model_message_mut()
                    .expect("Model message")
                    .content
                    .push(part);
            }
        }
        vec![fact]
    };
    Ok(facts
        .into_iter()
        .map(|fact| tag(fact, &message.id))
        .collect())
}

fn append(history: &mut SessionHistory, messages: Vec<RuntimeMessage>) -> Option<EntryRange> {
    let mut first = None;
    for message in messages {
        history.push(message);
        first = first.or(history.active_leaf());
    }
    first.map(|first| EntryRange {
        first,
        last: history.active_leaf().expect("Appended message"),
    })
}

fn clear_legacy(conversation: &mut Conversation) {
    for message in &mut conversation.messages {
        message.model_messages.clear();
        message.api_messages.clear();
    }
    if let Some(summary) = &mut conversation.context_state.summary {
        summary.replay = None;
    }
}

fn invalidate_discarded_summary(conversation: &mut Conversation) -> Result<(), String> {
    let state = conversation
        .builtin_history
        .as_ref()
        .ok_or("Missing canonical history")?;
    let Some(previous) = state.archived_branches.last() else {
        return Ok(());
    };
    let current: HashSet<_> = state
        .snapshot
        .history()
        .active_path()
        .iter()
        .map(|entry| entry.id)
        .collect();
    let mut history = state.snapshot.clone().into_history();
    history.switch_branch(previous.leaf)?;
    if history.active_path().iter().any(|entry| {
        matches!(entry.kind, SessionEntryKind::Compaction { .. }) && !current.contains(&entry.id)
    }) {
        if let Some(summary) = &mut conversation.context_state.summary {
            summary.stale = true;
            conversation.context_state.status = "stale".into();
        }
    }
    Ok(())
}

pub(crate) fn ensure_migrated(conversation: &mut Conversation) -> Result<(), String> {
    if conversation.agent_runtime.is_external() || conversation.builtin_history.is_some() {
        return Ok(());
    }
    let mut history = SessionHistory::from(vec![RuntimeMessage::system("")]);
    let mut bindings = HashMap::new();
    let mut groups = HashSet::new();
    let summary = active_summary(conversation);
    let summary_through = summary.map(|s| {
        s.replay
            .as_ref()
            .map(|r| r.through_message_id.as_str())
            .unwrap_or(&s.source_until_message_id)
    });
    let mut clear_indices: HashSet<_> = conversation
        .context_state
        .clear_boundaries
        .iter()
        .filter_map(|boundary| {
            conversation
                .messages
                .iter()
                .position(|message| message.id == boundary.source_until_message_id)
        })
        .collect();
    clear_indices.extend(conversation.context_clear_until_index());
    for (index, message) in conversation.messages.iter().enumerate() {
        if let Some(group) = message
            .group_id
            .as_ref()
            .filter(|_| message.role == "assistant")
        {
            if groups.insert(group.clone()) {
                let parent = history.active_leaf();
                let mut selected = parent;
                for sibling in conversation
                    .messages
                    .iter()
                    .filter(|m| m.role == "assistant" && m.group_id.as_ref() == Some(group))
                {
                    history.switch_branch(parent)?;
                    let range = append(&mut history, message_facts(sibling)?);
                    bindings.insert(sibling.id.clone(), range);
                    if !group_answer_excluded_from_context(conversation, sibling) {
                        selected = history.active_leaf();
                    }
                }
                history.switch_branch(selected)?;
            }
        } else {
            bindings.insert(
                message.id.clone(),
                append(&mut history, message_facts(message)?),
            );
        }
        if clear_indices.contains(&index) {
            history.reset_context(vec![RuntimeMessage::system("")])?;
        }
        if summary_through == Some(message.id.as_str()) {
            let summary = summary.expect("Summary boundary");
            let mut replacement = vec![import_legacy(&summary_message(summary))?];
            if let Some(replay) = &summary.replay {
                replacement.extend(
                    replay
                        .messages
                        .iter()
                        .map(import_legacy)
                        .collect::<Result<Vec<_>, _>>()?,
                );
            }
            if history.context().len() == 1 {
                replacement.insert(0, RuntimeMessage::system(""));
                history.reset_context(replacement)?;
            } else {
                history.compact(CompactionPlan {
                    covered: 1..history.context().len(),
                    replacement,
                })?;
            }
        }
    }
    let snapshot = history.snapshot()?;
    conversation.builtin_history = Some(Box::new(BuiltinHistory {
        snapshot,
        active_branch_id: uuid::Uuid::new_v4().to_string(),
        bindings,
        archived_branches: Vec::new(),
        migration: BuiltinHistoryMigration {
            version: 1,
            source_revision: conversation.revision,
            ui_fallback_message_ids: conversation
                .messages
                .iter()
                .filter(|message| {
                    message.model_messages.is_empty() && message.api_messages.is_empty()
                })
                .map(|message| message.id.clone())
                .collect(),
        },
    }));
    clear_legacy(conversation);
    Ok(())
}

pub(crate) fn admit_input(
    conversation: &mut Conversation,
    ui_id: &str,
    messages: Vec<RuntimeMessage>,
) -> Result<(), String> {
    ensure_migrated(conversation)?;
    let state = conversation
        .builtin_history
        .as_mut()
        .ok_or("External conversation")?;
    if let Some(range) = state.bindings.get(ui_id) {
        let expected: Vec<_> = messages
            .into_iter()
            .map(|message| tag(message, ui_id))
            .collect();
        if range_messages(&state.snapshot.clone().into_history(), *range)? == expected {
            return Ok(());
        }
        return Err("Input was already admitted with different model content".into());
    }
    let mut history = state.snapshot.clone().into_history();
    let range = append(
        &mut history,
        messages
            .into_iter()
            .map(|message| tag(message, ui_id))
            .collect(),
    );
    state.snapshot = history.snapshot()?;
    state.bindings.insert(ui_id.into(), range);
    Ok(())
}

fn pending_report_ids(conversation: &Conversation) -> HashSet<String> {
    let Some(state) = &conversation.builtin_history else {
        return HashSet::new();
    };
    let active: HashSet<_> = state
        .snapshot
        .history()
        .active_path()
        .iter()
        .map(|entry| entry.id)
        .collect();
    conversation
        .messages
        .iter()
        .filter(|message| {
            !message
                .irc_message
                .as_ref()
                .is_some_and(|message| message.observation)
                && (message.id.starts_with("subagent-result-")
                    || message
                        .irc_message
                        .as_ref()
                        .is_some_and(|message| !message.observation))
                && !state
                    .bindings
                    .get(&message.id)
                    .copied()
                    .flatten()
                    .is_some_and(|range| active.contains(&range.last))
        })
        .map(|message| message.id.clone())
        .collect()
}

pub(crate) fn needs_report_admission(conversation: &Conversation) -> bool {
    !pending_report_ids(conversation).is_empty()
}

pub(crate) fn admit_pending_reports(conversation: &mut Conversation) -> Result<(), String> {
    let mut candidate = conversation.clone();
    ensure_migrated(&mut candidate)?;
    let pending = pending_report_ids(&candidate);
    if pending.is_empty() {
        *conversation = candidate;
        return Ok(());
    }
    let state = candidate
        .builtin_history
        .as_mut()
        .ok_or("External conversation")?;
    let mut history = state.snapshot.clone().into_history();
    for message in candidate
        .messages
        .iter()
        .filter(|message| pending.contains(&message.id))
    {
        state.bindings.insert(
            message.id.clone(),
            append(&mut history, message_facts(message)?),
        );
    }
    state.snapshot = history.snapshot()?;
    clear_legacy(&mut candidate);
    *conversation = candidate;
    Ok(())
}

fn refresh_active_bindings(state: &mut BuiltinHistory, history: &SessionHistory) {
    let mut bindings: HashMap<String, EntryRange> = HashMap::new();
    let active: HashSet<_> = history.active_path().iter().map(|entry| entry.id).collect();
    for entry in history.active_path() {
        if let SessionEntryKind::Message(message) = &entry.kind {
            if let Some(owner) = message.source.get("_ui_message_id").and_then(Value::as_str) {
                bindings
                    .entry(owner.into())
                    .and_modify(|range| range.last = entry.id)
                    .or_insert(EntryRange {
                        first: entry.id,
                        last: entry.id,
                    });
            }
        }
    }
    for (id, range) in bindings {
        if !state
            .bindings
            .get(&id)
            .copied()
            .flatten()
            .is_some_and(|range| active.contains(&range.first) && active.contains(&range.last))
        {
            state.bindings.insert(id, Some(range));
        }
    }
}

pub(crate) fn commit_completed_run(
    conversation: &mut Conversation,
    ui_id: &str,
    source: &SessionHistory,
    source_anchor: Option<EntryId>,
    select: bool,
) -> Result<(), String> {
    ensure_migrated(conversation)?;
    let state = conversation
        .builtin_history
        .as_mut()
        .ok_or("External conversation")?;
    let mut history = state.snapshot.clone().into_history();
    let source_path = source.active_path();
    let start = match source_anchor {
        Some(anchor) => {
            source_path
                .iter()
                .position(|entry| entry.id == anchor)
                .ok_or("Unknown run anchor")?
                + 1
        }
        None => 0,
    };
    let mut entries = source.entries().to_vec();
    let mut descendants = HashSet::new();
    for entry in &mut entries {
        if source_anchor.is_none()
            || entry.parent_id == source_anchor
            || entry.parent_id.is_some_and(|id| descendants.contains(&id))
        {
            descendants.insert(entry.id);
            if history.entries().iter().any(|existing| existing == entry) {
                continue;
            }
            if let SessionEntryKind::Message(message) = &mut entry.kind {
                message
                    .source
                    .entry("_ui_message_id")
                    .or_insert_with(|| Value::String(ui_id.into()));
            }
        }
    }
    let source = SessionHistory::from_entries(entries, source.active_leaf())?;
    let graft = history.graft_branch(&source, source_anchor)?;
    let mut bindings: HashMap<String, EntryRange> = HashMap::new();
    for entry in &source.active_path()[start..] {
        let owner = match &entry.kind {
            SessionEntryKind::Message(message) => message
                .source
                .get("_ui_message_id")
                .and_then(Value::as_str)
                .unwrap_or(ui_id),
            _ => ui_id,
        };
        let id = graft.ids.get(&entry.id).copied().unwrap_or(entry.id);
        bindings
            .entry(owner.into())
            .and_modify(|range| range.last = id)
            .or_insert(EntryRange {
                first: id,
                last: id,
            });
    }
    if select {
        history.switch_branch(graft.leaf)?;
    }
    let snapshot = history.snapshot()?;
    state.bindings.insert(ui_id.into(), bindings.remove(ui_id));
    state
        .bindings
        .extend(bindings.into_iter().map(|(id, range)| (id, Some(range))));
    state.snapshot = snapshot;
    clear_legacy(conversation);
    Ok(())
}

pub(crate) fn commit_interrupted_run(
    conversation: &mut Conversation,
    ui_id: &str,
    source: &SessionHistory,
    source_anchor: Option<EntryId>,
    source_branch_id: &str,
) -> Result<(), String> {
    let state = conversation
        .builtin_history
        .as_mut()
        .ok_or("Interrupted run requires canonical history")?;
    if state.active_branch_id != source_branch_id || state.snapshot.active_leaf() != source_anchor {
        return Err("Interrupted run no longer belongs to the selected branch".into());
    }
    if state
        .snapshot
        .entries()
        .iter()
        .any(|entry| !source.entries().iter().any(|candidate| candidate == entry))
    {
        return Err("Interrupted run no longer extends the stored history".into());
    }
    let path = source.active_path();
    let first = match source_anchor {
        Some(anchor) => {
            path.iter()
                .position(|entry| entry.id == anchor)
                .ok_or("Interrupted run anchor is not on the selected path")?
                + 1
        }
        None => 0,
    };
    let mut bindings: HashMap<String, EntryRange> = HashMap::new();
    for entry in &path[first..] {
        let owner = match &entry.kind {
            SessionEntryKind::Message(message) => message
                .source
                .get("_ui_message_id")
                .and_then(Value::as_str)
                .unwrap_or(ui_id),
            _ => ui_id,
        };
        bindings
            .entry(owner.into())
            .and_modify(|range| range.last = entry.id)
            .or_insert(EntryRange {
                first: entry.id,
                last: entry.id,
            });
    }
    let snapshot = source.snapshot()?;
    state.snapshot = snapshot;
    state.bindings.insert(ui_id.into(), bindings.remove(ui_id));
    state
        .bindings
        .extend(bindings.into_iter().map(|(id, range)| (id, Some(range))));
    clear_legacy(conversation);
    Ok(())
}

pub(crate) fn archive_branch(conversation: &mut Conversation) -> Result<(), String> {
    ensure_migrated(conversation)?;
    clear_legacy(conversation);
    let state = conversation
        .builtin_history
        .as_mut()
        .ok_or("External conversation")?;
    state.archived_branches.push(ArchivedBuiltinBranch {
        id: state.active_branch_id.clone(),
        leaf: state.snapshot.active_leaf(),
        messages: conversation.messages.clone(),
        group_selections: conversation.group_selections.clone(),
        bindings: state.bindings.clone(),
        context_state: conversation.context_state.clone(),
    });
    state.active_branch_id = uuid::Uuid::new_v4().to_string();
    Ok(())
}

pub(crate) fn switch_branch(
    conversation: &mut Conversation,
    branch_id: &str,
) -> Result<(), String> {
    ensure_migrated(conversation)?;
    let state = conversation
        .builtin_history
        .as_ref()
        .ok_or("External conversation")?;
    if state.active_branch_id == branch_id {
        return Ok(());
    }
    let index = state
        .archived_branches
        .iter()
        .position(|branch| branch.id == branch_id)
        .ok_or("Unknown branch")?;
    let mut history = state.snapshot.clone().into_history();
    history.switch_branch(state.archived_branches[index].leaf)?;
    let snapshot = history.snapshot()?;
    let lifecycle_id = conversation.context_state.lifecycle_id.saturating_add(1);
    let measurement_seq = conversation.context_state.measurement_seq;
    archive_branch(conversation)?;
    let state = conversation
        .builtin_history
        .as_mut()
        .expect("Migrated history");
    let branch = state.archived_branches.remove(index);
    state.active_branch_id = branch.id;
    state.snapshot = snapshot;
    state.bindings = branch.bindings;
    conversation.messages = branch.messages;
    conversation.group_selections = branch.group_selections;
    conversation.context_state = branch.context_state;
    conversation.context_state.lifecycle_id = lifecycle_id;
    conversation.context_state.measurement_seq = measurement_seq;
    conversation.context_state.request_measurement = None;
    conversation.context_state.reported_context_tokens = None;
    conversation.context_state.last_measured_at = 0;
    Ok(())
}

pub(crate) fn map_runtime_messages(
    state: &mut BuiltinHistory,
    transform: impl FnMut(&mut RuntimeMessage),
) -> Result<(), String> {
    let mut history = state.snapshot.clone().into_history();
    map_history_messages(&mut history, transform)?;
    let mut snapshot = history.snapshot()?;
    if let Some(config) = state.snapshot.config() {
        snapshot = snapshot.with_config(config.clone());
    }
    state.snapshot = snapshot;
    Ok(())
}

pub(crate) fn map_history_messages(
    history: &mut SessionHistory,
    mut transform: impl FnMut(&mut RuntimeMessage),
) -> Result<(), String> {
    let mut entries = history.entries().to_vec();
    for entry in &mut entries {
        match &mut entry.kind {
            SessionEntryKind::Message(message) => transform(message),
            SessionEntryKind::Compaction { replacement, .. }
            | SessionEntryKind::ContextReset { replacement } => {
                for message in replacement {
                    transform(message);
                }
            }
        }
    }
    *history = SessionHistory::from_entries(entries, history.active_leaf())?;
    Ok(())
}

fn range_messages(
    history: &SessionHistory,
    range: Option<EntryRange>,
) -> Result<Vec<RuntimeMessage>, String> {
    let Some(range) = range else {
        return Ok(Vec::new());
    };
    let mut path = history.clone();
    path.switch_branch(Some(range.last))?;
    let entries = path.active_path();
    let first = entries
        .iter()
        .position(|entry| entry.id == range.first)
        .ok_or("Message binding is not a path")?;
    Ok(entries[first..]
        .iter()
        .filter_map(|entry| match &entry.kind {
            SessionEntryKind::Message(message) => Some(message.clone()),
            _ => None,
        })
        .collect())
}

fn message_parent(conversation: &Conversation, ui_id: &str) -> Result<Option<EntryId>, String> {
    let index = conversation
        .messages
        .iter()
        .position(|message| message.id == ui_id)
        .ok_or("Unknown message")?;
    let state = conversation
        .builtin_history
        .as_ref()
        .ok_or("Missing canonical history")?;
    let history = state.snapshot.clone().into_history();
    if let Some(range) = state.bindings.get(ui_id).copied().flatten() {
        return history
            .entries()
            .iter()
            .find(|entry| entry.id == range.first)
            .map(|entry| entry.parent_id)
            .ok_or_else(|| "Missing message binding".into());
    }
    let group = conversation.messages[index].group_id.as_ref();
    let active: HashSet<_> = history.active_path().iter().map(|entry| entry.id).collect();
    for message in conversation.messages[..index].iter().rev() {
        if message.role == "assistant" && group.is_some() && message.group_id.as_ref() == group {
            continue;
        }
        if let Some(range) = state.bindings.get(&message.id).copied().flatten() {
            if active.contains(&range.last) {
                return Ok(Some(range.last));
            }
        }
    }
    Ok(history.active_path().first().and_then(|entry| {
        matches!(&entry.kind, SessionEntryKind::Message(message) if message.role().is_none())
            .then_some(entry.id)
    }))
}

/// Replays immutable facts after a changed answer without carrying its old summary forward.
fn replace_range(
    conversation: &mut Conversation,
    ui_id: &str,
    replacement: Vec<RuntimeMessage>,
    reuse: Option<EntryRange>,
) -> Result<(), String> {
    let empty_parent = message_parent(conversation, ui_id)?;
    let selected_empty = conversation
        .messages
        .iter()
        .find(|message| message.id == ui_id)
        .is_some_and(|message| !group_answer_excluded_from_context(conversation, message));
    let state = conversation
        .builtin_history
        .as_mut()
        .ok_or("Missing canonical history")?;
    let range = state.bindings.get(ui_id).copied().flatten();
    let mut history = state.snapshot.clone().into_history();
    let path: Vec<_> = history.active_path().into_iter().cloned().collect();
    let selected = range.and_then(|range| path.iter().position(|entry| entry.id == range.first));
    let old_leaf = history.active_leaf();
    let mut independent = Vec::new();
    let (parent, suffix) = if let Some(first) = selected {
        let last = path
            .iter()
            .position(|entry| Some(entry.id) == range.map(|r| r.last))
            .ok_or("Incomplete message binding")?;
        for entry in &path[first..=last] {
            if let SessionEntryKind::Message(message) = &entry.kind {
                if let Some(owner) = message
                    .source
                    .get("_ui_message_id")
                    .and_then(Value::as_str)
                    .filter(|owner| *owner != ui_id && state.bindings.contains_key(*owner))
                {
                    if !replacement.iter().any(|message| {
                        message.source.get("_ui_message_id").and_then(Value::as_str) == Some(owner)
                    }) {
                        independent.push(message.clone());
                    }
                }
            }
        }
        (path[first].parent_id, path[last + 1..].to_vec())
    } else if let Some(range) = range {
        let first = history
            .entries()
            .iter()
            .find(|entry| entry.id == range.first)
            .ok_or("Missing message binding")?;
        (first.parent_id, Vec::new())
    } else {
        let start = empty_parent
            .and_then(|parent| path.iter().position(|entry| entry.id == parent))
            .map_or(0, |index| index + 1);
        (
            empty_parent,
            if selected_empty {
                path[start..].to_vec()
            } else {
                Vec::new()
            },
        )
    };
    history.switch_branch(parent)?;
    let reusable = reuse.filter(|range| {
        history
            .entries()
            .iter()
            .any(|entry| entry.id == range.first && entry.parent_id == parent)
    });
    let new_range = if let Some(range) = reusable {
        history.switch_branch(Some(range.last))?;
        Some(range)
    } else {
        append(&mut history, replacement)
    };
    history.extend(independent);
    let mut ids = HashMap::new();
    for entry in suffix {
        match entry.kind {
            SessionEntryKind::Message(message) => history.push(message),
            SessionEntryKind::ContextReset { replacement } => history.reset_context(replacement)?,
            SessionEntryKind::Compaction { .. } => continue,
        }
        ids.insert(entry.id, history.active_leaf().expect("Copied entry"));
    }
    let changes_active = selected.is_some() || (range.is_none() && selected_empty);
    if !changes_active {
        history.switch_branch(old_leaf)?;
    }
    let snapshot = history.snapshot()?;
    for binding in state.bindings.values_mut().flatten() {
        binding.first = ids.get(&binding.first).copied().unwrap_or(binding.first);
        binding.last = ids.get(&binding.last).copied().unwrap_or(binding.last);
    }
    state.bindings.insert(ui_id.into(), new_range);
    if changes_active {
        refresh_active_bindings(state, &history);
    }
    state.snapshot = snapshot;
    Ok(())
}

pub(crate) fn edit_message(
    conversation: &mut Conversation,
    ui_id: &str,
    content: &str,
) -> Result<(), String> {
    let mut candidate = conversation.clone();
    ensure_migrated(&mut candidate)?;
    let index = candidate
        .messages
        .iter()
        .position(|message| message.id == ui_id)
        .ok_or("Unknown message")?;
    let role = match candidate.messages[index].role.as_str() {
        "user" => ModelRole::User,
        "assistant" => ModelRole::Assistant,
        _ => return Err("Only user and assistant messages can be edited".into()),
    };
    let state = candidate.builtin_history.as_ref().expect("Migrated");
    let mut replacement = range_messages(
        &state.snapshot.clone().into_history(),
        state.bindings.get(ui_id).copied().flatten(),
    )?;
    if let Some(final_message) = replacement.iter_mut().rfind(|message| {
        message.role() == Some(role)
            && message.model_message().is_some_and(|m| {
                !m.content
                    .iter()
                    .any(|part| matches!(part, MessagePart::ToolCall { .. }))
            })
    }) {
        let model = final_message.model_message_mut().expect("Model message");
        model.content.retain(|part| match part {
            MessagePart::Text { .. } => false,
            MessagePart::AnthropicNativeBlock { block, .. } => block["type"] != "text",
            _ => true,
        });
        model.content.push(MessagePart::Text {
            text: content.into(),
        });
    } else {
        replacement.push(tag(RuntimeMessage::text(role, content), ui_id));
    }
    archive_branch(&mut candidate)?;
    replace_range(&mut candidate, ui_id, replacement, None)?;
    candidate.messages[index].content = content.into();
    if candidate.builtin_history.as_ref().is_some_and(|state| {
        state
            .archived_branches
            .last()
            .is_some_and(|branch| branch.leaf != state.snapshot.active_leaf())
    }) {
        candidate.invalidate_summary_from(index);
    }
    invalidate_discarded_summary(&mut candidate)?;
    *conversation = candidate;
    Ok(())
}

pub(crate) fn select_group_answer(
    conversation: &mut Conversation,
    group_id: &str,
    ui_id: &str,
) -> Result<(), String> {
    let mut candidate = conversation.clone();
    ensure_migrated(&mut candidate)?;
    if !candidate
        .messages
        .iter()
        .any(|m| m.id == ui_id && m.role == "assistant" && m.group_id.as_deref() == Some(group_id))
    {
        return Err("Selected answer does not belong to the group".into());
    }
    let previous = candidate
        .messages
        .iter()
        .find(|m| {
            m.role == "assistant"
                && m.group_id.as_deref() == Some(group_id)
                && !group_answer_excluded_from_context(&candidate, m)
        })
        .map(|m| m.id.clone())
        .ok_or("Group has no selected answer")?;
    if previous == ui_id {
        return Ok(());
    }
    archive_branch(&mut candidate)?;
    let state = candidate.builtin_history.as_ref().expect("Migrated");
    let reuse = state.bindings.get(ui_id).copied().flatten();
    let replacement = range_messages(&state.snapshot.clone().into_history(), reuse)?;
    let previous_binding = state.bindings.get(&previous).copied().flatten();
    replace_range(&mut candidate, &previous, replacement, reuse)?;
    let state = candidate.builtin_history.as_mut().expect("Migrated");
    let selected_binding = state.bindings.get(&previous).copied().flatten();
    state.bindings.insert(previous, previous_binding);
    state.bindings.insert(ui_id.into(), selected_binding);
    candidate.select_group_answer(group_id.into(), ui_id.into());
    invalidate_discarded_summary(&mut candidate)?;
    *conversation = candidate;
    Ok(())
}

pub(crate) fn delete_answer(conversation: &mut Conversation, ui_id: &str) -> Result<(), String> {
    let mut candidate = conversation.clone();
    ensure_migrated(&mut candidate)?;
    let index = candidate
        .messages
        .iter()
        .position(|message| message.id == ui_id && message.role == "assistant")
        .ok_or("Unknown assistant answer")?;
    let selected = !group_answer_excluded_from_context(&candidate, &candidate.messages[index]);
    archive_branch(&mut candidate)?;
    let before_removal = candidate.clone();
    let removed = candidate.messages.remove(index);
    if let Some(group) = &removed.group_id {
        if candidate
            .group_selections
            .get(group)
            .is_some_and(|id| id == ui_id)
        {
            candidate.group_selections.remove(group);
        }
    }
    if selected {
        let fallback = removed
            .group_id
            .as_ref()
            .and_then(|group| {
                candidate.messages.iter().find(|m| {
                    m.role == "assistant"
                        && m.group_id.as_ref() == Some(group)
                        && !group_answer_excluded_from_context(&candidate, m)
                })
            })
            .map(|m| m.id.clone());
        let state = candidate.builtin_history.as_ref().expect("Migrated");
        let reuse = fallback
            .as_ref()
            .and_then(|id| state.bindings.get(id).copied().flatten());
        let replacement = range_messages(&state.snapshot.clone().into_history(), reuse)?;
        let mut replacement_view = before_removal;
        replace_range(&mut replacement_view, ui_id, replacement, reuse)?;
        candidate.builtin_history = replacement_view.builtin_history;
        let state = candidate.builtin_history.as_mut().expect("Migrated");
        if let Some(fallback) = fallback {
            state
                .bindings
                .insert(fallback, state.bindings.get(ui_id).copied().flatten());
        }
        candidate.invalidate_summary_from(index);
    }
    candidate
        .builtin_history
        .as_mut()
        .expect("Migrated")
        .bindings
        .remove(ui_id);
    invalidate_discarded_summary(&mut candidate)?;
    *conversation = candidate;
    Ok(())
}

pub(crate) fn truncate_before(conversation: &mut Conversation, ui_id: &str) -> Result<(), String> {
    let mut candidate = conversation.clone();
    ensure_migrated(&mut candidate)?;
    let index = candidate
        .messages
        .iter()
        .position(|message| message.id == ui_id && message.role == "user")
        .ok_or("Unknown user message")?;
    let mut history = history(&candidate)?;
    let range = candidate
        .builtin_history
        .as_ref()
        .expect("Migrated")
        .bindings
        .get(ui_id)
        .copied()
        .flatten()
        .ok_or("User message has no model entry")?;
    history.rewind(range.first)?;
    archive_branch(&mut candidate)?;
    candidate.messages.truncate(index);
    let retained: HashSet<_> = candidate.messages.iter().map(|m| m.id.clone()).collect();
    candidate
        .group_selections
        .retain(|_, id| retained.contains(id));
    let state = candidate.builtin_history.as_mut().expect("Migrated");
    state.snapshot = history.snapshot()?;
    state.bindings.retain(|id, _| retained.contains(id));
    super::commands::context::mark_summary_stale_if_needed(&mut candidate, index);
    invalidate_discarded_summary(&mut candidate)?;
    *conversation = candidate;
    Ok(())
}

pub(crate) fn prepare_regenerate(
    conversation: &mut Conversation,
    ui_id: &str,
    new_content: Option<&str>,
) -> Result<(), String> {
    let mut candidate = conversation.clone();
    ensure_migrated(&mut candidate)?;
    let index = candidate
        .messages
        .iter()
        .position(|message| message.id == ui_id)
        .ok_or("Unknown message")?;
    let user = candidate.messages[index].role == "user";
    if !user && new_content.is_some() {
        return Err("编辑内容仅支持用户消息".into());
    }
    if !user && candidate.messages[index].role != "assistant" {
        return Err("仅支持重新生成助手回复或重试用户消息".into());
    }
    if new_content.is_some_and(|text| text.trim().is_empty()) {
        return Err("消息内容不能为空".into());
    }
    let current_turn = !candidate.messages[index + 1..]
        .iter()
        .any(|message| message.role == "user");
    let keep = index + usize::from(user);
    let reports: Vec<_> = if current_turn {
        candidate.messages[keep..]
            .iter()
            .filter(|message| {
                message.role == "assistant"
                    && (message.id.starts_with("subagent-result-") || message.irc_message.is_some())
            })
            .cloned()
            .collect()
    } else {
        Vec::new()
    };
    let state = candidate.builtin_history.as_ref().expect("Migrated");
    let mut history = state.snapshot.clone().into_history();
    let range = state.bindings.get(ui_id).copied().flatten();
    let parent = message_parent(&candidate, ui_id)?;
    history.switch_branch(if user && new_content.is_none() {
        Some(range.ok_or("User message has no model entry")?.last)
    } else {
        parent
    })?;
    let report_facts: Vec<_> = reports
        .iter()
        .map(|message| {
            range_messages(
                &state.snapshot.clone().into_history(),
                state.bindings.get(&message.id).copied().flatten(),
            )
            .map(|facts| (message.id.clone(), facts))
        })
        .collect::<Result<_, _>>()?;
    archive_branch(&mut candidate)?;
    candidate.messages.truncate(keep);
    let state = candidate.builtin_history.as_mut().expect("Migrated");
    if let Some(content) = new_content {
        let message = &mut candidate.messages[index];
        message.content = content.trim().into();
        message.timestamp = chrono::Local::now().timestamp();
        state
            .bindings
            .insert(ui_id.into(), append(&mut history, message_facts(message)?));
    }
    for (id, facts) in report_facts {
        if !history.active_path().iter().any(|entry| {
            state
                .bindings
                .get(&id)
                .copied()
                .flatten()
                .is_some_and(|range| range.last == entry.id)
        }) {
            state.bindings.insert(id, append(&mut history, facts));
        }
    }
    candidate.messages.extend(reports);
    let retained: HashSet<_> = candidate
        .messages
        .iter()
        .map(|message| message.id.clone())
        .collect();
    candidate
        .group_selections
        .retain(|_, id| retained.contains(id));
    state.bindings.retain(|id, _| retained.contains(id));
    state.snapshot = history.snapshot()?;
    candidate.invalidate_summary_from(if user && new_content.is_none() {
        index + 1
    } else {
        index
    });
    invalidate_discarded_summary(&mut candidate)?;
    *conversation = candidate;
    Ok(())
}

pub(crate) fn fork_prefix(
    source: &mut Conversation,
    retained: &[ChatMessage],
) -> Result<Box<BuiltinHistory>, String> {
    ensure_migrated(source)?;
    if let Some(anchor) = retained.last() {
        if let Some(group) = source
            .messages
            .iter()
            .find(|message| message.id == anchor.id)
            .and_then(|message| message.group_id.clone())
        {
            select_group_answer(source, &group, &anchor.id)?;
        }
    }
    let mut state = source
        .builtin_history
        .clone()
        .ok_or("External conversation")?;
    let retained_ids: HashSet<_> = retained.iter().map(|message| message.id.as_str()).collect();
    let removed_first: HashSet<_> = state
        .bindings
        .iter()
        .filter(|(id, _)| !retained_ids.contains(id.as_str()))
        .filter_map(|(_, range)| range.map(|range| range.first))
        .collect();
    let mut history = state.snapshot.clone().into_history();
    if let Some(entry) = history
        .active_path()
        .into_iter()
        .find(|entry| removed_first.contains(&entry.id))
    {
        history.switch_branch(entry.parent_id)?;
    }
    if retained.is_empty() {
        history.switch_branch(history.entries().first().map(|entry| entry.id))?;
    }
    state.snapshot = history.snapshot()?;
    state.active_branch_id = uuid::Uuid::new_v4().to_string();
    state.archived_branches.clear();
    state
        .bindings
        .retain(|id, _| retained_ids.contains(id.as_str()));
    let mut fork = source.clone();
    fork.messages = retained.to_vec();
    fork.builtin_history = Some(state);
    admit_pending_reports(&mut fork)?;
    fork.builtin_history
        .ok_or_else(|| "Missing fork history".into())
}

pub(crate) fn reset_context(conversation: &mut Conversation) -> Result<(), String> {
    ensure_migrated(conversation)?;
    let state = conversation
        .builtin_history
        .as_mut()
        .ok_or("External conversation")?;
    let mut history = state.snapshot.clone().into_history();
    history.reset_context(vec![RuntimeMessage::system("")])?;
    state.snapshot = history.snapshot()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn conversation(messages: Value) -> Conversation {
        serde_json::from_value(json!({
            "id":"conversation", "title":"History", "provider_id":"provider", "model":"model",
            "messages":messages, "created_at":1, "updated_at":1,
        }))
        .unwrap()
    }

    fn message(id: &str, role: &str, content: &str) -> Value {
        json!({"id":id,"role":role,"content":content,"timestamp":1})
    }

    fn sibling(id: &str, content: &str, error: bool) -> Value {
        let mut value = message(id, "assistant", content);
        value["group_id"] = json!("group");
        value["stream_outcome"] = json!(if error { "error" } else { "completed" });
        value
    }

    fn texts(conversation: &Conversation) -> Vec<String> {
        history(conversation)
            .unwrap()
            .context()
            .iter()
            .filter(|message| message.role().is_some())
            .map(RuntimeMessage::text_content)
            .collect()
    }

    #[test]
    fn migration_retains_siblings_and_defaults_to_first_successful_answer() {
        let mut conversation = conversation(json!([
            message("u", "user", "question"),
            sibling("failed", "failed", true),
            sibling("a", "answer", false),
            sibling("b", "alternative", false),
            message("u2", "user", "followup"),
        ]));
        ensure_migrated(&mut conversation).unwrap();
        assert_eq!(texts(&conversation), ["question", "answer", "followup"]);
        assert_eq!(history(&conversation).unwrap().entries().len(), 6);
        select_group_answer(&mut conversation, "group", "b").unwrap();
        assert_eq!(
            texts(&conversation),
            ["question", "alternative", "followup"]
        );
        let restored: Conversation =
            serde_json::from_value(serde_json::to_value(&conversation).unwrap()).unwrap();
        assert_eq!(texts(&restored), texts(&conversation));
    }

    #[test]
    fn inactive_deletion_creates_an_independent_ui_branch_at_the_same_leaf() {
        let mut conversation = conversation(json!([
            message("u", "user", "question"),
            sibling("a", "answer", false),
            sibling("b", "alternative", false),
        ]));
        ensure_migrated(&mut conversation).unwrap();
        let before = conversation.builtin_history.as_ref().unwrap().clone();
        delete_answer(&mut conversation, "b").unwrap();
        let after = conversation.builtin_history.as_ref().unwrap();
        assert_eq!(before.snapshot.active_leaf(), after.snapshot.active_leaf());
        assert_ne!(before.active_branch_id, after.active_branch_id);
        assert_eq!(conversation.messages.len(), 2);
        conversation.context_state.lifecycle_id = 10;
        conversation.context_state.measurement_seq = 17;
        switch_branch(&mut conversation, &before.active_branch_id).unwrap();
        assert_eq!(conversation.messages.len(), 3);
        assert_eq!(conversation.context_state.lifecycle_id, 11);
        assert_eq!(conversation.context_state.measurement_seq, 17);
    }

    #[test]
    fn deleting_selected_answer_replays_first_successful_sibling_and_keeps_followup() {
        let mut conversation = conversation(json!([
            message("u", "user", "question"),
            sibling("a", "answer", false),
            sibling("failed", "error", true),
            sibling("b", "alternative", false),
            message("u2", "user", "followup"),
        ]));
        ensure_migrated(&mut conversation).unwrap();
        delete_answer(&mut conversation, "a").unwrap();
        assert_eq!(
            texts(&conversation),
            ["question", "alternative", "followup"]
        );
        assert!(history(&conversation).unwrap().entries().iter().any(|entry| matches!(&entry.kind, SessionEntryKind::Message(m) if m.text_content() == "answer")));
    }

    #[test]
    fn exact_compaction_replay_is_captured_before_legacy_fields_are_cleared() {
        let mut conversation = conversation(json!([
            message("u", "user", "old question"),
            message("a", "assistant", "old answer"),
            message("u2", "user", "next"),
        ]));
        conversation.context_state.summary = Some(serde_json::from_value(json!({
            "id":"summary", "content":"remember", "source_until_message_id":"u",
            "token_estimate_before":100, "token_estimate_after":10, "created_at":2,
            "replay":{"through_message_id":"a", "messages":[{"role":"assistant","content":"exact retained body"}]},
        })).unwrap());
        ensure_migrated(&mut conversation).unwrap();
        let texts = texts(&conversation);
        assert!(texts[0].contains("remember"));
        assert_eq!(&texts[1..], ["exact retained body", "next"]);
        assert!(conversation
            .context_state
            .summary
            .as_ref()
            .unwrap()
            .replay
            .is_none());
        assert!(history(&conversation)
            .unwrap()
            .entries()
            .iter()
            .any(|entry| matches!(entry.kind, SessionEntryKind::Compaction { .. })));
        edit_message(&mut conversation, "a", "corrected").unwrap();
        assert_eq!(
            super::tests::texts(&conversation),
            ["old question", "corrected", "next"]
        );
        assert!(conversation.context_state.summary.as_ref().unwrap().stale);
    }

    #[test]
    fn editing_before_clear_keeps_the_clear_on_the_new_branch() {
        let mut conversation = conversation(json!([
            message("u", "user", "old question"),
            message("a", "assistant", "old answer"),
            message("u2", "user", "new question"),
        ]));
        conversation.context_state.clear_boundaries.push(
            serde_json::from_value(
                json!({"id":"clear","source_until_message_id":"a","created_at":2}),
            )
            .unwrap(),
        );
        ensure_migrated(&mut conversation).unwrap();
        edit_message(&mut conversation, "a", "edited").unwrap();
        assert_eq!(texts(&conversation), ["new question"]);
    }

    #[test]
    fn assistant_edit_preserves_tool_rounds_reasoning_sources_and_original_facts() {
        let mut answer = message("a", "assistant", "answer");
        answer["api_messages"] = json!([
            {"role":"assistant","content":null,"tool_calls":[{"id":"call","type":"function","function":{"name":"read","arguments":"{}"}}]},
            {"role":"tool","tool_call_id":"call","content":"file body"},
            {"role":"assistant","content":"answer","reasoning_content":"reason","source_ui_message_id":"source"},
        ]);
        let mut conversation = conversation(json!([message("u", "user", "question"), answer]));
        ensure_migrated(&mut conversation).unwrap();
        let before = history(&conversation).unwrap();
        edit_message(&mut conversation, "a", "edited").unwrap();
        let after = history(&conversation).unwrap();
        assert_eq!(after.context()[2..4], before.context()[2..4]);
        let final_message = after.context().last().unwrap();
        assert_eq!(final_message.source["source_ui_message_id"], "source");
        assert!(final_message
            .model_message()
            .unwrap()
            .content
            .iter()
            .any(|part| matches!(part, MessagePart::Reasoning { text } if text == "reason")));
        assert!(after.entries().starts_with(before.entries()));
        assert!(conversation.messages[1].model_messages.is_empty());
        assert!(conversation.messages[1].api_messages.is_empty());
    }

    #[test]
    fn migration_failure_retains_original_data() {
        let mut answer = message("a", "assistant", "answer");
        answer["api_messages"] = json!([{"role":"assistant", "content":[{"type":"unknown_required_payload", "data":"opaque"}]}]);
        let mut conversation = conversation(json!([message("u", "user", "question"), answer]));
        let before = serde_json::to_value(&conversation).unwrap();
        assert!(ensure_migrated(&mut conversation).is_err());
        assert_eq!(serde_json::to_value(conversation).unwrap(), before);
    }

    #[test]
    fn migration_does_not_repair_an_unfinished_selected_tool_round() {
        let mut answer = message("a", "assistant", "partial");
        answer["api_messages"] = json!([{"role":"assistant","tool_calls":[{"id":"call","type":"function","function":{"name":"read","arguments":"{}"}}]}]);
        let mut conversation = conversation(json!([message("u", "user", "question"), answer]));
        let before = serde_json::to_value(&conversation).unwrap();
        assert!(ensure_migrated(&mut conversation).is_err());
        assert_eq!(serde_json::to_value(conversation).unwrap(), before);
    }

    #[test]
    fn missing_attachment_reference_survives_migration_and_inactive_branches() {
        let mut user = message("u", "user", "question");
        user["attachments"] =
            json!([{"id":"image","type":"image","name":"missing.png","path":"missing.png"}]);
        let mut conversation = conversation(json!([user, message("a", "assistant", "answer")]));
        ensure_migrated(&mut conversation).unwrap();
        truncate_before(&mut conversation, "u").unwrap();
        assert_eq!(
            history(&conversation).unwrap().attachment_references()[0].reference,
            "missing.png"
        );
        assert!(texts(&conversation).is_empty());
    }

    #[test]
    fn completed_sibling_run_grafts_without_overwriting_the_selected_run() {
        let mut conversation = conversation(json!([message("u", "user", "question")]));
        ensure_migrated(&mut conversation).unwrap();
        let mut first = history(&conversation).unwrap();
        let anchor = first.active_leaf();
        let mut second = first.clone();
        first.push(RuntimeMessage::text(ModelRole::Assistant, "first"));
        second.push(RuntimeMessage::text(ModelRole::Assistant, "second"));
        commit_completed_run(&mut conversation, "a", &first, anchor, true).unwrap();
        commit_completed_run(&mut conversation, "b", &second, anchor, false).unwrap();
        assert_eq!(texts(&conversation), ["question", "first"]);
        assert_eq!(history(&conversation).unwrap().entries().len(), 4);
        let restored: Conversation =
            serde_json::from_value(serde_json::to_value(conversation).unwrap()).unwrap();
        assert_eq!(texts(&restored), ["question", "first"]);
    }

    #[test]
    fn regenerate_keeps_user_identity_attachment_and_archives_the_old_answer() {
        let mut user = message("u", "user", "question");
        user["attachments"] =
            json!([{"id":"image","type":"image","name":"image.png","path":"image.png"}]);
        let mut conversation = conversation(json!([user, message("a", "assistant", "answer")]));
        prepare_regenerate(&mut conversation, "u", Some("edited")).unwrap();
        assert_eq!(conversation.messages.len(), 1);
        assert_eq!(conversation.messages[0].id, "u");
        assert_eq!(conversation.messages[0].attachments.len(), 1);
        assert!(texts(&conversation)[0].contains("edited"));
        let state = conversation.builtin_history.as_ref().unwrap();
        assert_eq!(state.archived_branches[0].messages[0].content, "question");
        assert_eq!(state.archived_branches[0].messages[1].content, "answer");
    }

    #[test]
    fn fork_selects_an_inactive_answer_without_losing_immutable_future_facts() {
        let mut source = conversation(json!([
            message("u", "user", "question"),
            sibling("a", "answer", false),
            sibling("b", "alternative", false),
            message("u2", "user", "followup"),
            message("a2", "assistant", "future"),
        ]));
        let retained = vec![source.messages[0].clone(), source.messages[2].clone()];
        let fork = fork_prefix(&mut source, &retained).unwrap();
        let mut forked = source.clone();
        forked.messages = retained;
        forked.builtin_history = Some(fork);
        assert_eq!(texts(&forked), ["question", "alternative"]);
        assert!(history(&forked).unwrap().entries().iter().any(|entry| matches!(&entry.kind, SessionEntryKind::Message(message) if message.text_content() == "future")));
        assert!(forked
            .builtin_history
            .as_ref()
            .unwrap()
            .archived_branches
            .is_empty());
    }

    #[test]
    fn malformed_app_history_version_is_rejected_without_discarding_the_tree() {
        let mut conversation = conversation(json!([message("u", "user", "question")]));
        ensure_migrated(&mut conversation).unwrap();
        let mut value = serde_json::to_value(&conversation).unwrap();
        value["builtin_history"]["migration"]["version"] = json!(2);
        assert!(serde_json::from_value::<Conversation>(value).is_err());
        let mut value = serde_json::to_value(&conversation).unwrap();
        value["builtin_history"]["future_required_field"] = json!(true);
        assert!(serde_json::from_value::<Conversation>(value).is_err());
    }

    #[test]
    fn interrupted_run_retains_pending_facts_without_selecting_or_executing_them() {
        let mut conversation = conversation(json!([message("u", "user", "question")]));
        ensure_migrated(&mut conversation).unwrap();
        let state = conversation.builtin_history.as_ref().unwrap();
        let branch_id = state.active_branch_id.clone();
        let anchor = state.snapshot.active_leaf();
        let mut source = history(&conversation).unwrap();
        source.push(import_legacy(&json!({"role":"assistant","tool_calls":[{"id":"pending","type":"function","function":{"name":"read","arguments":"{}"}}]})).unwrap());
        let source = SessionHistory::from_entries(source.entries().to_vec(), anchor).unwrap();
        commit_interrupted_run(&mut conversation, "a", &source, anchor, &branch_id).unwrap();
        assert_eq!(texts(&conversation), ["question"]);
        assert_eq!(history(&conversation).unwrap().entries(), source.entries());
        assert_eq!(
            conversation.builtin_history.as_ref().unwrap().bindings["a"],
            None
        );
        let unchanged = serde_json::to_value(&conversation).unwrap();
        assert!(commit_interrupted_run(
            &mut conversation,
            "a",
            &source,
            anchor,
            "different branch"
        )
        .is_err());
        assert_eq!(serde_json::to_value(conversation).unwrap(), unchanged);
    }

    #[test]
    fn request_projection_keeps_message_positions_and_leaves_canonical_media_untouched() {
        let mut video = RuntimeMessage::text(ModelRole::User, "video question");
        video
            .source
            .insert("source_ui_message_id".into(), json!("source"));
        video
            .model_message_mut()
            .unwrap()
            .content
            .push(MessagePart::Video {
                mime_type: "video/mp4".into(),
                data: String::new(),
                path: Some("missing.mp4".into()),
            });
        let call = import_legacy(&json!({"role":"assistant","tool_calls":[{"id":"call","type":"function","function":{"name":"read","arguments":"{}"}}]})).unwrap();
        let result =
            import_legacy(&json!({"role":"tool","tool_call_id":"call","content":"result"}))
                .unwrap();
        let original = vec![RuntimeMessage::system("prompt"), video, call, result];
        let projected = super::super::commands::context::project_canonical_history(
            None,
            "conversation",
            &original,
            false,
        )
        .unwrap();
        assert_eq!(projected.len(), original.len());
        assert_eq!(projected[1].source, original[1].source);
        assert!(projected[1]
            .text_content()
            .contains("Raw video is not included"));
        assert!(
            matches!(&original[1].model_message().unwrap().content[1],MessagePart::Video { data,.. } if data.is_empty())
        );
        assert_eq!(projected[0], original[0]);
        assert_eq!(projected[2..], original[2..]);
        let mut conversation = conversation(json!([message("u", "user", "question")]));
        ensure_migrated(&mut conversation).unwrap();
        let before = serde_json::to_value(&conversation.builtin_history).unwrap();
        let request = super::super::commands::context::build_chat_api_messages(
            None,
            "temporary prompt",
            &conversation,
            None,
            None,
            &[],
        )
        .unwrap();
        assert_eq!(request[0]["content"], "temporary prompt");
        assert_eq!(
            serde_json::to_value(conversation.builtin_history).unwrap(),
            before
        );
    }

    #[test]
    fn editing_and_deleting_an_answer_preserve_interleaved_report_facts() {
        let mut conversation = conversation(json!([message("u", "user", "question")]));
        ensure_migrated(&mut conversation).unwrap();
        let mut source = history(&conversation).unwrap();
        let anchor = source.active_leaf();
        source.push(import_legacy(&json!({"role":"assistant","tool_calls":[{"id":"call","type":"function","function":{"name":"read","arguments":"{}"}}]})).unwrap());
        source.push(
            import_legacy(&json!({"role":"tool","tool_call_id":"call","content":"file body"}))
                .unwrap(),
        );
        source.push(tag(
            RuntimeMessage::text(ModelRole::Developer, "independent report"),
            "subagent-result-report",
        ));
        source.push(RuntimeMessage::text(ModelRole::Assistant, "answer"));
        commit_completed_run(&mut conversation, "a", &source, anchor, true).unwrap();
        conversation
            .messages
            .push(serde_json::from_value(message("a", "assistant", "answer")).unwrap());
        conversation.messages.push(
            serde_json::from_value(message(
                "subagent-result-report",
                "assistant",
                "independent report",
            ))
            .unwrap(),
        );
        let mut edited = conversation.clone();
        edit_message(&mut edited, "a", "edited answer").unwrap();
        assert!(texts(&edited)
            .iter()
            .any(|text| text == "independent report"));
        assert!(history(&edited)
            .unwrap()
            .context()
            .iter()
            .filter_map(RuntimeMessage::model_message)
            .flat_map(|message| &message.content)
            .any(|part| matches!(part,MessagePart::ToolCall{id,..} if id == "call")));
        delete_answer(&mut edited, "a").unwrap();
        assert_eq!(texts(&edited), ["question", "independent report"]);
        assert!(!needs_report_admission(&edited));
        delete_answer(&mut conversation, "a").unwrap();
        assert_eq!(texts(&conversation), ["question", "independent report"]);
        assert!(!needs_report_admission(&conversation));
    }

    #[test]
    fn idle_report_admission_happens_once_and_context_clear_does_not_revive_it() {
        let mut conversation = conversation(json!([message("u", "user", "question")]));
        ensure_migrated(&mut conversation).unwrap();
        conversation.messages.push(
            serde_json::from_value(message(
                "subagent-result-report",
                "assistant",
                "report body",
            ))
            .unwrap(),
        );
        assert!(needs_report_admission(&conversation));
        admit_pending_reports(&mut conversation).unwrap();
        assert!(!needs_report_admission(&conversation));
        assert!(texts(&conversation)
            .iter()
            .any(|text| text.contains("report body")));
        reset_context(&mut conversation).unwrap();
        admit_pending_reports(&mut conversation).unwrap();
        assert!(texts(&conversation).is_empty());
        assert!(!needs_report_admission(&conversation));
    }
}
