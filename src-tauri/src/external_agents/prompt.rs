use crate::chat::Conversation;
use crate::external_agents::skill_stage::{with_skill_root_preamble, SKILLS_CWD_ALIAS};

pub struct ComposedExternalPrompt {
    pub full_prompt: String,
    pub instructions_block: String,
}

pub fn is_cli_slash_input(content: &str) -> bool {
    content.trim_start().starts_with('/')
}

pub fn compose_external_prompt_passthrough(latest_user_message: &str) -> ComposedExternalPrompt {
    ComposedExternalPrompt {
        full_prompt: latest_user_message.trim().to_string(),
        instructions_block: String::new(),
    }
}

pub fn compose_external_prompt(
    conversation: &Conversation,
    daemon_instructions: &str,
    skill_body: Option<&str>,
    skill_dir: Option<&str>,
    skill_folder: Option<&str>,
    skip_instructions: bool,
    skip_transcript: bool,
    latest_user_message: &str,
) -> ComposedExternalPrompt {
    let skill_section = match (skill_body, skill_dir, skill_folder) {
        (Some(body), Some(dir), Some(folder)) => with_skill_root_preamble(body, dir, folder),
        (Some(body), _, _) => body.to_string(),
        _ => String::new(),
    };

    let mut instructions_parts = Vec::new();
    if !skip_instructions {
        if !daemon_instructions.trim().is_empty() {
            instructions_parts.push(daemon_instructions.trim().to_string());
        }
        if !skill_section.trim().is_empty() {
            instructions_parts.push(skill_section);
        }
    }

    let instructions_block = instructions_parts.join("\n\n---\n\n");

    let transcript = if skip_transcript {
        String::new()
    } else {
        build_transcript(conversation)
    };

    let mut full = String::new();
    if !instructions_block.is_empty() {
        full.push_str("# Instructions (read first)\n\n");
        full.push_str(&instructions_block);
        full.push_str("\n\n---\n\n");
    }
    full.push_str("# User request\n\n");
    if !transcript.is_empty() {
        full.push_str(&transcript);
        full.push('\n');
    }
    full.push_str(latest_user_message.trim());

    ComposedExternalPrompt {
        full_prompt: full,
        instructions_block,
    }
}

fn build_transcript(conversation: &Conversation) -> String {
    // 末条 user 消息由 `# User request` 尾部的 latest 唯一承载,transcript 必须排除它,
    // 否则同一条消息在 prompt 中出现两次(缺陷 1 / C1 / N9)。按索引精确定位最后一条
    // user——而非按文本匹配——避免用户连续两轮发相同文本时把历史里的旧消息误跳。
    let skip_idx = conversation.messages.iter().rposition(|m| m.role == "user");
    let mut lines = Vec::new();
    for (idx, message) in conversation.messages.iter().enumerate() {
        if Some(idx) == skip_idx {
            continue;
        }
        let role = message.role.as_str();
        let label = match role {
            "user" => "user",
            "assistant" => "assistant",
            _ => continue,
        };
        let text = message.content.trim();
        if text.is_empty() {
            continue;
        }
        lines.push(format!("## {label}\n{text}"));
    }
    lines.join("\n\n")
}

pub fn cwd_hint(cwd: &str) -> String {
    format!(
        "Your working directory is `{cwd}`. Active skill files may appear under `{SKILLS_CWD_ALIAS}/`."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::types::{
        AgentPlanState, AgentRuntimeConfig, AgentTodoState, ChatMessage, Conversation,
        ConversationContextState,
    };

    fn msg(role: &str, content: &str) -> ChatMessage {
        serde_json::from_value(serde_json::json!({
            "id": format!("m-{role}-{content}"),
            "role": role,
            "content": content,
            "timestamp": 0,
        }))
        .unwrap()
    }

    fn empty_conversation() -> Conversation {
        Conversation {
            id: "c1".to_string(),
            title: "t".to_string(),
            provider_id: "p".to_string(),
            model: "m".to_string(),
            messages: vec![],
            agent_runtime: AgentRuntimeConfig::default(),
            active_skill_id: None,
            assistant_id: None,
            assistant_snapshot: None,
            created_at: 0,
            updated_at: 0,
            pinned: false,
            folder: None,
            project_id: None,
            set_id: None,
            context_state: ConversationContextState::default(),
            agent_todo_state: AgentTodoState::default(),
            agent_plan_state: AgentPlanState::default(),
            knowledge_base_ids: Vec::new(),
            force_knowledge_search: false,
            thinking_level: None,
            reply_models: Vec::new(),
            group_selections: std::collections::HashMap::new(),
            forked_from: None,
        }
    }

    #[test]
    fn compose_includes_instructions_and_user_request() {
        let conv = empty_conversation();
        let composed = compose_external_prompt(
            &conv,
            "system rules",
            Some("skill body"),
            Some("/skills/x"),
            Some("x-abc"),
            false,
            true,
            "hello",
        );
        assert!(composed.full_prompt.contains("# Instructions"));
        assert!(composed.full_prompt.contains("skill body"));
        assert!(composed.full_prompt.contains("hello"));
    }

    #[test]
    fn transcript_excludes_last_user_so_latest_appears_once() {
        // 非空会话:2 user + 1 assistant,末条 user 即 latest。skip_transcript=false 时
        // build_transcript 参与,末条文本必须只在尾部出现一次,历史消息保留。
        let mut conv = empty_conversation();
        conv.messages = vec![
            msg("user", "first question"),
            msg("assistant", "an answer"),
            msg("user", "latest question"),
        ];
        let composed = compose_external_prompt(
            &conv,
            "system rules",
            None,
            None,
            None,
            false,
            false,
            "latest question",
        );
        assert_eq!(
            composed.full_prompt.matches("latest question").count(),
            1,
            "latest user message must appear exactly once, got: {}",
            composed.full_prompt
        );
        // 历史仍在。
        assert!(composed.full_prompt.contains("first question"));
        assert!(composed.full_prompt.contains("an answer"));
    }

    #[test]
    fn transcript_skips_last_user_even_when_regenerate_tail_is_assistant() {
        // regenerate 形态:消息列表末位是 assistant,但最后一条 user 在其之前。
        // rposition 精确定位末条 user 并跳过,latest 仍由尾部唯一承载。
        let mut conv = empty_conversation();
        conv.messages = vec![
            msg("user", "earlier turn"),
            msg("assistant", "earlier reply"),
            msg("user", "regen target"),
            msg("assistant", "stale reply being regenerated"),
        ];
        let composed =
            compose_external_prompt(&conv, "sys", None, None, None, false, false, "regen target");
        assert_eq!(
            composed.full_prompt.matches("regen target").count(),
            1,
            "the last user message must not be duplicated: {}",
            composed.full_prompt
        );
        assert!(composed.full_prompt.contains("earlier turn"));
        assert!(composed.full_prompt.contains("earlier reply"));
        assert!(composed
            .full_prompt
            .contains("stale reply being regenerated"));
    }

    #[test]
    fn is_cli_slash_input_detects_leading_slash() {
        assert!(is_cli_slash_input("/compact"));
        assert!(is_cli_slash_input("  /model gpt-5"));
        assert!(!is_cli_slash_input("hello /compact"));
        assert!(!is_cli_slash_input("plain text"));
    }

    #[test]
    fn passthrough_prompt_is_raw_slash_without_wrapper() {
        let composed = compose_external_prompt_passthrough("  /model gpt-5  ");
        assert_eq!(composed.full_prompt, "/model gpt-5");
        assert!(composed.instructions_block.is_empty());
        assert!(!composed.full_prompt.contains("# Instructions"));
    }
}
