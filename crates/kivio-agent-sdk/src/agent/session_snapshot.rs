use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{EntryId, SessionEntry, SessionHistory};
use crate::model::{GenerateOptions, ModelCapabilities, ToolChoice};

pub const SESSION_SNAPSHOT_VERSION: u32 = 1;

/// Durable facts and selection, validated before construction. Hosts rebind live resources.
#[derive(Debug, Clone)]
pub struct SessionSnapshot {
    history: SessionHistory,
    config: Option<SessionSnapshotConfig>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionSnapshotConfig {
    pub provider_id: String,
    pub provider_label: String,
    pub api_format: String,
    pub model: String,
    pub conversation_id: String,
    pub options: SessionSnapshotOptions,
    pub capabilities: ModelCapabilities,
}

/// Only standard generation settings are durable. Hosts rebind vendor options.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionSnapshotOptions {
    pub temperature: Option<f64>,
    pub max_tokens: u32,
    pub thinking_enabled: bool,
    pub thinking_level: Option<String>,
    pub builtin_web_search: bool,
    pub tool_choice: ToolChoice,
}

impl From<&GenerateOptions> for SessionSnapshotOptions {
    fn from(options: &GenerateOptions) -> Self {
        Self {
            temperature: options.temperature,
            max_tokens: options.max_tokens,
            thinking_enabled: options.thinking_enabled,
            thinking_level: options.thinking_level.clone(),
            builtin_web_search: options.builtin_web_search,
            tool_choice: options.tool_choice,
        }
    }
}

impl From<SessionSnapshotOptions> for GenerateOptions {
    fn from(options: SessionSnapshotOptions) -> Self {
        Self {
            temperature: options.temperature,
            max_tokens: options.max_tokens,
            thinking_enabled: options.thinking_enabled,
            thinking_level: options.thinking_level,
            builtin_web_search: options.builtin_web_search,
            tool_choice: options.tool_choice,
            ..Self::default()
        }
    }
}

impl SessionHistory {
    /// Checks all facts and complete selected tool rounds; transient overrides are excluded.
    pub fn snapshot(&self) -> Result<SessionSnapshot, String> {
        Ok(SessionSnapshot {
            history: Self::from_entries(self.entries().to_vec(), self.active_leaf())?,
            config: None,
        })
    }
}

impl SessionSnapshot {
    pub fn version(&self) -> u32 {
        SESSION_SNAPSHOT_VERSION
    }

    pub fn entries(&self) -> &[SessionEntry] {
        self.history.entries()
    }

    pub fn history(&self) -> &SessionHistory {
        &self.history
    }

    pub fn active_leaf(&self) -> Option<EntryId> {
        self.history.active_leaf()
    }

    pub fn config(&self) -> Option<&SessionSnapshotConfig> {
        self.config.as_ref()
    }

    pub fn with_config(mut self, config: SessionSnapshotConfig) -> Self {
        self.config = Some(config);
        self
    }

    pub fn into_history(self) -> SessionHistory {
        self.history
    }
}

impl Serialize for SessionSnapshot {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Snapshot<'a> {
            version: u32,
            entries: &'a [SessionEntry],
            active_leaf: Option<EntryId>,
            #[serde(skip_serializing_if = "Option::is_none")]
            config: Option<&'a SessionSnapshotConfig>,
        }
        Snapshot {
            version: self.version(),
            entries: self.entries(),
            active_leaf: self.active_leaf(),
            config: self.config(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SessionSnapshot {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Snapshot {
            version: u32,
            entries: Vec<serde_json::Value>,
            #[serde(deserialize_with = "Option::deserialize")]
            active_leaf: Option<EntryId>,
            config: Option<SessionSnapshotConfig>,
        }
        let snapshot = Snapshot::deserialize(deserializer)?;
        if snapshot.version != SESSION_SNAPSHOT_VERSION {
            return Err(serde::de::Error::custom(format!(
                "Unsupported session snapshot version {}",
                snapshot.version
            )));
        }
        let entries = snapshot
            .entries
            .into_iter()
            .enumerate()
            .map(|(index, entry)| {
                serde_json::from_value(entry)
                    .map_err(|error| serde::de::Error::custom(format!("entries[{index}]: {error}")))
            })
            .collect::<Result<Vec<SessionEntry>, D::Error>>()?;
        let history = SessionHistory::from_entries(entries, snapshot.active_leaf)
            .map_err(serde::de::Error::custom)?;
        Ok(Self {
            history,
            config: snapshot.config,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agent::{session::CompactionPlan, RuntimeMessage, SessionEntryKind},
        model::ModelRole,
    };
    use serde_json::{json, Value};

    fn history() -> SessionHistory {
        SessionHistory::from(vec![
            RuntimeMessage::system("original"),
            RuntimeMessage::text(ModelRole::User, "question"),
        ])
    }

    #[test]
    fn round_trip_keeps_branches_compactions_sources_and_maximum_ids() {
        let mut history = history();
        let mut answer = RuntimeMessage::text(ModelRole::Assistant, "answer");
        answer.source.insert("host".into(), json!({"id":"opaque"}));
        history.push(answer);
        let first = history.active_leaf();
        history
            .compact(CompactionPlan {
                covered: 1..3,
                replacement: vec![RuntimeMessage::system("summary")],
            })
            .unwrap();
        history.fork(EntryId(1)).unwrap();
        history
            .reset_context(vec![RuntimeMessage::text(ModelRole::User, "reset")])
            .unwrap();
        history.switch_branch(first).unwrap();
        history.set_system_prompt("transient");
        let snapshot = history.snapshot().unwrap();
        let encoded = serde_json::to_value(&snapshot).unwrap();
        assert!(!encoded.to_string().contains("transient"));
        assert_eq!(encoded["entries"][0]["id"], "0");
        assert!(encoded.get("config").is_none());
        let restored = serde_json::from_value::<SessionSnapshot>(encoded)
            .unwrap()
            .into_history();
        assert_eq!(restored.entries(), history.entries());
        assert_eq!(restored.active_leaf(), first);
        assert_eq!(restored.context()[0].text_content(), "original");

        let high = SessionHistory::from_entries(
            vec![SessionEntry {
                id: EntryId(u64::MAX),
                parent_id: None,
                kind: SessionEntryKind::Message(RuntimeMessage::text(ModelRole::User, "max")),
            }],
            Some(EntryId(u64::MAX)),
        )
        .unwrap();
        let encoded = serde_json::to_string(&high.snapshot().unwrap()).unwrap();
        assert!(encoded.contains("\"18446744073709551615\""));
        let mut restored = serde_json::from_str::<SessionSnapshot>(&encoded)
            .unwrap()
            .into_history();
        restored.push(RuntimeMessage::text(ModelRole::Assistant, "next"));
        assert_eq!(restored.active_leaf(), Some(EntryId(0)));
    }

    #[test]
    fn malformed_snapshots_fail_with_entry_location_without_dropping_payloads() {
        let original = serde_json::to_value(history().snapshot().unwrap()).unwrap();
        for invalid in [
            json!(0),
            json!(1.5),
            json!(-1),
            json!("-1"),
            json!("1.5"),
            json!("+1"),
            json!("18446744073709551616"),
        ] {
            let mut value = original.clone();
            value["entries"][0]["id"] = invalid;
            let error = serde_json::from_value::<SessionSnapshot>(value)
                .unwrap_err()
                .to_string();
            assert!(error.contains("entries[0]"), "{error}");
        }
        for pointer in ["/entries/0/kind/type", "/entries/0/kind/value/body/type"] {
            let mut value = original.clone();
            *value.pointer_mut(pointer).unwrap() = json!("future_required_payload");
            assert!(serde_json::from_value::<SessionSnapshot>(value)
                .unwrap_err()
                .to_string()
                .contains("entries[0]"));
        }
        let mut unknown = original.clone();
        unknown["version"] = json!(2);
        assert!(serde_json::from_value::<SessionSnapshot>(unknown)
            .unwrap_err()
            .to_string()
            .contains("Unsupported"));
        let mut bad_parent = original.clone();
        bad_parent["entries"][1]["parent_id"] = json!("900");
        assert!(serde_json::from_value::<SessionSnapshot>(bad_parent)
            .unwrap_err()
            .to_string()
            .contains("entries[1]"));
        let mut bad_leaf = original;
        bad_leaf["active_leaf"] = json!("900");
        assert!(serde_json::from_value::<SessionSnapshot>(bad_leaf)
            .unwrap_err()
            .to_string()
            .contains("active_leaf"));
    }

    #[test]
    fn config_cannot_persist_or_accept_vendor_options() {
        let options = GenerateOptions {
            provider_options: json!({"api_key":"secret"}),
            ..GenerateOptions::default()
        };
        let config = SessionSnapshotConfig {
            provider_id: "provider".into(),
            provider_label: "Provider".into(),
            api_format: "chat".into(),
            model: "model".into(),
            conversation_id: "conversation".into(),
            options: (&options).into(),
            capabilities: ModelCapabilities::default(),
        };
        let mut encoded =
            serde_json::to_value(history().snapshot().unwrap().with_config(config)).unwrap();
        assert!(!encoded.to_string().contains("secret"));
        let restored = serde_json::from_value::<SessionSnapshot>(encoded.clone()).unwrap();
        let restored_options: GenerateOptions = restored.config().unwrap().options.clone().into();
        assert_eq!(restored_options.provider_options, json!({}));
        encoded["config"]["options"]["provider_options"] = Value::Null;
        assert!(serde_json::from_value::<SessionSnapshot>(encoded).is_err());
    }
}
