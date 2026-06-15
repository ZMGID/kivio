use serde_json::Value;

use crate::chat::types::{ChatAssistantSnapshot, ContextUsageSegment};
use crate::mcp::ChatToolDefinition;
use crate::settings::{chat_no_think_instruction, default_chat_system_prompt, ChatToolsConfig};
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
) -> bool {
    if !crate::settings::is_skill_enabled(chat_tools, skill_id) {
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
    custom_system_prompt: &str,
    memory_prompt: Option<&str>,
    agent_plan_prompt: Option<&str>,
    agent_ask_user_prompt: Option<&str>,
    agent_todo_prompt: Option<&str>,
    project_context: Option<&ProjectPromptContext>,
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
        custom_system_prompt,
        memory_prompt,
        agent_plan_prompt,
        agent_ask_user_prompt,
        agent_todo_prompt,
        project_context,
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
    custom_system_prompt: &str,
    memory_prompt: Option<&str>,
    agent_plan_prompt: Option<&str>,
    agent_ask_user_prompt: Option<&str>,
    agent_todo_prompt: Option<&str>,
    project_context: Option<&ProjectPromptContext>,
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
            matches!(
                tool.as_str(),
                "read_file" | "list_dir" | "search_files" | "glob_files" | "stat_path"
            )
        }) {
            action_examples.push("reading or searching project files");
        }
        if available_builtin_tools
            .iter()
            .any(|tool| matches!(tool.as_str(), "run_command" | "run_python"))
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
                " 若用户只问今天/明天/星期几等可由上文「当前本地时间」直接推算的日期问题，直接回答，不要调用工具。",
            );
        } else {
            runtime.push_str(
                " If the user only asks for today/tomorrow/weekday derivable from the system local time above, answer directly without calling tools.",
            );
        }
        append_context_segment(
            &mut prompt,
            &mut segments,
            "runtime_context",
            "Runtime context",
            &runtime,
        );
        if let Some(native_prompt) = native_tools_prompt(available_builtin_tools, language) {
            append_context_segment(
                &mut prompt,
                &mut segments,
                "native_tools",
                "Native tools",
                &native_prompt,
            );
        }
    }

    let include_catalog = chat_tools.skill_auto_match
        || active_skill_id.is_some()
        || chat_tools.skill_fallback_mode != "legacy_full_body";
    if include_catalog {
        let catalog =
            skills::format_catalog(registry, active_skill_id, tools_available, |skill_id| {
                skill_allowed_for_conversation(chat_tools, assistant_snapshot, skill_id)
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
                ". Call skill_activate with this name only because the user pinned it; otherwise prefer enabled built-in tools when they fit.",
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
        let builtin_hint = if available_builtin_tools.is_empty() {
            "内置工具".to_string()
        } else {
            format!("内置工具（{}）", available_builtin_tools.join(", "))
        };
        if language.starts_with("zh") {
            append_context_segment(
                &mut prompt,
                &mut segments,
                "skills",
                "Skills",
                &format!("Skill 目录仅供参考：仅当用户明确需要某个 Skill 的能力（或点名 Skill 名称）时才 skill_activate。泛泛请求若已启用 {builtin_hint} 能覆盖，应优先使用对应内置工具；不要只因 Skill 描述里提到 Python/脚本/联网就激活无关 Skill。"),
            );
        } else {
            let builtin_hint = if available_builtin_tools.is_empty() {
                "built-in tools".to_string()
            } else {
                format!("built-in tools ({})", available_builtin_tools.join(", "))
            };
            append_context_segment(
                &mut prompt,
                &mut segments,
                "skills",
                "Skills",
                &format!("The skill catalog is optional: call skill_activate only when the user clearly needs that skill (or names it). For generic requests covered by enabled {builtin_hint}, prefer the corresponding built-in tool instead of activating an unrelated skill just because its description mentions Python, scripts, or web access."),
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
    tool.name == recommended
        || tool.id == recommended
        || tool.openai_tool_name() == recommended
        || tool
            .server_id
            .as_deref()
            .map(|server_id| format!("{server_id}:{}", tool.name) == recommended)
            .unwrap_or(false)
}

fn native_tools_prompt(available_builtin_tools: &[String], language: &str) -> Option<String> {
    let native_tool_names = available_builtin_tools
        .iter()
        .filter(|tool| tool.as_str() != crate::chat::ask_user::ASK_USER_TOOL_NAME)
        .cloned()
        .collect::<Vec<_>>();
    if native_tool_names.is_empty() {
        return None;
    }
    let list = native_tool_names.join(", ");
    // 运行时取值,让同一份 prompt 在不同平台都说真话(run_command 的 shell 是编译期 cfg 选的)。
    let (os_name, shell_name) = if cfg!(target_os = "windows") {
        ("Windows", "cmd.exe")
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
    let zh_live_access_hint = match (has_web_search, has_web_fetch) {
        (true, true) => "实时搜索或网页读取必须优先用 web_search/web_fetch 或对应 Skill 脚本。",
        (true, false) => "实时搜索必须优先用 web_search 或对应 Skill 脚本。",
        (false, true) => "网页读取必须优先用 web_fetch 或对应 Skill 脚本。",
        (false, false) => "需要联网/API 访问时，请启用对应联网工具或使用对应 Skill 脚本。",
    };
    let en_live_access_hint = match (has_web_search, has_web_fetch) {
        (true, true) => {
            "Use web_search/web_fetch or the relevant Skill script for live web/API access."
        }
        (true, false) => "Use web_search or the relevant Skill script for live web/API access.",
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
        let generated_file_hint = if has_run_python {
            "\n- 用户用自然语言要求“生成/整理/导出/发我”报告、摘要、表格、数据集、图表、Markdown、CSV、JSON、TXT、HTML 或 XLSX 文件时，主动调用 run_python 生成对应相对路径产物；不要要求用户说出 run_python 或 Python。成功后只简短说明已生成文件，文件卡片会展示给用户。若用户给出明确宿主路径或要求保存到本地某处，改用 write_file。"
        } else {
            ""
        };
        format!(
            "内置工具（已启用）：{list}。只能调用此列表中的内置工具。\n\
- 项目对话中文件/命令工具的相对路径以项目根目录为根；写入明确的绝对路径或 ~/ 路径（如 ~/Desktop/x.html）会落到项目外的全局位置。非项目对话用绝对路径或 ~/ 路径。\n\
- 用户明确要求保存/修改/删除本地文件或给出目标路径时才动文件：小改用 edit_file，新建或整文件覆盖用 write_file。只要求“生成代码块”时直接在回答里输出，不调用 write_file。写入成功后简短说明路径即可，不要复述文件内容。\n\
- 写入/删除/移动类工具和 run_command 可能需要用户确认；memory_read（按需读 L2，L1 已注入）、memory_search（按关键词检索 L2，找不准标题时优先用它）和 memory_modify 无需确认。\n\
- 运行环境：{os_name}，run_command 经 {shell_name} 执行；命令语法须匹配该 shell（Windows 用 `%VAR%`、`dir`、`\\`；Unix 用 `$VAR`、`ls`、`/`）。每次 run_command 都是全新进程，cwd 不跨调用保留——切目录用 `cwd` 参数，别靠上一条 `cd`。要跑多行或带引号的代码，先用 write_file 写成脚本再执行，或用 run_python，别塞进 `python -c \"...\"` 这类内联命令（内联引号在各 shell 下都脆弱）。工具返回硬性拒绝时换策略，别把同一动作换几种写法反复试；失败命令不要原样重跑；别为一次性探测或清理往项目里扔临时脚本。\n\
- run_command 在宿主 shell 从项目根目录执行，非零退出码即失败；含空格的路径必须用 `cwd` 参数，禁止 `cd 路径 && 命令`；不要同时传 `cwd` 又在 command 里写 `cd ... &&`。`npm run dev` / `tauri dev` / `vite` 等长驻 dev 命令会自动后台启动并立刻返回 pid，不要重复启动。破坏性、联网、改环境的命令先说明并等确认。Skill 脚本走 skill_run_script；不要用 pip 装宿主包绕过沙盒。\n\
- run_python 在 Pyodide 沙盒运行，用于数据运算、分析、文档处理、图表和聊天产物文件生成；不要用它生成或打印代码答案，代码直接写在回答里。无宿主文件系统访问；files 挂载本地文件后用 KIVIO_INPUT_FILES[n] 路径，numpy、pandas、matplotlib、pillow、openpyxl、pypdf 可直接 import。产物保存为相对路径文件名（如 report.md、summary.csv、data.json、page.html、report.xlsx、chart.png），应用会自动捕获并显示文件卡片；不要 print base64。\n\
- {zh_live_access_hint}"
        ) + generated_file_hint + image_generation_hint
    } else {
        let image_generation_hint = if has_image_generation {
            "\n- When the user asks to create, generate, or draw an image, call mixer_generate_image; do not merely describe it."
        } else {
            "\n- Image generation is not enabled; if asked, explain that an image model must be configured under Mixer first."
        };
        let generated_file_hint = if has_run_python {
            "\n- When the user naturally asks you to generate, export, send, package, or provide a report, summary, table, dataset, chart, Markdown, CSV, JSON, TXT, HTML, or XLSX file, proactively call run_python to create the artifact as a relative output file; do not ask the user to mention run_python or Python. After success, briefly say the file was generated; Kivio will show the file card. If the user gives an explicit host path or asks to save somewhere local, use write_file instead."
        } else {
            ""
        };
        format!(
            "Built-in tools enabled: {list}. Only call tools in this list.\n\
- In project conversations, relative paths in file/command tools resolve from the project root; writing an explicit absolute or ~/ path (e.g. ~/Desktop/x.html) targets that global location outside the project. Non-project conversations use absolute or ~/ paths.\n\
- Touch files only when the user explicitly asks to save/modify/delete local files or gives a target path: edit_file for small edits, write_file for new files or whole-file overwrites. If asked for a code block without saving, answer inline. After a write, state the path briefly; do not repeat the file content.\n\
- Write/delete/move tools and run_command may need user approval; memory_read (L2 on demand; L1 is auto-injected), memory_search (keyword search over L2; prefer it when you are unsure of the exact heading), and memory_modify do not.\n\
- Runtime environment: {os_name}; run_command runs via {shell_name}. Match that shell's syntax (Windows: `%VAR%`, `dir`, `\\`; Unix: `$VAR`, `ls`, `/`). Each run_command is a fresh process — cwd does NOT persist across calls; switch directories with the `cwd` parameter, not a prior `cd`. To run multi-line or quoted code, write it to a file with write_file and run that, or use run_python — do not cram it into inline commands like `python -c \"...\"` (inline quotes are fragile across shells). When a tool returns a hard rejection, change strategy instead of retrying variants of the same action; never re-run a failed command unchanged; don't drop one-off probe or cleanup scripts into the project.\n\
- run_command runs on the host shell from the project root; non-zero exit means failure. Paths with spaces must use the `cwd` parameter—never `cd path && command`; do not combine `cwd` with a leading `cd ... &&` prefix. Long-running dev commands such as `npm run dev`, `tauri dev`, and `vite` start in the background automatically and return a pid immediately; do not start the same dev server twice. Explain and get confirmation before destructive, network, or environment-changing commands. Skill scripts go through skill_run_script; never use host pip to bypass the run_python sandbox.\n\
- run_python runs in a Pyodide sandbox for data computation, analysis, document processing, charts, and chat deliverable file generation; never use it to generate or print code answers — write code directly in the answer. No host filesystem access; mount files via the files parameter and use KIVIO_INPUT_FILES[n] paths. numpy, pandas, matplotlib, pillow, openpyxl, pypdf import directly. Save artifacts to relative filenames (report.md, summary.csv, data.json, page.html, report.xlsx, chart.png); Kivio auto-captures them and shows file cards. No base64 printing.\n\
- {en_live_access_hint}"
        ) + generated_file_hint + image_generation_hint
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
            "",
            None,
            None,
            None,
            None,
            None,
        );

        assert!(prompt.contains("run_python"));
        assert!(!prompt.contains("web_search"));
        assert!(!prompt.contains("web_fetch"));
    }

    #[test]
    fn chat_prompt_treats_run_python_as_generated_file_tool() {
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
            "",
            None,
            None,
            None,
            None,
            None,
        );

        assert!(prompt.contains("主动调用 run_python"));
        assert!(prompt.contains("不要要求用户说出 run_python 或 Python"));
        assert!(prompt.contains("文件卡片"));
        assert!(prompt.contains("report.md"));
        assert!(prompt.contains("report.xlsx"));
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
            &["write_file".to_string()],
            None,
            None,
            None,
            "",
            None,
            None,
            None,
            None,
            None,
        );

        assert!(prompt.contains("生成代码块"));
        assert!(prompt.contains("不调用 write_file"));
        assert!(prompt.contains("不要复述文件内容"));
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
            "doc"
        ));
        // 不在白名单内的技能被拒。
        assert!(!skill_allowed_for_conversation(
            &chat_tools,
            Some(&assistant),
            "pdf"
        ));
        // 无助手 = 不限(只看全局 enable)。
        assert!(skill_allowed_for_conversation(&chat_tools, None, "pdf"));
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
    }

    #[test]
    fn estimate_tokens_counts_ascii_and_cjk() {
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
        assert_eq!(estimate_tokens("你好ab"), 3);
    }
}
