use tauri::{AppHandle, Manager};

use super::types::{
    AgentHandoff, AgentIdentity, AgentProvider, AgentRuntimeConfig, Conversation,
    ExternalHistoryMethod, ExternalHistoryTransition,
};
use crate::state::AppState;

fn identity(
    app: &AppHandle,
    provider: AgentProvider,
    runtime: &AgentRuntimeConfig,
    model: &str,
) -> AgentIdentity {
    let (name, model) = match &provider {
        AgentProvider::External { agent_id } => (
            crate::external_agents::registry::get_agent_def(agent_id).map(|def| def.name.into()),
            runtime
                .external_model
                .as_deref()
                .filter(|model| *model != "default"),
        ),
        AgentProvider::Builtin { provider_id } | AgentProvider::Chat { provider_id } => (
            app.try_state::<AppState>().and_then(|state| {
                state
                    .settings_read()
                    .get_provider(provider_id)
                    .map(|p| p.name.clone())
            }),
            Some(model),
        ),
    };
    AgentIdentity {
        provider,
        name,
        model: model.filter(|s| !s.trim().is_empty()).map(str::to_owned),
    }
}

pub(crate) fn native_branch_owner(chat: &Conversation) -> AgentIdentity {
    AgentIdentity {
        provider: AgentProvider::selected(&chat.agent_runtime, &chat.provider_id),
        name: None,
        model: (!chat.model.trim().is_empty()).then(|| chat.model.clone()),
    }
}

pub(crate) fn restore_native_branch(
    chat: &mut Conversation,
    owner: Option<AgentIdentity>,
) -> Result<(), String> {
    let target = native_branch_owner(chat);
    match owner {
        Some(source) if source.provider == target.provider => Ok(()),
        Some(source) => apply_selection(chat, source, target),
        None => {
            // Older archives did not record who owned the SDK leaf.
            let block =
                crate::external_agents::prompt::provider_handoff_block(chat, &chat.messages);
            super::session_history::apply_provider_handoff(chat, &block, false)
        }
    }
}

pub(crate) fn timeline_message(message: &super::ChatMessage) -> bool {
    message.irc_message.is_none() && !message.id.starts_with("subagent-result-")
}

/// Part of the same JSON commit as the selected runtime. Returns whether this commit retires
/// the old CLI binding. A selection preview at the end of history replaces the preceding preview.
pub(crate) fn update_selection(
    app: &AppHandle,
    chat: &mut Conversation,
    previous: &AgentRuntimeConfig,
    provider_id: &str,
    model: &str,
    imported: bool,
) -> Result<bool, String> {
    let from = AgentProvider::selected(previous, provider_id);
    let to = AgentProvider::selected(&chat.agent_runtime, &chat.provider_id);
    let changed = from != to;
    if !changed
        && model == chat.model
        && previous.external_model == chat.agent_runtime.external_model
    {
        return Ok(false);
    }
    if changed && imported {
        return Err("导入的 CLI 会话只能使用原代理继续。".into());
    }
    if matches!(&to, AgentProvider::External { agent_id } if agent_id.is_empty()) {
        return Err("请选择外部 CLI 代理。".into());
    }
    let old_external = previous.is_external();
    let new_external = chat.agent_runtime.is_external();
    if changed && old_external && chat.external_cwd.is_none() {
        chat.external_cwd = crate::external_agents::session::load_live_handle(app, &chat.id)
            .map(|handle| handle.cwd)
            .filter(|cwd| !cwd.is_empty());
    }
    if changed {
        if let AgentProvider::External { agent_id } = &from {
            crate::external_agents::session::handoff::park_for_switch(app, chat, agent_id)?;
        }
    }
    let source = identity(app, from, previous, model);
    let target = identity(app, to, &chat.agent_runtime, &chat.model);
    apply_selection(chat, source, target)?;
    if !changed {
        return Ok(false);
    }
    if !new_external {
        if let Some(state) = app.try_state::<AppState>() {
            super::commands::context::invalidate_context_measurement(&state, chat);
        }
        let context = &mut chat.context_state;
        context.context_source = None;
        context.external_agent_id = None;
        context.external_model = None;
        context.cache_hit_rate = None;
        context.context_window_tokens = None;
        context.context_window_estimated = false;
        context.auto_compact_threshold_tokens = None;
        context.estimated_input_tokens = 0;
    }
    if let Some(goal) = chat
        .goal_state
        .as_mut()
        .filter(|goal| super::goal::is_running(goal.status))
    {
        goal.version += 1;
        goal.status = super::types::GoalStatus::Paused;
        goal.status_reason = Some("Paused because the execution context changed".into());
        goal.active_run_id = None;
        goal.updated_at = chrono::Local::now().timestamp();
    }
    Ok(old_external || new_external)
}

fn apply_selection(
    chat: &mut Conversation,
    source: AgentIdentity,
    target: AgentIdentity,
) -> Result<(), String> {
    let changed = source.provider != target.provider;
    let old_external = matches!(source.provider, AgentProvider::External { .. });
    let new_external = matches!(target.provider, AgentProvider::External { .. });
    let after = chat
        .messages
        .iter()
        .rev()
        .find(|m| timeline_message(m))
        .map(|m| m.id.clone());
    let mut reverted = false;
    if let Some(after) = after {
        let pending = chat
            .agent_handoffs
            .last()
            .is_some_and(|h| h.after_message_id == after);
        if changed || pending {
            let source = if pending {
                chat.agent_handoffs.pop().unwrap().from
            } else {
                source
            };
            reverted = source.provider == target.provider;
            if !reverted {
                chat.agent_handoffs.push(AgentHandoff {
                    after_message_id: after,
                    from: source,
                    to: target,
                });
            }
        }
    }
    if !changed {
        return Ok(());
    }
    if new_external {
        if !old_external {
            super::session_history::suspend_for_external(chat)?;
        }
        chat.external_history = (!chat.messages.is_empty() || old_external).then(|| {
            let mut transition =
                ExternalHistoryTransition::new(chat, ExternalHistoryMethod::ProviderHandoff);
            transition.allow_provider_resume = true;
            transition
        });
        crate::external_agents::context::invalidate_external_history_meter(chat);
    } else {
        chat.external_history = None;
        if old_external {
            super::session_history::resume_from_external(chat)?;
        }
        if !chat.messages.is_empty() {
            let block =
                crate::external_agents::prompt::provider_handoff_block(chat, &chat.messages);
            super::session_history::apply_provider_handoff(chat, &block, reverted)?;
        }
    }
    Ok(())
}

/// Keep boundaries attached to retained UI history. Several removed provider segments collapse to
/// one pending switch at the retained tip; a fully reverted preview leaves no divider.
pub(crate) fn retained_handoffs(
    source: &Conversation,
    retained: &[super::ChatMessage],
) -> Vec<AgentHandoff> {
    if source.agent_handoffs.is_empty() {
        return Vec::new();
    }
    let ids: std::collections::HashSet<_> = retained.iter().map(|m| m.id.as_str()).collect();
    let markers: std::collections::HashMap<_, _> = source
        .agent_handoffs
        .iter()
        .map(|h| (h.after_message_id.as_str(), h))
        .collect();
    let mut last = None;
    let mut projected: Vec<AgentHandoff> = Vec::new();
    for message in &source.messages {
        if ids.contains(message.id.as_str()) && timeline_message(message) {
            last = Some(&message.id);
        }
        if let (Some(after), Some(marker)) = (last, markers.get(message.id.as_str())) {
            let source = if projected
                .last()
                .is_some_and(|m| &m.after_message_id == after)
            {
                projected.pop().unwrap().from
            } else {
                marker.from.clone()
            };
            if source.provider != marker.to.provider {
                projected.push(AgentHandoff {
                    after_message_id: after.clone(),
                    from: source,
                    to: marker.to.clone(),
                });
            }
        }
    }
    projected
}

pub(crate) fn restore_after_truncation(
    chat: &mut Conversation,
    before: &Conversation,
    prefix_end: usize,
) -> Result<(), String> {
    let Some(last) = before.agent_handoffs.last() else {
        return Ok(());
    };
    let prefix = &chat.messages[..prefix_end];
    let crosses = !prefix.iter().any(|m| m.id == last.after_message_id)
        || prefix.last().is_some_and(|m| m.id == last.after_message_id);
    chat.agent_handoffs = retained_handoffs(before, prefix);
    if crosses && !chat.agent_runtime.is_external() {
        let block = crate::external_agents::prompt::provider_handoff_block(chat, prefix);
        super::session_history::apply_provider_handoff(chat, &block, false)?;
        // Regeneration has already retained the question in the UI. Admit it after the rebuilt
        // context so the destination receives it exactly once and subsequent edits retain binding.
        super::session_history::readmit_tail(chat, prefix_end)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::types::AgentRuntimeKind;
    use serde_json::json;

    fn select(chat: &mut Conversation, cli: Option<&str>, provider: &str, model: &str) {
        let before = AgentIdentity {
            provider: AgentProvider::selected(&chat.agent_runtime, &chat.provider_id),
            name: None,
            model: Some(if chat.agent_runtime.is_external() {
                chat.agent_runtime.external_model.clone().unwrap()
            } else {
                chat.model.clone()
            }),
        };
        chat.provider_id = provider.into();
        chat.model = model.into();
        chat.agent_runtime = AgentRuntimeConfig {
            kind: if cli.is_some() {
                AgentRuntimeKind::External
            } else {
                AgentRuntimeKind::Builtin
            },
            external_agent_id: cli.map(str::to_owned),
            external_model: cli.map(|_| model.into()),
            ..Default::default()
        };
        let after = AgentIdentity {
            provider: AgentProvider::selected(&chat.agent_runtime, &chat.provider_id),
            name: None,
            model: Some(model.into()),
        };
        apply_selection(chat, before, after).unwrap();
    }

    fn message(id: &str, role: &str, content: &str) -> super::super::ChatMessage {
        serde_json::from_value(json!({"id": id, "role": role, "content": content, "timestamp": 1}))
            .unwrap()
    }

    #[test]
    fn provider_handoff_native_branch_restores_private_context_only_to_its_owner() {
        use crate::chat::session_history::{
            archive_branch, ensure_migrated, history, switch_branch,
        };
        let mut chat: Conversation = serde_json::from_value(json!({
            "id":"chat","title":"chat","provider_id":"api-a","model":"a","created_at":1,"updated_at":1,
            "messages":[{"id":"u","role":"user","content":"question","timestamp":1},
            {"id":"a","role":"assistant","content":"public fact","timestamp":1,
             "api_messages":[{"role":"assistant","content":"private wire transcript"}]}]
        })).unwrap();
        ensure_migrated(&mut chat).unwrap();
        archive_branch(&mut chat).unwrap();
        let archive = chat.builtin_history.as_ref().unwrap().archived_branches[0]
            .id
            .clone();
        let mut same = chat.clone();
        switch_branch(&mut same, &archive).unwrap();
        assert_eq!(
            history(&same)
                .unwrap()
                .context()
                .last()
                .unwrap()
                .text_content(),
            "private wire transcript"
        );
        for legacy in [false, true] {
            let mut switched = chat.clone();
            select(&mut switched, None, "api-b", "b");
            if legacy {
                switched.builtin_history.as_mut().unwrap().archived_branches[0].owner = None;
            }
            switch_branch(&mut switched, &archive).unwrap();
            let leaf = history(&switched).unwrap();
            let context = leaf.context().last().unwrap().text_content();
            assert!(context.contains("public fact"));
            assert!(!context.contains("private wire transcript"));
            assert_eq!(switched.agent_handoffs.len(), usize::from(!legacy));
        }
    }

    #[test]
    fn provider_handoff_fork_before_boundary_replays_only_retained_public_facts() {
        use crate::chat::session_history::{ensure_migrated, fork_prefix};
        let mut chat: Conversation = serde_json::from_value(json!({
            "id":"chat","title":"chat","provider_id":"api-a","model":"a","created_at":1,"updated_at":1,
            "messages":[{"id":"u","role":"user","content":"question","timestamp":1},
            {"id":"a","role":"assistant","content":"retained public answer","timestamp":1,
             "api_messages":[{"role":"assistant","content":"private wire transcript"}]},
            {"id":"u2","role":"user","content":"discarded followup","timestamp":2},
            {"id":"a2","role":"assistant","content":"discarded answer","timestamp":2}]
        })).unwrap();
        ensure_migrated(&mut chat).unwrap();
        select(&mut chat, None, "api-b", "b");
        let retained = chat.messages[..2].to_vec();
        let branch = fork_prefix(&mut chat, &retained).unwrap();
        let leaf = branch.snapshot.into_history();
        let block = leaf.context().last().unwrap().text_content();
        assert!(block.contains("retained public answer"));
        assert!(!block.contains("private wire transcript") && !block.contains("discarded"));
    }

    #[test]
    fn provider_handoff_reload_switch_back_and_picker_reselection() {
        use crate::chat::session_history::{ensure_migrated, history};
        use crate::external_agents::prompt::{
            compose_external_prompt_passthrough, compose_history_handoff,
        };
        let mut chat: Conversation = serde_json::from_value(json!({
            "id":"chat","title":"chat","provider_id":"api-a","model":"a",
            "created_at":1,"updated_at":1,"messages":[
                {"id":"u","role":"user","content":"before switching","timestamp":1},
                {"id":"a","role":"assistant","content":"public answer","timestamp":1,
                 "api_messages":[{"role":"assistant","content":"private provider replay"}]}
            ]
        }))
        .unwrap();
        ensure_migrated(&mut chat).unwrap();
        select(&mut chat, None, "api-b", "b");
        let native = history(&chat).unwrap();
        let block = native.context().last().unwrap().text_content();
        assert!(block.contains("public answer"));
        assert!(!block.contains("private provider replay"));
        select(&mut chat, None, "api-a", "a");
        assert!(chat.agent_handoffs.is_empty());
        assert_eq!(
            history(&chat)
                .unwrap()
                .context()
                .last()
                .unwrap()
                .text_content(),
            "private provider replay"
        );

        select(&mut chat, Some("claude"), "api-a", "opus");
        select(&mut chat, Some("codex"), "api-a", "x");
        select(&mut chat, Some("codex"), "api-a", "y");
        let mut reopened: Conversation =
            serde_json::from_slice(&serde_json::to_vec(&chat).unwrap()).unwrap();
        let parked = reopened
            .builtin_history
            .as_ref()
            .unwrap()
            .active_branch_id
            .clone();
        assert!(crate::chat::session_history::branch_history(&reopened).is_none());
        assert!(crate::chat::session_history::switch_branch(&mut reopened, &parked).is_err());
        let markers = &reopened.agent_handoffs;
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].after_message_id, "a");
        assert_eq!(
            markers[0].from.provider,
            AgentProvider::Builtin {
                provider_id: "api-a".into()
            }
        );
        assert_eq!(markers[0].to.model.as_deref(), Some("y"));
        let prompt = compose_history_handoff(
            &reopened,
            compose_external_prompt_passthrough("continue"),
            "continue",
        )
        .unwrap();
        assert_eq!(prompt.full_prompt.matches("before switching").count(), 1);
        assert_eq!(prompt.full_prompt.matches("continue").count(), 1);
        assert!(prompt.instructions_block.is_empty());
        assert!(crate::external_agents::session::native_prompt_matches(
            &prompt.full_prompt,
            "continue"
        ));
        reopened
            .messages
            .push(message("cli-user", "user", "add the second fact"));
        reopened
            .messages
            .push(message("cli-answer", "assistant", "second fact is jade"));
        select(&mut reopened, None, "api-a", "a");
        assert_eq!(reopened.agent_handoffs.len(), 2);
        assert!(reopened.external_history.is_none());
        let history = history(&reopened).unwrap();
        let block = history.context().last().unwrap().text_content();
        assert_eq!(block.matches("second fact is jade").count(), 1);
        assert!(block.contains("public answer"));
        assert!(!block.contains("private provider replay"));
        let before = reopened.clone();
        crate::chat::session_history::truncate_before(&mut reopened, "cli-user").unwrap();
        let end = reopened.messages.len();
        restore_after_truncation(&mut reopened, &before, end).unwrap();
        assert!(reopened.agent_handoffs.is_empty());
        let rewound = crate::chat::session_history::history(&reopened).unwrap();
        let prompt = rewound.context().last().unwrap().text_content();
        assert!(prompt.contains("public answer"));
        assert!(!prompt.contains("second fact is jade"));
        assert_eq!(reopened.messages.last().unwrap().id, "a");
    }
}
