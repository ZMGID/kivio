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

/// Compose the prompt for one external-CLI turn.
///
/// History replay is abolished (R3): every external CLI now has a native session (claude
/// `--resume` / codex thread / ACP `session/load` / pi `--session-id`), so the CLI itself holds
/// the conversation history. A turn therefore only ever sends the **latest** user message. The
/// first turn of a session additionally prepends the instructions block (system prompt + memory +
/// active skill) so the CLI has the system directives; on a resume turn (`skip_instructions`) the
/// message is sent bare — the CLI already has both the history and the instructions.
pub fn compose_external_prompt(
    daemon_instructions: &str,
    skill_body: Option<&str>,
    skill_dir: Option<&str>,
    skill_folder: Option<&str>,
    skip_instructions: bool,
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

    let mut full = String::new();
    if !instructions_block.is_empty() {
        full.push_str("# Instructions (read first)\n\n");
        full.push_str(&instructions_block);
        full.push_str("\n\n---\n\n");
        full.push_str("# User request\n\n");
    }
    full.push_str(latest_user_message.trim());

    ComposedExternalPrompt {
        full_prompt: full,
        instructions_block,
    }
}

pub fn cwd_hint(cwd: &str) -> String {
    format!(
        "Your working directory is `{cwd}`. Active skill files may appear under `{SKILLS_CWD_ALIAS}/`."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_includes_instructions_and_user_request() {
        let composed = compose_external_prompt(
            "system rules",
            Some("skill body"),
            Some("/skills/x"),
            Some("x-abc"),
            false,
            "hello",
        );
        assert!(composed.full_prompt.contains("# Instructions"));
        assert!(composed.full_prompt.contains("skill body"));
        assert!(composed.full_prompt.contains("hello"));
    }

    #[test]
    fn compose_first_turn_sends_only_latest_no_history() {
        // 历史重放已废除（R3）：compose 不再接收会话历史，prompt 只含最新一条消息 +（首轮）
        // instructions。断言 prompt 里不含任何历史 transcript 结构（`## user` / `## assistant`）。
        let composed =
            compose_external_prompt("system rules", None, None, None, false, "latest question");
        assert_eq!(
            composed.full_prompt.matches("latest question").count(),
            1,
            "latest user message must appear exactly once: {}",
            composed.full_prompt
        );
        assert!(!composed.full_prompt.contains("## user"));
        assert!(!composed.full_prompt.contains("## assistant"));
        assert!(composed.full_prompt.contains("# User request"));
    }

    #[test]
    fn compose_resume_turn_is_bare_latest_message() {
        // skip_instructions=true（resume 轮：CLI 已持有历史与系统指令）→ 只发裸的最新消息，
        // 无 instructions / User request 包裹。
        let composed = compose_external_prompt(
            "system rules",
            Some("skill body"),
            None,
            None,
            true,
            "  follow up  ",
        );
        assert_eq!(composed.full_prompt, "follow up");
        assert!(composed.instructions_block.is_empty());
        assert!(!composed.full_prompt.contains("# Instructions"));
        assert!(!composed.full_prompt.contains("# User request"));
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
