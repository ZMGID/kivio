use tauri::AppHandle;

use crate::chat::types::{TurnModel, TurnModelSelection};
use crate::chat::{ChatMessage, Conversation};

/// Called inside the same mutation that admits or regenerates the visible question.
pub(super) fn record_turn_model(
    conversation: &mut Conversation,
    index: usize,
    settings: &crate::settings::Settings,
) {
    let runtime = &conversation.agent_runtime;
    let current = if let Some(id) = runtime
        .external_agent_id
        .as_deref()
        .filter(|_| runtime.is_external())
    {
        let definition = crate::external_agents::registry::get_agent_def(id);
        let model = runtime
            .external_model
            .as_deref()
            .filter(|id| !id.is_empty() && *id != "default");
        let model_name = definition.and_then(|def| {
            def.fallback_models
                .iter()
                .find(|(id, _)| Some(*id) == model)
                .map(|(_, label)| label.to_string())
        });
        TurnModelSelection {
            provider: format!("cli:{id}"),
            provider_name: definition.map(|agent| agent.name).unwrap_or(id).to_owned(),
            model: model.map(str::to_owned),
            model_name,
            effort: runtime
                .external_reasoning
                .clone()
                .filter(|value| !value.is_empty() && value != "default"),
        }
    } else {
        let provider = settings.get_provider(&conversation.provider_id);
        let effort = super::resolve_thinking(
            conversation.thinking_level.as_deref(),
            settings.chat.thinking_enabled,
            provider,
            &conversation.model,
        )
        .ok()
        .and_then(|(enabled, level)| if enabled { level } else { Some("off".into()) });
        TurnModelSelection {
            provider: conversation.provider_id.clone(),
            provider_name: provider
                .map(|p| p.name.clone())
                .unwrap_or_else(|| conversation.provider_id.clone()),
            model: Some(conversation.model.clone()),
            model_name: None,
            effort,
        }
    };
    let previous = conversation.messages[..index]
        .iter()
        .rev()
        .find(|message| message.role == "user" && message.irc_message.is_none())
        .and_then(|message| message.turn_model.as_ref())
        .map(|turn| &turn.current)
        .filter(|last| {
            last.provider != current.provider
                || last.model != current.model
                || last.effort != current.effort
        })
        .cloned();
    conversation.messages[index].turn_model = Some(Box::new(TurnModel { current, previous }));
}

pub(super) fn emit_turn_model(app: &AppHandle, conversation: &Conversation, message: &ChatMessage) {
    if let Some(turn_model) = &message.turn_model {
        crate::chat::protocol::emit_conversation_event(
            app,
            &conversation.id,
            conversation.revision,
            crate::chat::protocol::ChatConversationEvent::TurnModelUpdated {
                message_id: message.id.clone(),
                turn_model: (**turn_model).clone(),
            },
        );
    }
}
