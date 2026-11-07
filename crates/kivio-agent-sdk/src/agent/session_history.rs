use super::{history::RuntimeMessage, session::CompactionPlan};
use crate::model::{MessagePart, ModelRole};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntryId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryRange {
    /// Inclusive bounds in the immutable entry log.
    pub first: EntryId,
    pub last: EntryId,
}

/// A projected message points to its producing entry and the facts it represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextOrigin {
    pub entry_id: EntryId,
    pub index: usize,
    pub covered: EntryRange,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SessionEntryKind {
    Message(RuntimeMessage),
    Compaction {
        covered: EntryRange,
        replaced: Vec<ContextOrigin>,
        retained_after: Option<ContextOrigin>,
        replacement: Vec<RuntimeMessage>,
    },
    ContextReset {
        replacement: Vec<RuntimeMessage>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionEntry {
    pub id: EntryId,
    pub parent_id: Option<EntryId>,
    pub kind: SessionEntryKind,
}

/// Owns immutable session facts and their current model context.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionHistory {
    entries: Vec<SessionEntry>,
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
        let id = EntryId(self.entries.len() as u64);
        self.entries.push(SessionEntry {
            id,
            parent_id: self.entries.last().map(|entry| entry.id),
            kind,
        });
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
        let range = &plan.covered;
        if range.start >= range.end || range.end > self.context.len() {
            return Err("Compaction must cover a nonempty current context range".into());
        }
        for boundary in [range.start, range.end] {
            if boundary > 0
                && boundary < self.origins.len()
                && self.origins[boundary - 1].entry_id == self.origins[boundary].entry_id
                && matches!(
                    self.entries[self.origins[boundary].entry_id.0 as usize].kind,
                    SessionEntryKind::Compaction { .. }
                )
            {
                return Err("Compaction cannot split an earlier replacement".into());
            }
        }
        if !complete_tool_rounds(&self.context[..range.start])
            || !complete_tool_rounds(&self.context[range.clone()])
            || !complete_tool_rounds(&self.context[range.end..])
            || !complete_tool_rounds(&plan.replacement)
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

    pub(crate) fn set_system_prompt(&mut self, prompt: &str) {
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
                |origin| match &self.entries[origin.entry_id.0 as usize].kind {
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
}

pub(crate) fn complete_tool_rounds(history: &[RuntimeMessage]) -> bool {
    let mut pending = std::collections::HashSet::new();
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
    pending.is_empty()
}

#[cfg(test)]
mod tests {
    use super::super::history::import_history;
    use super::*;
    use serde_json::json;

    fn message(text: &str) -> RuntimeMessage {
        RuntimeMessage::text(ModelRole::User, text)
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
