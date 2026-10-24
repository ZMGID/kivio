//! Shared conversation-mode tool filtering.
use crate::mcp::ChatToolDefinition;
pub(crate) fn apply_agent_plan_tool_filter(
    tools: &mut Vec<ChatToolDefinition>,
    plan_mode: bool,
) -> Vec<ChatToolDefinition> {
    if !plan_mode {
        return Vec::new();
    }
    let mut blocked = Vec::new();
    tools.retain(|tool| {
        let allowed = agent_plan_allows_tool(tool);
        if !allowed {
            blocked.push(tool.clone());
        }
        allowed
    });
    blocked
}

/// Chat mode: conversational research tools only — gated by `ChatModeConfig` toggles.
/// Blocks local fs mutation, shell, sub-agents, skills, todos, and write-capable MCP.
pub(crate) fn apply_chat_mode_tool_filter(
    tools: &mut Vec<ChatToolDefinition>,
    chat_mode: bool,
    config: &crate::settings::ChatModeConfig,
) -> Vec<ChatToolDefinition> {
    if !chat_mode {
        return Vec::new();
    }
    let mut blocked = Vec::new();
    tools.retain(|tool| {
        let allowed = chat_mode_allows_tool(tool, config);
        if !allowed {
            blocked.push(tool.clone());
        }
        allowed
    });
    blocked
}

pub(crate) fn chat_mode_allows_tool(
    tool: &ChatToolDefinition,
    config: &crate::settings::ChatModeConfig,
) -> bool {
    if tool.source == "mixer" && tool.name == "mixer_video_analysis" {
        return true;
    }
    if tool.source == "native" && crate::chat::ask_user::is_ask_user_tool_name(&tool.name) {
        return true;
    }
    if tool.source == "mcp" {
        return config.mcp_read_only && tool.is_read_only_tool();
    }
    if tool.source != "native" {
        return false;
    }
    match tool.name.as_str() {
        "web_search" | "search_web" => config.web_search,
        "web_fetch" => config.web_fetch,
        "knowledge_search" => config.knowledge_search,
        "automation_list" | "automation_get" | "automation_runs" => true,
        "memory_read" | "memory_search" => config.memory_tools,
        _ => false,
    }
}

pub(crate) fn agent_plan_allows_tool(tool: &ChatToolDefinition) -> bool {
    if tool.source == "mixer" && tool.name == "mixer_video_analysis" {
        return true;
    }
    if tool.source == "native" && crate::chat::ask_user::is_ask_user_tool_name(&tool.name) {
        return true;
    }
    if tool.source == "native" && crate::chat::todo::is_agent_todo_tool_name(&tool.name) {
        return true;
    }
    if tool.source == "native" {
        return tool.is_read_only_tool();
    }
    if tool.source == "mcp" {
        return tool.is_read_only_tool();
    }
    tool.source == "skill" && tool.name == "skill"
}

/// 会话级三态联网搜索（任务 07-23）：按有效模式收敛第三方 `search_web` 工具的暴露。
/// - `ThirdParty`：保证 `search_web` 存在（列表因全局开关缺席时，若已配置搜索 key 则补回）。
/// - `Builtin` / `Off`：移除 `search_web`（内置走请求体注入，Off 完全不联网）。
///
/// 内置注入不在此处——由 `AgentRunConfig.builtin_web_search_active()` 经 `GenerateOptions`
/// 驱动各适配器；此函数只管客户端工具的去留。
pub(crate) fn apply_web_search_mode_tool_filter(
    tools: &mut Vec<ChatToolDefinition>,
    mode: crate::chat::types::WebSearchMode,
    settings: &crate::settings::Settings,
) {
    const SEARCH_WEB_ID: &str = "native__web_search";
    match mode {
        crate::chat::types::WebSearchMode::ThirdParty => {
            let present = tools.iter().any(|t| t.id == SEARCH_WEB_ID);
            if !present && crate::mcp::search_policy::web_search_configured(settings) {
                tools.push(crate::mcp::types::native_web_search_tool());
            }
        }
        crate::chat::types::WebSearchMode::Builtin | crate::chat::types::WebSearchMode::Off => {
            tools.retain(|t| t.id != SEARCH_WEB_ID);
        }
    }
}

pub(crate) fn apply_inline_code_request_tool_filter(
    tools: &mut Vec<ChatToolDefinition>,
    last_user_api_content: Option<&str>,
) {
    if !should_answer_inline_without_file_write(last_user_api_content) {
        return;
    }
    tools.retain(|tool| !(tool.source == "native" && tool.name == "write"));
}

pub(crate) fn should_answer_inline_without_file_write(last_user_api_content: Option<&str>) -> bool {
    let Some(content) = last_user_api_content else {
        return false;
    };
    let user_text = content
        .split("[已添加附件]")
        .next()
        .unwrap_or(content)
        .trim();
    if user_text.is_empty() {
        return false;
    }
    let normalized = user_text.to_ascii_lowercase();
    if has_explicit_file_write_intent(user_text, &normalized) {
        return false;
    }
    has_inline_code_request_intent(user_text, &normalized)
}

pub(crate) fn has_explicit_file_write_intent(text: &str, normalized: &str) -> bool {
    const ZH_MARKERS: &[&str] = &[
        "保存",
        "写入",
        "写到",
        "写进",
        "输出到",
        "导出",
        "创建文件",
        "生成文件",
        "另存为",
        "存成",
        "落盘",
    ];
    const EN_MARKERS: &[&str] = &[
        "save",
        "create file",
        "output file",
        "output to",
        "export",
        "save as",
        "write to",
        "file named",
    ];
    ZH_MARKERS.iter().any(|marker| text.contains(marker))
        || EN_MARKERS.iter().any(|marker| normalized.contains(marker))
}

pub(crate) fn has_inline_code_request_intent(text: &str, normalized: &str) -> bool {
    const ZH_MARKERS: &[&str] = &["```", "代码块", "代码框", "围栏代码"];
    const EN_MARKERS: &[&str] = &["```", "code block", "fenced code"];
    ZH_MARKERS.iter().any(|marker| text.contains(marker))
        || EN_MARKERS.iter().any(|marker| normalized.contains(marker))
}


use crate::chat::types::WebSearchMode;
impl WebSearchMode {
    /// Antigravity OAuth cannot use hosted Google Search. Preserve explicit Off.
    pub fn for_provider(self, provider: &crate::settings::ModelProvider) -> Self {
        if self == Self::Builtin && crate::provider_oauth::antigravity::is_provider(provider) {
            Self::ThirdParty
        } else {
            self
        }
    }

    /// 解析会话的**有效**联网搜索模式（任务 07-23）。
    /// 会话未显式设置（`None`）时回退全局 `native_tools.web_search`：
    /// on ⇒ `ThirdParty`，off ⇒ `Off`，保证旧对话行为逐字节不变。
    pub fn resolve(
        conv_mode: Option<WebSearchMode>,
        settings: &crate::settings::Settings,
    ) -> WebSearchMode {
        conv_mode.unwrap_or({
            if settings.chat_tools.native_tools.web_search {
                WebSearchMode::ThirdParty
            } else {
                WebSearchMode::Off
            }
        })
    }
}
