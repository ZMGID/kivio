use crate::chat::types::{AdditionalDirectory, ChatMessage};
use crate::external_agents::skill_stage::{with_skill_root_preamble, SKILLS_CWD_ALIAS};
use crate::external_agents::types::RuntimeAgentDef;

#[derive(Clone)]
pub struct ComposedExternalPrompt {
    pub full_prompt: String,
    pub instructions_block: String,
}

pub fn is_cli_slash_input(content: &str) -> bool {
    content.trim_start().starts_with('/')
}

/// 该 CLI 的会话级系统指令是否走**启动 flag** 而不是 prompt 正文。
///
/// 目前只有 claude：`--append-system-prompt-file <path>`（隐藏 flag，`--help` 里没有；
/// claude 2.1.220 本机零副作用探针确认存在——不给值时报的是
/// `option '--append-system-prompt-file <file>' argument missing`，而不是 `unknown option`）。
/// 语义是「追加到 claude 原生系统提示之后」，不替换。
///
/// 为什么必须改走 flag：塞进正文的那条消息会被 CLI 自己的上下文压缩摘要掉甚至丢弃，
/// 而 `skip_instructions`（内容没变就不重发）保证了永远不补发 ⇒ 长会话跑一阵子后
/// 用户配置的系统提示与 Memory 静默失效。启动 flag 每次进程启动都重新注入，
/// 与对话历史无关，压缩影响不到。
///
/// 其余 8 个 CLI 仍走正文注入（它们没有等价 flag，或语义不同——audit N5 记着 pi 曾把
/// 目录塞进 `--append-system-prompt`）。给任何 CLI 加这条路之前先按 spec 第 12 条核实语义。
pub fn instructions_via_launch_flag(def: &RuntimeAgentDef) -> bool {
    def.instructions_via_launch_flag
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
/// the conversation history. A turn therefore only ever sends the **latest** user message.
///
/// `daemon_instructions`（全局系统提示 + 集指令 + Memory + cwd 提示）是**会话级常量**，只在首轮拼进正文，
/// resume 轮由 `skip_instructions` 抑制；走启动 flag 的 CLI（claude，见
/// `instructions_via_launch_flag`）由调用方直接传空串，完全不进正文。
///
/// `skill_body` 则**每轮都拼**：active skill 是 per-turn 的选择（用户可以中途换 skill），
/// 被 `skip_instructions` 一起抑制的话，在 resume 轮新激活的 skill 正文根本发不出去。
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
    if !skip_instructions && !daemon_instructions.trim().is_empty() {
        instructions_parts.push(daemon_instructions.trim().to_string());
    }
    if !skill_section.trim().is_empty() {
        instructions_parts.push(skill_section);
    }

    assemble_prompt(instructions_parts, latest_user_message)
}

fn assemble_prompt(
    instructions_parts: Vec<String>,
    latest_user_message: &str,
) -> ComposedExternalPrompt {
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

/// Upper bound for the history carried into a fresh native session after a rewind. Newest
/// messages are kept; the CLI still receives the full latest request.
const REWIND_HISTORY_MAX_CHARS: usize = 60_000;

/// After a rewind, a CLI without native branching starts a fresh native session. This is the one
/// exception to "no history replay" (R3): the visible Kivio history that remains is carried once
/// as text, so the CLI knows what the user still sees and nothing that was removed. Tool calls are
/// represented only by the text around them.
///
/// `messages` is the visible history before the request being sent.
pub fn rewind_history_block(messages: &[ChatMessage]) -> String {
    let mut entries: Vec<String> = messages
        .iter()
        .filter(|message| matches!(message.role.as_str(), "user" | "assistant"))
        .filter(|message| !message.content.trim().is_empty())
        .map(|message| {
            let speaker = if message.role == "user" {
                "User"
            } else {
                "Assistant"
            };
            format!("{speaker}:\n{}", message.content.trim())
        })
        .collect();
    if entries.is_empty() {
        return String::new();
    }
    let mut kept = Vec::new();
    let mut used = 0usize;
    while let Some(entry) = entries.pop() {
        let size = entry.chars().count();
        if used + size > REWIND_HISTORY_MAX_CHARS {
            if kept.is_empty() {
                // Even the newest message alone is over budget: keep its beginning.
                let head: String = entry.chars().take(REWIND_HISTORY_MAX_CHARS).collect();
                kept.push(format!("{head}\n(This message is truncated.)"));
            } else {
                entries.push(entry);
            }
            break;
        }
        used += size;
        kept.push(entry);
    }
    kept.reverse();
    let omitted = if entries.is_empty() {
        ""
    } else {
        "(Earlier messages are omitted.)\n\n"
    };
    format!(
        "## Conversation so far\n\nThe user rewound this conversation, so it continues in a new session. \
         This is the conversation that remains; treat anything else you may recall as removed.\n\n{omitted}{}",
        kept.join("\n\n")
    )
}

/// Compose the first prompt of a fresh native session that replaces a rewound one.
pub fn compose_external_prompt_with_history(
    composed: ComposedExternalPrompt,
    history_block: &str,
    latest_user_message: &str,
) -> ComposedExternalPrompt {
    if history_block.trim().is_empty() {
        return composed;
    }
    let mut parts = Vec::new();
    if !composed.instructions_block.trim().is_empty() {
        parts.push(composed.instructions_block);
    }
    parts.push(history_block.to_string());
    assemble_prompt(parts, latest_user_message)
}

pub fn cwd_hint(cwd: &str, additional_directories: &[AdditionalDirectory]) -> String {
    let mut text = format!(
        "Your working directory is `{cwd}`. Active skill files may appear under `{SKILLS_CWD_ALIAS}/`."
    );
    if !additional_directories.is_empty() {
        text.push_str(
            " Additional directories attached to this conversation (not the working directory — use these absolute paths; relative paths still resolve in the working directory):",
        );
        for directory in additional_directories {
            text.push_str(&format!(
                "\n- {} — `{}`",
                directory.display_name(),
                directory.path
            ));
        }
    }
    text
}

/// Session-level instructions for an external CLI: global system prompt, live
/// set instructions, Memory, then cwd hint. Empty pieces are omitted; cwd is
/// always present. Callers skip the first three for slash passthrough by
/// passing empty strings / `None`.
pub fn build_external_daemon_instructions(
    global_system_prompt: &str,
    set_system_prompt: Option<&str>,
    memory_body: &str,
    cwd: &str,
    additional_directories: &[AdditionalDirectory],
) -> String {
    let mut daemon_instructions = String::new();
    if !global_system_prompt.trim().is_empty() {
        daemon_instructions.push_str(global_system_prompt.trim());
        daemon_instructions.push_str("\n\n");
    }
    if let Some(set_prompt) = set_system_prompt.map(str::trim).filter(|s| !s.is_empty()) {
        daemon_instructions.push_str("## Set instructions\n\n");
        daemon_instructions.push_str(set_prompt);
        daemon_instructions.push_str("\n\n");
    }
    if !memory_body.trim().is_empty() {
        daemon_instructions.push_str("## Memory\n\n");
        daemon_instructions.push_str(memory_body.trim());
        daemon_instructions.push('\n');
    }
    daemon_instructions.push_str(&cwd_hint(cwd, additional_directories));
    daemon_instructions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(role: &str, content: &str) -> ChatMessage {
        serde_json::from_value(serde_json::json!({
            "id": format!("{role}-{content}"),
            "role": role,
            "content": content,
            "timestamp": 0,
        }))
        .unwrap()
    }

    #[test]
    fn rewind_history_keeps_the_visible_turns_before_the_request() {
        let history = rewind_history_block(&[
            message("user", "1"),
            message("assistant", "noted 1"),
            message("user", "2"),
            message("assistant", " "),
        ]);
        assert!(history.contains("User:\n1\n\nAssistant:\nnoted 1\n\nUser:\n2"));
        assert!(!history.contains("omitted"));
        assert!(rewind_history_block(&[]).is_empty());

        let composed = compose_external_prompt_with_history(
            compose_external_prompt("system rules", None, None, None, false, "3"),
            &history,
            "3",
        );
        // The request stays last so native prompt matching still sees only "3".
        assert!(composed.full_prompt.starts_with("# Instructions"));
        assert!(composed.full_prompt.ends_with("# User request\n\n3"));
        assert!(composed.full_prompt.contains("system rules"));
        assert!(crate::external_agents::session::native_prompt_matches(
            &composed.full_prompt,
            "3"
        ));
    }

    #[test]
    fn rewind_history_is_bounded_and_keeps_the_newest_messages() {
        let long = "x".repeat(REWIND_HISTORY_MAX_CHARS / 2);
        let history = rewind_history_block(&[
            message("user", &format!("oldest {long}")),
            message("user", &format!("middle {long}")),
            message("user", "newest"),
        ]);
        assert!(history.contains("(Earlier messages are omitted.)"));
        assert!(!history.contains("oldest"));
        assert!(history.contains("middle"));
        assert!(history.ends_with("User:\nnewest"));

        let huge = rewind_history_block(&[
            message("user", "before"),
            message("assistant", &"y".repeat(REWIND_HISTORY_MAX_CHARS * 2)),
        ]);
        assert!(huge.contains("(This message is truncated.)"));
        assert!(huge.contains("(Earlier messages are omitted.)"));
        assert!(huge.chars().count() < REWIND_HISTORY_MAX_CHARS + 1_000);
    }

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
        // skip_instructions=true（resume 轮：CLI 已持有历史与会话级系统指令）→ 会话级
        // instructions 不再重发，只发裸的最新消息。
        let composed =
            compose_external_prompt("system rules", None, None, None, true, "  follow up  ");
        assert_eq!(composed.full_prompt, "follow up");
        assert!(composed.instructions_block.is_empty());
        assert!(!composed.full_prompt.contains("# Instructions"));
        assert!(!composed.full_prompt.contains("# User request"));
    }

    /// **skill 正文每轮都要发**：active skill 是 per-turn 的选择（用户可以中途换 skill），
    /// 被 `skip_instructions` 一起抑制的话，resume 轮新激活的 skill 正文根本发不出去。
    #[test]
    fn compose_resume_turn_still_carries_the_active_skill_body() {
        let composed = compose_external_prompt(
            "system rules",
            Some("skill body"),
            None,
            None,
            true,
            "follow up",
        );
        assert!(composed.full_prompt.contains("skill body"));
        assert!(composed.full_prompt.contains("follow up"));
        // 会话级 instructions 仍然被抑制（那才是 skip_instructions 的目的）。
        assert!(!composed.full_prompt.contains("system rules"));
    }

    /// 走启动 flag 的 CLI（claude）由调用方传空 instructions ⇒ 正文里一个字都不带，
    /// 但 skill 正文照旧。
    #[test]
    fn compose_with_launch_flag_instructions_keeps_body_clean() {
        let composed =
            compose_external_prompt("", Some("skill body"), None, None, false, "question");
        assert!(composed.full_prompt.contains("skill body"));
        assert!(composed.full_prompt.contains("question"));
        assert_eq!(composed.instructions_block, "skill body");
    }

    #[test]
    fn instructions_via_launch_flag_only_claude() {
        let def = crate::external_agents::registry::get_agent_def("claude").expect("claude def");
        assert!(instructions_via_launch_flag(def));
        for other in [
            "codex", "pi", "kimi", "opencode", "grok", "gemini", "hermes",
        ] {
            let def = crate::external_agents::registry::get_agent_def(other).expect("agent def");
            assert!(
                !instructions_via_launch_flag(def),
                "{other} 没有核实过等价 flag，必须仍走正文注入（spec 第 12 条）"
            );
        }
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

    #[test]
    fn daemon_instructions_include_live_set_prompt() {
        let instructions = build_external_daemon_instructions(
            "global identity",
            Some("Always answer in 文言文."),
            "remember the vault path",
            "/tmp/work",
            &[],
        );
        assert!(instructions.contains("global identity"), "{instructions}");
        assert!(
            instructions.contains("## Set instructions\n\nAlways answer in 文言文."),
            "{instructions}"
        );
        assert!(
            instructions.contains("## Memory\n\nremember the vault path"),
            "{instructions}"
        );
        assert!(instructions.contains("`/tmp/work`"), "{instructions}");
        let set_at = instructions.find("## Set instructions").expect("set");
        let memory_at = instructions.find("## Memory").expect("memory");
        assert!(set_at < memory_at, "set instructions must precede Memory");
    }

    #[test]
    fn daemon_instructions_omit_blank_set_and_global_prompt() {
        let instructions =
            build_external_daemon_instructions("  ", Some("   "), "", "/home/me/proj", &[]);
        assert!(
            !instructions.contains("## Set instructions"),
            "{instructions}"
        );
        assert!(!instructions.contains("## Memory"), "{instructions}");
        assert_eq!(instructions, cwd_hint("/home/me/proj", &[]));
    }

    #[test]
    fn daemon_instructions_list_additional_directories() {
        let extra = [AdditionalDirectory {
            path: "/home/me/biz-a".to_string(),
            name: Some("biz-a".to_string()),
        }];
        let instructions =
            build_external_daemon_instructions("", None, "", "/home/me/proj", &extra);
        assert!(instructions.contains("`/home/me/proj`"), "{instructions}");
        assert!(instructions.contains("`/home/me/biz-a`"), "{instructions}");
        assert!(instructions.contains("biz-a"), "{instructions}");
    }
}
