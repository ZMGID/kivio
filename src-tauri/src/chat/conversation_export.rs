//! Shared Markdown rendering for the original and native desktop.
use crate::chat::conversation_data::{
    Attachment, ChatMessage, ChatMessageSegmentKind, ChatMessageSegmentPhase, Conversation,
};
use chrono::{Local, TimeZone};

#[derive(Clone, Copy)]
struct ExportLabels {
    created: &'static str,
    updated: &'static str,
    model: &'static str,
    user: &'static str,
    assistant: &'static str,
    image: &'static str,
    attachment: &'static str,
}

fn labels(language: &str) -> ExportLabels {
    if language.eq_ignore_ascii_case("en") {
        ExportLabels {
            created: "Created",
            updated: "Updated",
            model: "Model",
            user: "User",
            assistant: "Assistant",
            image: "Image",
            attachment: "Attachment",
        }
    } else {
        ExportLabels {
            created: "创建时间",
            updated: "更新时间",
            model: "模型",
            user: "用户",
            assistant: "助手",
            image: "图片",
            attachment: "附件",
        }
    }
}

fn format_timestamp(timestamp: i64) -> String {
    Local
        .timestamp_opt(timestamp, 0)
        .single()
        .map(|value| value.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| timestamp.to_string())
}

fn markdown_title(title: &str) -> String {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        "Conversation".to_string()
    } else {
        title
    }
}

fn attachment_placeholder(attachment: &Attachment, labels: ExportLabels) -> String {
    let kind = if attachment.attachment_type == "image" {
        labels.image
    } else {
        labels.attachment
    };
    let name = attachment.name.replace(['\r', '\n'], " ");
    if name.trim().is_empty() {
        format!("[{kind}]")
    } else {
        format!("[{kind}: {name}]")
    }
}

/// Display/export projection, independent of stored content and model replay.
/// Kept in sync with messageBody.ts through tests/fixtures/message-body.json.
fn message_body_text(message: &ChatMessage) -> String {
    if message.role != "assistant" {
        return message.content.clone();
    }
    let mut segments: Vec<_> = message.segments.iter().collect();
    segments.sort_by_key(|segment| segment.order);
    let mut parts: Vec<&str> = Vec::new();
    let mut raw_parts: Vec<&str> = Vec::new();
    let mut final_parts: Vec<&str> = Vec::new();
    for segment in segments {
        if segment.kind != ChatMessageSegmentKind::Text {
            continue;
        }
        raw_parts.push(segment.text.as_deref().unwrap_or_default());
        let text = segment.text.as_deref().unwrap_or_default().trim();
        if text.is_empty() {
            continue;
        }
        // Only normalize_assistant_segments' known fallback can be a duplicate;
        // identical independently generated text must still be retained.
        let synthesized = segment
            .id
            .strip_prefix("seg_")
            .and_then(|id| id.strip_suffix("_synthesis_text"))
            .is_some_and(|order| !order.is_empty() && order.bytes().all(|b| b.is_ascii_digit()))
            && matches!(
                segment.phase,
                ChatMessageSegmentPhase::Plain | ChatMessageSegmentPhase::Synthesis
            );
        if synthesized && text == parts.join("\n\n") {
            continue;
        }
        parts.push(text);
        if matches!(
            segment.phase,
            ChatMessageSegmentPhase::Plain | ChatMessageSegmentPhase::Synthesis
        ) {
            final_parts.push(text);
        }
    }
    let content = message.content.trim();
    let is_content_mirror = parts.contains(&content)
        || content == parts.join("\n\n")
        || content == raw_parts.join("").trim()
        || content == final_parts.join("\n\n");
    if !content.is_empty() && !is_content_mirror {
        parts.push(content);
    }
    parts.join("\n\n")
}

fn render_message(message: &ChatMessage, labels: ExportLabels) -> Option<String> {
    let body = message_body_text(message);
    let body = body.trim();
    if body.is_empty() && message.attachments.is_empty() {
        return None;
    }

    let role = if message.role == "user" {
        labels.user
    } else if message.role == "assistant" {
        labels.assistant
    } else {
        return None;
    };

    let mut section = format!("**{role}** · {}\n", format_timestamp(message.timestamp));
    if !body.is_empty() {
        section.push('\n');
        section.push_str(body);
        section.push('\n');
    }
    for attachment in &message.attachments {
        section.push('\n');
        section.push_str(&attachment_placeholder(attachment, labels));
        section.push('\n');
        // 虚拟文本附件（memory://）没有磁盘文件可引用，正文只存在附件记录里：
        // 直接内联渲染，否则导出的对话会永久缺失这部分用户输入。
        if let Some(text) = &attachment.content {
            let body = text.trim_end();
            if !body.is_empty() {
                section.push_str("\n附件正文：\n````text\n");
                section.push_str(body);
                section.push_str("\n````");
            }
        }
    }
    Some(section.trim_end().to_string())
}

pub fn render_conversation_markdown(conversation: &Conversation, language: &str) -> String {
    let labels = labels(language);
    let mut sections = vec![format!(
        "# {}\n\n- {}: {}\n- {}: {}\n- {}: {}",
        markdown_title(&conversation.title),
        labels.created,
        format_timestamp(conversation.created_at),
        labels.updated,
        format_timestamp(conversation.updated_at),
        labels.model,
        conversation.model,
    )];

    sections.extend(
        conversation
            .messages
            .iter()
            .filter_map(|message| render_message(message, labels)),
    );
    format!("{}\n", sections.join("\n\n---\n\n"))
}
