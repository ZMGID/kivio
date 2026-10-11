//! One create/edit path for the UI commands and the model tools.
//! A failed create removes the assistant and profile it just wrote.
//! It does not delete an existing conversation or any global memory.

use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use crate::chat::storage::{
    create_assistant, get_assistant, remove_unopened_assistant, update_assistant,
};
use crate::chat::{ChatAssistant, Conversation};
use crate::settings::Settings;
use crate::state::AppState;

use super::memory_scope::{self, global_injection_label};
use super::policy::{
    self, explicit_policy, legacy_profile, merge_edit, prepare_create, resolve_capability,
    resolve_model, GrantSet,
};
use super::store::{ClearedPrimary, InsertNewError, InsertRestore, ProfileStore};
use super::types::{
    assistant_id_from_conversation, bot_conversation_id, validate_assistant_id, BotCreateRequest,
    BotEditRequest, BotListItem, BotMemoryView, BotModelWrite, BotProfile, BotProfileDetail,
    BotRuntimeScope, CapabilitySource, ModelSource, ResolvedCapability, ResolvedModelSource,
    SharedMemoryNote, CHANGED_EVENT, LEGACY_EXPLANATION,
};

fn lifecycle_mutex() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Held across create, update, primary, shared-note, and assistant-catalog edits.
/// Acquire this before the assistant catalog lock. Do not acquire it again
/// while it is already held: `tokio::sync::Mutex` is not reentrant.
pub struct LifecycleGuard {
    _guard: tokio::sync::MutexGuard<'static, ()>,
}

impl LifecycleGuard {
    pub async fn acquire() -> Self {
        Self {
            _guard: lifecycle_mutex().lock().await,
        }
    }
}

/// Next revision for one Bot. `expected` is this Bot's revision, not a global counter.
/// `None` accepts the revision just loaded inside the lifecycle lock.
pub fn plan_revision(current: u64, expected: Option<u64>) -> Result<u64, String> {
    if let Some(expected) = expected {
        if expected != current {
            return Err(format!(
                "Bot 配置已被其他保存更新（当前版本 {current}，提交基于 {expected}），没有覆盖"
            ));
        }
    }
    current
        .checked_add(1)
        .ok_or_else(|| "Bot 配置版本号已满，没有覆盖".to_string())
}

pub fn granted_set(app: &AppHandle, settings: &Settings) -> Result<GrantSet, String> {
    let mcp_server_ids = settings
        .chat_tools
        .servers
        .iter()
        .filter(|server| server.enabled && !server.id.trim().is_empty())
        .map(|server| server.id.clone())
        .collect();
    let registry = crate::skills::build_registry(app, &settings.chat_tools.skill_scan_paths)?;
    let obsidian = !settings.obsidian_vault_path.trim().is_empty();
    let skill_ids = registry
        .records
        .iter()
        .filter(|record| {
            crate::settings::skill_globally_available(
                &settings.chat_tools,
                &record.meta.id,
                obsidian,
            )
        })
        .map(|record| record.meta.id.clone())
        .collect();
    Ok(GrantSet {
        mcp_server_ids,
        skill_ids,
    })
}

pub async fn create_bot(
    app: &AppHandle,
    state: &AppState,
    request: BotCreateRequest,
) -> Result<BotProfileDetail, String> {
    let _guard = LifecycleGuard::acquire().await;
    let settings = state.settings_read().clone();
    let grants = granted_set(app, &settings)?;
    let now = chrono::Local::now().timestamp();
    let prepared = prepare_create(
        format!("asst_{}", Uuid::new_v4()),
        request.name,
        request.description.unwrap_or_default(),
        request.system_prompt.unwrap_or_default(),
        request.icon.unwrap_or_default(),
        request.color.unwrap_or_default(),
        request.primary.unwrap_or(false),
        request.capability.as_ref(),
        request.approval.as_ref(),
        request.model.as_ref(),
        now,
    )?;
    let assistant = create_assistant(app, prepared.assistant)?;
    let mut profile = prepared.profile;
    profile.assistant_id = assistant.id.clone();
    profile.revision = 1;
    if profile.capability.source == CapabilitySource::Explicit {
        profile.capability.mcp_server_ids = Some(assistant.mcp_server_ids.clone());
        profile.capability.skill_ids = Some(assistant.skill_ids.clone());
    }
    let store = match ProfileStore::for_app(app) {
        Ok(store) => store,
        Err(err) => {
            return Err(compensate_failed_create(app, &assistant.id, None, None, err).await);
        }
    };
    let restore = match store.insert_new(&profile) {
        Ok(restore) => restore,
        Err(InsertNewError { message, restore }) => {
            return Err(compensate_failed_create(
                app,
                &assistant.id,
                Some(&store),
                restore.as_ref(),
                message,
            )
            .await);
        }
    };
    if let Err(err) = open_fixed_chat(app, state, &assistant.id).await {
        return Err(compensate_failed_create(
            app,
            &assistant.id,
            Some(&store),
            Some(&restore),
            err,
        )
        .await);
    }
    let (stored_provider, stored_model) = stored_chat_model(app, &assistant);
    let detail = match detail_for(
        &assistant,
        &profile,
        true,
        &grants,
        &settings,
        &stored_provider,
        &stored_model,
    ) {
        Ok(detail) => detail,
        Err(err) => {
            return Err(compensate_failed_create(
                app,
                &assistant.id,
                Some(&store),
                Some(&restore),
                err,
            )
            .await);
        }
    };
    publish(app, &assistant.id);
    Ok(detail)
}

pub async fn update_bot(
    app: &AppHandle,
    state: &AppState,
    request: BotEditRequest,
) -> Result<BotProfileDetail, String> {
    let _guard = LifecycleGuard::acquire().await;
    let assistant_id = validate_assistant_id(&request.assistant_id)?.to_string();
    require_fixed_chat(app, &assistant_id)?;
    let settings = state.settings_read().clone();
    let existing = get_assistant(app, &assistant_id)?;
    let store = ProfileStore::for_app(app)?;
    let loaded = store.load(&assistant_id)?;
    let persisted = loaded.is_some();
    let current = loaded.unwrap_or_else(|| legacy_profile(&existing, existing.updated_at));
    let next_revision = plan_revision(current.revision, request.expected_revision)?;
    let now = chrono::Local::now().timestamp();
    let prepared = merge_edit(
        &existing,
        &current,
        request.name.as_deref(),
        request.description.as_deref(),
        request.system_prompt.as_deref(),
        request.icon.as_deref(),
        request.color.as_deref(),
        request.primary,
        request.capability.as_ref(),
        request.approval.as_ref(),
        request.model.as_ref(),
        now,
    )?;
    let grants = if prepared.profile.capability.source == CapabilitySource::LegacyWhitelist {
        GrantSet::default()
    } else {
        granted_set(app, &settings)?
    };
    let chat = claim_fixed_chat(app, &assistant_id)?;
    let saved = update_assistant(app, prepared.assistant)?;
    let mut profile = prepared.profile;
    profile.assistant_id = saved.id.clone();
    profile.revision = next_revision;
    if profile.capability.source == CapabilitySource::Explicit {
        profile.capability.mcp_server_ids = Some(saved.mcp_server_ids.clone());
        profile.capability.skill_ids = Some(saved.skill_ids.clone());
    }
    let cleared = match store.compare_and_swap(current.revision, &profile) {
        Ok(cleared) => cleared,
        Err(err) => {
            let restore = restore_assistant_if_own(app, &saved, &existing);
            return Err(match restore {
                Ok(()) => err,
                Err(restore_err) => format!("{err}；助手回滚：{restore_err}"),
            });
        }
    };
    if let Err(err) = apply_fixed_persona_cas(app, state, &saved, &chat).await {
        let assistant_restore = restore_assistant_if_own(app, &saved, &existing);
        let profile_restore = undo_profile_write(
            &store,
            &assistant_id,
            persisted,
            next_revision,
            &current,
            &cleared,
        );
        return Err(format!(
            "{err}；助手回滚：{}；配置回滚：{}；固定私聊仍是版本 {}",
            assistant_restore
                .as_ref()
                .map(|_| "已恢复")
                .unwrap_or_else(|restore| restore.as_str()),
            profile_restore
                .as_ref()
                .map(|_| "已恢复")
                .unwrap_or_else(|restore| restore.as_str()),
            chat.revision
        ));
    }
    publish(app, &saved.id);
    let (stored_provider, stored_model) = stored_chat_model(app, &saved);
    detail_for(
        &saved,
        &profile,
        true,
        &grants,
        &settings,
        &stored_provider,
        &stored_model,
    )
}

pub async fn set_primary_bot(
    app: &AppHandle,
    assistant_id: &str,
    primary: bool,
) -> Result<BotProfile, String> {
    let _guard = LifecycleGuard::acquire().await;
    let assistant_id = validate_assistant_id(assistant_id)?.to_string();
    require_fixed_chat(app, &assistant_id)?;
    let assistant = get_assistant(app, &assistant_id)?;
    let store = ProfileStore::for_app(app)?;
    let now = chrono::Local::now().timestamp();
    let (expected, mut profile) = match store.load(&assistant_id)? {
        Some(profile) => (profile.revision, profile),
        None => (0, legacy_profile(&assistant, now)),
    };
    profile.primary = primary;
    profile.updated_at = now;
    profile.revision = plan_revision(expected, Some(expected))?;
    store.compare_and_swap(expected, &profile)?;
    publish(app, &assistant_id);
    Ok(profile)
}

pub fn profile_detail(
    app: &AppHandle,
    settings: &Settings,
    assistant_id: &str,
) -> Result<BotProfileDetail, String> {
    let assistant_id = validate_assistant_id(assistant_id)?;
    let assistant = get_assistant(app, assistant_id)?;
    let store = ProfileStore::for_app(app)?;
    let loaded = store.load(assistant_id)?;
    let persisted = loaded.is_some();
    let profile = loaded.unwrap_or_else(|| legacy_profile(&assistant, assistant.updated_at));
    let grants = match profile.capability.source {
        CapabilitySource::LegacyWhitelist => GrantSet::default(),
        _ => granted_set(app, settings)?,
    };
    let (stored_provider, stored_model) = stored_chat_model(app, &assistant);
    detail_for(
        &assistant,
        &profile,
        persisted,
        &grants,
        settings,
        &stored_provider,
        &stored_model,
    )
}

pub async fn list_bots(app: &AppHandle) -> Result<Vec<BotListItem>, String> {
    let conversations = crate::chat::repository::repository(app)
        .list_bots(app)
        .await
        .map_err(crate::chat::repository::repository_error)?;
    let store = ProfileStore::for_app(app)?;
    let mut items = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for conversation in conversations {
        let Some(assistant_id) = assistant_id_from_conversation(&conversation.id) else {
            continue;
        };
        seen.insert(assistant_id.to_string());
        items.push(list_item(app, &store, assistant_id, &conversation.title));
    }
    if let Ok(profiles) = store.list() {
        for profile in profiles {
            if seen.insert(profile.assistant_id.clone()) {
                let mut item = list_item(app, &store, &profile.assistant_id, &profile.assistant_id);
                if item.read_error.is_none() {
                    item.read_error = Some("有 Bot 配置，但固定私聊不存在".into());
                }
                items.push(item);
            }
        }
    }
    Ok(items)
}

pub fn runtime_scope(
    app: &AppHandle,
    settings: &Settings,
    conversation: &Conversation,
) -> Result<Option<BotRuntimeScope>, String> {
    let assistant_id = match super::context::resolve_stable_assistant_id(app, &conversation.id)? {
        Some(id) => id,
        None => return Ok(None),
    };
    if let Some(recorded) = conversation
        .assistant_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        if recorded != assistant_id {
            return Err(format!(
                "会话「{}」记录的助手是 {recorded}，稳定 Bot 是 {assistant_id}，没有改用全局记忆",
                conversation.id
            ));
        }
    }
    let assistant = get_assistant(app, &assistant_id)?;
    let store = ProfileStore::for_app(app)?;
    let loaded = store.load(&assistant_id)?;
    let persisted = loaded.is_some();
    let (global_provider, global_model) = settings.effective_chat_model();
    let (provider_id, model, model_source, model_explanation, rewrite_model) = resolve_model(
        loaded.as_ref(),
        &assistant,
        &conversation.provider_id,
        &conversation.model,
        &global_provider,
        &global_model,
    )?;
    let profile = loaded
        .clone()
        .unwrap_or_else(|| legacy_profile(&assistant, assistant.updated_at));
    let grants = if persisted && profile.capability.source != CapabilitySource::LegacyWhitelist {
        granted_set(app, settings)?
    } else {
        GrantSet::default()
    };
    let mut capability = resolve_capability(
        &profile.capability,
        conversation
            .assistant_snapshot
            .as_ref()
            .map(|snapshot| snapshot.mcp_server_ids.as_slice())
            .unwrap_or(assistant.mcp_server_ids.as_slice()),
        conversation
            .assistant_snapshot
            .as_ref()
            .map(|snapshot| snapshot.skill_ids.as_slice())
            .unwrap_or(assistant.skill_ids.as_slice()),
        &grants,
    );
    if !persisted {
        capability = ResolvedCapability {
            source: CapabilitySource::LegacyWhitelist,
            explanation: LEGACY_EXPLANATION.to_string(),
            mcp_server_ids: conversation
                .assistant_snapshot
                .as_ref()
                .map(|snapshot| snapshot.mcp_server_ids.clone())
                .unwrap_or_else(|| assistant.mcp_server_ids.clone()),
            skill_ids: conversation
                .assistant_snapshot
                .as_ref()
                .map(|snapshot| snapshot.skill_ids.clone())
                .unwrap_or_else(|| assistant.skill_ids.clone()),
            dropped_mcp_server_ids: Vec::new(),
            dropped_skill_ids: Vec::new(),
            rewrite_snapshot: false,
        };
    }
    let (approval_override, approval_source, approval_explanation) = match loaded.as_ref() {
        Some(profile) if profile.approval.policy.is_some() => (
            profile.approval.policy.clone(),
            profile.approval.source,
            profile.approval.explanation.clone(),
        ),
        Some(profile) => (
            None,
            profile.approval.source,
            profile.approval.explanation.clone(),
        ),
        None => (
            None,
            super::types::ApprovalSource::InheritGlobal,
            super::types::INHERIT_APPROVAL_EXPLANATION.to_string(),
        ),
    };
    let notes = loaded
        .as_ref()
        .map(|profile| profile.shared_memory.clone())
        .unwrap_or_default();
    let memory_dir = crate::chat::memory::bot_memory_dir(app, &assistant_id)?;
    let (memory_prompt, memory_warning) =
        memory_scope::private_memory_prompt(&memory_dir, &notes, settings.chat_memory.enabled)?;
    Ok(Some(BotRuntimeScope {
        assistant_id: assistant_id.clone(),
        conversation_id: conversation.id.clone(),
        primary: loaded.as_ref().is_some_and(|profile| profile.primary),
        persisted_profile: persisted,
        provider_id,
        model,
        model_source,
        model_explanation,
        rewrite_model,
        capability,
        approval_override,
        approval_source,
        approval_explanation,
        memory_prompt,
        memory_warning,
    }))
}

/// In-memory only. Does not write the conversation or global settings.
/// A Bot that must rewrite capability but has no snapshot fails closed.
pub fn apply_runtime_scope(
    conversation: &mut Conversation,
    scope: &BotRuntimeScope,
) -> Result<(), String> {
    if scope.capability.rewrite_snapshot && conversation.assistant_snapshot.is_none() {
        return Err("Bot 缺少助手快照，没有按不受限工具继续".into());
    }
    if scope.rewrite_model {
        conversation.provider_id.clone_from(&scope.provider_id);
        conversation.model.clone_from(&scope.model);
        if let Some(snapshot) = conversation.assistant_snapshot.as_mut() {
            snapshot.provider_id.clone_from(&scope.provider_id);
            snapshot.model.clone_from(&scope.model);
        }
    }
    if scope.capability.rewrite_snapshot {
        if let Some(snapshot) = conversation.assistant_snapshot.as_mut() {
            snapshot
                .mcp_server_ids
                .clone_from(&scope.capability.mcp_server_ids);
            snapshot.skill_ids.clone_from(&scope.capability.skill_ids);
        }
    }
    Ok(())
}

pub fn hide_global_memory_tools(tools: &mut Vec<crate::mcp::ChatToolDefinition>) {
    tools.retain(|tool| !is_global_memory_tool(&tool.name));
}

pub fn is_global_memory_tool(name: &str) -> bool {
    matches!(name, "memory_read" | "memory_search" | "memory_modify")
}

pub fn apply_persona(
    conversation: &mut Conversation,
    assistant: &ChatAssistant,
    default_provider: &str,
    default_model: &str,
) {
    let snapshot = crate::chat::ChatAssistantSnapshot::from(assistant);
    let model_changed = conversation
        .assistant_snapshot
        .as_ref()
        .map(|old| old.provider_id != snapshot.provider_id || old.model != snapshot.model)
        .unwrap_or(true);
    if model_changed {
        conversation.provider_id =
            non_empty(&snapshot.provider_id).unwrap_or_else(|| default_provider.to_string());
        conversation.model =
            non_empty(&snapshot.model).unwrap_or_else(|| default_model.to_string());
    }
    conversation.title.clone_from(&assistant.name);
    conversation.assistant_snapshot = Some(snapshot);
}

/// Catalog assistant edit. The lifecycle lock is acquired here, before
/// `update_assistant`, and held through profile CAS and the fixed-chat persona
/// sync. Callers must not already hold [`LifecycleGuard`].
pub async fn commit_assistant_edit(
    app: &AppHandle,
    state: &AppState,
    assistant: ChatAssistant,
) -> Result<ChatAssistant, String> {
    let _guard = LifecycleGuard::acquire().await;
    let existing = get_assistant(app, &assistant.id)?;
    let chat = claim_fixed_chat_if_present(app, &assistant.id)?;
    let saved = update_assistant(app, assistant)?;
    if let Err(err) = sync_profile_and_persona(app, state, &existing, &saved, chat).await {
        let restore = restore_assistant_if_own(app, &saved, &existing);
        return Err(match restore {
            Ok(()) => err,
            Err(restore_err) => format!("{err}；助手回滚：{restore_err}"),
        });
    }
    Ok(saved)
}

async fn sync_profile_and_persona(
    app: &AppHandle,
    state: &AppState,
    before: &ChatAssistant,
    after: &ChatAssistant,
    chat: Option<FixedChatClaim>,
) -> Result<(), String> {
    let written = sync_profile_locked(app, before, after)?;
    let Some(chat) = chat else {
        return Ok(());
    };
    if let Err(err) = apply_fixed_persona_cas(app, state, after, &chat).await {
        if let Some(written) = written.as_ref() {
            let undo = undo_profile_write(
                &written.store,
                &after.id,
                true,
                written.written_revision,
                &written.previous,
                &written.cleared,
            );
            return Err(match undo {
                Ok(()) => format!("{err}；固定私聊仍是版本 {}", chat.revision),
                Err(undo_err) => format!(
                    "{err}；配置回滚：{undo_err}；固定私聊仍是版本 {}",
                    chat.revision
                ),
            });
        }
        return Err(format!("{err}；固定私聊仍是版本 {}", chat.revision));
    }
    Ok(())
}

struct ProfileCommit {
    store: ProfileStore,
    previous: BotProfile,
    written_revision: u64,
    cleared: Vec<ClearedPrimary>,
}

fn sync_profile_locked(
    app: &AppHandle,
    before: &ChatAssistant,
    after: &ChatAssistant,
) -> Result<Option<ProfileCommit>, String> {
    let store = ProfileStore::for_app(app)?;
    let Some(previous) = store.load(&after.id)? else {
        return Ok(None);
    };
    let mut profile = previous.clone();
    let mut changed = before.name != after.name
        || before.description != after.description
        || before.system_prompt != after.system_prompt
        || before.icon != after.icon
        || before.color != after.color
        || before.installed != after.installed
        || before.enabled != after.enabled
        || before.archived != after.archived;
    if before.mcp_server_ids != after.mcp_server_ids || before.skill_ids != after.skill_ids {
        profile.capability = explicit_policy(after.mcp_server_ids.clone(), after.skill_ids.clone());
        changed = true;
    }
    if before.provider_id != after.provider_id || before.model != after.model {
        let source = if after.provider_id.trim().is_empty() && after.model.trim().is_empty() {
            ModelSource::InheritGlobal
        } else {
            ModelSource::Explicit
        };
        let (policy, _, _) = policy::model_from_write(Some(&BotModelWrite {
            source,
            provider_id: Some(after.provider_id.clone()),
            model: Some(after.model.clone()),
        }))?;
        profile.model = policy;
        changed = true;
    }
    if !changed {
        return Ok(None);
    }
    let expected = previous.revision;
    profile.revision = plan_revision(expected, Some(expected))?;
    profile.updated_at = chrono::Local::now().timestamp();
    let cleared = store.compare_and_swap(expected, &profile)?;
    Ok(Some(ProfileCommit {
        store,
        previous,
        written_revision: profile.revision,
        cleared,
    }))
}

struct FixedChatClaim {
    id: String,
    revision: u64,
}

/// Revision captured before assistant and profile writes. The later persona
/// write uses `mutate_expected` with this revision, so a message that landed
/// in between is kept and this edit is rolled back instead.
fn claim_fixed_chat(app: &AppHandle, assistant_id: &str) -> Result<FixedChatClaim, String> {
    let id = bot_conversation_id(assistant_id);
    let conversation = crate::chat::storage::load_conversation(app, &id)?;
    if conversation.id != id {
        return Err(format!("固定私聊「{id}」标识不一致，没有改写"));
    }
    if let Some(owner) = conversation.assistant_id.as_deref() {
        if owner != assistant_id {
            return Err(format!(
                "固定私聊「{id}」属于助手 {owner}，没有改写成 {assistant_id}"
            ));
        }
    }
    Ok(FixedChatClaim {
        id,
        revision: conversation.revision,
    })
}

fn claim_fixed_chat_if_present(
    app: &AppHandle,
    assistant_id: &str,
) -> Result<Option<FixedChatClaim>, String> {
    let id = bot_conversation_id(assistant_id);
    if crate::chat::storage::conversation_file_path(app, &id)?.exists() {
        claim_fixed_chat(app, assistant_id).map(Some)
    } else {
        Ok(None)
    }
}

async fn apply_fixed_persona_cas(
    app: &AppHandle,
    state: &AppState,
    assistant: &ChatAssistant,
    claim: &FixedChatClaim,
) -> Result<u64, String> {
    let (default_provider, default_model) = state.settings_read().effective_chat_model();
    let assistant = assistant.clone();
    let id = claim.id.clone();
    let expected = claim.revision;
    let saved = crate::chat::repository::repository(app)
        .mutate_expected(app, &id, Some(expected), move |conversation| {
            apply_persona(conversation, &assistant, &default_provider, &default_model);
            Ok(())
        })
        .await
        .map_err(crate::chat::repository::repository_error)?;
    Ok(saved.revision)
}

pub fn memory_view(app: &AppHandle, assistant_id: &str) -> Result<BotMemoryView, String> {
    let assistant_id = validate_assistant_id(assistant_id)?.to_string();
    let dir = crate::chat::memory::bot_memory_dir(app, &assistant_id)?;
    let (l1, l2) = memory_scope::read_private_layers(&dir)?;
    let shared_memory = ProfileStore::for_app(app)?
        .load(&assistant_id)?
        .map(|profile| profile.shared_memory)
        .unwrap_or_default();
    Ok(BotMemoryView {
        assistant_id,
        private_directory: dir.display().to_string(),
        global_injection: global_injection_label(),
        l1,
        l2,
        shared_memory,
    })
}

pub fn save_bot_memory(
    app: &AppHandle,
    assistant_id: &str,
    layer: &str,
    content: &str,
) -> Result<BotMemoryView, String> {
    let assistant_id = validate_assistant_id(assistant_id)?.to_string();
    let _ = get_assistant(app, &assistant_id)?;
    let dir = crate::chat::memory::bot_memory_dir(app, &assistant_id)?;
    memory_scope::write_private_layer(&dir, layer, content)?;
    publish(app, &assistant_id);
    memory_view(app, &assistant_id)
}

pub fn clear_bot_memory(
    app: &AppHandle,
    assistant_id: &str,
    layer: &str,
) -> Result<BotMemoryView, String> {
    let assistant_id = validate_assistant_id(assistant_id)?.to_string();
    let dir = crate::chat::memory::bot_memory_dir(app, &assistant_id)?;
    memory_scope::clear_private_layer(&dir, layer)?;
    publish(app, &assistant_id);
    memory_view(app, &assistant_id)
}

pub async fn add_shared_memory(
    app: &AppHandle,
    assistant_id: &str,
    content: &str,
) -> Result<BotProfile, String> {
    let _guard = LifecycleGuard::acquire().await;
    let assistant_id = validate_assistant_id(assistant_id)?.to_string();
    let assistant = get_assistant(app, &assistant_id)?;
    let store = ProfileStore::for_app(app)?;
    let now = chrono::Local::now().timestamp();
    let (expected, mut profile) = match store.load(&assistant_id)? {
        Some(profile) => (profile.revision, profile),
        None => (0, legacy_profile(&assistant, now)),
    };
    let note_content = memory_scope::validate_shared_note(content, profile.shared_memory.len())?;
    profile.shared_memory.push(SharedMemoryNote {
        id: format!("note_{}", Uuid::new_v4()),
        content: note_content,
        created_at: now,
    });
    profile.updated_at = now;
    profile.revision = plan_revision(expected, Some(expected))?;
    store.compare_and_swap(expected, &profile)?;
    publish(app, &assistant_id);
    Ok(profile)
}

pub async fn delete_shared_memory(
    app: &AppHandle,
    assistant_id: &str,
    note_id: &str,
) -> Result<BotProfile, String> {
    let _guard = LifecycleGuard::acquire().await;
    let assistant_id = validate_assistant_id(assistant_id)?.to_string();
    let store = ProfileStore::for_app(app)?;
    let now = chrono::Local::now().timestamp();
    let Some(mut profile) = store.load(&assistant_id)? else {
        return Err("Bot 配置不存在".into());
    };
    let before = profile.shared_memory.len();
    profile.shared_memory.retain(|note| note.id != note_id);
    if profile.shared_memory.len() == before {
        return Err("没有这条共享摘录".into());
    }
    let expected = profile.revision;
    profile.updated_at = now;
    profile.revision = plan_revision(expected, Some(expected))?;
    store.compare_and_swap(expected, &profile)?;
    publish(app, &assistant_id);
    Ok(profile)
}

fn detail_for(
    assistant: &ChatAssistant,
    profile: &BotProfile,
    persisted: bool,
    grants: &GrantSet,
    settings: &Settings,
    stored_provider: &str,
    stored_model: &str,
) -> Result<BotProfileDetail, String> {
    let (global_provider, global_model) = settings.effective_chat_model();
    let (resolved_provider_id, resolved_model, model_source, model_explanation, _) = resolve_model(
        persisted.then_some(profile),
        assistant,
        stored_provider,
        stored_model,
        &global_provider,
        &global_model,
    )?;
    let model_source = if persisted {
        model_source
    } else if assistant.provider_id.trim().is_empty() && assistant.model.trim().is_empty() {
        ResolvedModelSource::ConversationFrozen
    } else {
        model_source
    };
    let resolved = resolve_capability(
        &profile.capability,
        &assistant.mcp_server_ids,
        &assistant.skill_ids,
        grants,
    );
    Ok(BotProfileDetail {
        assistant_id: assistant.id.clone(),
        name: assistant.name.clone(),
        description: assistant.description.clone(),
        icon: assistant.icon.clone(),
        color: assistant.color.clone(),
        system_prompt: assistant.system_prompt.clone(),
        provider_id: assistant.provider_id.clone(),
        model: assistant.model.clone(),
        conversation_id: bot_conversation_id(&assistant.id),
        persisted_profile: persisted,
        profile: profile.clone(),
        resolved,
        resolved_provider_id,
        resolved_model,
        model_source,
        model_explanation,
    })
}

fn list_item(
    app: &AppHandle,
    store: &ProfileStore,
    assistant_id: &str,
    title: &str,
) -> BotListItem {
    let assistant = get_assistant(app, assistant_id).ok();
    let loaded = store.load(assistant_id);
    match loaded {
        Ok(Some(profile)) => BotListItem {
            assistant_id: assistant_id.to_string(),
            name: assistant
                .as_ref()
                .map(|item| item.name.clone())
                .unwrap_or_else(|| title.to_string()),
            conversation_id: bot_conversation_id(assistant_id),
            primary: profile.primary,
            persisted_profile: true,
            capability_source: Some(profile.capability.source),
            capability_explanation: profile.capability.explanation,
            model_source: Some(match profile.model.source {
                ModelSource::InheritGlobal => ResolvedModelSource::InheritGlobal,
                ModelSource::Explicit => ResolvedModelSource::Explicit,
            }),
            approval_source: Some(profile.approval.source),
            read_error: assistant.is_none().then_some("助手不存在".into()),
        },
        Ok(None) => BotListItem {
            assistant_id: assistant_id.to_string(),
            name: assistant
                .as_ref()
                .map(|item| item.name.clone())
                .unwrap_or_else(|| title.to_string()),
            conversation_id: bot_conversation_id(assistant_id),
            primary: false,
            persisted_profile: false,
            capability_source: Some(CapabilitySource::LegacyWhitelist),
            capability_explanation: LEGACY_EXPLANATION.to_string(),
            model_source: Some(ResolvedModelSource::ConversationFrozen),
            approval_source: Some(super::types::ApprovalSource::InheritGlobal),
            read_error: assistant.is_none().then_some("助手不存在".into()),
        },
        Err(err) => BotListItem {
            assistant_id: assistant_id.to_string(),
            name: title.to_string(),
            conversation_id: bot_conversation_id(assistant_id),
            primary: false,
            persisted_profile: true,
            capability_source: None,
            capability_explanation: String::new(),
            model_source: None,
            approval_source: None,
            read_error: Some(err),
        },
    }
}

async fn open_fixed_chat(
    app: &AppHandle,
    state: &AppState,
    assistant_id: &str,
) -> Result<Conversation, String> {
    crate::chat::commands::catalog::create_chat_conversation_internal(
        app,
        state,
        None,
        None,
        None,
        None,
        None,
        Some(assistant_id.to_string()),
        crate::chat::commands::catalog::ConversationCreation::Bot,
    )
    .await
}

fn require_fixed_chat(app: &AppHandle, assistant_id: &str) -> Result<(), String> {
    let bot_id = bot_conversation_id(assistant_id);
    if crate::chat::storage::conversation_file_path(app, &bot_id)?.exists() {
        Ok(())
    } else {
        Err("Bot 的固定私聊不存在，没有把这次操作报成成功".into())
    }
}

fn undo_profile_write(
    store: &ProfileStore,
    assistant_id: &str,
    persisted: bool,
    written_revision: u64,
    previous: &BotProfile,
    cleared: &[ClearedPrimary],
) -> Result<(), String> {
    let own = if persisted {
        if previous.assistant_id != assistant_id {
            return Err("回滚配置时助手 id 不一致，没有覆盖其他 Bot".into());
        }
        store.restore_if_current_revision(assistant_id, written_revision, previous)
    } else if previous.assistant_id != assistant_id {
        Err("回滚配置时助手 id 不一致，没有删除其他 Bot".into())
    } else {
        store.delete_if_current_revision(assistant_id, written_revision)
    };
    let primaries = store.restore_cleared(cleared);
    match (own, primaries) {
        (Ok(()), Ok(())) => Ok(()),
        (own, primaries) => Err(format!(
            "{}；主 Bot 标记：{}",
            own.as_ref()
                .map(|_| "本 Bot 已恢复")
                .unwrap_or_else(|err| err.as_str()),
            primaries
                .as_ref()
                .map(|_| "已恢复")
                .unwrap_or_else(|err| err.as_str())
        )),
    }
}

fn restore_assistant_if_own(
    app: &AppHandle,
    written: &ChatAssistant,
    previous: &ChatAssistant,
) -> Result<(), String> {
    if written.id != previous.id {
        return Err("回滚助手时 id 不一致，没有覆盖其他助手".into());
    }
    let current = get_assistant(app, &written.id)?;
    if serde_json::to_value(&current).ok() != serde_json::to_value(written).ok() {
        return Err(format!(
            "助手「{}」已有其他写入，没有用旧记录覆盖",
            written.id
        ));
    }
    update_assistant(app, previous.clone()).map(|_| ())
}

/// Removes only the assistant, profile, and empty fixed chat this create wrote.
/// A profile whose revision has moved, or a chat that already has messages, is left in place.
async fn compensate_failed_create(
    app: &AppHandle,
    assistant_id: &str,
    store: Option<&ProfileStore>,
    restore: Option<&InsertRestore>,
    err: String,
) -> String {
    let mut notes = Vec::new();
    let bot_id = bot_conversation_id(assistant_id);
    let chat_removed = match crate::chat::storage::conversation_file_path(app, &bot_id) {
        Ok(path) if path.exists() => match crate::chat::storage::load_conversation(app, &bot_id) {
            Ok(conversation)
                if conversation.messages.is_empty()
                    && conversation.assistant_id.as_deref() == Some(assistant_id)
                    && conversation.id == bot_id =>
            {
                match crate::chat::repository::repository(app)
                    .delete(app, &bot_id)
                    .await
                {
                    Ok(_) => true,
                    Err(delete_err) => {
                        notes.push(format!(
                            "固定私聊回滚：{}；助手和配置都保留",
                            crate::chat::repository::repository_error(delete_err)
                        ));
                        false
                    }
                }
            }
            Ok(_) => {
                notes.push("固定私聊已有消息或其他助手，没有删除本次助手和配置".to_string());
                false
            }
            Err(read_err) => {
                notes.push(format!(
                    "固定私聊无法读取，没有删除本次助手和配置：{read_err}"
                ));
                false
            }
        },
        Ok(_) => true,
        Err(path_err) => {
            notes.push(format!("固定私聊路径无法确认：{path_err}"));
            true
        }
    };
    if !chat_removed {
        return if notes.is_empty() {
            err
        } else {
            format!("{err}；{}", notes.join("；"))
        };
    }
    if let Some(restore) = restore {
        if restore.created_id != assistant_id {
            notes.push("补偿拒绝：配置回滚 id 与本次创建的助手不一致".to_string());
        } else if let Some(store) = store {
            if let Err(rollback_err) = store.rollback_insert(restore) {
                notes.push(format!("配置回滚：{rollback_err}"));
            }
        }
    }
    if let Err(assistant_err) = remove_unopened_assistant(app, assistant_id) {
        if assistant_err.contains("已经有固定") {
            notes.push(format!("助手回滚：{assistant_err}"));
        } else if let Err(row_err) =
            crate::chat::storage::remove_assistant_index_row(app, assistant_id)
        {
            notes.push(format!("助手回滚：{assistant_err}；{row_err}"));
        }
    }
    if notes.is_empty() {
        err
    } else {
        format!("{err}；{}", notes.join("；"))
    }
}

fn publish(app: &AppHandle, assistant_id: &str) {
    let _ = app.emit("chat-assistants-changed", assistant_id);
    let _ = app.emit(CHANGED_EVENT, assistant_id);
}

fn stored_chat_model(app: &AppHandle, assistant: &ChatAssistant) -> (String, String) {
    let bot_id = bot_conversation_id(&assistant.id);
    crate::chat::storage::load_conversation(app, &bot_id)
        .ok()
        .map(|conversation| (conversation.provider_id, conversation.model))
        .unwrap_or_else(|| (assistant.provider_id.clone(), assistant.model.clone()))
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
