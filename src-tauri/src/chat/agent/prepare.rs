use serde_json::Value;

use crate::chat::types::{ChatAssistantSnapshot, ContextUsageSegment};
use crate::mcp::ChatToolDefinition;
use crate::settings::{chat_no_think_instruction, default_chat_system_prompt, ChatToolsConfig};
use crate::skills;

use super::types::{AgentPhase, AgentStepResult, AgentStreamPolicy};

const LEGACY_GENERAL_ASSISTANT_SYSTEM_PROMPT: &str =
    "你是 Kivio 的通用助手。回答要清晰、直接，并在信息不足时主动说明假设。";

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
    if skill.allowed_tools.is_empty() {
        return;
    }
    tools.retain(|tool| {
        tool.source == "skill"
            || is_native_skill_tool_name(&tool.name)
            || is_kivio_builtin_tool(tool)
            || skill
                .allowed_tools
                .iter()
                .any(|recommended| tool_matches_recommended_name(tool, recommended))
    });
}

pub fn apply_assistant_tool_preset(
    tools: &mut Vec<ChatToolDefinition>,
    assistant_snapshot: Option<&ChatAssistantSnapshot>,
) {
    let preset = assistant_snapshot
        .map(|assistant| assistant.tool_preset.trim())
        .filter(|preset| !preset.is_empty())
        .unwrap_or("inherit");
    match preset {
        "none" => tools.clear(),
        "skills" => tools.retain(|tool| tool.source == "skill"),
        "inherit" | "all" => {}
        _ => {}
    }
}

pub fn apply_assistant_data_connectors_tool_filter(
    tools: &mut Vec<ChatToolDefinition>,
    assistant_snapshot: Option<&ChatAssistantSnapshot>,
) {
    let Some(assistant) = assistant_snapshot else {
        return;
    };
    let mut has_explicit_scope = false;
    for connector in assistant
        .data_connectors
        .iter()
        .filter(|connector| connector.enabled && connector.configured)
    {
        if connector
            .server_id
            .as_ref()
            .is_some_and(|id| !id.trim().is_empty())
            || connector
                .tool_ids
                .iter()
                .any(|tool_id| !tool_id.trim().is_empty())
        {
            has_explicit_scope = true;
            break;
        }
    }
    if !has_explicit_scope {
        return;
    }

    tools.retain(|tool| {
        tool.source == "skill"
            || assistant
                .data_connectors
                .iter()
                .filter(|connector| connector.enabled && connector.configured)
                .any(|connector| data_connector_allows_tool(connector, tool))
    });
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
    const BUILTIN_NAMES: &[&str] = &[
        "web_search",
        "web_fetch",
        "read_file",
        "write_file",
        "edit_file",
        "run_command",
        "run_python",
        "memory_read",
        "memory_modify",
        "mixer_generate_image",
        "todo_write",
        "todo_update",
    ];
    if BUILTIN_NAMES.contains(&function_name) {
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
    if tool.source == "native" && crate::chat::todo::is_agent_todo_tool_name(&tool.name) {
        return true;
    }
    tool.source == "native" && matches!(tool.name.as_str(), "memory_read" | "memory_modify")
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
    agent_todo_prompt: Option<&str>,
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
        agent_todo_prompt,
    )
    .0
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
    agent_todo_prompt: Option<&str>,
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

    if let Some(todo) = agent_todo_prompt
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        append_context_segment(&mut prompt, &mut segments, "agent_todo", "Agent todo", todo);
    }

    if tools_available {
        let mut action_examples = vec!["activating a skill", "reading a file", "running a script"];
        if available_builtin_tools
            .iter()
            .any(|tool| matches!(tool.as_str(), "web_search" | "web_fetch"))
        {
            action_examples.push("using the web");
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
                crate::settings::is_skill_enabled(chat_tools, skill_id)
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
    let mut suite_meta = Vec::new();
    if !assistant.source.trim().is_empty() {
        suite_meta.push(format!("source {}", assistant.source.trim()));
    }
    if !assistant.version.trim().is_empty() {
        suite_meta.push(format!("version {}", assistant.version.trim()));
    }
    if !suite_meta.is_empty() {
        parts.push(format!(
            "Assistant suite metadata: {}",
            suite_meta.join(", ")
        ));
    }
    if !assistant.description.trim().is_empty() {
        parts.push(format!(
            "Assistant purpose: {}",
            assistant.description.trim()
        ));
    }
    let assistant_system_prompt = assistant.system_prompt.trim();
    let is_legacy_general_identity = assistant.id == "asst_builtin_general"
        && assistant_system_prompt == LEGACY_GENERAL_ASSISTANT_SYSTEM_PROMPT;
    if !assistant_system_prompt.is_empty() && !is_legacy_general_identity {
        parts.push(format!(
            "Assistant instructions:\n{}",
            assistant_system_prompt
        ));
    }
    if !assistant.greeting.trim().is_empty() {
        parts.push(format!("Assistant greeting: {}", assistant.greeting.trim()));
    }
    if !assistant.conversation_starters.is_empty() {
        parts.push(format!(
            "Representative starter prompts: {}",
            assistant.conversation_starters.join(" | ")
        ));
    }
    let quick_commands = assistant
        .quick_commands
        .iter()
        .filter(|command| command.enabled)
        .filter(|command| !command.name.trim().is_empty() || !command.slash.trim().is_empty())
        .map(|command| {
            let slash = if command.slash.trim().is_empty() {
                "(no slash)"
            } else {
                command.slash.trim()
            };
            let mut line = format!("- {slash} / {}", command.name.trim());
            if !command.description.trim().is_empty() {
                line.push_str(&format!(": {}", command.description.trim()));
            }
            if !command.prompt.trim().is_empty() {
                line.push_str(&format!(
                    " Prompt to apply when invoked: {}",
                    command.prompt.trim()
                ));
            }
            if !command.starter_text.trim().is_empty() {
                line.push_str(&format!(" Starter text: {}", command.starter_text.trim()));
            }
            line
        })
        .collect::<Vec<_>>();
    if !quick_commands.is_empty() {
        parts.push(format!(
            "Assistant quick commands. When the user's message starts with one of these slash commands, follow its command prompt for that turn:\n{}",
            quick_commands.join("\n")
        ));
    }

    let data_connectors = assistant
        .data_connectors
        .iter()
        .filter(|connector| connector.enabled)
        .filter(|connector| !connector.name.trim().is_empty())
        .map(|connector| {
            let mut line = format!(
                "- {} ({})",
                connector.name.trim(),
                if connector.kind.trim().is_empty() {
                    "connector"
                } else {
                    connector.kind.trim()
                }
            );
            if !connector.configured {
                line.push_str(" [not configured]");
            }
            if connector.required {
                line.push_str(" [required]");
            }
            if !connector.description.trim().is_empty() {
                line.push_str(&format!(": {}", connector.description.trim()));
            }
            if !connector.tool_ids.is_empty() {
                line.push_str(&format!(" Tools: {}", connector.tool_ids.join(", ")));
            }
            if let Some(server_id) = connector
                .server_id
                .as_ref()
                .filter(|id| !id.trim().is_empty())
            {
                line.push_str(&format!(" Server: {}", server_id.trim()));
            }
            line
        })
        .collect::<Vec<_>>();
    if !data_connectors.is_empty() {
        parts.push(format!(
            "Assistant data connectors. Use only configured connectors and only claim connector access after a tool result is returned:\n{}",
            data_connectors.join("\n")
        ));
    }

    let knowledge_skills = assistant
        .knowledge_skills
        .iter()
        .filter(|skill| skill.enabled)
        .filter(|skill| !skill.name.trim().is_empty())
        .map(|skill| {
            let mut line = format!("- {}", skill.name.trim());
            if !skill.description.trim().is_empty() {
                line.push_str(&format!(": {}", skill.description.trim()));
            }
            if !skill.trigger_phrases.is_empty() {
                line.push_str(&format!(" Triggers: {}", skill.trigger_phrases.join(", ")));
            }
            if let Some(skill_id) = skill.skill_id.as_ref().filter(|id| !id.trim().is_empty()) {
                line.push_str(&format!(" Bound Skill: {skill_id}"));
            }
            if !skill.prompt.trim().is_empty() {
                line.push_str(&format!(" Instructions: {}", skill.prompt.trim()));
            }
            if !skill.recommended_tools.is_empty() {
                line.push_str(&format!(
                    " Recommended tools: {}",
                    skill.recommended_tools.join(", ")
                ));
            }
            line
        })
        .collect::<Vec<_>>();
    if !knowledge_skills.is_empty() {
        parts.push(format!(
            "Assistant knowledge skills. When the user request matches a trigger, apply the matching skill guidance before answering:\n{}",
            knowledge_skills.join("\n")
        ));
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

fn tool_matches_recommended_name(tool: &ChatToolDefinition, recommended: &str) -> bool {
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

fn data_connector_allows_tool(
    connector: &crate::chat::types::AssistantDataConnector,
    tool: &ChatToolDefinition,
) -> bool {
    if connector
        .server_id
        .as_deref()
        .map(str::trim)
        .filter(|server_id| !server_id.is_empty())
        .is_some_and(|server_id| tool.server_id.as_deref() == Some(server_id))
    {
        return true;
    }
    connector
        .tool_ids
        .iter()
        .any(|tool_id| tool_matches_recommended_name(tool, tool_id))
}

fn native_tools_prompt(available_builtin_tools: &[String], language: &str) -> Option<String> {
    if available_builtin_tools.is_empty() {
        return None;
    }
    let list = available_builtin_tools.join(", ");
    let has_web_search = available_builtin_tools
        .iter()
        .any(|tool| tool.as_str() == "web_search");
    let has_web_fetch = available_builtin_tools
        .iter()
        .any(|tool| tool.as_str() == "web_fetch");
    let has_image_generation = available_builtin_tools
        .iter()
        .any(|tool| tool.as_str() == "mixer_generate_image");
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
            "用户要求创建、生成、绘制图片时，必须调用 mixer_generate_image；不要只用文字描述会生成什么。mixer_generate_image 使用设置中的混音器生图模型，返回可渲染图片 artifact。"
        } else {
            "生图工具未启用；用户要求生成图片时，说明需要先在「混音器」里配置生图模型。"
        };
        format!(
            "内置工具（已启用）：{list}。只允许调用这里列出的内置工具。memory_read 可读取 L1/L2 记忆；L1 已在启用记忆时默认注入，通常不需要再读，L2 必须通过 memory_read 按需读取。memory_modify 用于追加、替换、删除或归档记忆，L1 只能保存每次都该知道的短约束且最多 5000 字节，L2 保存长期流程和知识且不会自动加载。read_file 可读取应用能访问的本地文本文件；write_file、edit_file 仍只能写入用户主目录下的文件；只有当用户明确要求保存/写入/创建本地文件、或给出目标路径时，才调用 write_file/edit_file；如果用户要求“生成代码块”“用 ```html 包起来”“给完整代码”“做一个 demo”但没有明确要求保存文件，应直接在回答中生成内容，不要调用 write_file；write_file 成功后只简短说明保存路径和结果，不要再把完整文件内容复述一遍，除非用户明确要求同时保存并展示全文。run_command 可在任意已存在的工作目录中执行终端命令。write_file、edit_file、run_command 可能会请求用户确认；memory_read / memory_modify 无需确认；run_command 非零退出码代表执行失败，不要用它运行 Skill 自带脚本，Skill 脚本必须走 skill_run_script。run_command 不得用 pip/pip3/python -m pip 安装包来绕过 run_python 沙盒失败；只有用户明确要求修改本机 Python 环境时，才能设置 allow_host_python_package_install=true 且使用 --user 或虚拟环境。run_python 在 Pyodide 沙盒中运行，不能直接访问或修改本机文件系统；处理本地文档或聊天附件时，把可读取的文件路径传入 run_python 的 files 数组，应用会把文件复制到 Pyodide 虚拟文件系统，并在 Python 全局变量 KIVIO_INPUT_FILES 中提供虚拟路径。导入 numpy、matplotlib、pandas、scipy、sympy、scikit-learn、statsmodels、pillow、seaborn、micropip 等常用包时会自动加载，缺失的 Pyodide 兼容包可在沙盒内通过 micropip 联网下载。run_python 适合数据运算、统计分析、机器学习基础分析、文档分析和生成图表；用 run_python 生成图像/图表时，保存为 Pyodide 当前目录下的相对文件名（例如 output.png），不要保存到 /Users 等本机路径，不要 print base64 或 data:image URL；应用会自动捕获并渲染生成的图片。联网搜索、网页读取、生产 API 调用等任务若有专门工具，应优先使用已启用的专门工具或对应 Skill 脚本；{zh_live_access_hint}不要为了 Python 包使用 host pip 安装，除非用户明确要求操作本机环境。用户要用 Python 跑代码/计算时优先 run_python，不要用 skill_run_script，除非用户点名某个 Skill。"
        ) + image_generation_hint
    } else {
        let image_generation_hint = if has_image_generation {
            " When the user asks to create, generate, or draw an image, you must call mixer_generate_image; do not merely describe the image. mixer_generate_image uses the Mixer image generation model configured in Settings and returns renderable image artifacts."
        } else {
            " Image generation is not enabled; if the user asks to generate an image, explain that an image generation model must be configured under Mixer first."
        };
        format!(
            "Built-in tools enabled: {list}. Only call built-in tools listed here. memory_read reads L1/L2 memory; L1 is already injected by default when memory is enabled, so usually read L2 on demand. memory_modify appends, replaces, removes, or archives memory; L1 is short online memory limited to 5000 bytes, while L2 stores long-term workflows and knowledge and is never auto-loaded. read_file can read local text files that the app can access; write_file and edit_file can only write files under the user home directory. Call write_file/edit_file only when the user explicitly asks to save/write/create a local file or provides a target path. If the user asks for a code block, fenced HTML, complete code, or a demo without explicitly asking to save it, generate the content directly in the assistant answer and do not call write_file. After a successful write_file call, briefly state the saved path/result without repeating the full file content unless the user explicitly asked to both save and display it. run_command can execute terminal commands in any existing working directory. write_file, edit_file, and run_command may require user approval; memory_read and memory_modify do not; run_command treats non-zero exit codes as failures. Do not use run_command to run Skill bundled scripts; use skill_run_script. Do not use pip/pip3/python -m pip through run_command to bypass run_python sandbox failures; only set allow_host_python_package_install=true when the user explicitly asks to modify the host Python environment, and then use --user or a virtual environment. run_python runs in a Pyodide sandbox with no direct host filesystem access. To analyze local documents or chat attachments, pass readable file paths in the run_python files array; the app copies them into the Pyodide virtual filesystem and exposes the virtual paths through the Python global KIVIO_INPUT_FILES. run_python auto-loads common packages when imported, including numpy, matplotlib, pandas, scipy, sympy, scikit-learn, statsmodels, pillow, seaborn, and micropip; missing Pyodide-compatible packages may be downloaded inside the sandbox with micropip. Use it for data computation, statistical analysis, basic machine-learning analysis, document analysis, code execution, and charts. When generating images/charts with run_python, save them to relative filenames in the Pyodide current directory such as output.png; do not save to host paths such as /Users, and do not print base64 or data:image URLs. The app captures and renders generated images automatically. For web search, page reading, and production API calls, prefer enabled dedicated tools or the relevant Skill script when those dedicated tools are available; {en_live_access_hint} Do not use host pip to install Python packages unless the user explicitly asks to modify the host Python environment. For generic Python requests, use run_python—not skill_run_script—unless the user named a specific skill."
        ) + image_generation_hint
    };
    if has_image_generation && !prompt.ends_with('.') && !prompt.ends_with('。') {
        prompt.push('.');
    }
    Some(prompt)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_assistant_snapshot(tool_preset: &str, skill_id: Option<&str>) -> ChatAssistantSnapshot {
        ChatAssistantSnapshot {
            id: "asst_test".to_string(),
            name: "Test Assistant".to_string(),
            description: String::new(),
            source: "user".to_string(),
            version: "1.0.0".to_string(),
            system_prompt: String::new(),
            provider_id: String::new(),
            model: String::new(),
            skill_id: skill_id.map(str::to_string),
            tool_preset: tool_preset.to_string(),
            conversation_starters: Vec::new(),
            greeting: String::new(),
            quick_commands: Vec::new(),
            data_connectors: Vec::new(),
            knowledge_skills: Vec::new(),
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
        );

        assert!(prompt.contains("run_python"));
        assert!(!prompt.contains("web_search"));
        assert!(!prompt.contains("web_fetch"));
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
        );

        assert!(prompt.contains("用 ```html 包起来"));
        assert!(prompt.contains("不要调用 write_file"));
        assert!(prompt.contains("不要再把完整文件内容复述一遍"));
    }

    #[test]
    fn custom_chat_prompt_is_not_overridden_by_general_assistant_identity() {
        let registry = skills::SkillRegistry::default();
        let chat_tools = crate::settings::ChatToolsConfig::default();
        let mut assistant = test_assistant_snapshot("inherit", None);
        assistant.id = "asst_builtin_general".to_string();
        assistant.name = "通用助手".to_string();
        assistant.system_prompt =
            "你是 Kivio 的通用助手。回答要清晰、直接，并在信息不足时主动说明假设。".to_string();

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
            Some(&assistant),
            "你",
            None,
            None,
        );

        assert!(prompt.starts_with("你\n\nActive assistant: 通用助手"));
        assert!(!prompt.contains("你是 Kivio 的通用助手"));
    }

    #[test]
    fn assistant_tool_preset_none_disables_all_tools() {
        let assistant = test_assistant_snapshot("none", Some("doc"));
        let mut tools = vec![
            crate::mcp::types::native_skill_activate_tool(),
            crate::mcp::types::native_web_fetch_tool(),
            test_mcp_tool(),
        ];

        apply_assistant_tool_preset(&mut tools, Some(&assistant));

        assert!(tools.is_empty());
    }

    #[test]
    fn assistant_tool_preset_skills_keeps_only_skill_runtime_tools() {
        let assistant = test_assistant_snapshot("skills", Some("doc"));
        let mut tools = vec![
            crate::mcp::types::native_skill_activate_tool(),
            crate::mcp::types::native_skill_read_file_tool(),
            crate::mcp::types::native_web_fetch_tool(),
            test_mcp_tool(),
        ];

        apply_assistant_tool_preset(&mut tools, Some(&assistant));

        assert_eq!(tools.len(), 2);
        assert!(tools.iter().all(|tool| tool.source == "skill"));
        assert!(tools.iter().any(|tool| tool.name == "skill_activate"));
        assert!(tools.iter().any(|tool| tool.name == "skill_read_file"));
    }

    #[test]
    fn assistant_tool_preset_inherit_and_all_leave_tools_unchanged() {
        for preset in ["inherit", "all", "unexpected"] {
            let assistant = test_assistant_snapshot(preset, None);
            let mut tools = vec![
                crate::mcp::types::native_skill_activate_tool(),
                crate::mcp::types::native_web_fetch_tool(),
                test_mcp_tool(),
            ];

            apply_assistant_tool_preset(&mut tools, Some(&assistant));

            assert_eq!(tools.len(), 3, "preset {preset} should not filter tools");
        }
    }

    #[test]
    fn assistant_data_connectors_filter_tools_when_explicitly_scoped() {
        let mut assistant = test_assistant_snapshot("inherit", None);
        assistant.data_connectors = vec![crate::chat::types::AssistantDataConnector {
            id: "python".to_string(),
            name: "Python".to_string(),
            kind: "builtin_tool".to_string(),
            description: String::new(),
            tool_ids: vec!["run_python".to_string()],
            server_id: None,
            required: false,
            enabled: true,
            configured: true,
        }];
        let mut tools = vec![
            crate::mcp::types::native_skill_activate_tool(),
            crate::mcp::types::native_run_python_tool(),
            crate::mcp::types::native_web_fetch_tool(),
            test_mcp_tool(),
        ];

        apply_assistant_data_connectors_tool_filter(&mut tools, Some(&assistant));

        assert_eq!(tools.len(), 2);
        assert!(tools.iter().any(|tool| tool.name == "skill_activate"));
        assert!(tools.iter().any(|tool| tool.name == "run_python"));
        assert!(!tools.iter().any(|tool| tool.name == "web_fetch"));
        assert!(!tools.iter().any(|tool| tool.name == "search"));
    }

    #[test]
    fn assistant_data_connectors_without_explicit_scope_do_not_filter() {
        let mut assistant = test_assistant_snapshot("inherit", None);
        assistant.data_connectors = vec![crate::chat::types::AssistantDataConnector {
            id: "image_attachment".to_string(),
            name: "Image attachment".to_string(),
            kind: "file".to_string(),
            description: String::new(),
            tool_ids: Vec::new(),
            server_id: None,
            required: false,
            enabled: true,
            configured: true,
        }];
        let mut tools = vec![
            crate::mcp::types::native_skill_activate_tool(),
            crate::mcp::types::native_web_fetch_tool(),
            test_mcp_tool(),
        ];

        apply_assistant_data_connectors_tool_filter(&mut tools, Some(&assistant));

        assert_eq!(tools.len(), 3);
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
