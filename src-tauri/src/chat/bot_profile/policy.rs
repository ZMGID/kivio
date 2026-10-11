//! Pure capability, model, and approval resolution.
//! Granted defaults are the ids the user has already enabled, not every install.

use crate::chat::ChatAssistant;

use super::types::{
    ApprovalSource, BotApprovalPolicy, BotApprovalWrite, BotCapabilityPolicy, BotCapabilityWrite,
    BotModelPolicy, BotModelWrite, BotProfile, CapabilitySource, ModelSource, PreparedBot,
    ResolvedCapability, ResolvedModelSource, APPROVAL_POLICIES, EXPLICIT_EXPLANATION,
    EXPLICIT_MODEL_EXPLANATION, FROZEN_MODEL_EXPLANATION, GRANTED_DEFAULTS_EXPLANATION,
    INHERIT_APPROVAL_EXPLANATION, INHERIT_MODEL_EXPLANATION, LEGACY_EXPLANATION,
    OVERRIDE_APPROVAL_EXPLANATION, PROFILE_VERSION,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GrantSet {
    pub mcp_server_ids: Vec<String>,
    pub skill_ids: Vec<String>,
}

pub fn granted_defaults_policy() -> BotCapabilityPolicy {
    BotCapabilityPolicy {
        source: CapabilitySource::GrantedDefaults,
        mcp_server_ids: None,
        skill_ids: None,
        explanation: GRANTED_DEFAULTS_EXPLANATION.to_string(),
    }
}

pub fn explicit_policy(mcp_server_ids: Vec<String>, skill_ids: Vec<String>) -> BotCapabilityPolicy {
    BotCapabilityPolicy {
        source: CapabilitySource::Explicit,
        mcp_server_ids: Some(mcp_server_ids),
        skill_ids: Some(skill_ids),
        explanation: EXPLICIT_EXPLANATION.to_string(),
    }
}

pub fn legacy_policy(assistant: &ChatAssistant) -> BotCapabilityPolicy {
    BotCapabilityPolicy {
        source: CapabilitySource::LegacyWhitelist,
        mcp_server_ids: Some(assistant.mcp_server_ids.clone()),
        skill_ids: Some(assistant.skill_ids.clone()),
        explanation: LEGACY_EXPLANATION.to_string(),
    }
}

pub fn inherit_model() -> BotModelPolicy {
    BotModelPolicy {
        source: ModelSource::InheritGlobal,
        explanation: INHERIT_MODEL_EXPLANATION.to_string(),
    }
}

pub fn explicit_model() -> BotModelPolicy {
    BotModelPolicy {
        source: ModelSource::Explicit,
        explanation: EXPLICIT_MODEL_EXPLANATION.to_string(),
    }
}

pub fn inherit_approval() -> BotApprovalPolicy {
    BotApprovalPolicy {
        source: ApprovalSource::InheritGlobal,
        policy: None,
        explanation: INHERIT_APPROVAL_EXPLANATION.to_string(),
    }
}

pub fn capability_from_write(
    write: Option<&BotCapabilityWrite>,
) -> Result<BotCapabilityPolicy, String> {
    let Some(write) = write else {
        return Ok(granted_defaults_policy());
    };
    match write.source {
        CapabilitySource::GrantedDefaults => Ok(granted_defaults_policy()),
        CapabilitySource::Explicit => Ok(explicit_policy(
            normalize_ids(write.mcp_server_ids.as_deref().unwrap_or(&[])),
            normalize_ids(write.skill_ids.as_deref().unwrap_or(&[])),
        )),
        CapabilitySource::LegacyWhitelist => Err(
            "不能把历史白名单写成新的能力来源；不改能力，或明确选择 granted_defaults / explicit"
                .into(),
        ),
    }
}

pub fn model_from_write(
    write: Option<&BotModelWrite>,
) -> Result<(BotModelPolicy, String, String), String> {
    let Some(write) = write else {
        return Ok((inherit_model(), String::new(), String::new()));
    };
    match write.source {
        ModelSource::InheritGlobal => Ok((inherit_model(), String::new(), String::new())),
        ModelSource::Explicit => {
            let provider_id = write
                .provider_id
                .as_deref()
                .unwrap_or("")
                .trim()
                .to_string();
            let model = write.model.as_deref().unwrap_or("").trim().to_string();
            if provider_id.is_empty() || model.is_empty() {
                return Err(
                    "明确模型需要同时提供 providerId 和 model。沿用全局默认时不要只填一个。".into(),
                );
            }
            Ok((explicit_model(), provider_id, model))
        }
    }
}

pub fn approval_from_write(write: Option<&BotApprovalWrite>) -> Result<BotApprovalPolicy, String> {
    let Some(write) = write else {
        return Ok(inherit_approval());
    };
    match write.source {
        ApprovalSource::InheritGlobal => Ok(inherit_approval()),
        ApprovalSource::Override => {
            let policy = write
                .policy
                .as_deref()
                .map(str::trim)
                .filter(|policy| !policy.is_empty())
                .ok_or("审批覆盖需要 policy")?;
            if !APPROVAL_POLICIES.contains(&policy) {
                return Err(format!(
                    "不认识的审批策略「{policy}」。允许：{}",
                    APPROVAL_POLICIES.join(", ")
                ));
            }
            Ok(BotApprovalPolicy {
                source: ApprovalSource::Override,
                policy: Some(policy.to_string()),
                explanation: OVERRIDE_APPROVAL_EXPLANATION.to_string(),
            })
        }
    }
}

pub fn prepare_create(
    assistant_id: String,
    name: String,
    description: String,
    system_prompt: String,
    icon: String,
    color: String,
    primary: bool,
    capability: Option<&BotCapabilityWrite>,
    approval: Option<&BotApprovalWrite>,
    model: Option<&BotModelWrite>,
    now: i64,
) -> Result<PreparedBot, String> {
    let capability = capability_from_write(capability)?;
    let (model_policy, provider_id, model_id) = model_from_write(model)?;
    let approval = approval_from_write(approval)?;
    let (mcp_server_ids, skill_ids) = match capability.source {
        CapabilitySource::Explicit => (
            capability.mcp_server_ids.clone().unwrap_or_default(),
            capability.skill_ids.clone().unwrap_or_default(),
        ),
        CapabilitySource::GrantedDefaults | CapabilitySource::LegacyWhitelist => {
            (Vec::new(), Vec::new())
        }
    };
    let assistant = crate::chat::ChatAssistant {
        id: assistant_id.clone(),
        name,
        description,
        icon,
        color,
        source: "user".to_string(),
        system_prompt,
        provider_id,
        model: model_id,
        mcp_server_ids,
        skill_ids,
        enabled: true,
        installed: true,
        archived: false,
        built_in: false,
        created_at: now,
        updated_at: now,
    };
    let profile = BotProfile {
        version: PROFILE_VERSION,
        assistant_id,
        primary,
        capability,
        model: model_policy,
        approval,
        shared_memory: Vec::new(),
        revision: 1,
        created_at: now,
        updated_at: now,
    };
    Ok(PreparedBot { assistant, profile })
}

pub fn merge_edit(
    assistant: &crate::chat::ChatAssistant,
    profile: &BotProfile,
    name: Option<&str>,
    description: Option<&str>,
    system_prompt: Option<&str>,
    icon: Option<&str>,
    color: Option<&str>,
    primary: Option<bool>,
    capability: Option<&BotCapabilityWrite>,
    approval: Option<&BotApprovalWrite>,
    model: Option<&BotModelWrite>,
    now: i64,
) -> Result<PreparedBot, String> {
    let mut next_assistant = assistant.clone();
    let mut next_profile = profile.clone();
    if let Some(name) = name {
        next_assistant.name = name.to_string();
    }
    if let Some(description) = description {
        next_assistant.description = description.to_string();
    }
    if let Some(system_prompt) = system_prompt {
        next_assistant.system_prompt = system_prompt.to_string();
    }
    if let Some(icon) = icon {
        next_assistant.icon = icon.to_string();
    }
    if let Some(color) = color {
        next_assistant.color = color.to_string();
    }
    if let Some(capability) = capability {
        next_profile.capability = capability_from_write(Some(capability))?;
        match next_profile.capability.source {
            CapabilitySource::Explicit => {
                next_assistant.mcp_server_ids = next_profile
                    .capability
                    .mcp_server_ids
                    .clone()
                    .unwrap_or_default();
                next_assistant.skill_ids = next_profile
                    .capability
                    .skill_ids
                    .clone()
                    .unwrap_or_default();
            }
            CapabilitySource::GrantedDefaults => {
                next_assistant.mcp_server_ids.clear();
                next_assistant.skill_ids.clear();
            }
            CapabilitySource::LegacyWhitelist => {}
        }
    }
    if let Some(model) = model {
        let (policy, provider_id, model_id) = model_from_write(Some(model))?;
        next_profile.model = policy;
        next_assistant.provider_id = provider_id;
        next_assistant.model = model_id;
    }
    if let Some(approval) = approval {
        next_profile.approval = approval_from_write(Some(approval))?;
    }
    if let Some(primary) = primary {
        next_profile.primary = primary;
    }
    next_assistant.updated_at = now;
    next_profile.updated_at = now;
    next_profile.assistant_id = next_assistant.id.clone();
    Ok(PreparedBot {
        assistant: next_assistant,
        profile: next_profile,
    })
}

pub fn resolve_capability(
    policy: &BotCapabilityPolicy,
    legacy_mcp: &[String],
    legacy_skills: &[String],
    grants: &GrantSet,
) -> ResolvedCapability {
    match policy.source {
        CapabilitySource::GrantedDefaults => {
            let mcp_server_ids = sorted_unique(&grants.mcp_server_ids);
            let skill_ids = sorted_unique(&grants.skill_ids);
            let mut explanation = GRANTED_DEFAULTS_EXPLANATION.to_string();
            if mcp_server_ids.is_empty() && skill_ids.is_empty() {
                explanation.push_str(
                    " 当前没有已启用的 MCP，也没有已授予的技能，所以这份 Bot 现在没有这些能力。",
                );
            }
            ResolvedCapability {
                source: CapabilitySource::GrantedDefaults,
                explanation,
                mcp_server_ids,
                skill_ids,
                dropped_mcp_server_ids: Vec::new(),
                dropped_skill_ids: Vec::new(),
                rewrite_snapshot: true,
            }
        }
        CapabilitySource::Explicit => {
            let stored_mcp = policy.mcp_server_ids.clone().unwrap_or_default();
            let stored_skills = policy.skill_ids.clone().unwrap_or_default();
            let (mcp_server_ids, dropped_mcp_server_ids) =
                intersect(&stored_mcp, &grants.mcp_server_ids);
            let (skill_ids, dropped_skill_ids) = intersect(&stored_skills, &grants.skill_ids);
            let mut explanation = EXPLICIT_EXPLANATION.to_string();
            if stored_mcp.is_empty() && stored_skills.is_empty() {
                explanation.push_str(" 保存的名单是空的，这是明确不启用，不是默认授予。");
            }
            append_dropped(
                &mut explanation,
                &dropped_mcp_server_ids,
                &dropped_skill_ids,
            );
            ResolvedCapability {
                source: CapabilitySource::Explicit,
                explanation,
                mcp_server_ids,
                skill_ids,
                dropped_mcp_server_ids,
                dropped_skill_ids,
                rewrite_snapshot: true,
            }
        }
        CapabilitySource::LegacyWhitelist => ResolvedCapability {
            source: CapabilitySource::LegacyWhitelist,
            explanation: LEGACY_EXPLANATION.to_string(),
            mcp_server_ids: legacy_mcp.to_vec(),
            skill_ids: legacy_skills.to_vec(),
            dropped_mcp_server_ids: Vec::new(),
            dropped_skill_ids: Vec::new(),
            rewrite_snapshot: false,
        },
    }
}

pub fn resolve_model(
    profile: Option<&BotProfile>,
    assistant: &ChatAssistant,
    conversation_provider: &str,
    conversation_model: &str,
    global_provider: &str,
    global_model: &str,
) -> Result<(String, String, ResolvedModelSource, String, bool), String> {
    let Some(profile) = profile else {
        return Ok((
            conversation_provider.to_string(),
            conversation_model.to_string(),
            ResolvedModelSource::ConversationFrozen,
            FROZEN_MODEL_EXPLANATION.to_string(),
            false,
        ));
    };
    match profile.model.source {
        ModelSource::InheritGlobal => Ok((
            global_provider.to_string(),
            global_model.to_string(),
            ResolvedModelSource::InheritGlobal,
            INHERIT_MODEL_EXPLANATION.to_string(),
            true,
        )),
        ModelSource::Explicit => {
            if assistant.provider_id.trim().is_empty() || assistant.model.trim().is_empty() {
                return Err(
                    "这份 Bot 的模型来源是明确覆盖，但助手上的供应商或模型是空的。没有改用全局默认。"
                        .into(),
                );
            }
            Ok((
                assistant.provider_id.clone(),
                assistant.model.clone(),
                ResolvedModelSource::Explicit,
                EXPLICIT_MODEL_EXPLANATION.to_string(),
                true,
            ))
        }
    }
}

pub fn legacy_profile(assistant: &ChatAssistant, now: i64) -> BotProfile {
    let model = if assistant.provider_id.trim().is_empty() && assistant.model.trim().is_empty() {
        inherit_model()
    } else {
        explicit_model()
    };
    BotProfile {
        version: PROFILE_VERSION,
        assistant_id: assistant.id.clone(),
        primary: false,
        capability: legacy_policy(assistant),
        model,
        approval: inherit_approval(),
        shared_memory: Vec::new(),
        revision: 0,
        created_at: now,
        updated_at: now,
    }
}

fn normalize_ids(values: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        let item = value.trim();
        if item.is_empty() || out.iter().any(|existing| existing == item) {
            continue;
        }
        out.push(item.to_string());
    }
    out
}

fn sorted_unique(values: &[String]) -> Vec<String> {
    let mut out = normalize_ids(values);
    out.sort();
    out
}

fn intersect(stored: &[String], grants: &[String]) -> (Vec<String>, Vec<String>) {
    let stored = normalize_ids(stored);
    let mut kept = Vec::new();
    let mut dropped = Vec::new();
    for id in stored {
        if grants.iter().any(|grant| grant == &id) {
            kept.push(id);
        } else {
            dropped.push(id);
        }
    }
    kept.sort();
    dropped.sort();
    (kept, dropped)
}

fn append_dropped(explanation: &mut String, mcp: &[String], skills: &[String]) {
    if mcp.is_empty() && skills.is_empty() {
        return;
    }
    explanation.push_str(" 这些已保存但当前未授予，所以不会启用：");
    if !mcp.is_empty() {
        explanation.push_str(" MCP ");
        explanation.push_str(&mcp.join(", "));
        explanation.push('。');
    }
    if !skills.is_empty() {
        explanation.push_str(" 技能 ");
        explanation.push_str(&skills.join(", "));
        explanation.push('。');
    }
}
