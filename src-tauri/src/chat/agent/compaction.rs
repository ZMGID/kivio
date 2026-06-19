use serde_json::{json, Value};

use crate::chat::model_metadata::{context_window_for_model, safe_context_window_for_model};

use super::loop_::{LoopEnv, RunState};
use super::planning::call_chat_completion_message_streamed;
use super::prepare::estimate_tokens;

/// 近期窗口（tokens）：从尾部往前累积整条消息，~该预算内的为受保护近期窗口、原样保留，
/// 其余旧段才进摘要。取代旧的固定 `KEEP_RECENT_RAW_MESSAGES` 条数（R7）。对齐 OpenCode
/// `DEFAULT_KEEP_TOKENS = 8_000`。
pub(crate) const RECENT_KEEP_TOKENS: usize = 8_000;
/// 估算占用超过窗口的该比例才触发压缩。
pub(crate) const COMPACT_TRIGGER_RATIO: f32 = 0.85;
/// 序列化喂给摘要模型时，单条 `[Tool result]` / `[Tool error]` 的字符上限（R5）。
/// 仅截工具输出——用户/助手/推理/工具入参全文保留。对齐 OpenCode `TOOL_OUTPUT_MAX_CHARS = 2_000`。
const TOOL_OUTPUT_SUMMARY_MAX_CHARS: usize = 2_000;
/// 摘要调用允许产生的最大输出 token 数（R9）。对齐 OpenCode `SUMMARY_OUTPUT_TOKENS = 4_096`。
const SUMMARY_OUTPUT_TOKENS: u32 = 4_096;

/// 摘要**输入** token 预算占窗口的比例（R1）。摘要请求自身**绝不能**超窗——否则就是
/// "用超窗的请求去救超窗"，每次压缩都失败、降级返回原始视图，最终主调用仍超窗报错。
/// 取保守的 0.5 而非更高比例的理由：
/// - 模型窗口元数据常偏乐观（实测标称 128k 的模型真实可用 ~100k）；
/// - Responses API 等有额外 token 计数开销（工具 schema、system、role 标注）；
/// - 近期 8k 窗口本就在摘要请求之外逐字保留，预算只需覆盖旧段摘要本身。
///
/// 0.5×window 给足余量，保证摘要请求恒定放得进窗口。
const SUMMARY_INPUT_BUDGET_RATIO: f32 = 0.5;

/// 当窗口未知（`window == 0`）时，摘要输入预算的兜底值（tokens）。
/// 取一个保守常见窗口的一半，既不跳过封顶又不会过度裁剪。
const SUMMARY_INPUT_BUDGET_FALLBACK_TOKENS: usize = 64_000;

/// 头尾裁剪时插入到中段的省略标记——告知摘要模型此处有更早历史被省略以放进摘要请求。
const HEAD_TAIL_OMISSION_MARKER: &str =
    "\n\n[... older history omitted to fit the summary request ...]\n\n";

/// 头尾裁剪保留预算偏向尾部的比例：头 ~40% / 尾 ~60%。近期工作（尾部）比早期意图更关键，
/// 但开头的任务目标/早期意图也需保留，故仍给头部留 ~40%。
const HEAD_BUDGET_FRACTION: f32 = 0.4;

/// 由 `replace_with_summary` 插入的摘要锚点前缀；anchored 链式摘要（R8）据此识别历史里已存在的
/// 上一份摘要，把它作为 `previous_summary` 让模型合并更新，而非从头再摘。
const SUMMARY_MARKER_PREFIX: &str = "[context summary]";

/// 摘要模型调用的 system prompt（R6，逐字对齐 Claude Code 的 `qH1`/`AU2` 流程）。
const SUMMARY_SYSTEM_PROMPT: &str =
    "You are a helpful AI assistant tasked with summarizing conversations.";

/// Claude Code 的 9 段结构化摘要 prompt（R6，verbatim 自 research/claude-code-compaction.md §3）。
/// 作为最后一条 user 指令追加在序列化后的对话历史之后；模型先在 `<analysis>` 里链式分析每条消息，
/// 再在 `<summary>` 里产出 9 段结构化摘要。安全约束/用户原话/next-step 逐字保留条款保留。
pub(crate) const CLAUDE_CODE_SUMMARY_PROMPT: &str = "Your task is to create a detailed summary of the conversation so far, paying close attention to the user's explicit requests and your previous actions.
This summary should be thorough in capturing technical details, code patterns, and architectural decisions that would be essential for continuing development work without losing context.

Before providing your final summary, wrap your analysis in <analysis> tags to organize your thoughts and ensure you've covered all necessary points. In your analysis process:

1. Chronologically analyze each message and section of the conversation. For each section thoroughly identify:
   - The user's explicit requests and intents
   - Your approach to addressing the user's requests
   - Key decisions, technical concepts and code patterns
   - Specific details like:
     - file names
     - full code snippets
     - function signatures
     - file edits
   - Errors that you ran into and how you fixed them
   - Pay special attention to specific user feedback that you received, especially if the user told you to do something differently.
   - Note any security-relevant instructions or constraints the user stated (e.g., sensitive files or data to avoid, operations that must not be performed, credential or secret handling rules). These MUST be preserved verbatim in the summary so they continue to apply after compaction.
2. Double-check for technical accuracy and completeness, addressing each required element thoroughly.

Your summary should include the following sections:

1. Primary Request and Intent: Capture all of the user's explicit requests and intents in detail
2. Key Technical Concepts: List all important technical concepts, technologies, and frameworks discussed.
3. Files and Code Sections: Enumerate specific files and code sections examined, modified, or created. Pay special attention to the most recent messages and include full code snippets where applicable and include a summary of why this file read or edit is important.
4. Errors and fixes: List all errors that you ran into, and how you fixed them. Pay special attention to specific user feedback that you received, especially if the user told you to do something differently.
5. Problem Solving: Document problems solved and any ongoing troubleshooting efforts.
6. All user messages: List ALL user messages that are not tool results. These are critical for understanding the users' feedback and changing intent. Preserve any security-relevant instructions or constraints verbatim so they remain in effect after compaction.
7. Pending Tasks: Outline any pending tasks that you have explicitly been asked to work on.
8. Current Work: Describe in detail precisely what was being worked on immediately before this summary request, paying special attention to the most recent messages from both user and assistant. Include file names and code snippets where applicable.
9. Optional Next Step: List the next step that you will take that is related to the most recent work you were doing. IMPORTANT: ensure that this step is DIRECTLY in line with the user's most recent explicit requests, and the task you were working on immediately before this summary request. If your last task was concluded, then only list next steps if they are explicitly in line with the users request. Do not start on tangential requests or really old requests that were already completed without confirming with the user first.
                       If there is a next step, include direct quotes from the most recent conversation showing exactly what task you were working on and where you left off. This should be verbatim to ensure there's no drift in task interpretation.

Here is an example of how your output should be structured:

<example>
<analysis>
[Your thought process, ensuring all points are covered thoroughly and accurately]
</analysis>

<summary>
1. Primary Request and Intent:
   [Detailed description]

2. Key Technical Concepts:
   - [Concept 1]
   - [Concept 2]
   - [...]

3. Files and Code Sections:
   - [File Name 1]
      - [Summary of why this file is important]
      - [Summary of the changes made to this file, if any]
      - [Important Code Snippet]
   - [File Name 2]
      - [Important Code Snippet]
   - [...]

4. Errors and fixes:
    - [Detailed description of error 1]:
      - [How you fixed the error]
      - [User feedback on the error if any]
    - [...]

5. Problem Solving:
   [Description of solved problems and ongoing troubleshooting]

6. All user messages:
    - [Detailed non tool use user message]
    - [...]

7. Pending Tasks:
   - [Task 1]
   - [Task 2]
   - [...]

8. Current Work:
   [Precise description of current work]

9. Optional Next Step:
   [Optional Next step to take]

</summary>
</example>

There may be additional summarization instructions provided in the included context. If so, remember to follow these instructions when creating the above summary. Examples of instructions include:
<example>
## Compact Instructions
When summarizing the conversation focus on typescript code changes and also remember the mistakes you made and how you fixed them.
</example>
<example>
# Summary instructions
When you are using compact - please focus on test output and code changes. Include file reads verbatim.
</example>";

/// 估算消息序列的 token 数：逐条把 content 字符串（以及非字符串 content / tool_calls
/// 等结构化字段的 JSON 序列化）喂给 chars 启发式累加。
pub(crate) fn estimate_messages_tokens(messages: &[Value]) -> usize {
    messages.iter().map(estimate_message_tokens).sum()
}

/// 单条消息的 token 估算（与 `estimate_messages_tokens` 的逐条逻辑一致，供近期窗口选取复用）。
fn estimate_message_tokens(message: &Value) -> usize {
    match message.get("content").and_then(Value::as_str) {
        Some(text) => {
            let extra = message
                .get("tool_calls")
                .map(|calls| estimate_tokens(&calls.to_string()))
                .unwrap_or(0);
            estimate_tokens(text) + extra + 4
        }
        None => estimate_tokens(&message.to_string()),
    }
}

/// 把单条消息渲染成角色标注文本行（R5）。用户/助手/推理/工具入参**全文保留**；仅
/// `[Tool result]` / `[Tool error]` 的内容截到 `TOOL_OUTPUT_SUMMARY_MAX_CHARS`（尾部加 `[truncated]`）。
/// 一条 assistant 消息可能同时带文本 + reasoning + 多个 tool_calls，全部展开为多行。
fn serialize_message(message: &Value) -> String {
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let mut lines: Vec<String> = Vec::new();

    match role {
        "system" => {
            if let Some(text) = message.get("content").and_then(Value::as_str) {
                if !text.trim().is_empty() {
                    lines.push(format!("[System]: {text}"));
                }
            }
        }
        "user" => {
            if let Some(text) = message.get("content").and_then(Value::as_str) {
                lines.push(format!("[User]: {text}"));
            } else if let Some(content) = message.get("content") {
                // 非字符串 content（如多模态 parts）——退回 JSON，保持信息不丢。
                lines.push(format!("[User]: {content}"));
            }
        }
        "assistant" => {
            if let Some(text) = message.get("content").and_then(Value::as_str) {
                if !text.trim().is_empty() {
                    lines.push(format!("[Assistant]: {text}"));
                }
            }
            if let Some(reasoning) = message.get("reasoning_content").and_then(Value::as_str) {
                if !reasoning.trim().is_empty() {
                    lines.push(format!("[Assistant reasoning]: {reasoning}"));
                }
            }
            if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
                for call in calls {
                    let function = call.get("function");
                    let name = function
                        .and_then(|f| f.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    let args = function
                        .and_then(|f| f.get("arguments"))
                        .map(|a| match a.as_str() {
                            Some(s) => s.to_string(),
                            None => a.to_string(),
                        })
                        .unwrap_or_default();
                    // 工具入参全文保留（不截断）。
                    lines.push(format!("[Assistant tool call]: {name}({args})"));
                }
            }
        }
        "tool" => {
            let content = match message.get("content").and_then(Value::as_str) {
                Some(text) => text.to_string(),
                None => message
                    .get("content")
                    .map(Value::to_string)
                    .unwrap_or_default(),
            };
            let is_error = message
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let clipped = clip_tool_output(&content);
            if is_error {
                lines.push(format!("[Tool error]: {clipped}"));
            } else {
                lines.push(format!("[Tool result]: {clipped}"));
            }
        }
        other => {
            // 未知角色：退回 JSON，保证不丢信息（极罕见）。
            lines.push(format!("[{other}]: {message}"));
        }
    }

    lines.join("\n")
}

/// `[Tool result]` / `[Tool error]` 的内容截断到 `TOOL_OUTPUT_SUMMARY_MAX_CHARS`，
/// 超出时尾部加 `[truncated]` 标记（复用现有 head-tail 风格的 `truncate_chars`）。
fn clip_tool_output(content: &str) -> String {
    if content.chars().count() <= TOOL_OUTPUT_SUMMARY_MAX_CHARS {
        return content.to_string();
    }
    let head: String = content
        .chars()
        .take(TOOL_OUTPUT_SUMMARY_MAX_CHARS)
        .collect();
    format!("{head}\n[truncated]")
}

/// 把旧段消息序列化成喂给摘要模型的角色标注文本（R5）。每条消息一段，用空行分隔。
fn serialize_for_summary(messages: &[Value]) -> String {
    messages
        .iter()
        .map(serialize_message)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// 把字符串按字符数前缀截取（不切多字节字符）。
fn take_chars(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

/// 把字符串按字符数后缀截取（保留末尾 n 个字符）。
fn take_chars_tail(text: &str, n: usize) -> String {
    let total = text.chars().count();
    if n >= total {
        return text.to_string();
    }
    text.chars().skip(total - n).collect()
}

/// 把序列化后的旧段文本头尾裁剪到 `budget_tokens`（R2）：保留开头（任务目标/早期意图）+
/// 结尾（最近工作），中间替换为 `HEAD_TAIL_OMISSION_MARKER`。偏向保留更多尾部（头 ~40% / 尾 ~60%）。
/// 未超预算则原样返回（R5，零行为变化）。
///
/// 在 token 预算上工作，但裁剪以字符为粒度：用 `estimate_tokens` 的 ASCII≈4 chars/token
/// 启发式把 token 预算换算成字符预算（保守按 4 倍），裁剪后再用 `estimate_tokens` 校验，
/// 若仍超预算则迭代收紧——硬保证返回结果 `estimate_tokens <= budget_tokens`（R2 兜底）。
fn clip_serialized_to_budget(serialized: &str, budget_tokens: usize) -> String {
    if estimate_tokens(serialized) <= budget_tokens {
        return serialized.to_string();
    }
    // 给省略标记留出 token 预算。
    let marker_tokens = estimate_tokens(HEAD_TAIL_OMISSION_MARKER);
    let content_budget = budget_tokens.saturating_sub(marker_tokens);

    // token 预算换算成字符预算：ASCII 约 4 chars/token，按 4 倍作为初始字符预算上限。
    let mut char_budget = content_budget.saturating_mul(4).max(1);

    // 迭代收紧：头尾按比例切，校验 estimate_tokens，超了就收紧字符预算重切。
    loop {
        let head_chars = ((char_budget as f32) * HEAD_BUDGET_FRACTION) as usize;
        let tail_chars = char_budget.saturating_sub(head_chars);
        let head = take_chars(serialized, head_chars);
        let tail = take_chars_tail(serialized, tail_chars);
        let clipped = format!("{head}{HEAD_TAIL_OMISSION_MARKER}{tail}");
        if estimate_tokens(&clipped) <= budget_tokens || char_budget <= 1 {
            return clipped;
        }
        // 仍超预算（多字节字符占 1 token/char 时换算偏乐观）——收紧字符预算重试。
        char_budget = char_budget * 3 / 4;
    }
}

/// 一条 assistant 消息是否携带 tool_calls（其后的 role=="tool" 结果不能与它拆到摘要/保留两侧）。
#[cfg_attr(not(test), allow(dead_code))]
fn has_tool_calls(message: &Value) -> bool {
    message
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|calls| !calls.is_empty())
        .unwrap_or(false)
}

/// 一条消息是否为 tool 结果（role=="tool"）。
fn is_tool_result(message: &Value) -> bool {
    message.get("role").and_then(Value::as_str) == Some("tool")
}

/// 按 token 选取受保护的近期窗口（R7）：在系统前缀之后的消息里，从尾部往前累积**整条**消息的
/// `estimate_message_tokens` 直到 ~`keep_tokens`，这些为原样保留的近期窗口；更早的为旧段、进摘要。
///
/// 约束：
/// - **不切断单条消息**（保 JSON 合法）——按整条累积，越过预算的那条整体归入旧段（除非配对保护）。
/// - **不拆 tool_call↔tool 配对**——若边界落在一条 assistant(tool_calls) 与其后的 tool 结果之间，
///   把成对的一组整体拉进近期窗口（往前移动边界，使旧段不以孤立的 tool 结果开头）。
///
/// 返回 `(system_prefix, old_segment, recent)`：系统前缀 = 开头连续 role=="system"；
/// old_segment = 系统前缀之后、近期窗口之前的旧段；recent = 受保护近期窗口。
fn select_recent_by_tokens(
    messages: &[Value],
    keep_tokens: usize,
) -> (Vec<Value>, Vec<Value>, Vec<Value>) {
    let system_end = messages
        .iter()
        .position(|m| m.get("role").and_then(Value::as_str) != Some("system"))
        .unwrap_or(messages.len());

    // 从尾部往前累积整条消息，直到超过 keep_tokens。`split` 是近期窗口的起始下标（含）。
    let mut total = 0usize;
    let mut split = messages.len();
    let mut idx = messages.len();
    while idx > system_end {
        idx -= 1;
        let next = total + estimate_message_tokens(&messages[idx]);
        if next > keep_tokens && idx + 1 < messages.len() {
            // 越过预算：保留 [idx+1..] 为近期窗口（不切断当前条，当前条归旧段）。
            split = idx + 1;
            break;
        }
        total = next;
        split = idx;
    }

    // 配对保护：若近期窗口以孤立的 tool 结果开头（其 assistant(tool_calls) 落在旧段尾），
    // 把边界往前移，使整组 tool_call↔tool 一起进近期窗口（不拆配对）。
    while split > system_end && is_tool_result(&messages[split]) {
        split -= 1;
    }
    // split 现在指向一条 assistant(tool_calls) 或一条普通消息；若它是 assistant(tool_calls)，
    // 它已被包含进近期窗口，其后续 tool 结果也都在窗口内——配对完整。

    (
        messages[..system_end].to_vec(),
        messages[system_end..split].to_vec(),
        messages[split..].to_vec(),
    )
}

/// 从历史里探测上一份摘要（anchored 链式摘要，R8）：`replace_with_summary` 插入的摘要消息是一条
/// content 以 `SUMMARY_MARKER_PREFIX` 开头的 user 消息。找到则返回其摘要正文（去掉前缀引导语），
/// 供作为 `previous_summary` 让模型合并更新；并把它从将进 head 的旧段里剔除（不重复进 head）。
fn extract_previous_summary(old_segment: &[Value]) -> Option<String> {
    old_segment.iter().find_map(|message| {
        if message.get("role").and_then(Value::as_str) != Some("user") {
            return None;
        }
        let content = message.get("content").and_then(Value::as_str)?;
        let trimmed = content.trim_start();
        if !trimmed.starts_with(SUMMARY_MARKER_PREFIX) {
            return None;
        }
        // 取摘要正文：摘要消息形如 "[context summary] <引导语>：\n<summary>"。
        // 找第一个换行后的内容；无换行则退回整条（去前缀）。
        let body = trimmed
            .split_once('\n')
            .map(|(_, rest)| rest)
            .unwrap_or(trimmed);
        Some(body.trim().to_string())
    })
}

/// 用摘要替换旧段，返回新的消息序列：系统前缀 + summary(user)/ack(assistant) 对 + 尾段。
/// user/assistant 成对插入保证 role 交替对严格 provider 合法。摘要 user 消息以
/// `SUMMARY_MARKER_PREFIX` 开头，供后续轮的 anchored 链式摘要识别。
fn replace_with_summary(system_prefix: Vec<Value>, summary: &str, recent: Vec<Value>) -> Vec<Value> {
    let mut out = system_prefix;
    out.push(json!({
        "role": "user",
        "content": format!(
            "{SUMMARY_MARKER_PREFIX} 以下是本次任务早前对话的压缩摘要（原始消息已省略以节省上下文）：\n{summary}"
        ),
    }));
    out.push(json!({
        "role": "assistant",
        "content": "已了解早前对话的摘要，继续当前任务。",
    }));
    out.extend(recent);
    out
}

/// 构造摘要请求的 user 指令体（R5/R6/R8/R10）：序列化后的旧段对话历史 + Claude Code 9 段 prompt；
/// 存在上一份摘要时把它作为 `<previous-summary>` 让模型合并更新；`focus`（手动 `/compact <focus>`）
/// 透传为 `## Compact Instructions`。
fn build_summary_user_content(
    serialized_history: &str,
    previous_summary: Option<&str>,
    focus: Option<&str>,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(serialized_history.to_string());
    if let Some(previous) = previous_summary {
        parts.push(format!(
            "Update the anchored summary below using the conversation history above.\nPreserve still-true details, remove stale details, and merge in the new facts.\n<previous-summary>\n{previous}\n</previous-summary>"
        ));
    }
    parts.push(CLAUDE_CODE_SUMMARY_PROMPT.to_string());
    if let Some(focus) = focus {
        let focus = focus.trim();
        if !focus.is_empty() {
            parts.push(format!("## Compact Instructions\n{focus}"));
        }
    }
    parts.join("\n\n")
}

/// 从摘要模型的回复里抽取摘要正文：取 `<summary>...</summary>` 内文（如有），否则整体 trim。
fn extract_summary_text(response: &str) -> String {
    if let Some(start) = response.find("<summary>") {
        let after = &response[start + "<summary>".len()..];
        if let Some(end) = after.find("</summary>") {
            return after[..end].trim().to_string();
        }
        return after.trim().to_string();
    }
    response.trim().to_string()
}

/// 摘要调用的最大输出 token：`min(config.max_output_tokens, SUMMARY_OUTPUT_TOKENS)`（R9）。
fn summary_output_tokens(config_max: u32) -> u32 {
    config_max.min(SUMMARY_OUTPUT_TOKENS)
}

/// 把消息序列压缩成 system 前缀 + 摘要对 + 近期窗口（R5–R9 的共享核心）。
/// `focus` 为手动 `/compact <focus>` 透传的聚焦指令（自动路径为 None）。
/// 成功返回压缩后的完整消息序列；空摘要 / 失败 / 无可摘要旧段时返回 None（调用方据此降级）。
///
/// 自动路径（`maybe_compact_send_view`）与手动路径（`force_compact`）都走这里，避免重复摘要逻辑。
/// `keep_tokens`：受保护近期窗口大小——自动路径传 `min(RECENT_KEEP_TOKENS, budget)`（窗口比 8000 还小的
/// 模型上，近期窗口不能大过压缩预算，否则压完仍超窗），手动路径传 `RECENT_KEEP_TOKENS`。
/// `window`：模型上下文窗口（tokens）——据此把**摘要请求自身的输入**封顶到
/// `window * SUMMARY_INPUT_BUDGET_RATIO`（R1/R2），保证摘要调用绝不超窗（"用超窗请求救超窗"的根因）。
/// `window == 0`（未知）时用 `SUMMARY_INPUT_BUDGET_FALLBACK_TOKENS` 兜底。
/// `cancel`：进行中取消的 future——自动路径传 host 的取消等待，手动路径传 `None`（强制压缩不取消）。
#[allow(clippy::too_many_arguments)]
async fn summarize_history(
    state: &crate::state::AppState,
    provider: &crate::settings::ModelProvider,
    model: &str,
    messages: &[Value],
    keep_tokens: usize,
    window: usize,
    config_max_output_tokens: u32,
    retry_attempts: usize,
    conversation_id: &str,
    message_id: &str,
    focus: Option<&str>,
    cancel: Option<std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>>>,
) -> Option<Vec<Value>> {
    let (system_prefix, old_segment, recent) = select_recent_by_tokens(messages, keep_tokens);
    if old_segment.is_empty() {
        // 没有可摘要的旧段（全在受保护近期窗口里）——压缩无能为力。
        return None;
    }

    // anchored 链式摘要（R8）：若旧段含上一份摘要，作为 previous_summary 合并更新，且不重复进 head。
    let previous_summary = extract_previous_summary(&old_segment);
    let head: Vec<Value> = if previous_summary.is_some() {
        old_segment
            .iter()
            .filter(|m| {
                !(m.get("role").and_then(Value::as_str) == Some("user")
                    && m.get("content")
                        .and_then(Value::as_str)
                        .map(|c| c.trim_start().starts_with(SUMMARY_MARKER_PREFIX))
                        .unwrap_or(false))
            })
            .cloned()
            .collect()
    } else {
        old_segment.clone()
    };

    // 摘要**输入**预算（R1）：window * ratio，未知窗口用兜底常量。
    let summary_input_budget = if window == 0 {
        SUMMARY_INPUT_BUDGET_FALLBACK_TOKENS
    } else {
        ((window as f32) * SUMMARY_INPUT_BUDGET_RATIO) as usize
    };
    // 为固定开销（system prompt + 9 段 Claude Code prompt + anchored previous_summary + focus）
    // 预留预算（R4：previous_summary 不被裁掉），剩余给序列化旧段 head。
    let fixed_overhead = estimate_tokens(SUMMARY_SYSTEM_PROMPT)
        + estimate_tokens(CLAUDE_CODE_SUMMARY_PROMPT)
        + previous_summary
            .as_deref()
            .map(estimate_tokens)
            .unwrap_or(0)
        + focus.map(estimate_tokens).unwrap_or(0);
    // head 预算 = 总输入预算 - 固定开销；至少保留一点，避免开销吃光预算时退化为 0。
    let head_budget = summary_input_budget
        .saturating_sub(fixed_overhead)
        .max(summary_input_budget / 4);

    // 序列化旧段，超 head 预算时头尾裁剪（R2）；未超则原样（R5，零变化）。
    let serialized = clip_serialized_to_budget(&serialize_for_summary(&head), head_budget);
    let user_content =
        build_summary_user_content(&serialized, previous_summary.as_deref(), focus);
    let summary_request = vec![
        json!({ "role": "system", "content": SUMMARY_SYSTEM_PROMPT }),
        json!({ "role": "user", "content": user_content }),
    ];

    // 摘要调用走**流式**路径（`call_chat_completion_message_streamed`）而非非流式 `generate`：
    // 部分 provider（如 openai_responses 代理）只可靠服务流式，非流式摘要调用会失败、导致压缩永远
    // 摘不动；agent 的 planning/synthesis 已证明流式可用。流式被普遍支持，对支持非流式的 provider 无退化。
    let call = call_chat_completion_message_streamed(
        state,
        provider,
        model,
        summary_request,
        None,
        retry_attempts,
        false,
        summary_output_tokens(config_max_output_tokens),
        conversation_id,
        message_id,
        "Chat context compaction",
    );

    let summary = match cancel {
        Some(cancel) => {
            tokio::select! {
                result = call => result,
                _ = cancel => {
                    // 取消进行中：放弃压缩，让后续 planning 自己检测取消并正常收尾。
                    return None;
                }
            }
        }
        None => call.await,
    };

    match summary {
        Ok(message) => {
            let raw = super::stop::assistant_content_from_api_message(&message);
            let text = extract_summary_text(&raw);
            if text.trim().is_empty() {
                eprintln!("Chat context compaction returned empty summary; keeping raw view");
                return None;
            }
            Some(replace_with_summary(system_prefix, text.trim(), recent))
        }
        Err(err) => {
            eprintln!("Chat context compaction failed: {err}; keeping raw view");
            None
        }
    }
}

/// 循环内上下文治理入口。返回本步应发送的消息视图：
/// - 未超限：原样 clone（零行为变化）。
/// - 超限：模型摘要——把系统前缀与受保护近期窗口之外的旧段压成一条结构化摘要（R5–R9），
///   成功后**写回 state.runtime_messages**（工作副本）并置 `state.compacted = true`
///   （供 finalize 把压缩后历史回传给跨轮调用方）；失败或取消则降级返回原始 clone
///   ——压缩是优化，绝不让它失败掉整轮。
///
/// `generated_api_messages`（持久化镜像）在任何分支都不被触碰。
pub(crate) async fn maybe_compact_send_view(env: &LoopEnv<'_>, state: &mut RunState) -> Vec<Value> {
    let config = env.config;
    // Gap 3: 用 safe_window（裸窗口 × SAFE_WINDOW_RATIO）作为所有压缩预算的基准——模型窗口
    // 元数据偏乐观，安全折扣给元数据偏差留余量。触发预算、摘要输入封顶都基于同一个 safe_window。
    let window = context_window_for_model(Some(&config.provider), &config.model).0;
    let safe_window = safe_context_window_for_model(Some(&config.provider), &config.model);
    if safe_window == 0 {
        return state.runtime_messages.clone();
    }
    let budget = (safe_window as f32 * COMPACT_TRIGGER_RATIO) as usize;
    let estimated = estimate_messages_tokens(&state.runtime_messages);
    if estimated <= budget {
        // 未超预算：本步无需压缩。重置 anti-thrashing 计数（Gap 2）——上下文已回到预算内。
        state.compaction_unresolved_rounds = 0;
        return state.runtime_messages.clone();
    }

    eprintln!(
        "Chat context compaction: est {estimated} tokens over budget {budget} (safe_window {safe_window}, window {window}); summarizing old history"
    );

    let cancel = env
        .host
        .wait_for_generation_inactive(&config.conversation_id, config.generation);
    // 受保护近期窗口默认 8000 token，但不得超过压缩预算——否则窗口比 8000 还小的模型上，
    // 整段历史会被近期窗口吞掉，没有可摘要的旧段，压缩永远救不了超窗。
    let keep_tokens = RECENT_KEEP_TOKENS.min(budget);
    let compacted = summarize_history(
        config.state,
        &config.provider,
        &config.model,
        &state.runtime_messages,
        keep_tokens,
        safe_window,
        config.max_output_tokens,
        config.retry_attempts,
        &config.conversation_id,
        &config.message_id,
        None,
        Some(cancel),
    )
    .await;

    match compacted {
        Some(compacted) => {
            let after = estimate_messages_tokens(&compacted);
            eprintln!("Chat context compaction: est {estimated} -> {after} tokens");
            // 摘要写回工作副本：后续轮次基于压缩后的历史继续，避免每轮重复摘要。
            // 置 compacted 标志：finalize 据此把压缩后的完整历史回传给跨轮调用方
            // （交互模式据 compacted_history 替换其累积历史，让压缩真正跨轮生效）。
            state.runtime_messages = compacted.clone();
            state.compacted = true;
            // Gap 2: 压缩确实把上下文压到了预算内吗？是 → anti-thrashing 计数清零；
            // 否（摘要成功但仍超预算，例如近期窗口本身已超）→ 计为一次未解决，防止后续轮空转。
            if after <= budget {
                state.compaction_unresolved_rounds = 0;
            } else {
                state.compaction_unresolved_rounds =
                    state.compaction_unresolved_rounds.saturating_add(1);
            }
            compacted
        }
        None => {
            // Gap 2: 需要压缩（超预算）但压缩没能减小上下文（摘要调用失败/为空/无旧段）——
            // 计为一次「未解决」。连续达到 COMPACTION_THRASH_LIMIT 次时，规划循环会据此优雅收尾，
            // 而不是反复触发压缩并失败 6+ 次后才报错。
            state.compaction_unresolved_rounds =
                state.compaction_unresolved_rounds.saturating_add(1);
            state.runtime_messages.clone()
        }
    }
}

/// 手动 `/compact [focus]`：强制压缩 `messages`（**无视预算**），走与自动路径相同的
/// serialize→summary→replace 核心（R10）。`focus` 透传进摘要 prompt。成功返回压缩后的完整历史
/// 供交互层替换其 `runtime_messages`；无可摘要旧段 / 空摘要 / 失败时返回 None（调用方据此提示）。
/// 强制压缩不接取消（用户主动触发）。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn force_compact(
    state: &crate::state::AppState,
    provider: &crate::settings::ModelProvider,
    model: &str,
    messages: &[Value],
    config_max_output_tokens: u32,
    retry_attempts: usize,
    conversation_id: &str,
    message_id: &str,
    focus: Option<&str>,
) -> Option<Vec<Value>> {
    // 手动路径自行计算窗口（自动路径已有 window；此处与触发判定同源的元数据查询）。
    // Gap 3：摘要输入封顶基于 safe_window（裸窗口 × SAFE_WINDOW_RATIO），与自动路径同源，
    // 保证两条路径的摘要请求预算一致。safe_window == 0（未知模型）时 summarize_history 用兜底常量。
    let safe_window = safe_context_window_for_model(Some(provider), model);
    summarize_history(
        state,
        provider,
        model,
        messages,
        RECENT_KEEP_TOKENS,
        safe_window,
        config_max_output_tokens,
        retry_attempts,
        conversation_id,
        message_id,
        focus,
        None,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_counts_content_and_structured_fields() {
        let messages = vec![
            json!({ "role": "user", "content": "abcd".repeat(10) }),
            json!({ "role": "assistant", "content": "", "tool_calls": [{"id": "x", "function": {"name": "f", "arguments": "{}"}}] }),
        ];
        assert!(estimate_messages_tokens(&messages) > 10);
    }

    #[test]
    fn serialize_keeps_user_and_assistant_full() {
        let big_user = "U".repeat(5_000);
        let big_assistant = "A".repeat(5_000);
        let messages = vec![
            json!({ "role": "user", "content": big_user.clone() }),
            json!({ "role": "assistant", "content": big_assistant.clone() }),
        ];
        let serialized = serialize_for_summary(&messages);
        // 用户/助手消息全文保留（不截断）。
        assert!(serialized.contains(&big_user));
        assert!(serialized.contains(&big_assistant));
        assert!(serialized.contains("[User]:"));
        assert!(serialized.contains("[Assistant]:"));
    }

    #[test]
    fn serialize_clips_tool_result_to_cap() {
        let huge = "T".repeat(10_000);
        let messages = vec![json!({ "role": "tool", "tool_call_id": "c1", "content": huge })];
        let serialized = serialize_for_summary(&messages);
        assert!(serialized.starts_with("[Tool result]:"));
        assert!(serialized.contains("[truncated]"));
        // The clipped tool output keeps at most the cap chars (+ marker), far less than 10k.
        let t_run = "T".repeat(TOOL_OUTPUT_SUMMARY_MAX_CHARS + 1);
        assert!(
            !serialized.contains(&t_run),
            "tool output must be clipped to the cap"
        );
        // But it does keep the cap-sized prefix.
        assert!(serialized.contains(&"T".repeat(TOOL_OUTPUT_SUMMARY_MAX_CHARS)));
    }

    #[test]
    fn serialize_renders_tool_error_and_tool_call() {
        let messages = vec![
            json!({
                "role": "assistant",
                "content": "let me read it",
                "tool_calls": [{
                    "id": "c1",
                    "type": "function",
                    "function": { "name": "read_file", "arguments": "{\"path\":\"main.rs\"}" }
                }]
            }),
            json!({ "role": "tool", "tool_call_id": "c1", "content": "boom", "is_error": true }),
        ];
        let serialized = serialize_for_summary(&messages);
        assert!(serialized.contains("[Assistant]: let me read it"));
        assert!(serialized.contains("[Assistant tool call]: read_file({\"path\":\"main.rs\"})"));
        assert!(serialized.contains("[Tool error]: boom"));
    }

    #[test]
    fn select_recent_by_tokens_splits_near_boundary() {
        let mut messages = vec![json!({ "role": "system", "content": "sys" })];
        // Each message ~ 250 tokens (1000 chars / 4). 40 messages ~ 10k tokens.
        for i in 0..40 {
            messages.push(json!({
                "role": if i % 2 == 0 { "user" } else { "assistant" },
                "content": "x".repeat(1_000)
            }));
        }
        let (sys, old, recent) = select_recent_by_tokens(&messages, 8_000);
        assert_eq!(sys.len(), 1, "system prefix protected");
        assert!(!old.is_empty(), "older messages go to the summary");
        assert!(!recent.is_empty(), "a recent tail is preserved");
        // The recent tail is bounded near 8000 tokens (whole messages, never split).
        let recent_tokens = estimate_messages_tokens(&recent);
        assert!(
            recent_tokens <= 8_000 + 300,
            "recent window ~8000 tokens (was {recent_tokens})"
        );
        // No message was split: every recent/old message is a full object from the input.
        assert_eq!(sys.len() + old.len() + recent.len(), messages.len());
        // Order preserved: old then recent reconstruct the post-system messages.
        assert_eq!(old[0]["content"], messages[1]["content"]);
        assert_eq!(recent.last().unwrap()["content"], messages[40]["content"]);
    }

    #[test]
    fn select_recent_never_splits_tool_call_pair() {
        // Build: system, then many small messages, then an assistant(tool_calls)
        // immediately followed by a large tool result that lands on the boundary.
        let mut messages = vec![json!({ "role": "system", "content": "sys" })];
        for _ in 0..10 {
            messages.push(json!({ "role": "user", "content": "x".repeat(1_000) }));
        }
        messages.push(json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [{ "id": "c1", "type": "function", "function": { "name": "read", "arguments": "{}" } }]
        }));
        // A big tool result that nudges the recent window to start right after it.
        messages.push(json!({ "role": "tool", "tool_call_id": "c1", "content": "y".repeat(30_000) }));
        // A trailing user message keeps the tail non-trivial.
        messages.push(json!({ "role": "user", "content": "done?" }));

        let (_sys, old, recent) = select_recent_by_tokens(&messages, 8_000);
        // The recent window must never START with an orphan tool result whose
        // assistant(tool_calls) got left in `old`.
        if let Some(first) = recent.first() {
            assert!(
                !is_tool_result(first),
                "recent window must not start with an orphan tool result"
            );
        }
        // And old must never END with an assistant(tool_calls) whose tool result was pulled away.
        if let Some(last) = old.last() {
            assert!(
                !has_tool_calls(last),
                "old segment must not end with a dangling tool_call whose result moved to recent"
            );
        }
    }

    #[test]
    fn extract_previous_summary_detects_anchored_marker() {
        let old = vec![
            json!({ "role": "user", "content": format!("{SUMMARY_MARKER_PREFIX} 引导语：\n1. Primary Request: build X") }),
            json!({ "role": "assistant", "content": "已了解" }),
            json!({ "role": "user", "content": "next question" }),
        ];
        let previous = extract_previous_summary(&old).expect("prior summary detected");
        assert!(previous.contains("Primary Request: build X"));
        // No marker present → None.
        let fresh = vec![json!({ "role": "user", "content": "just a question" })];
        assert!(extract_previous_summary(&fresh).is_none());
    }

    #[test]
    fn build_summary_prompt_carries_previous_summary_and_focus() {
        let content = build_summary_user_content(
            "[User]: hi\n\n[Assistant]: hello",
            Some("1. Primary Request: build X"),
            Some("focus on tests"),
        );
        // Anchored branch present.
        assert!(content.contains("Update the anchored summary below"));
        assert!(content.contains("<previous-summary>"));
        assert!(content.contains("1. Primary Request: build X"));
        // Focus passed through as Compact Instructions.
        assert!(content.contains("## Compact Instructions\nfocus on tests"));
        // The serialized history is included.
        assert!(content.contains("[User]: hi"));
    }

    #[test]
    fn build_summary_prompt_fresh_has_no_previous_block() {
        let content = build_summary_user_content("[User]: hi", None, None);
        assert!(!content.contains("Update the anchored summary"));
        assert!(!content.contains("<previous-summary>"));
        // The verbatim Claude Code prompt itself shows a `## Compact Instructions`
        // EXAMPLE, so we can't assert on that substring; assert no focus text was
        // injected instead (the focus from `/compact <focus>` would appear after it).
        assert!(!content.contains("## Compact Instructions\nfocus"));
    }

    #[test]
    fn summary_prompt_has_nine_sections_and_analysis() {
        // R6: the embedded Claude Code prompt must carry the 9 section headers + <analysis>.
        for header in [
            "1. Primary Request and Intent",
            "2. Key Technical Concepts",
            "3. Files and Code Sections",
            "4. Errors and fixes",
            "5. Problem Solving",
            "6. All user messages",
            "7. Pending Tasks",
            "8. Current Work",
            "9. Optional Next Step",
        ] {
            assert!(
                CLAUDE_CODE_SUMMARY_PROMPT.contains(header),
                "summary prompt missing section: {header}"
            );
        }
        assert!(CLAUDE_CODE_SUMMARY_PROMPT.contains("<analysis>"));
        assert!(CLAUDE_CODE_SUMMARY_PROMPT.contains("</analysis>"));
        assert!(CLAUDE_CODE_SUMMARY_PROMPT.contains("<summary>"));
        // The summarizer system prompt is the exact Claude Code string.
        assert_eq!(
            SUMMARY_SYSTEM_PROMPT,
            "You are a helpful AI assistant tasked with summarizing conversations."
        );
    }

    #[test]
    fn extract_summary_text_prefers_summary_tag() {
        let resp = "<analysis>thinking…</analysis>\n<summary>\n1. Primary Request: X\n</summary>";
        assert_eq!(extract_summary_text(resp), "1. Primary Request: X");
        // No tag → whole response trimmed.
        assert_eq!(extract_summary_text("  just text  "), "just text");
    }

    #[test]
    fn extract_summary_text_handles_unclosed_and_missing_tags() {
        // Open tag without a closing tag → everything after the open tag, trimmed.
        assert_eq!(
            extract_summary_text("<analysis>x</analysis>\n<summary>\n1. Request: Y\n"),
            "1. Request: Y"
        );
        // Multiple <summary> tags: takes the first opening and the first closing
        // after it (greedy on the prefix is fine — first complete block wins).
        assert_eq!(
            extract_summary_text("<summary>first</summary>\n<summary>second</summary>"),
            "first"
        );
        // Empty content between tags collapses to empty (caller treats as failure).
        assert_eq!(extract_summary_text("<summary></summary>"), "");
    }

    #[test]
    fn recent_window_all_tool_results_yields_empty_old_segment() {
        // Pathological: after the system prefix the entire (small) tail is tool
        // results. The pair-protection walk would slide the boundary back to
        // system_end, so there is no old segment to summarize → callers degrade
        // gracefully (summarize_history returns None). Verify no orphan tool ends
        // up at the START of old_segment, and old is empty here.
        let mut messages = vec![json!({ "role": "system", "content": "sys" })];
        for i in 0..3 {
            messages
                .push(json!({ "role": "tool", "tool_call_id": format!("c{i}"), "content": "ok" }));
        }
        let (sys, old, recent) = select_recent_by_tokens(&messages, 8_000);
        assert_eq!(sys.len(), 1);
        assert!(
            old.is_empty(),
            "all-tool tail leaves nothing summarizable (old must be empty, was {old:?})"
        );
        // Whatever lands in old must never START with an orphan tool result.
        if let Some(first) = old.first() {
            assert!(!is_tool_result(first));
        }
        assert_eq!(recent.len(), 3);
    }

    #[test]
    fn summary_output_tokens_caps_at_4096() {
        assert_eq!(summary_output_tokens(100_000), SUMMARY_OUTPUT_TOKENS);
        assert_eq!(summary_output_tokens(1_000), 1_000);
    }

    #[test]
    fn replace_with_summary_keeps_role_alternation_legal() {
        let sys = vec![json!({ "role": "system", "content": "sys" })];
        let recent = vec![
            json!({ "role": "user", "content": "latest question" }),
            json!({ "role": "assistant", "content": "latest answer" }),
        ];
        let out = replace_with_summary(sys, "the summary", recent);
        let roles: Vec<&str> = out
            .iter()
            .map(|message| message["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, vec!["system", "user", "assistant", "user", "assistant"]);
        assert!(out[1]["content"].as_str().unwrap().contains("the summary"));
        // The inserted summary carries the anchor marker for future chained summaries.
        assert!(out[1]["content"]
            .as_str()
            .unwrap()
            .starts_with(SUMMARY_MARKER_PREFIX));
    }

    #[test]
    fn budget_ratio_halves_the_window() {
        // R1: summary input budget is window * 0.5.
        assert_eq!(SUMMARY_INPUT_BUDGET_RATIO, 0.5);
        let window = 128_000usize;
        let budget = ((window as f32) * SUMMARY_INPUT_BUDGET_RATIO) as usize;
        assert_eq!(budget, 64_000);
    }

    #[test]
    fn clip_keeps_small_serialized_unchanged() {
        // R5: a serialized old segment already under budget is returned verbatim.
        let serialized = "[User]: hi\n\n[Assistant]: hello there";
        let out = clip_serialized_to_budget(serialized, 10_000);
        assert_eq!(out, serialized);
        assert!(!out.contains("omitted to fit"));
    }

    #[test]
    fn clip_head_tail_fits_budget_and_keeps_both_ends() {
        // R2: a serialized old segment far exceeding the budget is clipped HEAD+TAIL
        // to fit, keeping a recognizable beginning and end with a middle marker.
        let head_marker = "BEGINNING_TASK_GOAL";
        let tail_marker = "MOST_RECENT_WORK";
        let mut serialized = String::new();
        serialized.push_str(head_marker);
        serialized.push(' ');
        serialized.push_str(&"filler ".repeat(50_000)); // ~350k chars
        serialized.push_str(tail_marker);

        let budget = 4_000usize;
        let clipped = clip_serialized_to_budget(&serialized, budget);

        // Hard guarantee: result fits the budget.
        assert!(
            estimate_tokens(&clipped) <= budget,
            "clipped est {} must be <= budget {budget}",
            estimate_tokens(&clipped)
        );
        // Both ends survive.
        assert!(clipped.contains(head_marker), "head must survive");
        assert!(clipped.contains(tail_marker), "tail must survive");
        // The omission marker is present in the middle.
        assert!(clipped.contains("older history omitted to fit"));
        // Tail bias: tail budget (~60%) >= head budget (~40%).
        let marker_pos = clipped.find("older history omitted").unwrap();
        let head_part_len = marker_pos;
        let tail_part_len = clipped.len() - (marker_pos + "older history omitted".len());
        assert!(
            tail_part_len >= head_part_len,
            "tail ({tail_part_len}) should keep at least as much as head ({head_part_len})"
        );
    }

    #[test]
    fn clip_with_unicode_fits_budget() {
        // Multi-byte (CJK) content costs ~1 token/char; clipping must still fit the budget.
        let serialized = "开头任务".to_string() + &"上下文".repeat(40_000) + "最近工作";
        let budget = 2_000usize;
        let clipped = clip_serialized_to_budget(&serialized, budget);
        assert!(
            estimate_tokens(&clipped) <= budget,
            "unicode clipped est {} must be <= budget {budget}",
            estimate_tokens(&clipped)
        );
        assert!(clipped.contains("开头任务"));
        assert!(clipped.contains("最近工作"));
    }

    #[test]
    fn budget_fallback_when_window_unknown() {
        // window == 0 → use the fallback budget constant (no panic, capping still applies).
        let serialized = "x".repeat(SUMMARY_INPUT_BUDGET_FALLBACK_TOKENS * 4 * 2);
        // Mirror summarize_history's budget calc for window == 0.
        let budget = SUMMARY_INPUT_BUDGET_FALLBACK_TOKENS;
        let clipped = clip_serialized_to_budget(&serialized, budget);
        assert!(estimate_tokens(&clipped) <= budget);
        assert!(SUMMARY_INPUT_BUDGET_FALLBACK_TOKENS > 0);
    }
}
