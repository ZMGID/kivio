use super::{history::RuntimeMessage, session::CompactionPlan};
use crate::model::{MessagePart, ModelRole};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntryId(pub u64);

impl Serialize for EntryId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for EntryId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(serde::de::Error::custom(
                "EntryId must be a decimal u64 string",
            ));
        }
        value.parse().map(Self).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryRange {
    /// First and last represented facts along a path, not a numeric ID interval.
    pub first: EntryId,
    pub last: EntryId,
}

/// A projected message points to its producing entry and the facts it represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextOrigin {
    pub entry_id: EntryId,
    pub index: usize,
    pub covered: EntryRange,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum SessionEntryKind {
    Message(RuntimeMessage),
    Compaction {
        covered: EntryRange,
        replaced: Vec<ContextOrigin>,
        #[serde(deserialize_with = "Option::deserialize")]
        retained_after: Option<ContextOrigin>,
        replacement: Vec<RuntimeMessage>,
    },
    ContextReset {
        replacement: Vec<RuntimeMessage>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionEntry {
    pub id: EntryId,
    #[serde(deserialize_with = "Option::deserialize")]
    pub parent_id: Option<EntryId>,
    pub kind: SessionEntryKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraftedBranch {
    pub leaf: Option<EntryId>,
    /// Fresh IDs for imported entries. Already shared facts keep their IDs.
    pub ids: HashMap<EntryId, EntryId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttachmentReference<'a> {
    pub entry_id: EntryId,
    pub reference: &'a str,
}

/// Owns immutable session facts and their current model context.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionHistory {
    entries: Vec<SessionEntry>,
    entry_indices: HashMap<EntryId, usize>,
    active_leaf: Option<EntryId>,
    next_id: u64,
    context: Vec<RuntimeMessage>,
    origins: Vec<ContextOrigin>,
    system_override: Option<(ContextOrigin, String)>,
}

impl From<Vec<RuntimeMessage>> for SessionHistory {
    fn from(messages: Vec<RuntimeMessage>) -> Self {
        let mut history = Self::default();
        history.extend(messages);
        history
    }
}

impl SessionHistory {
    /// Imports facts without executing tools. Inactive tips may end with pending calls.
    /// The selected path must contain complete tool rounds.
    pub fn from_entries(
        entries: Vec<SessionEntry>,
        active_leaf: Option<EntryId>,
    ) -> Result<Self, String> {
        let mut history = Self::default();
        for (index, entry) in entries.into_iter().enumerate() {
            if history.entry_indices.contains_key(&entry.id) {
                return Err(format!(
                    "entries[{index}] ({}): Duplicate session entry ID",
                    entry.id.0
                ));
            }
            if entry
                .parent_id
                .is_some_and(|id| !history.entry_indices.contains_key(&id))
            {
                return Err(format!(
                    "entries[{index}] ({}): Entry parent must precede its child",
                    entry.id.0
                ));
            }
            history
                .entry_indices
                .insert(entry.id, history.entries.len());
            history.entries.push(entry);
        }
        history.next_id = history
            .entries
            .iter()
            .map(|entry| entry.id.0)
            .max()
            .map_or(0, |id| id.wrapping_add(1));
        for tip in history.leaves() {
            history
                .project(Some(tip))
                .map_err(|error| format!("path ending at {}: {error}", tip.0))?;
        }
        history
            .switch_branch(active_leaf)
            .map_err(|error| format!("active_leaf: {error}"))?;
        Ok(history)
    }

    /// Copies the source tree below its anchor, retaining this history's selection.
    pub fn graft_branch(
        &mut self,
        source: &Self,
        source_anchor: Option<EntryId>,
    ) -> Result<GraftedBranch, String> {
        let source_path = source.path(source.active_leaf)?;
        let prefix_len = match source_anchor {
            Some(anchor) => {
                source_path
                    .iter()
                    .position(|entry| entry.id == anchor)
                    .ok_or("Graft anchor must be on the source selected path")?
                    + 1
            }
            None => 0,
        };
        if self.path(source_anchor)? != source_path[..prefix_len] {
            return Err("Graft shared ancestors must exactly match destination facts".into());
        }
        let mut candidate = self.clone();
        let mut ids = HashMap::new();
        let mut descendants = HashSet::new();
        for entry in source.entries() {
            if source_anchor.is_some()
                && entry.parent_id != source_anchor
                && !entry
                    .parent_id
                    .is_some_and(|parent| descendants.contains(&parent))
            {
                continue;
            }
            descendants.insert(entry.id);
            let parent = entry
                .parent_id
                .map(|id| ids.get(&id).copied().unwrap_or(id));
            if parent == entry.parent_id
                && self
                    .entry_indices
                    .get(&entry.id)
                    .is_some_and(|index| self.entries[*index] == *entry)
            {
                continue;
            }
            let remap = |id: EntryId| ids.get(&id).copied().unwrap_or(id);
            let range = |value: EntryRange| EntryRange {
                first: remap(value.first),
                last: remap(value.last),
            };
            let origin = |value: ContextOrigin| ContextOrigin {
                entry_id: remap(value.entry_id),
                index: value.index,
                covered: range(value.covered),
            };
            let mut kind = entry.kind.clone();
            if let SessionEntryKind::Compaction {
                covered,
                replaced,
                retained_after,
                ..
            } = &mut kind
            {
                *covered = range(*covered);
                for item in replaced {
                    *item = origin(*item);
                }
                *retained_after = retained_after.map(origin);
            }
            candidate.active_leaf = parent;
            let id = candidate.record(kind);
            ids.insert(entry.id, id);
        }
        let leaf = source
            .active_leaf
            .map(|id| ids.get(&id).copied().unwrap_or(id));
        Self::from_entries(candidate.entries.clone(), leaf)?;
        let mut candidate = Self::from_entries(candidate.entries, self.active_leaf)?;
        candidate.system_override.clone_from(&self.system_override);
        candidate.apply_system_override();
        *self = candidate;
        Ok(GraftedBranch { leaf, ids })
    }

    pub fn active_leaf(&self) -> Option<EntryId> {
        self.active_leaf
    }

    pub fn active_path(&self) -> Vec<&SessionEntry> {
        self.path(self.active_leaf)
            .expect("Selected session path exists")
    }

    /// Returns structural tips in append order, including unfinished tool rounds.
    pub fn leaves(&self) -> Vec<EntryId> {
        let parents: HashSet<_> = self
            .entries
            .iter()
            .filter_map(|entry| entry.parent_id)
            .collect();
        self.entries
            .iter()
            .filter(|entry| !parents.contains(&entry.id))
            .map(|entry| entry.id)
            .collect()
    }

    /// Selects an inclusive endpoint. None selects an empty path without deleting facts.
    pub fn switch_branch(&mut self, leaf: Option<EntryId>) -> Result<(), String> {
        let (context, origins) = self.project(leaf)?;
        if !complete_tool_rounds(&context) {
            return Err("Navigation must preserve complete tool rounds".into());
        }
        self.active_leaf = leaf;
        self.context = context;
        self.origins = origins;
        self.system_override = None;
        Ok(())
    }

    /// Selects an inclusive anchor for subsequent appends in the same session.
    pub fn fork(&mut self, anchor: EntryId) -> Result<(), String> {
        self.switch_branch(Some(anchor))
    }

    /// Selects the user's parent and returns the original user input for editing.
    pub fn rewind(&mut self, user: EntryId) -> Result<RuntimeMessage, String> {
        let entry = self.entry(user)?;
        let SessionEntryKind::Message(message) = &entry.kind else {
            return Err("Rewind requires a user message entry".into());
        };
        if message.role() != Some(ModelRole::User) {
            return Err("Rewind requires a user message entry".into());
        }
        let message = message.clone();
        self.switch_branch(entry.parent_id)?;
        Ok(message)
    }

    /// Keeps a user anchor, replaces it on a new path, or excludes an assistant anchor.
    pub fn prepare_regenerate(
        &mut self,
        anchor: EntryId,
        replacement: Option<RuntimeMessage>,
    ) -> Result<(), String> {
        let entry = self.entry(anchor)?;
        let SessionEntryKind::Message(message) = &entry.kind else {
            return Err("Regeneration requires a user or assistant message entry".into());
        };
        let target = match (message.role(), replacement.as_ref()) {
            (Some(ModelRole::Assistant), None) => entry.parent_id,
            (Some(ModelRole::User), None) => Some(anchor),
            (Some(ModelRole::User), Some(message))
                if message.role() == Some(ModelRole::User)
                    && complete_tool_rounds(std::slice::from_ref(message)) =>
            {
                entry.parent_id
            }
            _ => {
                return Err(
                    "Regeneration replacement must replace a user with a user message".into(),
                )
            }
        };
        self.switch_branch(target)?;
        if let Some(message) = replacement {
            self.push(message);
        }
        Ok(())
    }

    /// Enumerates typed host references in every retained fact, including inactive paths.
    pub fn attachment_references(&self) -> Vec<AttachmentReference<'_>> {
        let mut references = Vec::new();
        for entry in &self.entries {
            let messages = match &entry.kind {
                SessionEntryKind::Message(message) => std::slice::from_ref(message),
                SessionEntryKind::Compaction { replacement, .. }
                | SessionEntryKind::ContextReset { replacement } => replacement.as_slice(),
            };
            for message in messages.iter().filter_map(RuntimeMessage::model_message) {
                for part in &message.content {
                    match part {
                        MessagePart::Image {
                            path: Some(path), ..
                        }
                        | MessagePart::Video {
                            path: Some(path), ..
                        }
                        | MessagePart::ImageUrl { url: path, .. } => {
                            references.push(AttachmentReference {
                                entry_id: entry.id,
                                reference: path,
                            })
                        }
                        MessagePart::ToolResult { artifacts, .. } => {
                            references.extend(artifacts.iter().filter_map(|artifact| {
                                artifact
                                    .path
                                    .as_deref()
                                    .map(|reference| AttachmentReference {
                                        entry_id: entry.id,
                                        reference,
                                    })
                            }))
                        }
                        _ => {}
                    }
                }
            }
        }
        references
    }

    pub fn entries(&self) -> &[SessionEntry] {
        &self.entries
    }

    pub fn context(&self) -> &[RuntimeMessage] {
        &self.context
    }

    pub fn origins(&self) -> &[ContextOrigin] {
        &self.origins
    }

    fn record(&mut self, kind: SessionEntryKind) -> EntryId {
        while self.entry_indices.contains_key(&EntryId(self.next_id)) {
            self.next_id = self.next_id.wrapping_add(1);
        }
        let id = EntryId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        self.entry_indices.insert(id, self.entries.len());
        self.entries.push(SessionEntry {
            id,
            parent_id: self.active_leaf,
            kind,
        });
        self.active_leaf = Some(id);
        id
    }

    pub fn push(&mut self, message: RuntimeMessage) {
        let id = self.record(SessionEntryKind::Message(message.clone()));
        self.origins.push(ContextOrigin {
            entry_id: id,
            index: 0,
            covered: EntryRange {
                first: id,
                last: id,
            },
        });
        self.context.push(message);
    }

    pub fn extend(&mut self, messages: impl IntoIterator<Item = RuntimeMessage>) {
        for message in messages {
            self.push(message);
        }
    }

    pub(crate) fn validate_compaction(&self, plan: &CompactionPlan) -> Result<(), String> {
        self.validate_compaction_range(
            &self.context,
            &self.origins,
            &plan.covered,
            &plan.replacement,
        )
    }

    fn validate_compaction_range(
        &self,
        context: &[RuntimeMessage],
        origins: &[ContextOrigin],
        range: &std::ops::Range<usize>,
        replacement: &[RuntimeMessage],
    ) -> Result<(), String> {
        if range.start >= range.end || range.end > context.len() {
            return Err("Compaction must cover a nonempty current context range".into());
        }
        for boundary in [range.start, range.end] {
            if boundary > 0
                && boundary < origins.len()
                && origins[boundary - 1].entry_id == origins[boundary].entry_id
                && matches!(
                    self.entry(origins[boundary].entry_id)?.kind,
                    SessionEntryKind::Compaction { .. }
                )
            {
                return Err("Compaction cannot split an earlier replacement".into());
            }
        }
        if !complete_tool_rounds(&context[..range.start])
            || !complete_tool_rounds(&context[range.clone()])
            || !complete_tool_rounds(&context[range.end..])
            || !complete_tool_rounds(replacement)
        {
            return Err("Compaction must preserve complete tool rounds".into());
        }
        Ok(())
    }

    pub fn compact(&mut self, plan: CompactionPlan) -> Result<(), String> {
        self.validate_compaction(&plan)?;
        let replaced = self.origins[plan.covered.clone()].to_vec();
        let covered = EntryRange {
            first: replaced.first().unwrap().covered.first,
            last: replaced.last().unwrap().covered.last,
        };
        let replacement_len = plan.replacement.len();
        let id = self.record(SessionEntryKind::Compaction {
            covered,
            replaced,
            retained_after: self.origins.get(plan.covered.end).copied(),
            replacement: plan.replacement,
        });
        let origins = (0..replacement_len).map(|index| ContextOrigin {
            entry_id: id,
            index,
            covered,
        });
        self.origins.splice(plan.covered.clone(), origins);
        self.rebuild_context();
        Ok(())
    }

    /// An explicit reset is a recorded boundary, never an edit to earlier facts.
    pub fn reset_context(&mut self, replacement: Vec<RuntimeMessage>) -> Result<(), String> {
        if !complete_tool_rounds(&replacement) {
            return Err("Context reset must preserve complete tool rounds".into());
        }
        let replacement_len = replacement.len();
        let id = self.record(SessionEntryKind::ContextReset { replacement });
        self.origins = (0..replacement_len)
            .map(|index| ContextOrigin {
                entry_id: id,
                index,
                covered: EntryRange {
                    first: id,
                    last: id,
                },
            })
            .collect();
        self.rebuild_context();
        Ok(())
    }

    /// Overrides the first projected system message without changing stored facts.
    /// Histories without a system message are unchanged.
    pub fn set_system_prompt(&mut self, prompt: &str) {
        self.system_override = self
            .context
            .iter()
            .position(|message| matches!(message.body, super::history::RuntimeBody::System(_)))
            .map(|index| (self.origins[index], prompt.to_owned()));
        self.apply_system_override();
    }

    fn apply_system_override(&mut self) {
        if let Some((origin, prompt)) = &self.system_override {
            if let Some(index) = self
                .origins
                .iter()
                .position(|candidate| candidate == origin)
            {
                self.context[index].body = super::history::RuntimeBody::System(prompt.clone());
            }
        }
    }

    fn rebuild_context(&mut self) {
        self.context = self
            .origins
            .iter()
            .map(
                |origin| match &self.entries[self.entry_indices[&origin.entry_id]].kind {
                    SessionEntryKind::Message(message) => message.clone(),
                    SessionEntryKind::Compaction { replacement, .. }
                    | SessionEntryKind::ContextReset { replacement } => {
                        replacement[origin.index].clone()
                    }
                },
            )
            .collect();
        self.apply_system_override();
    }

    fn entry(&self, id: EntryId) -> Result<&SessionEntry, String> {
        self.entry_indices
            .get(&id)
            .map(|&index| &self.entries[index])
            .ok_or_else(|| "Unknown session entry ID".into())
    }

    fn path(&self, mut leaf: Option<EntryId>) -> Result<Vec<&SessionEntry>, String> {
        let mut path = Vec::new();
        while let Some(id) = leaf {
            let entry = self.entry(id)?;
            path.push(entry);
            leaf = entry.parent_id;
        }
        path.reverse();
        Ok(path)
    }

    fn project(
        &self,
        leaf: Option<EntryId>,
    ) -> Result<(Vec<RuntimeMessage>, Vec<ContextOrigin>), String> {
        let mut context = Vec::new();
        let mut origins = Vec::new();
        for entry in self.path(leaf)? {
            let single = EntryRange {
                first: entry.id,
                last: entry.id,
            };
            match &entry.kind {
                SessionEntryKind::Message(message) => {
                    context.push(message.clone());
                    origins.push(ContextOrigin {
                        entry_id: entry.id,
                        index: 0,
                        covered: single,
                    });
                }
                SessionEntryKind::ContextReset { replacement } => {
                    if !valid_tool_rounds(&context, true) || !complete_tool_rounds(replacement) {
                        return Err("Context reset contains invalid tool rounds".into());
                    }
                    context.clone_from(replacement);
                    origins = (0..replacement.len())
                        .map(|index| ContextOrigin {
                            entry_id: entry.id,
                            index,
                            covered: single,
                        })
                        .collect();
                }
                SessionEntryKind::Compaction {
                    covered,
                    replaced,
                    retained_after,
                    replacement,
                } => {
                    let start = replaced
                        .first()
                        .and_then(|first| origins.iter().position(|origin| origin == first))
                        .ok_or("Compaction must replace ancestor origins")?;
                    let end = start + replaced.len();
                    if origins.get(start..end) != Some(replaced.as_slice())
                        || origins.get(end) != retained_after.as_ref()
                        || covered.first != replaced.first().unwrap().covered.first
                        || covered.last != replaced.last().unwrap().covered.last
                    {
                        return Err(
                            "Compaction metadata must match contiguous ancestor origins".into()
                        );
                    }
                    self.validate_compaction_range(&context, &origins, &(start..end), replacement)?;
                    context.splice(start..end, replacement.iter().cloned());
                    origins.splice(
                        start..end,
                        (0..replacement.len()).map(|index| ContextOrigin {
                            entry_id: entry.id,
                            index,
                            covered: *covered,
                        }),
                    );
                }
            }
        }
        if !valid_tool_rounds(&context, true) {
            return Err("Session path contains invalid tool rounds".into());
        }
        Ok((context, origins))
    }
}

pub(crate) fn complete_tool_rounds(history: &[RuntimeMessage]) -> bool {
    valid_tool_rounds(history, false)
}

fn valid_tool_rounds(history: &[RuntimeMessage], allow_pending: bool) -> bool {
    let mut pending = HashSet::new();
    for message in history.iter().filter_map(RuntimeMessage::model_message) {
        if message.role != ModelRole::Tool && !pending.is_empty() {
            return false;
        }
        if message.role == ModelRole::Tool
            && !message
                .content
                .iter()
                .any(|part| matches!(part, MessagePart::ToolResult { .. }))
        {
            return false;
        }
        for part in &message.content {
            match part {
                MessagePart::ToolCall { id, .. } if message.role == ModelRole::Assistant => {
                    if !pending.insert(id.as_str()) {
                        return false;
                    }
                }
                MessagePart::ToolResult { tool_call_id, .. } if message.role == ModelRole::Tool => {
                    if !pending.remove(tool_call_id.as_str()) {
                        return false;
                    }
                }
                MessagePart::ToolCall { .. } | MessagePart::ToolResult { .. } => return false,
                _ => {}
            }
        }
    }
    allow_pending || pending.is_empty()
}

#[cfg(test)]
mod tests {
    use super::super::history::import_history;
    use super::*;
    use serde_json::json;

    fn message(text: &str) -> RuntimeMessage {
        RuntimeMessage::text(ModelRole::User, text)
    }

    fn tool_round() -> Vec<RuntimeMessage> {
        import_history(vec![
            json!({"role":"assistant", "content":"", "tool_calls":[{"id":"call", "type":"function", "function":{"name":"echo", "arguments":"{}"}}]}),
            json!({"role":"tool", "tool_call_id":"call", "content":"result"}),
        ])
    }

    #[test]
    fn graft_remaps_colliding_suffix_and_compaction_origins_without_selecting_it() {
        let mut destination = SessionHistory::from(vec![message("shared")]);
        let anchor = destination.active_leaf();
        let mut source = destination.clone();
        destination.push(message("destination answer"));
        source.push(message("source answer"));
        source.push(message("retained"));
        source
            .compact(CompactionPlan {
                covered: 0..2,
                replacement: vec![RuntimeMessage::system("summary")],
            })
            .unwrap();
        source
            .compact(CompactionPlan {
                covered: 0..2,
                replacement: vec![RuntimeMessage::system("second summary")],
            })
            .unwrap();
        let destination_context = destination.context().to_vec();
        let destination_leaf = destination.active_leaf();
        let result = destination.graft_branch(&source, anchor).unwrap();
        assert_eq!(destination.active_leaf(), destination_leaf);
        assert_eq!(destination.context(), destination_context);
        assert_eq!(result.ids.len(), 4);
        assert_ne!(result.ids[&EntryId(1)], EntryId(1));
        assert_eq!(
            result.leaf,
            Some(result.ids[&source.active_leaf().unwrap()])
        );
        destination.switch_branch(result.leaf).unwrap();
        assert_eq!(destination.context(), source.context());
        let encoded = serde_json::to_string(&destination.snapshot().unwrap()).unwrap();
        let restored: super::super::SessionSnapshot = serde_json::from_str(&encoded).unwrap();
        assert_eq!(restored.into_history().context(), source.context());
    }

    #[test]
    fn graft_retains_colliding_pending_tips_and_shared_future_facts() {
        let mut destination = SessionHistory::from(vec![message("shared")]);
        let anchor = destination.active_leaf();
        destination.push(message("existing future"));
        let original = destination.clone();
        let mut first = destination.clone();
        first.switch_branch(anchor).unwrap();
        first.extend(tool_round());
        first
            .compact(CompactionPlan {
                covered: 1..3,
                replacement: vec![RuntimeMessage::system("first summary")],
            })
            .unwrap();
        let completed = first.active_leaf();
        first.push(tool_round().remove(0));
        let pending = first.active_leaf().unwrap();
        first.switch_branch(completed).unwrap();
        let mut second = original.clone();
        second.switch_branch(anchor).unwrap();
        second.extend(import_history(vec![
            json!({"role":"assistant", "tool_calls":[{"id":"second-call", "type":"function", "function":{"name":"echo", "arguments":"{}"}}]}),
            json!({"role":"tool", "tool_call_id":"second-call", "content":"second result"}),
        ]));
        second.push(RuntimeMessage::text(ModelRole::Assistant, "second answer"));
        let first_graft = destination.graft_branch(&first, anchor).unwrap();
        let second_graft = destination.graft_branch(&second, anchor).unwrap();
        assert_eq!(destination.active_leaf(), original.active_leaf());
        assert_eq!(destination.context(), original.context());
        assert_eq!(
            destination.entries().len(),
            original.entries().len() + 4 + 3
        );
        assert!(!first_graft.ids.contains_key(&EntryId(1)));
        assert_ne!(first_graft.ids[&EntryId(2)], second_graft.ids[&EntryId(2)]);
        assert_eq!(
            destination
                .entries()
                .iter()
                .find(|entry| entry.id == first_graft.ids[&pending])
                .unwrap()
                .kind,
            first.entry(pending).unwrap().kind
        );
        let before = destination.clone();
        assert!(destination
            .switch_branch(Some(first_graft.ids[&pending]))
            .is_err());
        assert_eq!(destination, before);
        destination.switch_branch(first_graft.leaf).unwrap();
        assert_eq!(destination.context(), first.context());
        let restored: super::super::SessionSnapshot =
            serde_json::from_str(&serde_json::to_string(&destination.snapshot().unwrap()).unwrap())
                .unwrap();
        destination = restored.into_history();
        destination.switch_branch(second_graft.leaf).unwrap();
        assert_eq!(destination.context(), second.context());
    }

    #[test]
    fn graft_rejects_mismatched_ancestors_and_invalid_suffix_atomically() {
        let mut destination = SessionHistory::from(vec![message("shared")]);
        let before = destination.clone();
        let mismatch = SessionHistory::from(vec![message("different")]);
        assert!(destination
            .graft_branch(&mismatch, Some(EntryId(0)))
            .is_err());
        assert_eq!(destination, before);
        let mut invalid = destination.clone();
        invalid.push(tool_round().remove(1));
        assert!(destination
            .graft_branch(&invalid, Some(EntryId(0)))
            .is_err());
        assert_eq!(destination, before);
        assert!(destination
            .graft_branch(&mismatch, Some(EntryId(99)))
            .is_err());
        assert_eq!(destination, before);
        let appended = destination.graft_branch(&mismatch, None).unwrap();
        assert_eq!(destination.active_leaf(), before.active_leaf());
        destination.switch_branch(appended.leaf).unwrap();
        assert_eq!(destination.context(), mismatch.context());
    }

    #[test]
    fn snapshot_rejects_selected_pending_tools_but_preserves_inactive_pending_tips() {
        let mut history = SessionHistory::from(vec![message("shared")]);
        let selected = history.active_leaf();
        history.push(tool_round().remove(0));
        assert!(history.snapshot().is_err());
        history.switch_branch(selected).unwrap();
        let restored = history.snapshot().unwrap().into_history();
        assert_eq!(restored.entries(), history.entries());
        assert_eq!(restored.active_leaf(), selected);
    }

    #[test]
    fn branching_replays_only_selected_ancestors_across_compactions_and_resets() {
        let mut history = SessionHistory::from(vec![message("root"), message("old")]);
        history.fork(EntryId(0)).unwrap();
        history.push(message("sibling"));
        let sibling = history.active_leaf();
        history.switch_branch(Some(EntryId(1))).unwrap();
        history.push(message("tail"));
        history
            .compact(CompactionPlan {
                covered: 0..3,
                replacement: vec![RuntimeMessage::system("summary")],
            })
            .unwrap();
        history.push(message("next"));
        history
            .compact(CompactionPlan {
                covered: 0..2,
                replacement: vec![RuntimeMessage::system("second summary")],
            })
            .unwrap();
        let compacted = history.active_leaf();
        let original = history.entries().to_vec();
        assert_eq!(
            history.context(),
            &[RuntimeMessage::system("second summary")]
        );
        history.switch_branch(sibling).unwrap();
        assert_eq!(history.context(), &[message("root"), message("sibling")]);
        history.reset_context(vec![message("reset")]).unwrap();
        let reset = history.active_leaf();
        history.switch_branch(compacted).unwrap();
        assert_eq!(
            history.context(),
            &[RuntimeMessage::system("second summary")]
        );
        assert_eq!(&history.entries()[..original.len()], original);
        assert_eq!(history.leaves(), [compacted.unwrap(), reset.unwrap()]);
        assert_eq!(
            history
                .active_path()
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            [
                EntryId(0),
                EntryId(1),
                EntryId(3),
                EntryId(4),
                EntryId(5),
                EntryId(6)
            ]
        );
        let restored = SessionHistory::from_entries(history.entries().to_vec(), reset).unwrap();
        assert_eq!(restored.context(), &[message("reset")]);
        assert_eq!(restored.entries(), history.entries());
    }

    #[test]
    fn rewind_and_regenerate_preserve_original_messages_and_sources() {
        let mut user = message("question");
        user.source.insert("message_id".into(), json!("original"));
        let answer = RuntimeMessage::text(ModelRole::Assistant, "answer");
        let mut history =
            SessionHistory::from(vec![RuntimeMessage::system("system"), user.clone(), answer]);
        let original = history.entries().to_vec();
        assert_eq!(history.rewind(EntryId(1)).unwrap(), user);
        assert_eq!(history.active_leaf(), Some(EntryId(0)));
        history.prepare_regenerate(EntryId(1), None).unwrap();
        assert_eq!(history.context().last(), Some(&user));
        history.prepare_regenerate(EntryId(2), None).unwrap();
        assert_eq!(history.active_leaf(), Some(EntryId(1)));
        history
            .prepare_regenerate(EntryId(1), Some(message("edited")))
            .unwrap();
        assert_eq!(
            history.entries().last().unwrap().parent_id,
            Some(EntryId(0))
        );
        assert_eq!(
            history.context(),
            &[RuntimeMessage::system("system"), message("edited")]
        );
        assert_eq!(&history.entries()[..3], original);
        let before = history.clone();
        for (anchor, replacement) in [
            (EntryId(0), None),
            (EntryId(2), Some(message("wrong anchor"))),
            (
                EntryId(1),
                Some(RuntimeMessage::text(ModelRole::Assistant, "wrong role")),
            ),
            (EntryId(99), None),
        ] {
            assert!(history.prepare_regenerate(anchor, replacement).is_err());
            assert_eq!(history, before);
        }
        assert!(history.rewind(EntryId(2)).is_err());
        assert_eq!(history, before);
        history.switch_branch(None).unwrap();
        history.push(message("new root"));
        assert_eq!(history.context(), &[message("new root")]);
        assert_eq!(history.entries().last().unwrap().parent_id, None);
    }

    #[test]
    fn imports_non_dense_ids_and_appends_above_the_highest_id() {
        let entries = vec![
            SessionEntry {
                id: EntryId(40),
                parent_id: None,
                kind: SessionEntryKind::Message(message("root")),
            },
            SessionEntry {
                id: EntryId(7),
                parent_id: Some(EntryId(40)),
                kind: SessionEntryKind::Message(message("child")),
            },
            SessionEntry {
                id: EntryId(90),
                parent_id: Some(EntryId(40)),
                kind: SessionEntryKind::Message(message("sibling")),
            },
        ];
        let mut history = SessionHistory::from_entries(entries, Some(EntryId(7))).unwrap();
        history
            .compact(CompactionPlan {
                covered: 0..2,
                replacement: vec![message("summary")],
            })
            .unwrap();
        assert_eq!(history.active_leaf(), Some(EntryId(91)));
        assert_eq!(
            history.entries().last().unwrap().parent_id,
            Some(EntryId(7))
        );
        let mut restored =
            SessionHistory::from_entries(history.entries().to_vec(), history.active_leaf())
                .unwrap();
        assert_eq!(restored.context(), &[message("summary")]);
        restored.switch_branch(Some(EntryId(90))).unwrap();
        assert_eq!(restored.context(), &[message("root"), message("sibling")]);
    }

    #[test]
    fn import_rejects_invalid_graphs() {
        let original = SessionHistory::from(vec![message("a"), message("b")])
            .entries()
            .to_vec();
        let mut invalid = Vec::new();
        let mut entries = original.clone();
        entries[1].id = entries[0].id;
        invalid.push(entries);
        for parent in [EntryId(1), EntryId(99)] {
            let mut entries = original.clone();
            entries[0].parent_id = Some(parent);
            invalid.push(entries);
        }
        let mut entries = original.clone();
        entries[1].parent_id = Some(EntryId(1));
        invalid.push(entries);
        for entries in invalid {
            assert!(SessionHistory::from_entries(entries, None).is_err());
        }
        assert!(SessionHistory::from_entries(original, Some(EntryId(99))).is_err());
    }

    #[test]
    fn imported_high_ids_wrap_and_skip_existing_ids_when_appending() {
        let mut history = SessionHistory::from_entries(
            vec![
                SessionEntry {
                    id: EntryId(0),
                    parent_id: None,
                    kind: SessionEntryKind::Message(message("root")),
                },
                SessionEntry {
                    id: EntryId(u64::MAX - 1),
                    parent_id: Some(EntryId(0)),
                    kind: SessionEntryKind::Message(message("imported")),
                },
            ],
            Some(EntryId(u64::MAX - 1)),
        )
        .unwrap();
        history.push(message("max ID"));
        assert_eq!(history.active_leaf(), Some(EntryId(u64::MAX)));
        let mut history =
            SessionHistory::from_entries(history.entries().to_vec(), history.active_leaf())
                .unwrap();
        history.push(message("wrapped ID"));
        assert_eq!(history.active_leaf(), Some(EntryId(1)));
        history.push(message("next ID"));
        assert_eq!(history.active_leaf(), Some(EntryId(2)));
        assert_eq!(
            history.context(),
            &[
                message("root"),
                message("imported"),
                message("max ID"),
                message("wrapped ID"),
                message("next ID")
            ]
        );
    }

    #[test]
    fn import_rejects_forged_compaction_origins_and_boundaries() {
        let mut history =
            SessionHistory::from(vec![message("a"), message("b"), message("retained")]);
        history
            .compact(CompactionPlan {
                covered: 0..2,
                replacement: vec![message("summary")],
            })
            .unwrap();
        let original = history.entries().to_vec();
        for mutation in 0..5 {
            let mut entries = original.clone();
            if let SessionEntryKind::Compaction {
                covered,
                replaced,
                retained_after,
                ..
            } = &mut entries[3].kind
            {
                match mutation {
                    0 => replaced[0].entry_id = EntryId(99),
                    1 => replaced[0].index = 1,
                    2 => replaced[0].covered.last = EntryId(1),
                    3 => *retained_after = None,
                    _ => covered.last = EntryId(2),
                }
            }
            assert!(SessionHistory::from_entries(entries, Some(EntryId(3))).is_err());
        }
        let mut entries = original;
        entries[3].parent_id = Some(EntryId(0));
        assert!(SessionHistory::from_entries(entries, None).is_err());

        history
            .compact(CompactionPlan {
                covered: 0..1,
                replacement: vec![message("one"), message("two")],
            })
            .unwrap();
        let first = history.origins()[0];
        let split = SessionEntry {
            id: EntryId(5),
            parent_id: history.active_leaf(),
            kind: SessionEntryKind::Compaction {
                covered: first.covered,
                replaced: vec![first],
                retained_after: Some(history.origins()[1]),
                replacement: vec![],
            },
        };
        let mut entries = history.entries().to_vec();
        entries.push(split);
        assert!(SessionHistory::from_entries(entries, None).is_err());
    }

    #[test]
    fn navigation_rejects_partial_tool_rounds_and_import_keeps_unfinished_inactive_tips() {
        let mut history = SessionHistory::from(vec![message("root")]);
        history.extend(tool_round());
        let complete = history.active_leaf();
        let before = history.clone();
        assert!(history.fork(EntryId(1)).is_err());
        assert_eq!(history, before);
        history.fork(EntryId(0)).unwrap();
        history.push(tool_round().remove(0));
        let pending = history.active_leaf();
        let entries = history.entries().to_vec();
        let restored = SessionHistory::from_entries(entries.clone(), complete).unwrap();
        assert_eq!(restored.context(), before.context());
        assert!(SessionHistory::from_entries(entries.clone(), pending).is_err());
        for invalid_tail in [message("interrupt"), tool_round().remove(1)] {
            let mut malformed = entries.clone();
            let parent = if invalid_tail.role() == Some(ModelRole::Tool) {
                complete
            } else {
                pending
            };
            malformed.push(SessionEntry {
                id: EntryId(4),
                parent_id: parent,
                kind: SessionEntryKind::Message(invalid_tail),
            });
            assert!(SessionHistory::from_entries(malformed, complete).is_err());
        }
    }

    #[test]
    fn navigation_discards_transient_prompt_override_without_changing_shared_facts() {
        let mut history =
            SessionHistory::from(vec![RuntimeMessage::system("original"), message("one")]);
        history.set_system_prompt("fallback");
        let before = history.clone();
        assert!(history.switch_branch(Some(EntryId(99))).is_err());
        assert_eq!(history, before);
        history.fork(EntryId(0)).unwrap();
        assert_eq!(history.context(), &[RuntimeMessage::system("original")]);
        history.push(message("two"));
        history.switch_branch(Some(EntryId(1))).unwrap();
        assert_eq!(history.context()[0], RuntimeMessage::system("original"));
    }

    #[test]
    fn references_include_inactive_facts_and_replacements_without_parsing_sources() {
        use crate::model::{ModelArtifact, ModelMessage};
        let mut input = RuntimeMessage::model(ModelMessage {
            role: ModelRole::User,
            content: vec![
                MessagePart::Image {
                    mime_type: "image/png".into(),
                    data: String::new(),
                    path: Some("original.png".into()),
                    detail: None,
                },
                MessagePart::Video {
                    mime_type: "video/mp4".into(),
                    data: String::new(),
                    path: Some("video.mp4".into()),
                },
            ],
        });
        input
            .source
            .insert("attachment_path".into(), json!("not-a-typed-reference"));
        let url = RuntimeMessage::model(ModelMessage {
            role: ModelRole::User,
            content: vec![MessagePart::ImageUrl {
                url: "https://example.org/image.png".into(),
                detail: None,
            }],
        });
        let mut history = SessionHistory::from(vec![input]);
        let mut round = tool_round();
        if let MessagePart::ToolResult { artifacts, .. } =
            &mut round[1].model_message_mut().unwrap().content[0]
        {
            artifacts.push(ModelArtifact {
                id: None,
                name: "result".into(),
                mime_type: "image/png".into(),
                data_url: String::new(),
                size_bytes: None,
                path: Some("result.png".into()),
            });
        }
        history.extend(round);
        history
            .compact(CompactionPlan {
                covered: 0..3,
                replacement: vec![url.clone()],
            })
            .unwrap();
        history.fork(EntryId(0)).unwrap();
        history.reset_context(vec![url]).unwrap();
        assert_eq!(
            history
                .attachment_references()
                .iter()
                .map(|reference| (reference.entry_id, reference.reference))
                .collect::<Vec<_>>(),
            [
                (EntryId(0), "original.png"),
                (EntryId(0), "video.mp4"),
                (EntryId(2), "result.png"),
                (EntryId(3), "https://example.org/image.png"),
                (EntryId(4), "https://example.org/image.png"),
            ]
        );
    }

    #[test]
    fn repeated_compactions_keep_facts_and_resolve_summary_coverage() {
        let mut history = SessionHistory::from(vec![message("a"), message("b"), message("c")]);
        let original = history.entries().to_vec();
        history
            .compact(CompactionPlan {
                covered: 0..2,
                replacement: vec![RuntimeMessage::system("summary one")],
            })
            .unwrap();
        history.push(message("d"));
        history
            .compact(CompactionPlan {
                covered: 0..2,
                replacement: vec![RuntimeMessage::system("summary two")],
            })
            .unwrap();
        assert_eq!(&history.entries()[..3], original);
        assert_eq!(
            history.context(),
            &[RuntimeMessage::system("summary two"), message("d")]
        );
        match &history.entries().last().unwrap().kind {
            SessionEntryKind::Compaction {
                covered,
                replaced,
                retained_after,
                ..
            } => {
                assert_eq!(
                    *covered,
                    EntryRange {
                        first: EntryId(0),
                        last: EntryId(2)
                    }
                );
                assert_eq!(replaced[0].entry_id, EntryId(3));
                assert_eq!(retained_after.unwrap().entry_id, EntryId(4));
            }
            _ => panic!("expected compaction"),
        }
        for pair in history.entries().windows(2) {
            assert_eq!(pair[1].parent_id, Some(pair[0].id));
        }
    }

    #[test]
    fn reset_keeps_facts_and_allows_compacting_only_older_reset_messages() {
        let mut history = SessionHistory::from(vec![message("old")]);
        let original = history.entries()[0].clone();
        history
            .reset_context(vec![
                message("reset a"),
                message("reset b"),
                message("recent"),
            ])
            .unwrap();
        history
            .compact(CompactionPlan {
                covered: 0..2,
                replacement: vec![RuntimeMessage::system("summary")],
            })
            .unwrap();
        assert_eq!(history.entries()[0], original);
        assert_eq!(
            history.context(),
            &[RuntimeMessage::system("summary"), message("recent")]
        );
        match &history.entries()[2].kind {
            SessionEntryKind::Compaction {
                replaced,
                retained_after,
                ..
            } => {
                assert_eq!(
                    replaced
                        .iter()
                        .map(|origin| origin.index)
                        .collect::<Vec<_>>(),
                    [0, 1]
                );
                assert_eq!(retained_after.unwrap().index, 2);
            }
            _ => panic!("expected compaction"),
        }
    }

    #[test]
    fn invalid_tool_and_summary_boundaries_leave_history_unchanged() {
        let mut history = SessionHistory::from(import_history(vec![
            json!({"role":"assistant", "content":"", "tool_calls":[{"id":"call", "type":"function", "function":{"name":"echo", "arguments":"{}"}}]}),
            json!({"role":"tool", "tool_call_id":"call", "content":"result"}),
            json!({"role":"user", "content":"next"}),
        ]));
        for covered in [0..1, 1..2] {
            let before = history.clone();
            assert!(history
                .compact(CompactionPlan {
                    covered,
                    replacement: vec![message("summary")]
                })
                .is_err());
            assert_eq!(history, before);
        }
        history
            .compact(CompactionPlan {
                covered: 0..2,
                replacement: vec![message("summary a"), message("summary b")],
            })
            .unwrap();
        let before = history.clone();
        assert!(history
            .compact(CompactionPlan {
                covered: 0..1,
                replacement: vec![]
            })
            .is_err());
        assert_eq!(history, before);
    }

    #[test]
    fn prompt_override_never_changes_facts_or_overwrites_replacement_summaries() {
        let mut history = SessionHistory::from(vec![
            RuntimeMessage::system("original"),
            message("question"),
        ]);
        let original = history.entries().to_vec();
        history.set_system_prompt("fallback");
        assert_eq!(history.context()[0].text_content(), "fallback");
        assert_eq!(history.entries(), original);
        history
            .compact(CompactionPlan {
                covered: 0..2,
                replacement: vec![RuntimeMessage::system("summary")],
            })
            .unwrap();
        history.push(message("next"));
        assert_eq!(history.context()[0].text_content(), "summary");
        assert_eq!(&history.entries()[..2], original);
        history
            .reset_context(vec![RuntimeMessage::system("reset system")])
            .unwrap();
        assert_eq!(history.context()[0].text_content(), "reset system");
    }
}
