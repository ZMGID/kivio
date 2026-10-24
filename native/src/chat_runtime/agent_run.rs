//! Assemble the shared Kivio agent loop from native settings and conversation owners.
use super::{ChatCompletion, ChatEvent, agent_host::NativeAgentHost};
use kivio_model_runtime as runtime;
use runtime::{GenerateRequest, provider_settings::ModelProvider, settings::Settings};
use std::{path::PathBuf, sync::Arc};

pub async fn run(
    provider: ModelProvider,
    mut request: GenerateRequest,
    settings: Settings,
    root: PathBuf,
    data_root: PathBuf,
    attempts: usize,
    events: async_channel::Sender<ChatEvent>,
    executor: Arc<runtime::native_tool_runtime::NativeToolExecutor>,
    chat_mode: bool,
) -> Result<ChatCompletion, String> {
    use runtime::agent::{AgentRunConfig, prepare};
    let conversation_id = request.metadata.conversation_id.clone().ok_or("缺少对话身份")?;
    let message_id = request.metadata.message_id.clone().ok_or("缺少回复身份")?;
    let catalog = root.join("conversations");
    let conversation = runtime::conversation_files::ConversationStore::new(catalog.clone()).load(&conversation_id).map_err(|e| e.to_string())?;
    let provider = runtime::provider_oauth::resolve_provider(&provider).await?;
    let model_host = runtime::RuntimeHost::shared(&provider, &data_root);
    model_host.set_request_debug_enabled(settings.chat_tools.request_debug_enabled);
    let mut prepared = executor.prepare(&settings, &conversation, chat_mode).await?;
    prepare::apply_assistant_mcp_restrictions(&mut prepared.tools, conversation.assistant_snapshot.as_ref());
    let mut effective_chat_tools = settings.chat_tools.clone();
    prepare::apply_skill_fallback_when_tools_unavailable(&mut effective_chat_tools, conversation.active_skill_id.as_deref(), !prepared.tools.is_empty());
    let language = runtime::settings::resolve_chat_language(&settings);
    let memory = if settings.chat_memory.enabled {
        runtime::memory_files::l1_prompt_block_at(&runtime::memory_files::memory_dir_at(&root)?)?
    } else { None };
    let set_prompt = match conversation.set_id.as_deref() {
        Some(id) => runtime::group_catalog::load_set_index(&catalog)?.sets.into_iter().find(|s| s.id == id).map(|s| s.system_prompt),
        None => None,
    };
    let project = runtime::group_catalog::load_project_index(&catalog)?.projects.into_iter().find(|p| {
        conversation.project_id.as_deref() == Some(p.id.as_str()) || (conversation.project_id.is_none() && conversation.folder.as_deref() == Some(p.name.as_str()))
    }).map(|p| prepare::ProjectPromptContext { name: p.name, root_path: p.root_path });
    let source = prepare::resolve_runtime_prompt_sources(chat_mode, &settings.chat.system_prompt, &settings.chat.chat_mode.system_prompt, &conversation.agent_plan_state);
    let names = prepare::available_builtin_tool_names(&prepared.tools);
    let ask_prompt = runtime::ask_user::format_prompt(prepared.tools.iter().any(|t| runtime::ask_user::is_ask_user_tool_name(&t.name)));
    let cwd = prepared.working_directory.to_string_lossy();
    let has_image = request.messages.iter().any(|m| m.content.iter().any(|part| matches!(part, runtime::MessagePart::Image { .. })));
    let selected_skill = conversation.active_skill_id.as_deref()
        .filter(|id| prepare::skill_allowed_for_conversation(&effective_chat_tools, conversation.assistant_snapshot.as_ref(), id))
        .and_then(|id| prepared.skill_registry.find(id))
        .map(runtime::skills::read_skill_record).transpose()?
        .map(|record| runtime::skills::SkillDetail { meta: record.meta, body: record.body });
    let build_prompt = |available: bool| prepare::build_chat_prompt(
        &language, has_image, request.options.thinking_enabled, &prepared.skill_registry,
        &effective_chat_tools, available, &names, conversation.active_skill_id.as_deref(), selected_skill.as_ref(),
        conversation.assistant_snapshot.as_ref(), set_prompt.as_deref(), &source.custom_system_prompt,
        chat_mode, memory.as_deref(), source.agent_plan_prompt.as_deref(), Some(&ask_prompt),
        project.as_ref(), Some(&cwd), None, &conversation.additional_directories,
    );
    let prompt = build_prompt(!prepared.tools.is_empty());
    let fallback = build_prompt(false);
    request.system = prompt.system;
    let mut runtime_messages = runtime::openai_messages_from_generate_request(&request);
    if !prepared.video_reports.is_empty() {
        runtime::video_analysis::apply_saved_reports(&mut runtime_messages, &prepared.video_reports, &language);
    }
    let run_tools = runtime::native_tool_runtime::NativeRunTools { executor: &executor, video: tokio::sync::Mutex::new(prepared.video_analysis) };
    let run_id = uuid::Uuid::new_v4().to_string();
    let hooks = runtime::native_hooks::dispatcher(root, &settings.chat_tools.hooks, runtime::hooks::RunContext {
        conversation_id: conversation_id.clone(), run_id: run_id.clone(), message_id: message_id.clone(), cwd: prepared.working_directory.clone(), model: format!("{}:{}", provider.id, request.model),
    });
    let web_search_mode = runtime::conversation_data::WebSearchMode::resolve(conversation.web_search_mode, &settings).for_provider(&provider);
    let mut host = NativeAgentHost { events, hooks, completed: false, workflow: prepared.workflow_hooks };
    let result = runtime::agent::run_agent_loop(AgentRunConfig {
        provider_runtime: &*model_host, conversation_id: conversation_id.clone(), tool_conversation_id: conversation_id,
        depth: 0, run_id, message_id, generation: 1, provider, model: request.model,
        runtime_messages, runtime_context: Some(prompt.context), tools: prepared.tools, blocked_tool_calls: prepared.blocked_tools,
        settings, effective_chat_tools, language, thinking_enabled: request.options.thinking_enabled,
        thinking_level: request.options.thinking_level, web_search_mode, max_output_tokens: request.options.max_tokens, retry_attempts: attempts,
        assistant_snapshot: conversation.assistant_snapshot, provider_tools_fallback_prompt: fallback,
        initial_cache_usage: None, initial_anchor_total_tokens: None, initial_anchor_trailing_estimate: 0,
        skill_registry: prepared.skill_registry, todo_state: conversation.agent_todo_state,
    }, &host, &run_tools).await;
    host.completed = true;
    let result = result?;
    Ok(ChatCompletion {
        text: result.content.clone(), reasoning: result.reasoning.clone(),
        provider_messages: runtime::model_messages_from_openai_messages(result.api_messages.clone()),
        usage: result.usage.clone(), finish_reason: None, agent_result: Some(result),
    })
}
