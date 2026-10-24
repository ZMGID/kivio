//! Shared structured file tool results and model-visible summaries.
use crate::native_tools::{FileMutationResult,ReadFileResult};
use super::types::McpToolCallResult;
const FILE_MUTATION_DIFF_MAX_LINES: usize = 80;

pub fn file_mutation_tool_result(result: FileMutationResult) -> Result<McpToolCallResult, String> {
    let summary = result.summary();
    let mut content = summary;
    if !result.warnings.is_empty() {
        content = format!("{}\n{}", content, result.warnings.join("\n"));
    }
    // 成功只回传摘要和警告；完整 diff 仍留在 structured_content 中用于审阅。
    // 失败保留原有诊断回显，不修改调用参数或实际文件内容。
    if !result.ok && !result.diff.trim().is_empty() {
        let lines: Vec<&str> = result.diff.lines().collect();
        if lines.len() > FILE_MUTATION_DIFF_MAX_LINES {
            let clipped = lines[..FILE_MUTATION_DIFF_MAX_LINES].join("\n");
            content = format!(
                "{}\n\n{}\n[... diff clipped: showing first {FILE_MUTATION_DIFF_MAX_LINES} of {} lines ...]",
                content,
                clipped,
                lines.len()
            );
        } else {
            content = format!("{}\n\n{}", content, result.diff);
        }
    }
    let is_error = !result.ok;
    let structured = serde_json::to_value(&result)
        .map_err(|err| format!("Serialize file mutation result failed: {err}"))?;
    Ok(McpToolCallResult {
        content,
        is_error,
        raw: structured.clone(),
        artifacts: Vec::new(),
        structured_content: Some(structured),
        follow_up_user_messages: Vec::new(),
    })
}

pub fn read_file_tool_result(result: ReadFileResult) -> Result<McpToolCallResult, String> {
    // structured_content 保留完整 ReadFileResult 给前端 ToolCallBlock 渲染（不变）。
    let structured = serde_json::to_value(&result)
        .map_err(|err| format!("Serialize read_file result failed: {err}"))?;
    // 模型看到的 content 改成 cat -n 风格（`行号\t内容` + 精简头），行号便于模型引用并构造
    // 后续 edit_file。行号仅供参考、不属于文件内容；read_file/edit_file 描述里已说明
    // old_string 不要带行号前缀。
    let content = format_read_file_for_model(&result);
    Ok(McpToolCallResult {
        content,
        is_error: false,
        raw: structured.clone(),
        artifacts: Vec::new(),
        structured_content: Some(structured),
        follow_up_user_messages: Vec::new(),
    })
}

/// 把 ReadFileResult 渲染成模型友好的 `cat -n` 文本：一行精简元数据头 + `右对齐行号\t原文`，
/// 续读通知贴在**末尾**（对齐 pi）：模型读完一页，最后一眼看到的就是「用 offset=N 继续」，
/// 不会被两千行正文冲掉。没截断就没有尾注。
fn format_read_file_for_model(result: &ReadFileResult) -> String {
    let mut out = format!(
        "{} — lines {}-{} of {}",
        result.path, result.start_line, result.end_line, result.total_lines
    );
    if !result.content.is_empty() {
        let start = result.start_line.max(1);
        out.push('\n');
        let numbered: Vec<String> = result
            .content
            .lines()
            .enumerate()
            .map(|(i, line)| format!("{:>6}\t{}", start + i, line))
            .collect();
        out.push_str(&numbered.join("\n"));
    }
    let footer = read_continuation_footer(result);
    if !footer.is_empty() {
        out.push_str("\n\n");
        out.push_str(&footer);
    }
    out
}

/// 尾注正文：优先用 `read_file` 产出的通知（已含触顶原因）；截断了却没通知时兜底给一句
/// 最简的「用 offset=N 继续」，保证「截断必有下一步」这条契约不依赖上游。
fn read_continuation_footer(result: &ReadFileResult) -> String {
    if !result.warnings.is_empty() {
        return result.warnings.join("\n");
    }
    match (result.truncated, result.next_offset) {
        (true, Some(next)) => format!("[More lines in file. Use offset={next} to continue.]"),
        (true, None) => "[Output truncated.]".to_string(),
        (false, _) => String::new(),
    }
}
