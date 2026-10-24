use tauri::AppHandle;

use crate::chat::agent::prepare as agent_prepare;
use crate::mcp::{self, ChatToolDefinition};
use crate::settings::{SessionModel, Settings};
use crate::skills;
use crate::state::AppState;

/// Determine which MCP servers can survive this conversation's tool filters
/// before opening any transports. None means unrestricted; Some([]) means none.
pub(super) fn allowed_mcp_server_ids<'a>(
    conversation: &'a crate::chat::Conversation,
    settings: &Settings,
) -> Option<&'a [String]> {
    if super::catalog::is_builder_conversation(conversation)
        || (conversation.agent_runtime.is_chat() && !settings.chat.chat_mode.mcp_read_only)
    {
        return Some(&[]);
    }
    conversation
        .assistant_snapshot
        .as_ref()
        .map(|assistant| assistant.mcp_server_ids.as_slice())
}

/// Resolve explicit skill commands anywhere outside code/URLs in the user task.
/// Returns the first skill ID and deduplicated instructions for all referenced
/// skills, with each command's following arguments substituted.
/// Never replace the user message: its command and task must survive display,
/// persistence, editing and replay. Skill activation never changes enabled tools.
///
/// `disable_model_invocation` only gates *model* auto-invocation, so it is
/// intentionally ignored here — an explicit user slash command may still trigger
/// such a skill. Availability is gated by `skill_allowed_for_conversation`
/// (Settings enable list, plugin availability, assistant allow-list).
pub(super) fn try_apply_skill_slash_trigger(
    registry: &skills::SkillRegistry,
    chat_tools: &crate::settings::ChatToolsConfig,
    assistant_snapshot: Option<&crate::chat::types::ChatAssistantSnapshot>,
    content: &str,
) -> Option<(String, String)> {
    let matches: Vec<_> = crate::chat::slash_commands::command_ranges(content)
        .into_iter()
        .filter_map(|range| {
            let record = registry.find_by_trigger(&content[range.clone()])?;
            agent_prepare::skill_allowed_for_conversation(
                chat_tools,
                assistant_snapshot,
                &record.meta.id,
            )
            .then_some((range, record))
        })
        .collect();
    let first_id = matches.first()?.1.meta.id.clone();
    let mut seen = std::collections::HashSet::new();
    let mut instructions = Vec::new();
    for (index, (range, record)) in matches.iter().enumerate() {
        if !seen.insert(&record.meta.id) {
            continue;
        }
        let end = matches
            .get(index + 1)
            .map_or(content.len(), |(range, _)| range.start);
        let args_raw = content[range.end..end].trim_start();
        let mut rendered = (*record).clone();
        rendered.body =
            skills::substitute_arguments(&record.body, args_raw, &record.meta.arguments);
        instructions.push(skills::activate_skill(&rendered));
    }
    Some((first_id, instructions.join("\n\n")))
}

/// Shared by replies (including retries) and context accounting. Explicit slash
/// instructions are loaded once, independently of the visible user content.
pub(super) fn resolve_request_skill(
    registry: &skills::SkillRegistry,
    chat_tools: &mut crate::settings::ChatToolsConfig,
    assistant_snapshot: Option<&crate::chat::types::ChatAssistantSnapshot>,
    content: &str,
    requested: Option<&str>,
) -> (Option<String>, Option<skills::SkillDetail>) {
    let slash = try_apply_skill_slash_trigger(registry, chat_tools, assistant_snapshot, content);
    let id = resolve_forced_skill_id(
        chat_tools,
        assistant_snapshot,
        registry,
        slash.as_ref().map(|(id, _)| id.as_str()).or(requested),
    );
    let detail = id
        .as_deref()
        .and_then(|id| registry.find(id))
        .map(|record| skills::SkillDetail {
            meta: record.meta.clone(),
            body: slash
                .as_ref()
                .map(|(_, body)| body.clone())
                .unwrap_or_else(|| record.body.clone()),
        });
    if slash.is_some() {
        chat_tools.skill_fallback_mode = "skill_md_only".to_string();
    }
    (id, detail)
}

pub(super) fn resolve_forced_skill_id(
    chat_tools: &crate::settings::ChatToolsConfig,
    assistant_snapshot: Option<&crate::chat::types::ChatAssistantSnapshot>,
    registry: &skills::SkillRegistry,
    requested: Option<&str>,
) -> Option<String> {
    let requested = requested.map(str::trim).filter(|id| !id.is_empty())?;
    let enabled = registry
        .records
        .iter()
        .filter(|record| {
            agent_prepare::skill_allowed_for_conversation(
                chat_tools,
                assistant_snapshot,
                &record.meta.id,
            )
        })
        .any(|record| {
            record.meta.id == requested
                || record.meta.name == requested
                || skills::slugify(requested) == record.meta.id
        });
    if enabled {
        Some(requested.to_string())
    } else {
        None
    }
}

#[derive(Default)]
pub(super) struct ChatToolList {
    pub tools: Vec<ChatToolDefinition>,
    pub unavailable_mcp_servers: Vec<String>,
}

/// Wait for the run's catalog while retaining the shared MCP connection tasks.
pub(super) async fn await_chat_tool_discovery(
    state: &AppState,
    conversation_id: &str,
    generation: u64,
    discovery: impl std::future::Future<Output = ChatToolList>,
) -> Result<ChatToolList, String> {
    tokio::select! {
        // A run cancelled during earlier preparation must not open transports.
        biased;
        _ = super::interaction::wait_for_chat_cancel(state, conversation_id, generation) => {
            Err("cancelled".into())
        }
        catalog = discovery => Ok(catalog),
    }
}

pub(crate) async fn list_tools_for_chat(
    app: &AppHandle,
    state: &AppState,
    settings: &Settings,
    session: Option<SessionModel<'_>>,
    allowed_mcp_server_ids: Option<&[String]>,
) -> ChatToolList {
    if !(settings.chat_tools.enabled
        || crate::settings::chat_native_tools_enabled(&settings.chat_tools)
        || crate::settings::chat_memory_tools_enabled(settings)
        || crate::settings::chat_image_generation_enabled_for_session(settings, session)
        || settings.advisor_model().is_some())
    {
        return ChatToolList::default();
    }
    let catalog =
        mcp::registry::list_enabled_tool_catalog_for_run(app, state, allowed_mcp_server_ids).await;
    let mut tools = catalog.tools;
    if let Some((provider_id, model)) =
        crate::chat::model_metadata::image_generation_model_for_session(settings, session)
    {
        if !tools.iter().any(|tool| tool.name == "mixer_generate_image") {
            let mut tool = mcp::types::mixer_generate_image_tool_for(Some(&model));
            let provider_name = settings
                .get_provider(&provider_id)
                .map(|provider| {
                    if provider.name.trim().is_empty() {
                        provider.id.clone()
                    } else {
                        provider.name.clone()
                    }
                })
                .unwrap_or(provider_id);
            tool.server_id = Some(format!("{provider_name} / {model}"));
            tools.push(tool);
        }
    }
    ChatToolList {
        tools,
        unavailable_mcp_servers: catalog.unavailable_mcp_servers,
    }
}

pub(super) fn append_agent_todo_tools(tools: &mut Vec<ChatToolDefinition>) {
    crate::chat::todo::append_tool_definitions(tools);
}

pub(super) fn append_goal_tools(
    tools: &mut Vec<ChatToolDefinition>,
    goal: Option<&crate::chat::types::GoalState>,
) -> bool {
    crate::chat::goal::append_tool_definitions(tools, goal)
}

pub(super) fn append_agent_ask_user_tools(tools: &mut Vec<ChatToolDefinition>) -> bool {
    crate::chat::ask_user::append_tool_definitions(tools);
    true
}

pub(crate) use crate::chat::agent::tool_policy::*;

#[cfg(test)]
mod discovery_tests {
    use super::*;
    use crate::chat::commands::reply_runtime::{ChatReplyGuard, ChatSendReservation};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;
    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn cancelled_cold_mcp_discovery_releases_the_conversation_for_resend() {
        let state = crate::state::test_app_state();
        let conversation_id = "conv_cancel_discovery";
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let server_task = tokio::spawn(async move {
            let (mut connection, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            assert!(connection.read(&mut request).await.unwrap() > 0);
            started_tx.send(()).unwrap();
            // Accept the actual MCP initialize request, but never answer it.
            std::future::pending::<()>().await;
            drop(connection);
        });
        let mut settings = Settings::default();
        settings.chat_tools.enabled = true;
        settings.chat_tools.servers = vec![crate::settings::ChatMcpServer {
            id: "cancel-cold-discovery".into(),
            name: "Stalled cold MCP".into(),
            enabled: true,
            transport: "streamable_http".into(),
            url: format!("http://{address}/mcp"),
            ..Default::default()
        }];
        let generation = state.chat_runtime().begin_generation(conversation_id);
        let reply = async {
            let _send = ChatSendReservation::try_acquire(&state, conversation_id).unwrap();
            let _reply =
                ChatReplyGuard::try_new(&state, conversation_id, "run-discovery", generation)
                    .unwrap();
            await_chat_tool_discovery(&state, conversation_id, generation, async {
                let (tools, unavailable_mcp_servers) =
                    mcp::registry::collect_enabled_mcp_tool_defs(&state, None, &settings, None)
                        .await;
                ChatToolList {
                    tools,
                    unavailable_mcp_servers,
                }
            })
            .await
        };
        let cancel = async {
            tokio::time::timeout(Duration::from_secs(1), started_rx)
                .await
                .expect("real MCP discovery must start before cancellation")
                .unwrap();
            assert!(ChatSendReservation::try_acquire(&state, conversation_id).is_none());
            state.cancel_chat_generation(conversation_id);
        };
        let (result, ()) =
            tokio::join!(tokio::time::timeout(Duration::from_secs(2), reply), cancel);
        server_task.abort();
        state.mcp_disconnect_all().await;
        assert!(
            result.is_ok(),
            "stopping during MCP discovery must not wait for its 30s handshake timeout"
        );
        assert!(matches!(result.unwrap(), Err(error) if error == "cancelled"));
        assert!(ChatSendReservation::try_acquire(&state, conversation_id).is_some());
        assert!(!state
            .chat_runtime()
            .is_generation_active(conversation_id, generation));
    }

    #[tokio::test]
    async fn cancelled_run_does_not_start_tool_discovery() {
        let state = crate::state::test_app_state();
        let id = "conv_already_cancelled";
        let generation = state.chat_runtime().begin_generation(id);
        state.cancel_chat_generation(id);
        let started = AtomicBool::new(false);
        let result = await_chat_tool_discovery(&state, id, generation, async {
            started.store(true, Ordering::SeqCst);
            ChatToolList::default()
        })
        .await;
        assert!(matches!(result, Err(error) if error == "cancelled"));
        assert!(!started.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn active_run_keeps_the_discovered_tool_catalog() {
        let state = crate::state::test_app_state();
        let id = "conv_discovery_success";
        let generation = state.chat_runtime().begin_generation(id);
        let result = await_chat_tool_discovery(&state, id, generation, async {
            ChatToolList {
                tools: vec![mcp::types::native_web_fetch_tool()],
                unavailable_mcp_servers: vec!["Unavailable fixture".into()],
            }
        })
        .await
        .unwrap();
        assert_eq!(result.tools[0].id, "native__web_fetch");
        assert_eq!(result.unavailable_mcp_servers, ["Unavailable fixture"]);
    }
}

#[cfg(test)]
mod web_search_filter_tests {
    use super::*;
    use crate::chat::types::WebSearchMode;

    #[test]
    fn off_and_builtin_remove_search_web() {
        let settings = crate::settings::Settings::default();
        for mode in [WebSearchMode::Off, WebSearchMode::Builtin] {
            let mut tools = vec![
                crate::mcp::types::native_web_search_tool(),
                crate::mcp::types::native_web_fetch_tool(),
            ];
            apply_web_search_mode_tool_filter(&mut tools, mode, &settings);
            assert!(
                !tools.iter().any(|t| t.id == "native__web_search"),
                "search_web should be stripped in {mode:?}"
            );
            assert!(
                tools.iter().any(|t| t.id == "native__web_fetch"),
                "unrelated tools must survive in {mode:?}"
            );
        }
    }

    #[test]
    fn third_party_keeps_existing_search_web() {
        let settings = crate::settings::Settings::default();
        let mut tools = vec![crate::mcp::types::native_web_search_tool()];
        apply_web_search_mode_tool_filter(&mut tools, WebSearchMode::ThirdParty, &settings);
        assert!(tools.iter().any(|t| t.id == "native__web_search"));
    }
}
