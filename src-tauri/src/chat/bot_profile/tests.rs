use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Barrier};
use std::thread;

use super::memory_scope::private_memory_prompt;
use super::policy::{self, resolve_capability, GrantSet};
use super::store::ProfileStore;
use super::types::{
    ApprovalSource, BotApprovalWrite, BotCapabilityWrite, BotCreateRequest, BotEditRequest,
    BotModelWrite, BotProfile, CapabilitySource, ModelSource, PROFILE_VERSION,
};
use crate::chat::memory::{self, MemoryLayer};
use crate::chat::storage::remove_assistant_from_index;
use crate::chat::{ChatAssistant, ChatAssistantIndex, Conversation};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("kivio-bot-profile-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn assistant(id: &str, name: &str) -> ChatAssistant {
    ChatAssistant {
        id: id.to_string(),
        name: name.to_string(),
        description: String::new(),
        icon: String::new(),
        color: "0".into(),
        source: "user".into(),
        system_prompt: "keep-prompt".into(),
        provider_id: String::new(),
        model: String::new(),
        mcp_server_ids: Vec::new(),
        skill_ids: Vec::new(),
        enabled: true,
        installed: true,
        archived: false,
        built_in: false,
        created_at: 10,
        updated_at: 10,
    }
}

fn profile(id: &str, primary: bool) -> BotProfile {
    BotProfile {
        version: PROFILE_VERSION,
        assistant_id: id.to_string(),
        primary,
        capability: policy::granted_defaults_policy(),
        model: policy::inherit_model(),
        approval: policy::inherit_approval(),
        shared_memory: Vec::new(),
        revision: 0,
        created_at: 10,
        updated_at: 10,
    }
}

#[test]
fn corrupt_profile_is_not_replaced() {
    let dir = TempDir::new();
    let store = ProfileStore::new(dir.0.clone());
    let path = dir.0.join("asst_broken.json");
    fs::write(&path, "{").unwrap();
    assert!(store.load("asst_broken").is_err());
    assert_eq!(fs::read(&path).unwrap(), b"{");
    let insert = store.insert_new(&profile("asst_broken", false));
    assert!(insert.is_err());
    assert_eq!(fs::read(&path).unwrap(), b"{");
}

#[test]
fn unsupported_version_is_not_overwritten() {
    let dir = TempDir::new();
    let path = dir.0.join("asst_old.json");
    fs::write(&path, r#"{"version":99,"assistantId":"asst_old"}"#).unwrap();
    let store = ProfileStore::new(dir.0.clone());
    assert!(store.load("asst_old").is_err());
    assert!(fs::read_to_string(&path).unwrap().contains("99"));
}

#[test]
fn failed_create_rollback_restores_the_previous_primary() {
    let dir = TempDir::new();
    let store = ProfileStore::new(dir.0.clone());
    store.insert_new(&profile("asst_old", true)).unwrap();
    let created = store.insert_new(&profile("asst_new", true)).unwrap();
    assert!(!store.load("asst_old").unwrap().unwrap().primary);
    assert!(store.load("asst_new").unwrap().unwrap().primary);
    store.rollback_insert(&created).unwrap();
    assert!(store.load("asst_old").unwrap().unwrap().primary);
    assert!(store.load("asst_new").unwrap().is_none());
}

#[test]
fn primary_updates_leave_exactly_one_primary() {
    let dir = TempDir::new();
    let store = Arc::new(ProfileStore::new(dir.0.clone()));
    store.insert_new(&profile("asst_a", true)).unwrap();
    store.insert_new(&profile("asst_b", false)).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for id in ["asst_a", "asst_b"] {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            for _ in 0..30 {
                store
                    .update(id, |profile| {
                        profile.primary = true;
                        Ok(())
                    })
                    .unwrap();
            }
        }));
    }
    for handle in handles {
        handle.join().unwrap();
    }
    let profiles = store.list().unwrap();
    assert_eq!(profiles.iter().filter(|profile| profile.primary).count(), 1);
    assert_eq!(profiles.len(), 2);
}

#[test]
fn profile_updates_do_not_lose_increments() {
    let dir = TempDir::new();
    let store = Arc::new(ProfileStore::new(dir.0.clone()));
    store.insert_new(&profile("asst_count", false)).unwrap();
    let mut handles = Vec::new();
    for _ in 0..8 {
        let store = Arc::clone(&store);
        handles.push(thread::spawn(move || {
            for _ in 0..20 {
                store
                    .update("asst_count", |profile| {
                        profile.updated_at += 1;
                        Ok(())
                    })
                    .unwrap();
            }
        }));
    }
    for handle in handles {
        handle.join().unwrap();
    }
    let saved = store.load("asst_count").unwrap().unwrap();
    assert_eq!(saved.updated_at, 170);
    assert_eq!(saved.created_at, 10);
}

#[test]
fn editor_revision_conflict_keeps_the_newer_profile() {
    assert!(super::lifecycle::plan_revision(4, Some(4)).unwrap() == 5);
    assert!(super::lifecycle::plan_revision(4, None).unwrap() == 5);
    let conflict = super::lifecycle::plan_revision(5, Some(4)).unwrap_err();
    assert!(conflict.contains("没有覆盖"));

    let dir = TempDir::new();
    let store = ProfileStore::new(dir.0.clone());
    let mut first = profile("asst_ada", false);
    first.revision = 1;
    first.updated_at = 20;
    store.insert_new(&first).unwrap();
    let mut winner = first.clone();
    winner.revision = 2;
    winner.updated_at = 30;
    store.compare_and_swap(1, &winner).unwrap();
    let mut stale = first.clone();
    stale.revision = 2;
    stale.updated_at = 21;
    let err = store.compare_and_swap(1, &stale).unwrap_err();
    assert!(err.contains("没有覆盖"));
    let saved = store.load("asst_ada").unwrap().unwrap();
    assert_eq!(saved.revision, 2);
    assert_eq!(saved.updated_at, 30);

    store
        .restore_if_current_revision("asst_ada", 2, &first)
        .unwrap();
    let restored = store.load("asst_ada").unwrap().unwrap();
    assert_eq!(restored.revision, 1);
    assert_eq!(restored.updated_at, 20);

    let mut newer = restored.clone();
    newer.revision = 2;
    newer.updated_at = 40;
    store.compare_and_swap(1, &newer).unwrap();
    let mut newest = newer.clone();
    newest.revision = 3;
    newest.updated_at = 50;
    store.compare_and_swap(2, &newest).unwrap();
    let err = store
        .restore_if_current_revision("asst_ada", 2, &first)
        .unwrap_err();
    assert!(err.contains("没有覆盖"));
    let kept = store.load("asst_ada").unwrap().unwrap();
    assert_eq!(kept.revision, 3);
    assert_eq!(kept.updated_at, 50);
}

#[test]
fn failed_create_rollback_does_not_cover_a_newer_primary() {
    let dir = TempDir::new();
    let store = ProfileStore::new(dir.0.clone());
    let mut old = profile("asst_old", true);
    old.revision = 1;
    let mut created_profile = profile("asst_new", true);
    created_profile.revision = 1;
    store.insert_new(&old).unwrap();
    let created = store.insert_new(&created_profile).unwrap();
    let mut moved = store.load("asst_old").unwrap().unwrap();
    assert_eq!(moved.revision, 2);
    assert!(!moved.primary);
    moved.revision = 3;
    moved.updated_at = 77;
    store.compare_and_swap(2, &moved).unwrap();
    let err = store.rollback_insert(&created).unwrap_err();
    assert!(err.contains("没有"));
    let kept = store.load("asst_old").unwrap().unwrap();
    assert_eq!(kept.revision, 3);
    assert_eq!(kept.updated_at, 77);
    assert!(store.load("asst_new").unwrap().is_none());
}

#[test]
fn profile_without_revision_field_loads_as_zero() {
    let dir = TempDir::new();
    let store = ProfileStore::new(dir.0.clone());
    let mut value = serde_json::to_value(profile("asst_old", false)).unwrap();
    value.as_object_mut().unwrap().remove("revision");
    fs::write(
        dir.0.join("asst_old.json"),
        serde_json::to_string(&value).unwrap(),
    )
    .unwrap();
    let loaded = store.load("asst_old").unwrap().unwrap();
    assert_eq!(loaded.revision, 0);
    assert!(!loaded.primary);
}

#[test]
fn private_memory_does_not_copy_or_clear_global_or_another_bot() {
    let dir = TempDir::new();
    let global = memory::memory_dir_at(&dir.0).unwrap();
    memory::save_layer_at(
        &global,
        MemoryLayer::L1,
        "# L1 Online Memory\n\nGLOBAL-SECRET",
    )
    .unwrap();
    let bot_a = memory::bot_memory_dir_at(&dir.0, "asst_a").unwrap();
    let bot_b = memory::bot_memory_dir_at(&dir.0, "asst_b").unwrap();
    memory::save_layer_at(&bot_a, MemoryLayer::L1, "# L1 Online Memory\n\nA-SECRET").unwrap();
    memory::save_layer_at(&bot_b, MemoryLayer::L1, "# L1 Online Memory\n\nB-SECRET").unwrap();
    let (prompt, warning) = private_memory_prompt(&bot_a, &[], true).unwrap();
    let prompt = prompt.unwrap();
    assert!(warning.is_none());
    assert!(prompt.contains("A-SECRET"));
    assert!(!prompt.contains("GLOBAL-SECRET"));
    assert!(!prompt.contains("B-SECRET"));
    assert!(prompt.contains("not the global Kivio memory"));
    memory::reset_layer_at(&bot_a, MemoryLayer::L1).unwrap();
    let global_text = fs::read_to_string(global.join("L1.md")).unwrap();
    let bot_b_text = fs::read_to_string(bot_b.join("L1.md")).unwrap();
    assert!(global_text.contains("GLOBAL-SECRET"));
    assert!(bot_b_text.contains("B-SECRET"));
    assert!(!fs::read_to_string(bot_a.join("L1.md"))
        .unwrap()
        .contains("A-SECRET"));
}

#[test]
fn shared_note_requires_an_explicit_excerpt() {
    let error = super::memory_scope::validate_shared_note("  ", 0).unwrap_err();
    assert!(error.contains("不会读取或复制全局记忆"));
}

#[test]
fn granted_defaults_follow_grants_and_do_not_keep_a_stale_cache() {
    let grants = GrantSet {
        mcp_server_ids: vec!["enabled".into()],
        skill_ids: vec!["pdf".into()],
    };
    let resolved = resolve_capability(
        &policy::granted_defaults_policy(),
        &["stale-mcp".into()],
        &["stale-skill".into()],
        &grants,
    );
    assert_eq!(resolved.mcp_server_ids, vec!["enabled".to_string()]);
    assert_eq!(resolved.skill_ids, vec!["pdf".to_string()]);
    assert!(resolved.rewrite_snapshot);
    assert!(resolved.explanation.contains("不是安装目录"));
    assert!(!resolved.mcp_server_ids.iter().any(|id| id == "stale-mcp"));
}

#[test]
fn explicit_empty_stays_empty_and_ungranted_ids_are_dropped() {
    let grants = GrantSet {
        mcp_server_ids: vec!["enabled".into()],
        skill_ids: vec!["pdf".into()],
    };
    let empty = resolve_capability(
        &policy::explicit_policy(Vec::new(), Vec::new()),
        &[],
        &[],
        &grants,
    );
    assert!(empty.mcp_server_ids.is_empty());
    assert!(empty.skill_ids.is_empty());
    assert!(empty.explanation.contains("明确不启用"));
    let partial = resolve_capability(
        &policy::explicit_policy(vec!["enabled".into(), "secret".into()], vec!["pdf".into()]),
        &[],
        &[],
        &grants,
    );
    assert_eq!(partial.mcp_server_ids, vec!["enabled".to_string()]);
    assert_eq!(partial.dropped_mcp_server_ids, vec!["secret".to_string()]);
    assert!(partial.explanation.contains("secret"));
}

#[test]
fn legacy_empty_whitelist_is_not_upgraded_to_grants() {
    let grants = GrantSet {
        mcp_server_ids: vec!["enabled".into()],
        skill_ids: vec!["pdf".into()],
    };
    let mut stored = policy::granted_defaults_policy();
    stored.source = CapabilitySource::LegacyWhitelist;
    stored.mcp_server_ids = Some(Vec::new());
    stored.skill_ids = Some(Vec::new());
    stored.explanation = super::types::LEGACY_EXPLANATION.into();
    let resolved = resolve_capability(&stored, &[], &[], &grants);
    assert!(resolved.mcp_server_ids.is_empty());
    assert!(resolved.skill_ids.is_empty());
    assert!(!resolved.rewrite_snapshot);
    assert!(resolved.explanation.contains("没有改写成新的默认授予"));
}

#[test]
fn edit_keeps_omitted_fields_and_rejects_half_a_model() {
    let current_assistant = assistant("asst_one", "Ada");
    let current = profile("asst_one", false);
    let merged = policy::merge_edit(
        &current_assistant,
        &current,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        11,
    )
    .unwrap();
    assert_eq!(merged.assistant.system_prompt, "keep-prompt");
    assert_eq!(merged.assistant.name, "Ada");
    assert_eq!(merged.assistant.created_at, 10);
    assert!(policy::model_from_write(Some(&BotModelWrite {
        source: ModelSource::Explicit,
        provider_id: Some("provider".into()),
        model: Some(String::new()),
    }))
    .is_err());
    assert!(policy::capability_from_write(Some(&BotCapabilityWrite {
        source: CapabilitySource::LegacyWhitelist,
        mcp_server_ids: None,
        skill_ids: None,
    }))
    .is_err());
    let inherited = policy::merge_edit(
        &current_assistant,
        &current,
        None,
        None,
        None,
        None,
        None,
        Some(true),
        Some(&BotCapabilityWrite {
            source: CapabilitySource::Explicit,
            mcp_server_ids: Some(Vec::new()),
            skill_ids: Some(Vec::new()),
        }),
        Some(&BotApprovalWrite {
            source: ApprovalSource::Override,
            policy: Some("always_confirm".into()),
        }),
        Some(&BotModelWrite {
            source: ModelSource::InheritGlobal,
            provider_id: Some("ignored".into()),
            model: Some("ignored".into()),
        }),
        12,
    )
    .unwrap();
    assert!(inherited.assistant.provider_id.is_empty());
    assert!(inherited.assistant.model.is_empty());
    assert!(inherited.profile.primary);
    assert_eq!(
        inherited.profile.capability.source,
        CapabilitySource::Explicit
    );
    assert_eq!(
        inherited.profile.approval.policy.as_deref(),
        Some("always_confirm")
    );
    assert_eq!(inherited.assistant.system_prompt, "keep-prompt");
}

#[test]
fn persona_edit_keeps_history() {
    let raw = r#"{
        "id": "conv_bot_asst_one",
        "title": "Old",
        "provider_id": "old-provider",
        "model": "old-model",
        "messages": [{"id":"msg_1","role":"user","content":"keep-me","timestamp":1}],
        "created_at": 1,
        "updated_at": 1,
        "assistant_snapshot": {"id":"asst_one","name":"Old","provider_id":"","model":""}
    }"#;
    let mut conversation: Conversation = serde_json::from_str(raw).unwrap();
    let mut next = assistant("asst_one", "Renamed");
    next.provider_id = "new-provider".into();
    next.model = "new-model".into();
    super::lifecycle::apply_persona(&mut conversation, &next, "global-provider", "global-model");
    assert_eq!(conversation.messages[0].content, "keep-me");
    assert_eq!(conversation.messages.len(), 1);
    assert_eq!(conversation.title, "Renamed");
    assert_eq!(conversation.provider_id, "new-provider");
    assert_eq!(conversation.model, "new-model");
    assert_eq!(conversation.id, "conv_bot_asst_one");
}

#[test]
fn removing_one_assistant_leaves_the_other() {
    let mut index = ChatAssistantIndex {
        assistants: vec![assistant("asst_a", "A"), assistant("asst_b", "B")],
    };
    let removed = remove_assistant_from_index(&mut index, "asst_a").unwrap();
    assert_eq!(removed.name, "A");
    assert_eq!(index.assistants.len(), 1);
    assert_eq!(index.assistants[0].id, "asst_b");
    assert!(remove_assistant_from_index(&mut index, "asst_missing").is_err());
    assert_eq!(index.assistants.len(), 1);
}

#[test]
fn member_thread_resolves_one_bot_and_does_not_fall_through() {
    let id = super::member_conversation_id("asst_ada", "room_1").unwrap();
    assert_eq!(id, "conv_bot_member_asst_ada@room_1");
    assert!(super::assistant_id_from_conversation(&id).is_none());
    assert_eq!(
        super::assistant_id_from_member_conversation(&id),
        Some("asst_ada")
    );
    assert_eq!(
        super::context::encoded_member_bot(&id).unwrap().as_deref(),
        Some("asst_ada")
    );
    assert!(super::context::encoded_member_bot("conv_bot_member_nope").is_err());
    assert!(super::context::encoded_member_bot("conv_ordinary")
        .unwrap()
        .is_none());
    assert_eq!(
        super::context::assistant_id_from_member_rows(
            &id,
            [
                ("conv_bot_asst_other", "asst_other"),
                (id.as_str(), "asst_ada")
            ],
        )
        .unwrap()
        .as_deref(),
        Some("asst_ada")
    );
    assert!(super::context::assistant_id_from_member_rows(
        &id,
        [(id.as_str(), "not-an-assistant")]
    )
    .is_err());
    assert!(super::context::assistant_id_from_member_rows(
        &id,
        [(id.as_str(), "asst_ada"), (id.as_str(), "asst_bee")]
    )
    .is_err());
    assert!(super::context::assistant_id_from_member_rows(
        "conv_bot_asst_ada",
        [("conv_bot_asst_ada", "asst_bee")]
    )
    .unwrap()
    .is_none());
}

#[test]
fn request_dtos_deserialize_camel_case() {
    let request: BotCreateRequest = serde_json::from_str(
        r#"{"name":"Ada","primary":true,"capability":{"source":"explicit","skillIds":[]}}"#,
    )
    .unwrap();
    assert_eq!(request.primary, Some(true));
    assert_eq!(request.description, None);
    let edit: BotEditRequest =
        serde_json::from_str(r#"{"assistantId":"asst_ada","expectedRevision":3}"#).unwrap();
    assert_eq!(edit.expected_revision, Some(3));
    let omitted: BotEditRequest = serde_json::from_str(r#"{"assistantId":"asst_ada"}"#).unwrap();
    assert_eq!(omitted.expected_revision, None);
    let capability = request.capability.unwrap();
    assert_eq!(capability.source, CapabilitySource::Explicit);
    assert!(capability.mcp_server_ids.is_none());
    assert!(capability.skill_ids.as_ref().unwrap().is_empty());
    let _schema = schemars::schema_for!(BotCreateRequest);
}

#[test]
fn unknown_approval_policy_is_rejected() {
    let error = policy::approval_from_write(Some(&BotApprovalWrite {
        source: ApprovalSource::Override,
        policy: Some("allow_everything".into()),
    }))
    .unwrap_err();
    assert!(error.contains("不认识的审批策略"));
}
