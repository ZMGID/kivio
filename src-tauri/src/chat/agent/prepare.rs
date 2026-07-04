use serde_json::Value;

use crate::chat::types::{ChatAssistantSnapshot, ContextUsageSegment};
use crate::mcp::ChatToolDefinition;
use crate::settings::{
    chat_no_think_instruction, default_chat_system_prompt, ChatToolsConfig, EmailAccountConfig,
};
use crate::skills;

use super::types::{AgentPhase, AgentStepResult, AgentStreamPolicy};

pub struct PrepareStepInput<'a> {
    pub step_number: u8,
    pub previous_steps: &'a [AgentStepResult],
    pub runtime_messages: &'a [Value],
    pub tools: &'a [ChatToolDefinition],
    pub phase: AgentPhase,
}

pub struct PreparedStep {
    pub active_tools: Vec<ChatToolDefinition>,
    pub runtime_messages: Vec<Value>,
    pub phase: AgentPhase,
    pub stream_policy: AgentStreamPolicy,
}

pub fn prepare_agent_step(input: PrepareStepInput<'_>) -> PreparedStep {
    let active_tools = match input.phase {
        AgentPhase::ToolLoop => input.tools.to_vec(),
        AgentPhase::Synthesis | AgentPhase::Plain => Vec::new(),
    };
    let stream_policy = match input.phase {
        AgentPhase::ToolLoop => AgentStreamPolicy::PlanningNoDoneUntilNoTools,
        AgentPhase::Synthesis | AgentPhase::Plain => AgentStreamPolicy::SynthesisAlwaysDone,
    };
    PreparedStep {
        active_tools,
        runtime_messages: input.runtime_messages.to_vec(),
        phase: input.phase,
        stream_policy,
    }
}

pub fn chat_tools_capable(
    provider: &crate::settings::ModelProvider,
    chat_tools: &ChatToolsConfig,
    memory_enabled: bool,
    image_generation_enabled: bool,
) -> bool {
    provider.supports_tools
        && (chat_tools.enabled
            || crate::settings::chat_native_tools_enabled(chat_tools)
            || memory_enabled
            || image_generation_enabled)
}

pub fn apply_active_skill_tool_filter(
    tools: &mut Vec<ChatToolDefinition>,
    skill: &skills::SkillRecord,
) {
    retain_tools_for_allowed(tools, &skill.allowed_tools);
}

/// Narrow `tools` to those a skill's `allowed_tools` permits, while always
/// keeping the skill-runtime tools and Kivio built-ins (so the model can still
/// read skill files, run skill scripts, and use core tools). An empty `allowed`
/// list is a no-op (skill declares no restriction).
///
/// This is intentionally **monotonic** (retain only, never re-expand): once a
/// model-activated skill scopes the tool set, a later activation cannot widen it
/// back. That matches the "scope tightening" semantics in the P2-B blueprint and
/// composes order-independently with Plan-mode filtering.
pub fn retain_tools_for_allowed(tools: &mut Vec<ChatToolDefinition>, allowed: &[String]) {
    if allowed.is_empty() {
        return;
    }
    tools.retain(|tool| {
        tool.source == "skill"
            || is_native_skill_tool_name(&tool.name)
            || is_kivio_builtin_tool(tool)
            || allowed
                .iter()
                .any(|recommended| tool_matches_recommended_name(tool, recommended))
    });
}

/// 按助手白名单收窄 MCP 工具：仅保留 server_id 在 `mcp_server_ids` 内的 MCP 工具。
/// 原生工具与技能工具不受影响（原生工具仍由全局聊天设置管控）。无助手快照 = 不限制。
/// 注意：空 `mcp_server_ids` 必须清空所有 MCP 工具（语义与 `retain_tools_for_allowed`
/// 的「空=不限」相反，故不能复用后者）。
pub fn apply_assistant_mcp_restrictions(
    tools: &mut Vec<ChatToolDefinition>,
    assistant_snapshot: Option<&ChatAssistantSnapshot>,
) {
    let Some(assistant) = assistant_snapshot else {
        return;
    };
    tools.retain(|tool| {
        if tool.source != "mcp" {
            return true;
        }
        match tool.server_id.as_deref() {
            Some(server_id) => assistant.mcp_server_ids.iter().any(|id| id == server_id),
            None => false,
        }
    });
}

/// 某技能在当前对话是否可用：全局已启用 **且**（无助手 = 不限；有助手 = 在其 skill_ids 白名单内）。
/// 空 skill_ids = 该助手不可用任何技能。
pub fn skill_allowed_for_conversation(
    chat_tools: &crate::settings::ChatToolsConfig,
    assistant_snapshot: Option<&ChatAssistantSnapshot>,
    skill_id: &str,
    email_accounts: &[EmailAccountConfig],
    obsidian_vault_configured: bool,
) -> bool {
    if !crate::settings::skill_globally_available(
        chat_tools,
        skill_id,
        email_accounts,
        obsidian_vault_configured,
    ) {
        return false;
    }
    match assistant_snapshot {
        Some(assistant) => assistant.skill_ids.iter().any(|id| id == skill_id),
        None => true,
    }
}

pub fn apply_skill_fallback_when_tools_unavailable(
    chat_tools: &mut ChatToolsConfig,
    active_skill_id: Option<&str>,
    tools_available: bool,
) {
    if !tools_available
        && active_skill_id
            .map(|id| !id.trim().is_empty())
            .unwrap_or(false)
        && chat_tools.skill_fallback_mode == "progressive"
    {
        chat_tools.skill_fallback_mode = "skill_md_only".to_string();
    }
}

pub fn available_builtin_tool_names(tools: &[ChatToolDefinition]) -> Vec<String> {
    let mut names = tools
        .iter()
        .filter(|tool| is_kivio_builtin_tool(tool))
        .map(|tool| tool.name.clone())
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

pub fn disabled_builtin_tool_feedback(function_name: &str) -> Option<String> {
    // Builtin name set = static native registry (17 native + todo/ask_user)
    // plus the non-native builtin sources listed here.
    const EXTRA_BUILTIN_NAMES: &[&str] = &["mixer_generate_image"];
    // 模型按 wire 名（保留名别名）调用——反查回内部名再比对注册表。
    let function_name = crate::mcp::types::resolve_reserved_wire_alias(function_name);
    let is_builtin = crate::mcp::native_registry::find_entry(function_name).is_some()
        || EXTRA_BUILTIN_NAMES.contains(&function_name);
    if is_builtin {
        Some(format!(
            "Kivio tool `{function_name}` is not enabled for this chat. Do not call it again; answer using the available context and enabled tools only."
        ))
    } else {
        None
    }
}

pub fn is_native_skill_tool_name(name: &str) -> bool {
    matches!(
        name,
        "skill_activate" | "skill_read_file" | "skill_run_script"
    )
}

pub fn is_kivio_builtin_tool(tool: &ChatToolDefinition) -> bool {
    matches!(tool.source.as_str(), "native" | "mixer")
        && !is_native_skill_tool_name(&tool.name)
        && !crate::chat::todo::is_agent_todo_tool_name(&tool.name)
}

pub fn builtin_tool_bypasses_approval(tool: &ChatToolDefinition) -> bool {
    if tool.source == "skill" && is_native_skill_tool_name(&tool.name) {
        return true;
    }
    tool.source == "native"
        && crate::mcp::native_registry::find_entry(&tool.name)
            .is_some_and(|entry| entry.bypasses_approval)
}

/// True for the native file/shell tools gated by one-time per-conversation
/// session consent (read/write/edit/bash/grep/glob). See
/// `native_registry::native_tool_requires_session_consent`.
pub fn tool_requires_session_consent(tool: &ChatToolDefinition) -> bool {
    tool.source == "native"
        && crate::mcp::native_registry::native_tool_requires_session_consent(&tool.name)
}

pub fn build_chat_system_prompt(
    language: &str,
    has_image: bool,
    thinking_enabled: bool,
    registry: &skills::SkillRegistry,
    chat_tools: &ChatToolsConfig,
    tools_available: bool,
    available_builtin_tools: &[String],
    active_skill_id: Option<&str>,
    active_skill_detail: Option<&skills::SkillDetail>,
    assistant_snapshot: Option<&ChatAssistantSnapshot>,
    set_system_prompt: Option<&str>,
    custom_system_prompt: &str,
    memory_prompt: Option<&str>,
    agent_plan_prompt: Option<&str>,
    agent_ask_user_prompt: Option<&str>,
    agent_todo_prompt: Option<&str>,
    project_context: Option<&ProjectPromptContext>,
    delivery_dir: Option<&str>,
    obsidian_vault_path: Option<&str>,
    email_accounts: &[EmailAccountConfig],
    email_accounts_prompt: Option<&str>,
) -> String {
    build_chat_system_prompt_with_segments(
        language,
        has_image,
        thinking_enabled,
        registry,
        chat_tools,
        tools_available,
        available_builtin_tools,
        active_skill_id,
        active_skill_detail,
        assistant_snapshot,
        set_system_prompt,
        custom_system_prompt,
        memory_prompt,
        agent_plan_prompt,
        agent_ask_user_prompt,
        agent_todo_prompt,
        project_context,
        delivery_dir,
        None,
        obsidian_vault_path,
        email_accounts,
        email_accounts_prompt,
    )
    .0
}

/// Project binding facts injected into the system prompt so the model knows
/// the default path base before generating file tool arguments.
#[derive(Debug, Clone)]
pub struct ProjectPromptContext {
    pub name: String,
    pub root_path: Option<String>,
}

fn project_context_prompt(project: &ProjectPromptContext, language: &str) -> String {
    match (&project.root_path, language.starts_with("zh")) {
        (Some(root), true) => format!(
            "当前是项目对话，项目「{}」已绑定文件夹：{root}。文件/命令工具的相对路径以该目录为根；写入明确的绝对路径或 ~/ 路径（如 ~/Desktop/x.html）会写到项目外的全局位置。",
            project.name
        ),
        (Some(root), false) => format!(
            "This is a project conversation. Project \"{}\" is bound to folder: {root}. Relative paths in file/command tools resolve from that root; writing an explicit absolute or ~/ path (e.g. ~/Desktop/x.html) targets that global location outside the project.",
            project.name
        ),
        (None, true) => format!(
            "当前是项目对话，但项目「{}」尚未绑定本地文件夹；文件/命令工具不可用，需要用户先在项目菜单中绑定文件夹。",
            project.name
        ),
        (None, false) => format!(
            "This is a project conversation, but project \"{}\" has no bound folder; file/command tools are unavailable until the user binds one from the project menu.",
            project.name
        ),
    }
}

pub fn build_chat_system_prompt_with_segments(
    language: &str,
    has_image: bool,
    thinking_enabled: bool,
    registry: &skills::SkillRegistry,
    chat_tools: &ChatToolsConfig,
    tools_available: bool,
    available_builtin_tools: &[String],
    active_skill_id: Option<&str>,
    active_skill_detail: Option<&skills::SkillDetail>,
    assistant_snapshot: Option<&ChatAssistantSnapshot>,
    set_system_prompt: Option<&str>,
    custom_system_prompt: &str,
    memory_prompt: Option<&str>,
    agent_plan_prompt: Option<&str>,
    agent_ask_user_prompt: Option<&str>,
    agent_todo_prompt: Option<&str>,
    project_context: Option<&ProjectPromptContext>,
    delivery_dir: Option<&str>,
    knowledge_base_prompt: Option<&str>,
    obsidian_vault_path: Option<&str>,
    email_accounts: &[EmailAccountConfig],
    email_accounts_prompt: Option<&str>,
) -> (String, Vec<ContextUsageSegment>) {
    let mut prompt = String::new();
    let mut segments = Vec::new();
    let base_prompt = if custom_system_prompt.trim().is_empty() {
        default_chat_system_prompt(language, has_image)
    } else {
        custom_system_prompt.trim().to_string()
    };
    append_context_segment(
        &mut prompt,
        &mut segments,
        "system_prompt",
        "System prompt",
        &base_prompt,
    );
    if let Some(assistant) = assistant_snapshot {
        let assistant_prompt = assistant_prompt_segment(assistant);
        if !assistant_prompt.trim().is_empty() {
            append_context_segment(
                &mut prompt,
                &mut segments,
                "assistant",
                "Assistant",
                &assistant_prompt,
            );
        }
    }
    // 集的系统提示词：实时注入（不冻结），随集编辑对集内所有对话立即生效。作为独立段落，
    // 与助手段并存（助手段提供人设/工具白名单，集段是这一组对话的统一指令）。
    if let Some(set_prompt) = set_system_prompt {
        let set_prompt = set_prompt.trim();
        if !set_prompt.is_empty() {
            append_context_segment(
                &mut prompt,
                &mut segments,
                "set",
                "Set instructions",
                set_prompt,
            );
        }
    }
    append_context_segment(
        &mut prompt,
        &mut segments,
        "runtime_context",
        "Runtime context",
        &crate::settings::chat_current_datetime_context(language),
    );

    if let Some(project) = project_context {
        append_context_segment(
            &mut prompt,
            &mut segments,
            "runtime_context",
            "Runtime context",
            &project_context_prompt(project, language),
        );
    }

    if let Some(memory) = memory_prompt
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        append_context_segment(
            &mut prompt,
            &mut segments,
            "memory_l1",
            "Memory / L1",
            memory,
        );
    }

    if let Some(kb) = knowledge_base_prompt
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        append_context_segment(
            &mut prompt,
            &mut segments,
            "knowledge_base",
            "Knowledge base",
            kb,
        );
    }

    if let Some(path) = obsidian_vault_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let text = if language.starts_with("zh") {
            format!(
                "Obsidian 笔记库路径：{path}\n\
                 这是一个本地 Obsidian markdown 笔记库，请用原生文件工具操作：\
                 list_dir 浏览目录（条目含修改时间，可找最近修改的笔记）、\
                 glob_files 按名找 *.md、search_files 按内容/关键词搜索、read_file 读笔记；\
                 笔记之间用 [[wikilink]] 双链互相引用。\n\
                 需要 Obsidian 语法或格式细节时，可激活 obsidian-markdown / obsidian-bases / \
                 json-canvas / obsidian-cli skill。"
            )
        } else {
            format!(
                "Obsidian vault path: {path}\n\
                 This is a local Obsidian markdown vault. Use the native file tools: \
                 list_dir to browse (entries include modified time), glob_files to find *.md by name, \
                 search_files to search by content/keyword, read_file to read a note; \
                 notes cross-reference each other via [[wikilink]].\n\
                 For Obsidian syntax or file-format details, activate the obsidian-markdown / \
                 obsidian-bases / json-canvas / obsidian-cli skills."
            )
        };
        append_context_segment(
            &mut prompt,
            &mut segments,
            "runtime_context",
            "Runtime context",
            &text,
        );
    }

    if let Some(text) = email_accounts_prompt
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        append_context_segment(
            &mut prompt,
            &mut segments,
            "runtime_context",
            "Runtime context",
            text,
        );
    }

    if let Some(plan) = agent_plan_prompt
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        append_context_segment(&mut prompt, &mut segments, "agent_plan", "Agent plan", plan);
    }

    if let Some(ask_user) = agent_ask_user_prompt
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        append_context_segment(
            &mut prompt,
            &mut segments,
            "agent_ask_user",
            "Agent ask_user",
            ask_user,
        );
    }

    if let Some(todo) = agent_todo_prompt
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        append_context_segment(&mut prompt, &mut segments, "agent_todo", "Agent todo", todo);
    }

    if tools_available {
        let mut action_examples = Vec::new();
        if available_builtin_tools
            .iter()
            .any(|tool| tool.as_str() == crate::chat::ask_user::ASK_USER_TOOL_NAME)
        {
            action_examples.push("asking the user a blocking clarification");
        }
        if available_builtin_tools.iter().any(|tool| {
            matches!(tool.as_str(), "read" | "grep" | "glob")
        }) {
            action_examples.push("reading or searching project files");
        }
        if available_builtin_tools
            .iter()
            .any(|tool| matches!(tool.as_str(), "bash" | "run_python"))
        {
            action_examples.push("running code or a command");
        }
        if available_builtin_tools
            .iter()
            .any(|tool| matches!(tool.as_str(), "web_search" | "web_fetch"))
        {
            action_examples.push("using the web");
        }
        if available_builtin_tools
            .iter()
            .any(|tool| tool.as_str() == "mixer_generate_image")
        {
            action_examples.push("generating an image");
        }
        if action_examples.is_empty() {
            action_examples.push("using an enabled tool");
        }
        let mut runtime = format!(
            "You have access to tools (functions). When the user's request requires action—such as {}—YOU MUST call the appropriate enabled tool instead of describing what to do. Never say \"I cannot run commands\" or \"you can do it yourself\" when an enabled tool is available for that action. Do not call tools that are not listed as enabled.",
            action_examples.join(", ")
        );
        runtime.push_str(
            " Only claim that a tool was used, a script was run, a file was read, or the web was searched after Kivio returns an actual tool result in the conversation.",
        );
        if language.starts_with("zh") {
            runtime.push_str(
                " 若用户只问今天/明天/星期几等可由上文「当前日期」直接推算的日期问题，直接回答，不要调用工具。",
            );
        } else {
            runtime.push_str(
                " If the user only asks for today/tomorrow/weekday derivable from the system date above, answer directly without calling tools.",
            );
        }
        append_context_segment(
            &mut prompt,
            &mut segments,
            "runtime_context",
            "Runtime context",
            &runtime,
        );
        if let Some(native_prompt) =
            native_tools_prompt(available_builtin_tools, language, delivery_dir)
        {
            append_context_segment(
                &mut prompt,
                &mut segments,
                "native_tools",
                "Native tools",
                &native_prompt,
            );
        }
        // Sub-agent delegation rules — only when the `agent` spawn tool is
        // available. The `agent` call is BLOCKING + single-result (Claude Code
        // Task model); to run sub-agents in parallel, emit MULTIPLE `agent` calls
        // in ONE message — they execute concurrently and each returns its result.
        // No polling/collection tool exists. Concise on purpose.
        if available_builtin_tools
            .iter()
            .any(|tool| tool.as_str() == crate::chat::sub_agent::AGENT_TOOL_NAME)
        {
            let background_prompt = if language.starts_with("zh") {
                "委派子 agent：每个 agent 调用都会阻塞、等子 agent 跑完并直接返回完整结果。要并行处理多个互相独立的子任务，就在同一条消息里发出多个 agent 调用——它们会并发执行，各自返回自己的结果。没有任何轮询或收集工具，也不要去找。"
            } else {
                "Delegating to sub-agents: each agent call BLOCKS, waits for the sub-agent to finish, and returns its full result directly. To run sub-agents in PARALLEL, emit MULTIPLE agent tool calls in a SINGLE message — they execute concurrently and each returns its own result. There is no polling or collection tool; do not look for one."
            };
            append_context_segment(
                &mut prompt,
                &mut segments,
                "native_tools",
                "Native tools",
                background_prompt,
            );
        }
    }

    let include_catalog = chat_tools.skill_auto_match
        || active_skill_id.is_some()
        || chat_tools.skill_fallback_mode != "legacy_full_body";
    if include_catalog {
        let obsidian_vault_configured = obsidian_vault_path
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
        let catalog = skills::format_catalog(registry, active_skill_id, tools_available, |skill_id| {
            skill_allowed_for_conversation(
                chat_tools,
                assistant_snapshot,
                skill_id,
                email_accounts,
                obsidian_vault_configured,
            )
        });
        if !catalog.is_empty() {
            append_context_segment(&mut prompt, &mut segments, "skills", "Skills", &catalog);
        }
    }

    if !chat_tools.skill_auto_match {
        append_context_segment(
            &mut prompt,
            &mut segments,
            "skills",
            "Skills",
            "Only activate skills that are enabled in Settings (listed in the catalog below).",
        );
    }

    let fallback = chat_tools.skill_fallback_mode.as_str();
    if let Some(skill_id) = active_skill_id.filter(|id| !id.trim().is_empty()) {
        let mut skill_prompt = format!("User pinned skill for this message: {skill_id}");
        if tools_available {
            skill_prompt.push_str(
                ". Activate it with skill_activate to load its full instructions for this message.",
            );
        } else if matches!(fallback, "skill_md_only" | "legacy_full_body") {
            skill_prompt.push_str(". Follow the Active Skill instructions below.");
        } else {
            skill_prompt.push_str(
                ". Progressive skill loading requires tool support; switch provider or set fallback to SKILL.md only.",
            );
        }
        append_context_segment(
            &mut prompt,
            &mut segments,
            "skills",
            "Skills",
            &skill_prompt,
        );
    } else if tools_available && chat_tools.skill_auto_match {
        if language.starts_with("zh") {
            append_context_segment(
                &mut prompt,
                &mut segments,
                "skills",
                "Skills",
                "当任务匹配某个 Skill 的描述时，主动用 skill_activate 激活它——无需用户点名，描述对得上就激活。激活后会加载该 Skill 的完整步骤指令，效果明显优于自行发挥。只跳过描述明显与当前任务无关的 Skill。",
            );
        } else {
            append_context_segment(
                &mut prompt,
                &mut segments,
                "skills",
                "Skills",
                "When the task matches a skill's description, call skill_activate for it proactively — you don't need the user to name it; a description match is enough. Activating loads that skill's full step-by-step instructions, which beat improvising. Only skip a skill whose description clearly doesn't fit the current task.",
            );
        }
    }

    if matches!(fallback, "skill_md_only" | "legacy_full_body") {
        if let Some(skill) = active_skill_detail {
            if !skill.body.trim().is_empty() {
                append_context_segment(
                    &mut prompt,
                    &mut segments,
                    "skills",
                    "Skills",
                    &format!("Active Skill:\n{}", skill.body),
                );
            }
        }
    }

    if !thinking_enabled && !tools_available {
        append_context_segment(
            &mut prompt,
            &mut segments,
            "runtime_context",
            "Runtime context",
            chat_no_think_instruction(language),
        );
    }
    (prompt, merge_context_segments(segments))
}

fn append_context_segment(
    prompt: &mut String,
    segments: &mut Vec<ContextUsageSegment>,
    id: &str,
    label: &str,
    content: &str,
) {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return;
    }
    if !prompt.is_empty() {
        prompt.push_str("\n\n");
    }
    prompt.push_str(trimmed);
    segments.push(ContextUsageSegment {
        id: id.to_string(),
        label: label.to_string(),
        estimated_tokens: estimate_tokens(trimmed),
        color: context_segment_color(id).map(str::to_string),
    });
}

fn assistant_prompt_segment(assistant: &ChatAssistantSnapshot) -> String {
    let mut parts = vec![format!("Active assistant: {}", assistant.name)];
    if !assistant.description.trim().is_empty() {
        parts.push(format!("Assistant purpose: {}", assistant.description.trim()));
    }
    let assistant_system_prompt = assistant.system_prompt.trim();
    if !assistant_system_prompt.is_empty() {
        parts.push(format!("Assistant instructions:\n{assistant_system_prompt}"));
    }
    parts.join("\n\n")
}

pub fn merge_context_segments(segments: Vec<ContextUsageSegment>) -> Vec<ContextUsageSegment> {
    let mut merged: Vec<ContextUsageSegment> = Vec::new();
    for segment in segments {
        if segment.estimated_tokens == 0 {
            continue;
        }
        if let Some(existing) = merged.iter_mut().find(|item| item.id == segment.id) {
            existing.estimated_tokens += segment.estimated_tokens;
        } else {
            merged.push(segment);
        }
    }
    merged
}

pub fn context_segment_color(id: &str) -> Option<&'static str> {
    match id {
        "system_prompt" => Some("#7A7A7A"),
        "assistant" => Some("#8A6FBD"),
        "set" => Some("#5C9A8B"),
        "runtime_context" => Some("#3E8B60"),
        "memory_l1" => Some("#4F9A9A"),
        "agent_plan" => Some("#8A724C"),
        "agent_todo" => Some("#5F7C5A"),
        "tool_definitions" => Some("#7553CF"),
        "skills" => Some("#BD8A3E"),
        "mcp" => Some("#B04B8D"),
        "native_tools" => Some("#4E7FB8"),
        "summarized_conversation" => Some("#BF3F66"),
        "conversation" => Some("#D07652"),
        "attachments" => Some("#6A8FBD"),
        _ => None,
    }
}

pub fn estimate_tokens(text: &str) -> usize {
    let mut ascii = 0usize;
    let mut non_ascii = 0usize;
    for ch in text.chars() {
        if ch.is_ascii() {
            ascii += 1;
        } else {
            non_ascii += 1;
        }
    }
    ascii.div_ceil(4) + non_ascii
}

pub(crate) fn tool_matches_recommended_name(tool: &ChatToolDefinition, recommended: &str) -> bool {
    let recommended = recommended.trim();
    if recommended.is_empty() {
        return false;
    }
    // 旧名归一化：persona/skill 白名单里写的旧工具名（find/ls/todo_update/list_background）
    // 规整到现名，避免改名后被静默剔除。
    let recommended = crate::mcp::types::canonical_tool_name(recommended);
    tool.name == recommended
        || tool.id == recommended
        || tool.openai_tool_name() == recommended
        || tool
            .server_id
            .as_deref()
            .map(|server_id| format!("{server_id}:{}", tool.name) == recommended)
            .unwrap_or(false)
}

fn native_tools_prompt(
    available_builtin_tools: &[String],
    language: &str,
    delivery_dir: Option<&str>,
) -> Option<String> {
    let native_tool_names = available_builtin_tools
        .iter()
        .filter(|tool| tool.as_str() != crate::chat::ask_user::ASK_USER_TOOL_NAME)
        .cloned()
        .collect::<Vec<_>>();
    if native_tool_names.is_empty() {
        return None;
    }
    // 提示词展示 wire 名（保留名规避后的别名）：模型必须按请求里声明的函数名调用，
    // 提示词与 tools 声明不一致会诱发未知工具调用。逻辑判断仍用内部名。
    let list = native_tool_names
        .iter()
        .map(|name| crate::mcp::types::apply_reserved_wire_alias(name))
        .collect::<Vec<_>>()
        .join(", ");
    // 运行时取值,让同一份 prompt 在不同平台都说真话(run_command 的 shell 是编译期 cfg 选的)。
    let (os_name, shell_name) = if cfg!(target_os = "windows") {
        ("Windows", "PowerShell")
    } else if cfg!(target_os = "macos") {
        ("macOS", "sh")
    } else {
        ("Linux", "sh")
    };
    let has_web_search = native_tool_names
        .iter()
        .any(|tool| tool.as_str() == "web_search");
    let has_web_fetch = native_tool_names
        .iter()
        .any(|tool| tool.as_str() == "web_fetch");
    let has_image_generation = native_tool_names
        .iter()
        .any(|tool| tool.as_str() == "mixer_generate_image");
    let has_run_python = native_tool_names
        .iter()
        .any(|tool| tool.as_str() == "run_python");
    let has_write = native_tool_names
        .iter()
        .any(|tool| tool.as_str() == "write");
    // The delivery directory is surfaced (with its absolute path) only when the
    // write tool is available — that's the channel the model writes deliverables
    // into. Without write, there is no plain-file delivery path to mention.
    let delivery_dir = delivery_dir
        .map(str::trim)
        .filter(|dir| !dir.is_empty() && has_write);
    let zh_live_access_hint = match (has_web_search, has_web_fetch) {
        (true, true) => "实时搜索或网页读取必须优先用 search_web/web_fetch 或对应 Skill 脚本。",
        (true, false) => "实时搜索必须优先用 search_web 或对应 Skill 脚本。",
        (false, true) => "网页读取必须优先用 web_fetch 或对应 Skill 脚本。",
        (false, false) => "需要联网/API 访问时，请启用对应联网工具或使用对应 Skill 脚本。",
    };
    let en_live_access_hint = match (has_web_search, has_web_fetch) {
        (true, true) => {
            "Use search_web/web_fetch or the relevant Skill script for live web/API access."
        }
        (true, false) => "Use search_web or the relevant Skill script for live web/API access.",
        (false, true) => "Use web_fetch or the relevant Skill script for web page access.",
        (false, false) => {
            "Enable the relevant web tool or use the relevant Skill script for live web/API access."
        }
    };
    let mut prompt = if language.starts_with("zh") {
        let image_generation_hint = if has_image_generation {
            "\n- 用户要求创建、生成、绘制图片时，必须调用 mixer_generate_image，不要只用文字描述。"
        } else {
            "\n- 生图工具未启用；用户要求生成图片时，说明需要先在「混音器」里配置生图模型。"
        };
        let generated_file_hint = match (delivery_dir, has_run_python) {
            (Some(dir), true) => format!(
                "\n- 文件交付分三种,按目标选:给用户的成品文件(报告/数据/代码/CSV/JSON/MD/HTML 等)→ 用 write 写到交付目录 `{dir}`,会自动显示可下载的文件卡片;修改用户项目里的文件 → write 到项目路径,不显示卡片;需要计算/数据分析/画图/库生成(如带格式 XLSX、PDF、渲染图)→ run_python,产物会自动落到交付目录并显示卡片。不要仅仅为了把已有内容写成文件而调用 run_python。"
            ),
            (Some(dir), false) => format!(
                "\n- 文件交付分两种,按目标选:给用户的成品文件(报告/数据/代码/CSV/JSON/MD/HTML 等)→ 用 write 写到交付目录 `{dir}`,会自动显示可下载的文件卡片;修改用户项目里的文件 → write 到项目路径,不显示卡片。"
            ),
            (None, true) => "\n- 需要计算/数据分析/画图/库生成(如带格式 XLSX、PDF、渲染图)的可下载文件用 run_python(产物以文件卡片交付);修改用户项目工作区里的文件用 write/edit。不要仅仅为了把已有内容写成文件而调用 run_python。".to_string(),
            (None, false) => String::new(),
        };
        format!(
            "内置工具（已启用）：{list}。只能调用此列表中的内置工具。\n\
- 项目对话中文件/命令工具的相对路径以项目根目录为根；写入明确的绝对路径或 ~/ 路径（如 ~/Desktop/x.html）会落到项目外的全局位置。非项目对话用绝对路径或 ~/ 路径。\n\
- 用户明确要求保存/修改/删除本地文件或给出目标路径时才动文件：小改用 edit，新建或整文件覆盖用 write。只要求“生成代码块”时直接在回答里输出，不调用 write。写入成功后简短说明路径即可，不要复述文件内容。\n\
- 写入/编辑类工具和 bash 可能需要用户确认；memory_read（按需读 L2，L1 已注入）、memory_search（按关键词检索 L2，找不准标题时优先用它）和 memory_modify 无需确认。\n\
- 运行环境：{os_name}，bash 经 {shell_name} 执行；命令语法须匹配该 shell（Windows 是 PowerShell：用全名 cmdlet 如 `Get-ChildItem`/`Get-Content`、环境变量写 `$env:VAR`、多条命令用 `;` 串联，别用已废弃的 `wmic`，也别对整盘做 `-Recurse` 递归扫描；Unix 用 `$VAR`、`ls`、`/`）。每次 bash 都是全新进程，cwd 不跨调用保留——切目录用 `cwd` 参数，别靠上一条 `cd`。要跑多行或带引号的代码，先用 write 写成脚本再执行，或用 run_python，别塞进 `python -c \"...\"` 这类内联命令（内联引号在各 shell 下都脆弱）。工具返回硬性拒绝时换策略，别把同一动作换几种写法反复试；失败命令不要原样重跑；别为一次性探测或清理往项目里扔临时脚本。\n\
- bash 在宿主 shell 从项目根目录执行，非零退出码即失败；含空格的路径必须用 `cwd` 参数，禁止 `cd 路径 && 命令`；不要同时传 `cwd` 又在 command 里写 `cd ... &&`。`npm run dev` / `tauri dev` / `vite` 等长驻 dev 命令会自动后台启动并立刻返回 pid，不要重复启动。破坏性、联网、改环境的命令先说明并等确认。Skill 脚本走 skill_run_script；不要用 pip 装宿主包绕过沙盒。\n\
- run_python 在 Pyodide 沙盒运行，用于数据运算、分析、文档处理、图表，以及需要 Python 库才能产出的文件（带格式 XLSX、PDF、渲染图）；不要用它生成或打印代码答案，也不要仅为把已有内容写成文件而调用它（那用 write 写到交付目录）。代码直接写在回答里。无宿主文件系统访问；files 挂载本地文件后用 KIVIO_INPUT_FILES[n] 路径，numpy、pandas、matplotlib、pillow、openpyxl、pypdf 可直接 import。产物保存为相对路径文件名（如 report.xlsx、chart.png、summary.csv），应用会自动捕获并显示文件卡片；不要 print base64。\n\
- {zh_live_access_hint}"
        ) + &generated_file_hint + image_generation_hint
    } else {
        let image_generation_hint = if has_image_generation {
            "\n- When the user asks to create, generate, or draw an image, call mixer_generate_image; do not merely describe it."
        } else {
            "\n- Image generation is not enabled; if asked, explain that an image model must be configured under Mixer first."
        };
        let generated_file_hint = match (delivery_dir, has_run_python) {
            (Some(dir), true) => format!(
                "\n- File delivery has three modes — pick by intent: a finished file FOR THE USER (report/data/code/CSV/JSON/MD/HTML, etc.) → write it into the delivery directory `{dir}` (it automatically shows a downloadable file card); editing a file in the user's project → write to the project path (no card); content that needs computation, data analysis, charts/plots, or a Python library to generate (e.g. a formatted XLSX, PDF, or rendered image) → run_python (its artifacts land in the delivery directory automatically and show a card). Do not call run_python merely to write out content you already have."
            ),
            (Some(dir), false) => format!(
                "\n- File delivery has two modes — pick by intent: a finished file FOR THE USER (report/data/code/CSV/JSON/MD/HTML, etc.) → write it into the delivery directory `{dir}` (it automatically shows a downloadable file card); editing a file in the user's project → write to the project path (no card)."
            ),
            (None, true) => "\n- Use run_python for downloadable files that need computation, data analysis, charts/plots, or a Python library to generate (e.g. a formatted XLSX, PDF, or rendered image); its output is delivered as a file card. To edit files in the user's project/workspace, use write/edit. Do not call run_python merely to write out content you already have.".to_string(),
            (None, false) => String::new(),
        };
        format!(
            "Built-in tools enabled: {list}. Only call tools in this list.\n\
- In project conversations, relative paths in file/command tools resolve from the project root; writing an explicit absolute or ~/ path (e.g. ~/Desktop/x.html) targets that global location outside the project. Non-project conversations use absolute or ~/ paths.\n\
- Touch files only when the user explicitly asks to save/modify/delete local files or gives a target path: edit for small edits, write for new files or whole-file overwrites. If asked for a code block without saving, answer inline. After a write, state the path briefly; do not repeat the file content.\n\
- Write/edit tools and bash may need user approval; memory_read (L2 on demand; L1 is auto-injected), memory_search (keyword search over L2; prefer it when you are unsure of the exact heading), and memory_modify do not.\n\
- Runtime environment: {os_name}; bash runs via {shell_name}. Match that shell's syntax (Windows is PowerShell: use full cmdlet names like `Get-ChildItem`/`Get-Content`, environment variables as `$env:VAR`, chain commands with `;`, do NOT use the removed `wmic`, and do NOT `-Recurse` the whole drive; Unix: `$VAR`, `ls`, `/`). Each bash call is a fresh process — cwd does NOT persist across calls; switch directories with the `cwd` parameter, not a prior `cd`. To run multi-line or quoted code, write it to a file with write and run that, or use run_python — do not cram it into inline commands like `python -c \"...\"` (inline quotes are fragile across shells). When a tool returns a hard rejection, change strategy instead of retrying variants of the same action; never re-run a failed command unchanged; don't drop one-off probe or cleanup scripts into the project.\n\
- bash runs on the host shell from the project root; non-zero exit means failure. Paths with spaces must use the `cwd` parameter—never `cd path && command`; do not combine `cwd` with a leading `cd ... &&` prefix. Long-running dev commands such as `npm run dev`, `tauri dev`, and `vite` start in the background automatically and return a job_id immediately; do not start the same dev server twice. Explain and get confirmation before destructive, network, or environment-changing commands. Skill scripts go through skill_run_script; never use host pip to bypass the run_python sandbox.\n\
- Background commands (bash with background:true, or auto-detected dev servers): the call returns a job_id immediately and hands control back to you — keep working, do NOT poll right away. Read incremental output and exit status with bash_output (pass the job_id; use the returned next_offset for the next read), list all tracked jobs by calling bash_output with no job_id, and stop one with kill_background. Keep polling bounded (≤20 checks); status in history may be stale, so refresh once with bash_output before reporting a background command's result. Background commands survive across turns until you kill them or the app exits, so kill_background a dev server when you no longer need it.\n\
- run_python runs in a Pyodide sandbox for data computation, analysis, document processing, charts, and generating files that REQUIRE a Python library (formatted XLSX, PDF, rendered images); never use it to generate or print code answers, and do not call it merely to write out content you already have (use write into the delivery directory for that). Write code directly in the answer. No host filesystem access; mount files via the files parameter and use KIVIO_INPUT_FILES[n] paths. numpy, pandas, matplotlib, pillow, openpyxl, pypdf import directly. Save artifacts to relative filenames (report.xlsx, chart.png, summary.csv); Kivio auto-captures them and shows file cards. No base64 printing.\n\
- {en_live_access_hint}"
        ) + &generated_file_hint + image_generation_hint
    };
    if has_image_generation && !prompt.ends_with('.') && !prompt.ends_with('。') {
        prompt.push('.');
    }
    Some(prompt)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_assistant_snapshot(
        mcp_server_ids: Vec<&str>,
        skill_ids: Vec<&str>,
    ) -> ChatAssistantSnapshot {
        ChatAssistantSnapshot {
            id: "asst_test".to_string(),
            name: "Test Assistant".to_string(),
            description: String::new(),
            source: "user".to_string(),
            system_prompt: String::new(),
            provider_id: String::new(),
            model: String::new(),
            mcp_server_ids: mcp_server_ids.into_iter().map(str::to_string).collect(),
            skill_ids: skill_ids.into_iter().map(str::to_string).collect(),
        }
    }

    fn test_mcp_tool() -> ChatToolDefinition {
        ChatToolDefinition {
            id: "mcp__demo__search".to_string(),
            name: "search".to_string(),
            description: "Search demo".to_string(),
            source: "mcp".to_string(),
            server_id: Some("demo".to_string()),
            server_name: Some("Demo".to_string()),
            input_schema: serde_json::json!({ "type": "object", "properties": {} }),
            sensitive: false,
            annotations: None,
            output_schema: None,
        }
    }

    #[test]
    fn is_native_skill_tool_name_matches_runtime_tools() {
        assert!(is_native_skill_tool_name("skill_activate"));
        assert!(is_native_skill_tool_name("skill_run_script"));
        assert!(!is_native_skill_tool_name("web_search"));
    }

    #[test]
    fn retain_tools_for_allowed_keeps_skill_and_builtins() {
        let mut tools = vec![
            crate::mcp::types::native_skill_activate_tool(),
            crate::mcp::types::native_run_python_tool(),
            crate::mcp::types::native_web_fetch_tool(),
            test_mcp_tool(),
        ];

        // Allow only web_fetch among non-builtin/skill tools.
        retain_tools_for_allowed(&mut tools, &["web_fetch".to_string()]);

        // skill_activate (skill runtime) and run_python / web_fetch (Kivio
        // built-ins) are always kept; the MCP "search" tool is dropped because
        // it is not skill/builtin and not in the allowed list.
        assert!(tools.iter().any(|tool| tool.name == "skill_activate"));
        assert!(tools.iter().any(|tool| tool.name == "run_python"));
        assert!(tools.iter().any(|tool| tool.name == "web_fetch"));
        assert!(!tools.iter().any(|tool| tool.name == "search"));
    }

    #[test]
    fn retain_tools_for_allowed_noop_when_empty() {
        let mut tools = vec![
            crate::mcp::types::native_skill_activate_tool(),
            test_mcp_tool(),
        ];
        let before = tools.len();
        retain_tools_for_allowed(&mut tools, &[]);
        assert_eq!(tools.len(), before);
    }

    #[test]
    fn chat_prompt_omits_disabled_web_tools() {
        let registry = skills::SkillRegistry::default();
        let mut chat_tools = crate::settings::ChatToolsConfig::default();
        chat_tools.native_tools.skill_runtime = true;
        chat_tools.native_tools.run_python = true;
        chat_tools.native_tools.web_search = false;
        chat_tools.native_tools.web_fetch = false;

        let prompt = build_chat_system_prompt(
            "zh-CN",
            false,
            false,
            &registry,
            &chat_tools,
            true,
            &["run_python".to_string()],
            None,
            None,
            None,
            None,
            "",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            None,
        );

        assert!(prompt.contains("run_python"));
        assert!(!prompt.contains("web_search"));
        assert!(!prompt.contains("web_fetch"));
    }

    #[test]
    fn chat_prompt_scopes_run_python_to_compute_deliverables() {
        let registry = skills::SkillRegistry::default();
        let mut chat_tools = crate::settings::ChatToolsConfig::default();
        chat_tools.native_tools.run_python = true;

        let prompt = build_chat_system_prompt(
            "zh-CN",
            false,
            false,
            &registry,
            &chat_tools,
            true,
            &["run_python".to_string()],
            None,
            None,
            None,
            None,
            "",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            None,
        );

        // run_python (no delivery dir) → only-run_python arm: scope it to
        // compute/library deliverables and explicitly discourage using it just
        // to write out existing content.
        assert!(prompt.contains("run_python"));
        assert!(prompt.contains("文件卡片"));
        assert!(prompt.contains("report.xlsx"));
        assert!(prompt.contains("不要仅仅为了把已有内容写成文件而调用 run_python"));
        // The old proactive-catch-all wording must be gone.
        assert!(!prompt.contains("主动调用 run_python"));
    }

    #[test]
    fn chat_prompt_offers_three_way_split_with_delivery_dir() {
        let registry = skills::SkillRegistry::default();
        let mut chat_tools = crate::settings::ChatToolsConfig::default();
        chat_tools.native_tools.run_python = true;
        chat_tools.native_tools.write_file = true;

        let prompt = build_chat_system_prompt(
            "zh-CN",
            false,
            false,
            &registry,
            &chat_tools,
            true,
            &[
                "run_python".to_string(),
                "write".to_string(),
            ],
            None,
            None,
            None,
            None,
            "",
            None,
            None,
            None,
            None,
            None,
            Some("/Users/me/Kivio/outputs/conv_abc"),
            None,
            &[],
            None,
        );

        // Delivery dir + run_python + write → three-way split: the delivery dir
        // absolute path is surfaced, all three routes are mentioned, and the
        // run_python catch-all guard remains.
        assert!(prompt.contains("/Users/me/Kivio/outputs/conv_abc"));
        assert!(prompt.contains("交付目录"));
        assert!(prompt.contains("run_python"));
        assert!(prompt.contains("不要仅仅为了把已有内容写成文件而调用 run_python"));
        // The removed deliver_file tool must not appear anywhere.
        assert!(!prompt.contains("deliver_file"));
    }

    #[test]
    fn chat_prompt_prevents_write_file_for_inline_code_requests() {
        let registry = skills::SkillRegistry::default();
        let mut chat_tools = crate::settings::ChatToolsConfig::default();
        chat_tools.native_tools.write_file = true;

        let prompt = build_chat_system_prompt(
            "zh-CN",
            false,
            false,
            &registry,
            &chat_tools,
            true,
            &["write".to_string()],
            None,
            None,
            None,
            None,
            "",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            None,
        );

        assert!(prompt.contains("生成代码块"));
        assert!(prompt.contains("不调用 write"));
        assert!(prompt.contains("不要复述文件内容"));
    }

    #[test]
    fn chat_prompt_includes_obsidian_vault_path() {
        let registry = skills::SkillRegistry::default();
        let chat_tools = crate::settings::ChatToolsConfig::default();

        let prompt = build_chat_system_prompt(
            "zh-CN",
            false,
            false,
            &registry,
            &chat_tools,
            false,
            &[],
            None,
            None,
            None,
            None,
            "",
            None,
            None,
            None,
            None,
            None,
            None,
            Some("/Users/me/Obsidian/MyVault"),
            &[],
            None,
        );

        assert!(prompt.contains("Obsidian 笔记库路径：/Users/me/Obsidian/MyVault"));
    }

    #[test]
    fn assistant_mcp_restrictions_keep_only_allowed_servers() {
        let assistant = test_assistant_snapshot(vec!["demo"], vec![]);
        let mut other = test_mcp_tool();
        other.server_id = Some("other".to_string());
        let mut tools = vec![
            crate::mcp::types::native_skill_activate_tool(),
            crate::mcp::types::native_web_fetch_tool(),
            test_mcp_tool(), // server_id = "demo"
            other,           // server_id = "other"
        ];

        apply_assistant_mcp_restrictions(&mut tools, Some(&assistant));

        // 原生工具保留,只有 allow-list 内的 MCP 工具保留。
        assert!(tools.iter().any(|t| t.name == "skill_activate"));
        assert!(tools.iter().any(|t| t.name == "web_fetch"));
        assert_eq!(tools.iter().filter(|t| t.source == "mcp").count(), 1);
        assert!(tools
            .iter()
            .any(|t| t.source == "mcp" && t.server_id.as_deref() == Some("demo")));
    }

    #[test]
    fn assistant_empty_mcp_list_drops_all_mcp_tools() {
        let assistant = test_assistant_snapshot(vec![], vec![]);
        let mut tools = vec![
            crate::mcp::types::native_web_fetch_tool(),
            test_mcp_tool(),
        ];

        apply_assistant_mcp_restrictions(&mut tools, Some(&assistant));

        assert!(tools.iter().all(|t| t.source != "mcp"));
        assert!(tools.iter().any(|t| t.name == "web_fetch"));
    }

    #[test]
    fn no_assistant_does_not_restrict_mcp() {
        let mut tools = vec![test_mcp_tool()];
        apply_assistant_mcp_restrictions(&mut tools, None);
        assert_eq!(tools.len(), 1);
    }

    #[test]
    fn skill_allowed_respects_assistant_allow_list() {
        let chat_tools = crate::settings::ChatToolsConfig::default(); // 默认无禁用技能
        let assistant = test_assistant_snapshot(vec![], vec!["doc"]);

        assert!(skill_allowed_for_conversation(
            &chat_tools,
            Some(&assistant),
            "doc",
            &[],
            false,
        ));
        // 不在白名单内的技能被拒。
        assert!(!skill_allowed_for_conversation(
            &chat_tools,
            Some(&assistant),
            "pdf",
            &[],
            false,
        ));
        // 无助手 = 不限(只看全局 enable)。
        assert!(skill_allowed_for_conversation(&chat_tools, None, "pdf", &[], false));
    }

    #[test]
    fn skill_allowed_hides_email_connector_skill_without_accounts() {
        let chat_tools = crate::settings::ChatToolsConfig::default();
        assert!(!skill_allowed_for_conversation(
            &chat_tools,
            None,
            crate::settings::EMAIL_CONNECTOR_SKILL_ID,
            &[],
            false,
        ));
        let account = crate::settings::EmailAccountConfig {
            id: "a".to_string(),
            display_name: "Test".to_string(),
            email: "a@example.com".to_string(),
            password: "secret".to_string(),
            imap_host: "imap.example.com".to_string(),
            imap_port: 993,
            imap_encryption: "tls".to_string(),
            smtp_host: "smtp.example.com".to_string(),
            smtp_port: 465,
            smtp_encryption: "tls".to_string(),
            is_default: true,
        };
        assert!(skill_allowed_for_conversation(
            &chat_tools,
            None,
            crate::settings::EMAIL_CONNECTOR_SKILL_ID,
            std::slice::from_ref(&account),
            false,
        ));
    }

    #[test]
    fn skill_allowed_hides_obsidian_skill_without_vault() {
        let chat_tools = crate::settings::ChatToolsConfig::default();
        // No vault → Obsidian skills are unavailable at the conversation level.
        assert!(!skill_allowed_for_conversation(
            &chat_tools,
            None,
            "obsidian-markdown",
            &[],
            false,
        ));
        // Vault configured → available.
        assert!(skill_allowed_for_conversation(
            &chat_tools,
            None,
            "obsidian-markdown",
            &[],
            true,
        ));
    }

    #[test]
    fn skill_fallback_switches_to_markdown_when_assistant_disables_tools() {
        let mut chat_tools = crate::settings::ChatToolsConfig::default();

        apply_skill_fallback_when_tools_unavailable(&mut chat_tools, Some("doc"), false);

        assert_eq!(chat_tools.skill_fallback_mode, "skill_md_only");
    }

    #[test]
    fn disabled_builtin_tool_feedback_is_hidden_model_feedback() {
        let feedback = disabled_builtin_tool_feedback("web_search")
            .expect("disabled builtin tools should produce model feedback");

        assert!(feedback.contains("not enabled"));
        assert!(feedback.contains("web_search"));
        assert!(disabled_builtin_tool_feedback("mcp__server__tool").is_none());
        // 模型按 wire 别名调用时同样识别为内置工具（保留名规避）。
        let alias_feedback = disabled_builtin_tool_feedback("search_web")
            .expect("wire alias resolves to the builtin tool");
        assert!(alias_feedback.contains("not enabled"));
    }

    #[test]
    fn native_tools_prompt_renders_wire_alias_for_web_search() {
        // 提示词必须展示 wire 名（search_web）——与 tools 声明一致，否则模型会调用
        // 未声明的 web_search（且该名会被 Cursor 系上游吞掉）。
        let names = vec!["web_fetch".to_string(), "web_search".to_string()];
        let prompt = native_tools_prompt(&names, "zh", None).expect("prompt");
        assert!(prompt.contains("search_web"), "{prompt}");
        assert!(!prompt.contains("web_search"), "{prompt}");
        let en = native_tools_prompt(&names, "en", None).expect("prompt");
        assert!(en.contains("search_web"), "{en}");
        assert!(!en.contains("web_search"), "{en}");
    }

    #[test]
    fn estimate_tokens_counts_ascii_and_cjk() {
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
        assert_eq!(estimate_tokens("你好ab"), 3);
    }
}
