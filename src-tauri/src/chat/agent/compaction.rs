use serde_json::{json, Value};

use crate::chat::model_metadata::{
    chat_max_output_tokens_for_model, context_window_for_model,
};
use crate::chat::types::{
    ChatMessage, CompactionBoundaryRecord, Conversation, ConversationContextSummary,
};
use crate::settings::Settings;
use crate::state::AppState;
use tauri::{AppHandle, Emitter};

use super::loop_::{LoopEnv, RunState};
use super::planning::call_chat_completion_message_streamed;
use super::prepare::estimate_tokens;

/// 近期窗口（tokens）：从尾部往前累积整条消息，~该预算内的为受保护近期窗口、原样保留，
/// 其余旧段才进摘要。取代旧的固定 `KEEP_RECENT_RAW_MESSAGES` 条数（R7）。对齐 Codex
/// `COMPACT_USER_MESSAGE_MAX_TOKENS ≈ 20_000` 量级（含 user+assistant 整条）。
pub(crate) const RECENT_KEEP_TOKENS: usize = 20_000;
/// 估算占用超过**裸**窗口的该比例才触发自动压缩。对齐 Codex `AUTO_COMPACT_RATIO = 0.90`
/// （用裸窗口而非 safe_window 折扣，统一落盘 / L2 / 手动三处触发基准）。
pub(crate) const AUTO_COMPACT_RATIO: f32 = 0.90;
/// 摘要内容字符数下限（质量兜底）：低于此值视为烂摘要，拒绝覆盖旧 summary。
/// 修复「收到 ✅」式过短摘要污染落盘 context_state.summary 的问题。
const MIN_SUMMARY_CHARS: usize = 200;
/// 链式重摘衰减告警阈值：累计压缩次数达到此值后，给用户一条 `context_state.warning`
/// 提示多次压缩可能降低准确性（对齐 Codex 的 WarningEvent）。摘要是有损的，反复 summary-of-summary
/// 会累积漂移/细节流失。
const DECAY_WARNING_COMPRESSION_COUNT: usize = 3;
/// 序列化喂给摘要模型时，单条 `[Tool result]` / `[Tool error]` 的字符上限（R5）。
/// 仅截工具输出——用户/助手/推理/工具入参全文保留。对齐 OpenCode `TOOL_OUTPUT_MAX_CHARS = 2_000`。
const TOOL_OUTPUT_SUMMARY_MAX_CHARS: usize = 2_000;
/// Microcompact（R-1）降级旧工具结果时替换成的短标记。触发压缩时先把 old_segment 里的工具结果
/// 换成此标记、重估预算；够了就跳过昂贵的 LLM 摘要（对齐 Claude Code 的 microcompact）。
const MICROCOMPACT_TOOL_MARKER: &str = "[earlier tool result omitted to save context]";
/// 摘要调用允许产生的最大输出 token 数（R9）。Claude Code 9 段 prompt 先吐 `<analysis>`
/// 再吐 `<summary>`，真实大旧段时二者合计易超 4096 被截——提到 8192 以容纳完整产出
/// （仍远小于窗口，安全）。`summary_output_tokens` 保持 `min()`：真实上限更小的模型不受影响。
const SUMMARY_OUTPUT_TOKENS: u32 = 8_192;

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

/// `build_chat_api_messages`（commands.rs）从落盘 `context_state.summary` 注入的
/// **system** 消息前缀。生成方（`commands.rs::summary_message`）与识别方
/// （`extract_previous_summary`）**共用同一常量**，防止格式漂移——历史上二者不一致
/// 曾导致 L2 压缩认不出注入的旧摘要，链式摘要退化为「只摘本轮 old_segment」，
/// run 结束整体覆盖旧 summary，跨轮静默丢掉早期上下文。
pub(crate) const PERSISTED_SUMMARY_PREFIX: &str = "Previous conversation summary:";

/// `replace_with_summary` 插入的摘要锚点后紧跟的固定 assistant ack 文案。抽成常量供
/// 插入方与 `summarize_history` 的 head 剔除方共用——否则链式重摘时上一轮的 ack 会作为
/// `[Assistant]:` 噪声行进入摘要输入。
const SUMMARY_ACK_TEXT: &str = "已了解早前对话的摘要，继续当前任务。";

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

/// `extract_previous_summary` 的返回：上一份摘要正文 + 它的来源，供 `summarize_history`
/// 决定如何从压缩后视图里剔除它（锚点在 old_segment→head，注入在 system 前缀→prefix）。
struct PreviousSummary {
    text: String,
    from_injected: bool,
}

/// 探测上一份摘要（anchored 链式摘要，R8），供作为 `previous_summary` 让模型合并更新。
/// 两种形态，**锚点优先**（它是同一 run 内更晚的 L2 产物，已含合并结果）：
/// - **锚点摘要**：`replace_with_summary` 插入 old_segment 的一条 content 以
///   `SUMMARY_MARKER_PREFIX` 开头的 **user** 消息；
/// - **注入摘要**：`build_chat_api_messages` 从落盘 summary 注入 system 前缀的一条 content 以
///   `PERSISTED_SUMMARY_PREFIX` 开头的 **system** 消息。
///
/// 无锚点时才回退到注入摘要——否则同 run 第二次 L2 压缩会回退到已过期的落盘 S1。
fn extract_previous_summary(system_prefix: &[Value], old_segment: &[Value]) -> Option<PreviousSummary> {
    // 锚点优先：old_segment 里的 user + SUMMARY_MARKER_PREFIX。
    let anchor = old_segment.iter().find_map(|message| {
        if message.get("role").and_then(Value::as_str) != Some("user") {
            return None;
        }
        let content = message.get("content").and_then(Value::as_str)?;
        let trimmed = content.trim_start();
        if !trimmed.starts_with(SUMMARY_MARKER_PREFIX) {
            return None;
        }
        Some(summary_body_after_first_line(trimmed))
    });
    if let Some(text) = anchor {
        return Some(PreviousSummary {
            text,
            from_injected: false,
        });
    }

    // 回退：system 前缀里的 system + PERSISTED_SUMMARY_PREFIX（注入摘要）。
    system_prefix.iter().find_map(|message| {
        if message.get("role").and_then(Value::as_str) != Some("system") {
            return None;
        }
        let content = message.get("content").and_then(Value::as_str)?;
        let trimmed = content.trim_start();
        if !trimmed.starts_with(PERSISTED_SUMMARY_PREFIX) {
            return None;
        }
        Some(PreviousSummary {
            text: summary_body_after_first_line(trimmed),
            from_injected: true,
        })
    })
}

/// 取摘要正文：摘要消息形如 "<前缀引导语>：\n<summary>"。找第一个换行后的内容；
/// 无换行则退回整条 trim。
fn summary_body_after_first_line(trimmed: &str) -> String {
    trimmed
        .split_once('\n')
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed)
        .trim()
        .to_string()
}

/// 是否为 `replace_with_summary` 插入的锚点摘要（user + `SUMMARY_MARKER_PREFIX`）。
fn is_anchor_summary(message: &Value) -> bool {
    message.get("role").and_then(Value::as_str) == Some("user")
        && message
            .get("content")
            .and_then(Value::as_str)
            .map(|c| c.trim_start().starts_with(SUMMARY_MARKER_PREFIX))
            .unwrap_or(false)
}

/// 是否为锚点摘要配对的固定 ack（assistant + `SUMMARY_ACK_TEXT`）。
fn is_summary_ack(message: &Value) -> bool {
    message.get("role").and_then(Value::as_str) == Some("assistant")
        && message.get("content").and_then(Value::as_str).map(str::trim) == Some(SUMMARY_ACK_TEXT)
}

/// 是否为 `build_chat_api_messages` 注入的落盘摘要（system + `PERSISTED_SUMMARY_PREFIX`）。
fn is_injected_summary(message: &Value) -> bool {
    message.get("role").and_then(Value::as_str) == Some("system")
        && message
            .get("content")
            .and_then(Value::as_str)
            .map(|c| c.trim_start().starts_with(PERSISTED_SUMMARY_PREFIX))
            .unwrap_or(false)
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
        "content": SUMMARY_ACK_TEXT,
    }));
    out.extend(recent);
    out
}

/// Microcompact 增量降级（R-1）：触发压缩时、发起 LLM 摘要**之前**的轻量兜底。
/// 把 old_segment（近期 `keep_tokens` 尾窗**之前**的段）里的 `role=="tool"` 结果内容换成
/// `MICROCOMPACT_TOOL_MARKER`，组回完整视图并重估 token。
///
/// **仅当降级足以把整体压回 `budget` 内才返回 `Some(view)`（= 可跳过昂贵摘要）**；否则 `None`，
/// 调用方落到既有 LLM 摘要路径。近期尾窗原样保留（近期工具结果不动）；不拆 tool_call↔tool 配对
/// （复用 `select_recent_by_tokens` 的切分与配对保护）；已是标记的工具结果不再重复降级（幂等）。
fn microcompact_send_view(
    messages: &[Value],
    keep_tokens: usize,
    budget: usize,
) -> Option<Vec<Value>> {
    let (system_prefix, old_segment, recent) = select_recent_by_tokens(messages, keep_tokens);
    if old_segment.is_empty() {
        return None;
    }
    let mut degraded_any = false;
    let degraded_old: Vec<Value> = old_segment
        .into_iter()
        .map(|message| {
            if !is_tool_result(&message) {
                return message;
            }
            let already_marker = message.get("content").and_then(Value::as_str)
                == Some(MICROCOMPACT_TOOL_MARKER);
            if already_marker {
                return message;
            }
            let mut degraded = message;
            if let Some(obj) = degraded.as_object_mut() {
                obj.insert(
                    "content".to_string(),
                    Value::String(MICROCOMPACT_TOOL_MARKER.to_string()),
                );
                degraded_any = true;
            }
            degraded
        })
        .collect();
    if !degraded_any {
        // old_segment 里没有可降级的工具结果——microcompact 无能为力，交给摘要。
        return None;
    }
    let mut view = system_prefix;
    view.extend(degraded_old);
    view.extend(recent);
    if estimate_messages_tokens(&view) <= budget {
        Some(view)
    } else {
        None
    }
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
/// runtime 消息上的来源 UI 消息 id 标注（由 `commands.rs::build_chat_api_messages` 注入；
/// 一条 UI 消息展开出的多条 runtime 消息共享同一 id）。发给 provider 前该字段会被
/// `model_message_from_openai_message` 剥离，绝不进 wire 请求。
pub(crate) const UI_MESSAGE_ID_KEY: &str = "_ui_message_id";

fn ui_message_id_of(message: &Value) -> Option<&str> {
    message.get(UI_MESSAGE_ID_KEY).and_then(Value::as_str)
}

/// 把 runtime 切分映射回 UI 消息：返回「其 runtime 展开**完全**落入旧段」的最后一条
/// UI 消息 id。旧实现按 user|assistant 条数当 `ui_message_order` 下标推算，工具多轮
/// 展开/多答组剔除/摘要锚点都会错位（错位的 boundary 落盘后会静默丢上下文）；现改为
/// 读 `_ui_message_id` 标注精确映射。
///
/// 若旧段末尾的 UI 消息有展开条残留在近期窗口（横跨边界），回退到旧段内上一个不同的
/// 完整 id。旧段无任何带标注消息（只有摘要锚点/系统注入）→ None（调用方不落盘
/// boundary，运行时压缩视图照常生效）。
pub(crate) fn source_until_message_id_for_split(
    runtime_messages: &[Value],
    keep_tokens: usize,
) -> Option<String> {
    let (_system_prefix, old_segment, recent) =
        select_recent_by_tokens(runtime_messages, keep_tokens);
    if old_segment.is_empty() {
        return None;
    }
    // 近期窗口里出现过的 id：这些 UI 消息有展开条不在旧段里，不能作为 boundary。
    let ids_in_recent: std::collections::HashSet<&str> =
        recent.iter().filter_map(ui_message_id_of).collect();
    old_segment
        .iter()
        .rev()
        .filter_map(ui_message_id_of)
        .find(|id| !ids_in_recent.contains(id))
        .map(str::to_string)
}

/// 把单条 UI `ChatMessage` 估算成 token 数：content + reasoning + 工具入参全文 + 结果预览。
/// 与 `estimate_message_tokens`（Value 版）口径一致，供落盘路径的 token 切点复用。
fn estimate_chat_message_tokens(message: &ChatMessage) -> usize {
    let mut total = estimate_tokens(&message.content);
    if let Some(reasoning) = message.reasoning.as_deref() {
        total += estimate_tokens(reasoning);
    }
    for tool in &message.tool_calls {
        total += estimate_tokens(&tool.name);
        total += estimate_tokens(&tool.arguments);
        if let Some(preview) = tool.result_preview.as_deref() {
            total += estimate_tokens(preview);
        }
        if let Some(err) = tool.error.as_deref() {
            total += estimate_tokens(err);
        }
    }
    total + 4
}

/// 落盘路径的 token 切点：在 `[summary_start, len)` 区间内，从尾部往前累积整条
/// `ChatMessage` 的 `estimate_chat_message_tokens`，直到 ~`keep_tokens` 预算用尽。
/// 返回 old_segment 末尾下标（含）；越界或无旧段返回 None。
///
/// 与 L2 `select_recent_by_tokens` 同语义：不切断单条消息；越预算的那条整体归入旧段。
pub(crate) fn token_split_chat_messages(
    messages: &[ChatMessage],
    summary_start: usize,
    keep_tokens: usize,
) -> Option<usize> {
    let len = messages.len();
    if summary_start >= len {
        return None;
    }
    let mut total = 0usize;
    let mut split = len; // recent 起始（含）；split-1 = old_segment 末尾
    let mut idx = len;
    while idx > summary_start {
        idx -= 1;
        let next = total + estimate_chat_message_tokens(&messages[idx]);
        if next > keep_tokens && idx + 1 < len {
            split = idx + 1;
            break;
        }
        total = next;
        split = idx;
    }
    if split <= summary_start {
        // 整段都进了近期窗口——没有可摘要旧段。
        return None;
    }
    Some(split - 1)
}

/// 把 UI `ChatMessage` 序列化成喂给摘要模型的角色标注文本。对齐 L2 `serialize_message`：
/// user/assistant/reasoning/工具入参**全文保留**；`result_preview` / `error` 截到
/// `TOOL_OUTPUT_SUMMARY_MAX_CHARS`（尾部加 `[truncated]`）。这是修复「收到 ✅」式烂摘要的根因——
/// 旧落盘路径只发 UI 文本 + 500 字工具预览，工具入参完全丢失；现在与 L2 等价。
fn serialize_chat_message_for_summary(message: &ChatMessage) -> String {
    let role = if message.role == "assistant" {
        "assistant"
    } else {
        "user"
    };
    let mut lines: Vec<String> = Vec::new();

    let text = message.content.trim();
    if !text.is_empty() {
        lines.push(format!("[{}]: {text}", capitalize_role(role)));
    }
    if let Some(reasoning) = message.reasoning.as_deref() {
        if !reasoning.trim().is_empty() {
            lines.push(format!("[Assistant reasoning]: {reasoning}"));
        }
    }
    for tool in &message.tool_calls {
        // 工具入参全文保留（不截断）——让摘要模型能看到具体读了哪个文件 / 跑了什么命令。
        lines.push(format!(
            "[Assistant tool call]: {}({})",
            tool.name, tool.arguments
        ));
        let output = tool
            .result_preview
            .clone()
            .or_else(|| tool.error.clone())
            .unwrap_or_default();
        if !output.trim().is_empty() {
            let clipped = clip_tool_output(&output);
            if tool.error.is_some() {
                lines.push(format!("[Tool error]: {clipped}"));
            } else {
                lines.push(format!("[Tool result]: {clipped}"));
            }
        }
    }

    lines.join("\n")
}

fn capitalize_role(role: &str) -> &'static str {
    match role {
        "assistant" => "Assistant",
        "user" => "User",
        _ => "User",
    }
}

/// 把旧段 `ChatMessage` 序列化成角色标注文本（每条一段，空行分隔）。
fn serialize_chat_messages_for_summary(messages: &[ChatMessage]) -> String {
    messages
        .iter()
        .map(serialize_chat_message_for_summary)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// 统一摘要调用核心（落盘 / L2 / 手动三处共用）：
/// 1. 把序列化后的旧段头尾裁剪到摘要**输入**预算（`window * SUMMARY_INPUT_BUDGET_RATIO`），
///    保证摘要请求自身绝不超窗（R1/R2）；`window == 0` 用兜底常量。
/// 2. 拼 Claude 9 段 prompt + anchored `<previous-summary>` + `focus`。
/// 3. **流式**调用压缩模型（`call_chat_completion_message_streamed`），抽 `<summary>` 正文。
/// 4. 质量兜底：空 / 过短 / 相对旧 summary 显著劣化 → 返回 None，**不覆盖**旧 summary。
///
/// 返回 summary 文本（已 trim）；失败 / 取消 / 质量不达标返回 None（调用方据此降级）。
#[allow(clippy::too_many_arguments)]
async fn compact_with_summary_model(
    state: &AppState,
    provider: &crate::settings::ModelProvider,
    model: &str,
    serialized_old_segment: &str,
    previous_summary: Option<&str>,
    focus: Option<&str>,
    window: usize,
    config_max_output_tokens: u32,
    retry_attempts: usize,
    conversation_id: &str,
    message_id: &str,
    cancel: Option<std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>>>,
) -> Option<String> {
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
        + previous_summary.map(estimate_tokens).unwrap_or(0)
        + focus.map(estimate_tokens).unwrap_or(0);
    // head 预算 = 总输入预算 - 固定开销；至少保留一点，避免开销吃光预算时退化为 0。
    let head_budget = summary_input_budget
        .saturating_sub(fixed_overhead)
        .max(summary_input_budget / 4);

    // 序列化旧段，超 head 预算时头尾裁剪（R2）；未超则原样（R5）。
    let serialized = clip_serialized_to_budget(serialized_old_segment, head_budget);
    let user_content = build_summary_user_content(&serialized, previous_summary, focus);
    let summary_request = vec![
        json!({ "role": "system", "content": SUMMARY_SYSTEM_PROMPT }),
        json!({ "role": "user", "content": user_content }),
    ];

    // 流式调用：部分 provider（如 openai_responses 代理）只可靠服务流式，非流式会失败。
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

    let raw = match summary {
        Ok(message) => super::stop::assistant_content_from_api_message(&message),
        Err(err) => {
            eprintln!("Chat context compaction failed: {err}; keeping raw view");
            return None;
        }
    };
    let text = extract_summary_text(&raw);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        eprintln!("Chat context compaction returned empty summary; keeping raw view");
        return None;
    }
    // 质量兜底（修复「收到 ✅」）：过短 / 相对旧 summary 显著劣化 → 拒绝覆盖。
    match summary_quality_guard(trimmed, previous_summary) {
        SummaryQuality::Ok => Some(trimmed.to_string()),
        SummaryQuality::Truncated => {
            eprintln!(
                "Chat context compaction summary truncated (<analysis> without <summary>); rejecting"
            );
            None
        }
        SummaryQuality::TooShort => {
            eprintln!(
                "Chat context compaction summary too short ({} chars < {MIN_SUMMARY_CHARS}); rejecting",
                trimmed.chars().count()
            );
            None
        }
        SummaryQuality::Degraded => {
            eprintln!(
                "Chat context compaction summary degraded ({} < 30% of previous {}); keeping previous",
                trimmed.chars().count(),
                previous_summary.map(|p| p.trim().chars().count()).unwrap_or(0)
            );
            None
        }
    }
}

/// 摘要质量判定结果（`compact_with_summary_model` 的质量兜底）。
#[derive(Debug, PartialEq, Eq)]
enum SummaryQuality {
    Ok,
    TooShort,
    Truncated,
    Degraded,
}

/// 纯函数质量兜底：
/// - 截断（含 `<analysis>` 却无 `<summary>`）：Claude 9 段格式先吐 `<analysis>` 再吐 `<summary>`，
///   流被截在两者之间时 `extract_summary_text` 回退返回整段 analysis 前言，可能 >200 字骗过长度闸 →
///   拒绝（`Truncated`）。非 9 段格式的纯摘要不含 `<analysis>`，不受影响。
/// - 过短（< `MIN_SUMMARY_CHARS`）→ `TooShort`。
/// - 相对旧 summary 显著劣化（< 旧 summary 长度 30%，且旧 summary 本身达标）→ `Degraded`。
///
/// 抽成纯函数便于单元测试「截断拒绝」「过短拒绝」「劣化拒绝」「链式合并达标通过」等用例。
fn summary_quality_guard(trimmed: &str, previous_summary: Option<&str>) -> SummaryQuality {
    if trimmed.contains("<analysis>") && !trimmed.contains("<summary>") {
        return SummaryQuality::Truncated;
    }
    let trimmed_len = trimmed.chars().count();
    if trimmed_len < MIN_SUMMARY_CHARS {
        return SummaryQuality::TooShort;
    }
    if let Some(previous) = previous_summary {
        let prev_len = previous.trim().chars().count();
        // 仅当旧 summary 本身达标时才用「30%」门槛——避免用一份烂旧 summary 卡死新摘要。
        if prev_len >= MIN_SUMMARY_CHARS && trimmed_len * 10 < prev_len * 3 {
            return SummaryQuality::Degraded;
        }
    }
    SummaryQuality::Ok
}

/// 链式重摘衰减告警（R-4）：累计压缩次数达 `DECAY_WARNING_COMPRESSION_COUNT` 时返回一条英文
/// `context_state.warning`（沿用现有告警都是后端原始英文串的惯例），否则 None。压缩成功后由
/// 两条落盘路径（`compact_conversation` + L2 写回）调用，替代无条件的 `warning = None`。
pub(crate) fn decay_warning_for(compression_count: usize) -> Option<String> {
    if compression_count >= DECAY_WARNING_COMPRESSION_COUNT {
        Some(format!(
            "This conversation has been compressed {compression_count} times; repeated compression can reduce accuracy. Consider starting a new conversation."
        ))
    } else {
        None
    }
}

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
) -> Option<(Vec<Value>, String)> {
    let (system_prefix, old_segment, recent) = select_recent_by_tokens(messages, keep_tokens);
    if old_segment.is_empty() {
        // 没有可摘要的旧段（全在受保护近期窗口里）——压缩无能为力。
        return None;
    }

    // anchored 链式摘要（R8）：若历史含上一份摘要（old_segment 锚点 或 system 前缀注入），
    // 作为 previous_summary 合并更新，且不重复进摘要输入 / 压缩后视图。
    let previous_summary = extract_previous_summary(&system_prefix, &old_segment);

    // head：从 old_segment 剔除锚点摘要及其配对 ack（避免上一轮摘要/ack 作为噪声再进摘要输入）。
    let head: Vec<Value> = if previous_summary.is_some() {
        old_segment
            .iter()
            .filter(|m| !is_anchor_summary(m) && !is_summary_ack(m))
            .cloned()
            .collect()
    } else {
        old_segment.clone()
    };

    // 注入摘要来自 system 前缀——它已并入新摘要，从压缩后视图的 system 前缀里剔除，避免重复。
    let system_prefix: Vec<Value> = if previous_summary
        .as_ref()
        .map(|p| p.from_injected)
        .unwrap_or(false)
    {
        system_prefix
            .into_iter()
            .filter(|m| !is_injected_summary(m))
            .collect()
    } else {
        system_prefix
    };

    // 序列化旧段 head（未裁剪），统一交由 `compact_with_summary_model` 做预算封顶 + 流式调用 + 质量兜底。
    let serialized_head = serialize_for_summary(&head);
    let summary_text = compact_with_summary_model(
        state,
        provider,
        model,
        &serialized_head,
        previous_summary.as_ref().map(|p| p.text.as_str()),
        focus,
        window,
        config_max_output_tokens,
        retry_attempts,
        conversation_id,
        message_id,
        cancel,
    )
    .await;

    match summary_text {
        Some(text) => Some((
            replace_with_summary(system_prefix, &text, recent),
            text,
        )),
        None => None,
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
    // 统一基准：裸窗口 × AUTO_COMPACT_RATIO（0.90），对齐 Codex。去掉 safe_window 折扣——
    // 触发 / 摘要输入封顶都用同一个裸窗口，三处触发（落盘 / L2 / 手动）口径一致。
    let window = context_window_for_model(Some(&config.provider), &config.model).0;
    if window == 0 {
        return state.runtime_messages.clone();
    }
    let budget = (window as f32 * AUTO_COMPACT_RATIO) as usize;
    let estimated = estimate_messages_tokens(&state.runtime_messages);
    if estimated <= budget {
        // 未超预算：本步无需压缩。重置 anti-thrashing 计数（Gap 2）——上下文已回到预算内。
        state.compaction_unresolved_rounds = 0;
        return state.runtime_messages.clone();
    }

    eprintln!(
        "Chat context compaction: est {estimated} tokens over budget {budget} (window {window}); summarizing old history"
    );

    env.host.emit_compaction_status(
        &config.conversation_id,
        "started",
        Some("agent_loop"),
        None,
    );

    // 受保护近期窗口默认 20k token，但不得超过压缩预算——否则小窗口模型上整段历史会被近期窗口
    // 吞掉，没有可摘要的旧段，压缩永远救不了超窗。
    let keep_tokens = RECENT_KEEP_TOKENS.min(budget);

    // Microcompact（R-1）：先尝试把旧段工具结果降级成标记，够了就跳过昂贵的 LLM 摘要
    // （对齐 Claude Code "能拖就拖、便宜优先"）。仅当降级足以回到预算内才走此分支。
    if let Some(degraded) = microcompact_send_view(&state.runtime_messages, keep_tokens, budget) {
        let after = estimate_messages_tokens(&degraded);
        eprintln!("Chat context microcompaction: est {estimated} -> {after} tokens (skipped summary)");
        state.runtime_messages = degraded.clone();
        state.compacted = true;
        state.compaction_unresolved_rounds = 0;
        env.host.emit_compaction_status(
            &config.conversation_id,
            "microcompacted",
            Some("agent_loop"),
            None,
        );
        return degraded;
    }

    // 降级不足以回到预算内——走重型 LLM 摘要。取消 future 只在这条路径需要。
    let cancel = env
        .host
        .wait_for_generation_inactive(&config.conversation_id, config.generation);
    let runtime_before_compact = state.runtime_messages.clone();
    let compacted = summarize_history(
        config.state,
        &config.provider,
        &config.model,
        &state.runtime_messages,
        keep_tokens,
        window,
        // 用模型真实 max output（而非 run 的 config.max_output_tokens），与持久化路径
        // compact_conversation 口径统一——否则 run 配的小输出会把摘要卡短、9 段产出被截。
        chat_max_output_tokens_for_model(
            Some(&config.provider),
            &config.model,
            config.max_output_tokens,
        ),
        config.retry_attempts,
        &config.conversation_id,
        &config.message_id,
        None,
        Some(cancel),
    )
    .await;

    match compacted {
        Some((compacted, summary_text)) => {
            let after = estimate_messages_tokens(&compacted);
            eprintln!("Chat context compaction: est {estimated} -> {after} tokens");
            state.runtime_messages = compacted.clone();
            state.compacted = true;
            if after <= budget {
                state.compaction_unresolved_rounds = 0;
            } else {
                state.compaction_unresolved_rounds =
                    state.compaction_unresolved_rounds.saturating_add(1);
            }
            if let Some(source_until_message_id) =
                source_until_message_id_for_split(&runtime_before_compact, keep_tokens)
            {
                let created_at = chrono::Local::now().timestamp();
                let summary_record = ConversationContextSummary {
                    id: format!("ctxsum_{}", uuid::Uuid::new_v4()),
                    content: summary_text.clone(),
                    source_message_ids: Vec::new(),
                    source_until_message_id: source_until_message_id.clone(),
                    token_estimate_before: estimated,
                    token_estimate_after: estimate_tokens(&summary_text),
                    created_at,
                    provider_id: config.provider.id.clone(),
                    model: config.model.clone(),
                    stale: false,
                };
                let boundary = CompactionBoundaryRecord {
                    id: format!("ctxbd_{}", uuid::Uuid::new_v4()),
                    source_until_message_id,
                    // 时间线锚点：触发压缩时 runtime 里最后一条可映射的 UI 消息（run 进行中
                    // assistant 尚未落库，即最后一条 user）——divider 标记压缩发生的时刻。
                    display_after_message_id: runtime_before_compact
                        .iter()
                        .rev()
                        .find_map(|m| m.get(UI_MESSAGE_ID_KEY).and_then(Value::as_str))
                        .map(str::to_string),
                    token_estimate_before: estimated,
                    token_estimate_after: after,
                    summary_content: summary_text,
                    trigger: "agent_loop".to_string(),
                    created_at,
                };
                env.host.emit_compaction_status(
                    &config.conversation_id,
                    "completed",
                    Some("agent_loop"),
                    Some(&boundary),
                );
                state.pending_compaction_boundary = Some(boundary);
                state.pending_compaction_summary = Some(summary_record);
            } else {
                // 压缩视图已生效但无法可靠映射回 UI 消息（旧段只有摘要锚点/系统注入）——
                // 不落盘 boundary，但必须发终止事件让前端"压缩中"归位。
                env.host.emit_compaction_status(
                    &config.conversation_id,
                    "completed",
                    Some("agent_loop"),
                    None,
                );
            }
            compacted
        }
        None => {
            // Gap 2: 需要压缩（超预算）但压缩没能减小上下文（摘要调用失败/为空/过短/无旧段）——
            // 计为一次「未解决」。连续达到 COMPACTION_THRASH_LIMIT 次时，规划循环会据此优雅收尾，
            // 而不是反复触发压缩并失败 6+ 次后才报错。
            state.compaction_unresolved_rounds =
                state.compaction_unresolved_rounds.saturating_add(1);
            // started 已发——失败也必须发终止事件，否则前端"压缩中"状态永久卡死。
            env.host.emit_compaction_status(
                &config.conversation_id,
                "failed",
                Some("agent_loop"),
                None,
            );
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
    let window = context_window_for_model(Some(provider), model).0;
    summarize_history(
        state,
        provider,
        model,
        messages,
        RECENT_KEEP_TOKENS,
        window,
        config_max_output_tokens,
        retry_attempts,
        conversation_id,
        message_id,
        focus,
        None,
    )
    .await
    .map(|(messages, _summary)| messages)
}

/// 手动压缩的保底切分（R4）：token 尾窗覆盖全部消息（无旧段）时，`/compact` 不该直接报
/// "没有足够的旧消息可以压缩"——旧行为（≤v2.7 落盘路径）小对话也可压。仅 `trigger == "manual"`
/// 且 `summary_start..len` 区间 UI 消息数 > 4 时生效：保留最后一条 user 及其后消息为近期窗口，
/// 其余进 old_segment；返回 old_segment 末尾下标。区间太短或末尾无可切点 → None（保持原报错）。
/// auto / agent_loop 触发条件不受影响（它们要超 90% 窗口才会走到这里）。
fn manual_fallback_split(
    messages: &[ChatMessage],
    summary_start: usize,
    trigger: &str,
) -> Option<usize> {
    if trigger != "manual" {
        return None;
    }
    let len = messages.len();
    if len.saturating_sub(summary_start) <= 4 {
        return None;
    }
    let last_user = messages[summary_start..]
        .iter()
        .rposition(|m| m.role == "user")
        .map(|offset| summary_start + offset)?;
    // 最后一条 user 之前必须还有可摘要内容。
    if last_user <= summary_start {
        return None;
    }
    Some(last_user - 1)
}

/// 累积 `source_message_ids`：上一份未过期 summary 的 ids ∪ 其 boundary 之后至 `until_id`
///（含）的全部消息 id（**按原始序列**，含多答排除臂）。落盘路径 `compact_conversation_inner`
/// 与 L2 run 结束写回（commands.rs）**共用**，防止两处累积口径分叉。
/// 找不到 `until_id` → 仅返回旧 ids（防御，不 panic）。
pub(crate) fn accumulate_source_ids(conversation: &Conversation, until_id: &str) -> Vec<String> {
    let prev = conversation
        .context_state
        .summary
        .as_ref()
        .filter(|s| !s.stale);
    let mut ids = prev
        .map(|s| s.source_message_ids.clone())
        .unwrap_or_default();
    let summary_start = prev
        .and_then(|s| {
            conversation
                .messages
                .iter()
                .position(|m| m.id == s.source_until_message_id)
        })
        .map(|idx| idx + 1)
        .unwrap_or(0);
    let Some(until_idx) = conversation.messages.iter().position(|m| m.id == until_id) else {
        return ids;
    };
    if until_idx < summary_start {
        return ids;
    }
    ids.extend(
        conversation.messages[summary_start..=until_idx]
            .iter()
            .map(|m| m.id.clone()),
    );
    ids
}

/// 落盘压缩统一入口（手动 `chat_compress_context` / 自动发送前 / L2 run 结束三处共用）。
/// 按 token 尾窗切 old_segment / recent_tail，序列化 old_segment（含完整工具转录，工具结果截 2000 字），
/// 调统一核心 `compact_with_summary_model`（Claude 9 段 prompt + 流式 + 质量兜底），写回
/// `context_state.summary` + `compaction_boundaries` + `compression_count`，发 `chat-compaction` 事件。
///
/// `trigger`: `"manual"` | `"auto"`。`focus`：手动 `/compact <focus>` 聚焦指令（自动为 None）。
/// 失败 / 无可摘要旧段 / 摘要质量不达标 → `Err`，**不覆盖**旧 summary。
///
/// 事件配对保证：入口发 `started`，任何 `Err` 出口发 `failed`，成功出口发 `completed`
/// （由 `compact_conversation_inner` 发）。前端靠终止事件把"压缩中"状态归位——
/// 缺失终止事件会让 UI 永久卡在压缩中。
pub(crate) async fn compact_conversation(
    app: &AppHandle,
    state: &AppState,
    settings: &Settings,
    conversation: &mut Conversation,
    trigger: &str,
    focus: Option<&str>,
) -> Result<(), String> {
    emit_compaction_event(app, &conversation.id, "started", Some(trigger), None);
    let result = compact_conversation_inner(app, state, settings, conversation, trigger, focus).await;
    if result.is_err() {
        emit_compaction_event(app, &conversation.id, "failed", Some(trigger), None);
    }
    result
}

async fn compact_conversation_inner(
    app: &AppHandle,
    state: &AppState,
    settings: &Settings,
    conversation: &mut Conversation,
    trigger: &str,
    focus: Option<&str>,
) -> Result<(), String> {
    // 上一份落盘 summary 之后才进 old_segment；其之前已被摘要覆盖，不重复进。
    let summary_start = conversation
        .context_state
        .summary
        .as_ref()
        .filter(|s| !s.stale)
        .and_then(|s| {
            conversation
                .messages
                .iter()
                .position(|m| m.id == s.source_until_message_id)
        })
        .map(|idx| idx + 1)
        .unwrap_or(0);

    let split = token_split_chat_messages(&conversation.messages, summary_start, RECENT_KEEP_TOKENS)
        .or_else(|| manual_fallback_split(&conversation.messages, summary_start, trigger))
        .ok_or_else(|| "没有足够的旧消息可以压缩".to_string())?;
    let old_segment = &conversation.messages[summary_start..=split];
    if old_segment.is_empty() {
        return Err("没有足够的旧消息可以压缩".to_string());
    }
    let source_until_message_id = old_segment
        .last()
        .map(|m| m.id.clone())
        .ok_or_else(|| "没有足够的旧消息可以压缩".to_string())?;

    // 摘要模型：mixer 选 auto 时跟随当前会话主模型（effective_compression_model_for_session）。
    let (provider_id, model) =
        settings.effective_compression_model_for_session(Some(session_model_for(conversation)));
    let provider = settings
        .get_provider(&provider_id)
        .ok_or_else(|| "Compression provider not found".to_string())?
        .clone();
    if provider.api_keys.is_empty() {
        return Err(format_chat_missing_api_key_error(&provider.name));
    }
    if model.trim().is_empty() {
        return Err(chat_missing_model_error());
    }
    let retry_attempts = if settings.retry_enabled {
        settings.retry_attempts as usize
    } else {
        1
    };
    let window = context_window_for_model(Some(&provider), &model).0;

    let previous_summary = conversation
        .context_state
        .summary
        .as_ref()
        .filter(|s| !s.stale)
        .map(|s| s.content.clone());
    let serialized_head = serialize_chat_messages_for_summary(old_segment);
    let message_id = source_until_message_id.clone();
    let summary_text = compact_with_summary_model(
        state,
        &provider,
        &model,
        &serialized_head,
        previous_summary.as_deref(),
        focus,
        window,
        chat_max_output_tokens_for_model(
            Some(&provider),
            &model,
            settings.chat.max_output_tokens,
        ),
        retry_attempts,
        &conversation.id,
        &message_id,
        None,
    )
    .await
    .ok_or_else(|| "Compression model returned an overly short or empty summary".to_string())?;

    let created_at = chrono::Local::now().timestamp();
    let token_estimate_before = previous_summary
        .as_deref()
        .map(estimate_tokens)
        .unwrap_or(0)
        + estimate_tokens(&serialized_head);
    let token_estimate_after = estimate_tokens(&summary_text);

    // 累积覆盖范围的全部消息 id（含多答排除臂——它们同样被 boundary 覆盖、不再 replay）。
    // 用共享 helper 而非 `old_segment.iter()`：批次 C 后 old_segment 会过滤掉排除臂，
    // 但 source_message_ids 必须按原始序列收集，两处口径由 helper 统一。
    let source_message_ids = accumulate_source_ids(conversation, &source_until_message_id);
    let compressed_message_count = source_message_ids.len();


    conversation.context_state.summary = Some(ConversationContextSummary {
        id: format!("ctxsum_{}", uuid::Uuid::new_v4()),
        content: summary_text.clone(),
        source_message_ids,
        source_until_message_id: source_until_message_id.clone(),
        token_estimate_before,
        token_estimate_after,
        created_at,
        provider_id,
        model,
        stale: false,
    });
    conversation.context_state.last_compressed_at = Some(created_at);
    conversation.context_state.compressed_message_count = compressed_message_count;
    conversation.context_state.compression_count = conversation
        .context_state
        .compression_count
        .saturating_add(1);

    let boundary_record = CompactionBoundaryRecord {
        id: format!("ctxbd_{}", uuid::Uuid::new_v4()),
        source_until_message_id,
        // 时间线锚点：divider 显示在「触发压缩时的最后一条消息」之后——标记压缩发生的
        // 时刻，而非 token 切分落点（切分落点在长对话里远高于触发点，观感是"横线跑上面去"）。
        display_after_message_id: conversation.messages.last().map(|m| m.id.clone()),
        token_estimate_before,
        token_estimate_after,
        summary_content: summary_text,
        trigger: trigger.to_string(),
        created_at,
    };
    conversation
        .context_state
        .compaction_boundaries
        .push(boundary_record.clone());
    // R-4：多次链式压缩后提示准确性下降；未达阈值则清空告警（清掉上一轮"压缩失败但已发送"等旧警告）。
    conversation.context_state.warning =
        decay_warning_for(conversation.context_state.compression_count);

    emit_compaction_event(
        app,
        &conversation.id,
        "completed",
        Some(trigger),
        Some(&boundary_record),
    );
    Ok(())
}

/// 发 `chat-compaction` 事件（与 commands.rs 的 `emit_chat_compaction_state` 同 payload）。
fn emit_compaction_event(
    app: &AppHandle,
    conversation_id: &str,
    phase: &str,
    trigger: Option<&str>,
    boundary: Option<&CompactionBoundaryRecord>,
) {
    let _ = app.emit(
        "chat-compaction",
        serde_json::json!({
            "conversationId": conversation_id,
            "phase": phase,
            "trigger": trigger,
            "boundary": boundary,
        }),
    );
}

/// 由当前会话解析出主模型（供 compression/title 等 auxiliary 任务在 mixer 选 auto 时跟随）。
fn session_model_for(conversation: &Conversation) -> crate::settings::SessionModel<'_> {
    crate::settings::SessionModel {
        provider_id: &conversation.provider_id,
        model: &conversation.model,
    }
}

/// 落盘路径「是否有可压缩旧段」判定（供 `should_auto_compress_context` 用）：
/// 在上一份未过期 summary 之后、按 `RECENT_KEEP_TOKENS` 尾窗切分后，是否还存在 old_segment。
pub(crate) fn has_compressible_old_segment(conversation: &Conversation) -> bool {
    let summary_start = conversation
        .context_state
        .summary
        .as_ref()
        .filter(|s| !s.stale)
        .and_then(|s| {
            conversation
                .messages
                .iter()
                .position(|m| m.id == s.source_until_message_id)
        })
        .map(|idx| idx + 1)
        .unwrap_or(0);
    token_split_chat_messages(&conversation.messages, summary_start, RECENT_KEEP_TOKENS).is_some()
}

fn format_chat_missing_api_key_error(provider_name: &str) -> String {
    format!(
        "Provider 「{provider_name}」未配置 API Key，无法执行上下文压缩。请在设置中添加该 Provider 的密钥。"
    )
}

fn chat_missing_model_error() -> String {
    "未选择压缩模型，请在设置中指定或保持 auto 跟随当前会话模型。".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::types::{ToolCallRecord, ToolCallStatus};

    fn chat_msg(id: &str, role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            role: role.to_string(),
            content: content.to_string(),
            attachments: Vec::new(),
            reasoning: None,
            artifacts: Vec::new(),
            tool_calls: Vec::new(),
            segments: Vec::new(),
            agent_plan: None,
            api_messages: Vec::new(),
            model_messages: Vec::new(),
            active_skill_id: None,
            run_entry: None,
            stream_outcome: None,
            usage: None,
            group_id: None,
            provider_id: None,
            model: None,
            timestamp: 0,
        }
    }

    fn test_conversation(messages: Vec<ChatMessage>) -> Conversation {
        Conversation {
            id: "conv_test".to_string(),
            title: "t".to_string(),
            provider_id: "p".to_string(),
            model: "m".to_string(),
            messages,
            agent_runtime: Default::default(),
            active_skill_id: None,
            assistant_id: None,
            assistant_snapshot: None,
            created_at: 0,
            updated_at: 0,
            pinned: false,
            folder: None,
            project_id: None,
            set_id: None,
            context_state: Default::default(),
            agent_todo_state: Default::default(),
            agent_plan_state: Default::default(),
            knowledge_base_ids: Vec::new(),
            thinking_level: None,
            reply_models: Vec::new(),
            group_selections: Default::default(),
        }
    }

    #[test]
    fn estimate_chat_message_tokens_counts_content_and_tools() {
        let mut m = chat_msg("m1", "assistant", &"abcd".repeat(100));
        m.reasoning = Some("r".repeat(40));
        m.tool_calls.push(ToolCallRecord {
            id: "c1".to_string(),
            name: "read".to_string(),
            source: String::new(),
            server_id: None,
            arguments: "{\"path\":\"/tmp/x\"}".to_string(),
            status: ToolCallStatus::Success,
            result_preview: Some("p".repeat(80)),
            error: None,
            duration_ms: None,
            started_at: None,
            completed_at: None,
            round: 0,
            sensitive: false,
            artifacts: Vec::new(),
            trace_id: None,
            span_id: None,
            structured_content: None,
        });
        let tokens = estimate_chat_message_tokens(&m);
        // content 100/4=25, reasoning 40/4=10, tool name+args+preview ~ 25+, +1
        assert!(tokens > 60);
    }

    #[test]
    fn token_split_chat_messages_keeps_recent_tail_in_budget() {
        // 每条 5004 tokens（20000 chars/4 + 4 每条开销）；3 条总 15012 ≤ 20000 → 全在尾窗 → None。
        let small: Vec<ChatMessage> = (0..3)
            .map(|i| chat_msg(&format!("s{i}"), "user", &"a".repeat(20_000)))
            .collect();
        assert!(token_split_chat_messages(&small, 0, RECENT_KEEP_TOKENS).is_none());

        // 5 条各 5004 tokens。从尾累积 3 条(15012) 后第 4 条 → 20016 > 20000 越预算 →
        // old_segment=[b0,b1], recent=[b2..b4], boundary=1。
        let big: Vec<ChatMessage> = (0..5)
            .map(|i| chat_msg(&format!("b{i}"), "user", &"a".repeat(20_000)))
            .collect();
        let split = token_split_chat_messages(&big, 0, RECENT_KEEP_TOKENS).expect("split");
        assert_eq!(split, 1);
    }

    #[test]
    fn token_split_chat_messages_respects_summary_start() {
        // summary_start=2：index 0/1 属于上一份 summary、不参与本次；只从 index 2 起往尾扫。
        // 6 条各 ~20004 tokens（80000 chars），尾窗 20000 只容 1 条 → 其余进 old_segment，
        // boundary 落在倒数第 2 条(index 4)，且必然 > summary_start。
        let msgs: Vec<ChatMessage> = (0..6)
            .map(|i| chat_msg(&format!("m{i}"), "user", &"a".repeat(80_000)))
            .collect();
        let split = token_split_chat_messages(&msgs, 2, RECENT_KEEP_TOKENS).expect("split");
        assert_eq!(split, msgs.len() - 2);
        assert!(split > 2);
    }

    #[test]
    fn serialize_chat_message_includes_full_tool_args_and_clipped_result() {
        let mut m = chat_msg("m1", "assistant", "let me read it");
        m.tool_calls.push(ToolCallRecord {
            id: "c1".to_string(),
            name: "read".to_string(),
            source: "native".to_string(),
            server_id: None,
            arguments: "{\"path\":\"/tmp/important.txt\"}".to_string(),
            status: ToolCallStatus::Success,
            result_preview: Some("T".repeat(10_000)),
            error: None,
            duration_ms: None,
            started_at: None,
            completed_at: None,
            round: 0,
            sensitive: false,
            artifacts: Vec::new(),
            trace_id: None,
            span_id: None,
            structured_content: None,
        });
        let s = serialize_chat_message_for_summary(&m);
        // 工具入参全文保留（修复「收到 ✅」根因）。
        assert!(s.contains("\"/tmp/important.txt\""));
        // 工具结果截到 2000 字（[truncated] 标记）。
        assert!(s.contains("[truncated]"));
        assert!(!s.contains(&"T".repeat(TOOL_OUTPUT_SUMMARY_MAX_CHARS + 1)));
    }

    #[test]
    fn summary_quality_guard_rejects_too_short() {
        // 「收到 ✅」式过短摘要 → TooShort，不覆盖旧 summary。
        let short = "收到 ✅";
        assert_eq!(
            summary_quality_guard(short, None),
            SummaryQuality::TooShort
        );
    }

    #[test]
    fn summary_output_tokens_caps_at_8192() {
        // R-3：容纳 9 段 analysis+summary，上限 8192；min() 语义保留。
        assert_eq!(SUMMARY_OUTPUT_TOKENS, 8_192);
        assert_eq!(summary_output_tokens(20_000), 8_192);
        assert_eq!(summary_output_tokens(200_000), 8_192);
        assert_eq!(summary_output_tokens(4_096), 4_096); // 真实上限更小的模型不受影响
    }

    #[test]
    fn decay_warning_for_fires_at_threshold() {
        // R-4：阈值 3；未达 → None；达到/超过 → Some 且含实际次数。
        assert_eq!(DECAY_WARNING_COMPRESSION_COUNT, 3);
        assert_eq!(decay_warning_for(0), None);
        assert_eq!(decay_warning_for(2), None);
        let w3 = decay_warning_for(3).expect("warning at threshold");
        assert!(w3.contains('3'));
        let w5 = decay_warning_for(5).expect("warning above threshold");
        assert!(w5.contains('5'));
    }

    #[test]
    fn summary_quality_guard_rejects_truncated_analysis() {
        // 截断的 9 段输出：吐了 <analysis> 前言但流断在 <summary> 之前，
        // extract_summary_text 回退返回整段 analysis（>200 字，能骗过长度闸）→ Truncated。
        let truncated = format!("<analysis>\n{}", "分析前言细节".repeat(60));
        assert!(truncated.chars().count() >= MIN_SUMMARY_CHARS);
        assert_eq!(
            summary_quality_guard(&truncated, None),
            SummaryQuality::Truncated
        );
    }

    #[test]
    fn summary_quality_guard_accepts_long_fresh_summary() {
        let long = "x".repeat(MIN_SUMMARY_CHARS + 10);
        assert_eq!(
            summary_quality_guard(&long, None),
            SummaryQuality::Ok
        );
    }

    #[test]
    fn summary_quality_guard_rejects_degraded_vs_previous() {
        // 旧 summary 达标（300 字），新 summary 仅 50 字 < 30%×300=90 → Degraded。
        let previous = "p".repeat(300);
        let degraded = "n".repeat(50);
        assert_eq!(
            summary_quality_guard(&degraded, Some(&previous)),
            SummaryQuality::Degraded
        );
    }

    #[test]
    fn summary_quality_guard_accepts_chain_merge_when_comparable() {
        // 链式合并：新 summary 与旧 summary 长度相当 → Ok（允许覆盖更新）。
        let previous = "p".repeat(300);
        let merged = "m".repeat(280);
        assert_eq!(
            summary_quality_guard(&merged, Some(&previous)),
            SummaryQuality::Ok
        );
    }

    #[test]
    fn summary_quality_guard_skips_30pct_gate_when_previous_short() {
        // 旧 summary 本身不达标 → 不用 30% 门槛，只要新 summary 达标即 Ok
        // （避免一份烂旧 summary 卡死新摘要）。
        let previous = "p".repeat(50);
        let fresh = "n".repeat(MIN_SUMMARY_CHARS + 5);
        assert_eq!(
            summary_quality_guard(&fresh, Some(&previous)),
            SummaryQuality::Ok
        );
    }

    /// 构造带 `_ui_message_id` 标注的 runtime 消息（模拟 build_chat_api_messages 的注入）。
    fn tagged(ui_id: &str, role: &str, content: &str) -> Value {
        json!({ "role": role, "content": content, UI_MESSAGE_ID_KEY: ui_id })
    }

    #[test]
    fn source_until_maps_by_ui_tag_with_tool_expansion() {
        // UI 消息 m2（assistant）展开成 3 条 runtime（tool_calls / tool / 最终答复），
        // 全部落在旧段 → boundary 精确落在 m2；旧的条数推算会把展开的每条都计数而错位。
        let messages = vec![
            json!({ "role": "system", "content": "sys" }),
            tagged("m1", "user", &"a".repeat(4_000)),
            {
                let mut m = json!({ "role": "assistant", "content": "", "tool_calls": [{ "id": "c1", "type": "function", "function": { "name": "read", "arguments": "{}" } }] });
                m.as_object_mut().unwrap().insert(UI_MESSAGE_ID_KEY.into(), json!("m2"));
                m
            },
            {
                let mut m = json!({ "role": "tool", "tool_call_id": "c1", "content": "b".repeat(4_000) });
                m.as_object_mut().unwrap().insert(UI_MESSAGE_ID_KEY.into(), json!("m2"));
                m
            },
            tagged("m2", "assistant", &"c".repeat(4_000)),
            tagged("m3", "user", "recent"),
        ];
        let until = source_until_message_id_for_split(&messages, 500);
        assert_eq!(until.as_deref(), Some("m2"));
    }

    #[test]
    fn source_until_skips_ui_message_straddling_boundary() {
        // m2 的展开条横跨边界（一部分在近期窗口）→ 不能作为 boundary，回退到 m1。
        let messages = vec![
            json!({ "role": "system", "content": "sys" }),
            tagged("m1", "user", &"a".repeat(8_000)),
            tagged("m2", "assistant", &"b".repeat(8_000)),
            tagged("m2", "assistant", "tail piece in recent"),
            tagged("m3", "user", "recent"),
        ];
        // keep=1000：recent 从尾部起 ~2 条小消息（m2 尾块 + m3），m2 首块在旧段 → 跨边界。
        let until = source_until_message_id_for_split(&messages, 1_000);
        assert_eq!(until.as_deref(), Some("m1"));
    }

    #[test]
    fn source_until_none_when_old_segment_untagged() {
        // 旧段只有摘要锚点/系统注入（无 _ui_message_id）→ None，调用方不落盘 boundary。
        let messages = vec![
            json!({ "role": "system", "content": "sys" }),
            json!({ "role": "user", "content": format!("{SUMMARY_MARKER_PREFIX} 摘要：\n{}", "s".repeat(8_000)) }),
            json!({ "role": "assistant", "content": "已了解早前对话的摘要，继续当前任务。" }),
            tagged("m9", "user", "recent question"),
        ];
        assert!(source_until_message_id_for_split(&messages, 1_000).is_none());
    }

    #[test]
    fn source_until_none_when_no_old_segment() {
        let messages = vec![
            json!({ "role": "system", "content": "sys" }),
            tagged("m1", "user", "hi"),
            tagged("m2", "assistant", "hello"),
        ];
        assert!(source_until_message_id_for_split(&messages, 8_000).is_none());
    }

    #[test]
    fn manual_fallback_split_keeps_last_user_pair() {
        // 6 条小消息（token 尾窗覆盖全部）：manual 保底切到最后一条 user 之前。
        let msgs: Vec<ChatMessage> = [
            ("m0", "user"), ("m1", "assistant"), ("m2", "user"),
            ("m3", "assistant"), ("m4", "user"), ("m5", "assistant"),
        ]
        .iter()
        .map(|(id, role)| chat_msg(id, role, "short"))
        .collect();
        assert!(token_split_chat_messages(&msgs, 0, RECENT_KEEP_TOKENS).is_none());
        // 最后一条 user 是 m4(index 4) → old_segment 末尾 = index 3。
        assert_eq!(manual_fallback_split(&msgs, 0, "manual"), Some(3));
        // 非手动触发不放宽。
        assert_eq!(manual_fallback_split(&msgs, 0, "auto"), None);
    }

    #[test]
    fn manual_fallback_split_rejects_short_conversations() {
        let msgs: Vec<ChatMessage> = [("m0", "user"), ("m1", "assistant"), ("m2", "user"), ("m3", "assistant")]
            .iter()
            .map(|(id, role)| chat_msg(id, role, "short"))
            .collect();
        // ≤ 4 条 → None（保持"没有足够的旧消息可以压缩"报错）。
        assert_eq!(manual_fallback_split(&msgs, 0, "manual"), None);
        // summary_start 之后区间太短同样拒绝。
        let six: Vec<ChatMessage> = (0..6)
            .map(|i| chat_msg(&format!("m{i}"), if i % 2 == 0 { "user" } else { "assistant" }, "s"))
            .collect();
        assert_eq!(manual_fallback_split(&six, 2, "manual"), None);
    }

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
    fn microcompact_reclaims_old_tool_results_and_skips_summary() {
        // old_segment 由两条大工具结果主导；降级后应回到预算内 → Some（可跳过摘要）。
        let messages = vec![
            json!({ "role": "system", "content": "sys" }),
            json!({ "role": "assistant", "content": "", "tool_calls": [{ "id": "c1", "type": "function", "function": { "name": "read", "arguments": "{}" } }] }),
            json!({ "role": "tool", "tool_call_id": "c1", "content": "T".repeat(40_000) }),
            json!({ "role": "assistant", "content": "", "tool_calls": [{ "id": "c2", "type": "function", "function": { "name": "read", "arguments": "{}" } }] }),
            json!({ "role": "tool", "tool_call_id": "c2", "content": "T".repeat(40_000) }),
            json!({ "role": "user", "content": "recent question" }),
            json!({ "role": "assistant", "content": "recent answer" }),
        ];
        // 总 ~20000 tok，budget 12000 → 超；keep 8000 → recent = 末尾两条小消息，两条大工具结果进 old。
        let view = microcompact_send_view(&messages, 8_000, 12_000).expect("microcompact should suffice");
        assert!(estimate_messages_tokens(&view) <= 12_000, "degraded view within budget");
        let markers = view
            .iter()
            .filter(|m| m.get("content").and_then(Value::as_str) == Some(MICROCOMPACT_TOOL_MARKER))
            .count();
        assert_eq!(markers, 2, "both old tool results degraded");
        assert_eq!(view[view.len() - 2]["content"], "recent question");
        assert_eq!(view[view.len() - 1]["content"], "recent answer");
    }

    #[test]
    fn microcompact_returns_none_when_insufficient() {
        // old 段含可降级工具结果 + 一条无法降级的大 user 文本；降级工具后仍超 budget → None。
        // keep_tokens 取小，确保大 user + 工具结果都落在 old（不被拉进近期窗口）。
        let messages = vec![
            json!({ "role": "system", "content": "sys" }),
            json!({ "role": "user", "content": "U".repeat(80_000) }), // ~20000 tok，无法降级
            json!({ "role": "tool", "tool_call_id": "c1", "content": "T".repeat(4_000) }), // 可降级
            json!({ "role": "assistant", "content": "recent answer" }),
        ];
        assert!(microcompact_send_view(&messages, 100, 12_000).is_none());
    }

    #[test]
    fn microcompact_returns_none_when_no_old_segment() {
        // 全在近期窗口内（无旧段）→ None。
        let messages = vec![
            json!({ "role": "system", "content": "sys" }),
            json!({ "role": "user", "content": "hi" }),
            json!({ "role": "assistant", "content": "hello" }),
        ];
        assert!(microcompact_send_view(&messages, 8_000, 12_000).is_none());
    }

    #[test]
    fn microcompact_leaves_recent_tool_results_untouched() {
        // 近期窗口里的工具结果不降级——只降 old_segment。
        let messages = vec![
            json!({ "role": "system", "content": "sys" }),
            json!({ "role": "tool", "tool_call_id": "old", "content": "T".repeat(40_000) }),
            json!({ "role": "user", "content": "q" }),
            json!({ "role": "tool", "tool_call_id": "recent", "content": "recent tool output kept" }),
            json!({ "role": "assistant", "content": "a" }),
        ];
        let view = microcompact_send_view(&messages, 8_000, 12_000).expect("suffices");
        assert!(view.iter().any(|m| m.get("content").and_then(Value::as_str) == Some(MICROCOMPACT_TOOL_MARKER)));
        assert!(view.iter().any(|m| m.get("content").and_then(Value::as_str) == Some("recent tool output kept")));
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
            json!({ "role": "assistant", "content": SUMMARY_ACK_TEXT }),
            json!({ "role": "user", "content": "next question" }),
        ];
        let previous = extract_previous_summary(&[], &old).expect("prior summary detected");
        assert!(previous.text.contains("Primary Request: build X"));
        assert!(!previous.from_injected, "anchor is not an injected summary");
        // No marker present → None.
        let fresh = vec![json!({ "role": "user", "content": "just a question" })];
        assert!(extract_previous_summary(&[], &fresh).is_none());
    }

    #[test]
    fn extract_previous_summary_detects_injected_system_summary() {
        // 落盘 summary 被 build_chat_api_messages 注入为 system 前缀消息（PERSISTED_SUMMARY_PREFIX）。
        let system_prefix = vec![
            json!({ "role": "system", "content": "you are a helpful assistant" }),
            json!({
                "role": "system",
                "content": format!("{PERSISTED_SUMMARY_PREFIX}\n1. Primary Request: earlier goal")
            }),
        ];
        let old_segment = vec![json!({ "role": "user", "content": "later question" })];
        let previous =
            extract_previous_summary(&system_prefix, &old_segment).expect("injected summary detected");
        assert!(previous.text.contains("Primary Request: earlier goal"));
        assert!(previous.from_injected, "must be flagged as injected for prefix stripping");
    }

    #[test]
    fn extract_previous_summary_prefers_anchor_over_injected() {
        // 同 run 内既有注入摘要（system 前缀，过期 S1）又有锚点摘要（old_segment，本轮更新）
        // → 取锚点，避免回退到已过期的落盘 summary。
        let system_prefix = vec![json!({
            "role": "system",
            "content": format!("{PERSISTED_SUMMARY_PREFIX}\nSTALE injected summary")
        })];
        let old_segment = vec![
            json!({ "role": "user", "content": format!("{SUMMARY_MARKER_PREFIX} 引导语：\nFRESH anchor summary") }),
            json!({ "role": "assistant", "content": SUMMARY_ACK_TEXT }),
        ];
        let previous =
            extract_previous_summary(&system_prefix, &old_segment).expect("summary detected");
        assert!(previous.text.contains("FRESH anchor summary"));
        assert!(!previous.text.contains("STALE"));
        assert!(!previous.from_injected);
    }

    #[test]
    fn injected_summary_stripped_from_view_and_head() {
        // 注入摘要识别后：head 序列化不含其正文，压缩后视图的 system 前缀不再含注入消息。
        let injected = json!({
            "role": "system",
            "content": format!("{PERSISTED_SUMMARY_PREFIX}\nEARLIER_SUMMARY_BODY")
        });
        let system_prefix = vec![json!({ "role": "system", "content": "sys" }), injected];
        assert!(is_injected_summary(&system_prefix[1]));
        // 剔除后前缀只剩真正的 system prompt。
        let stripped: Vec<Value> = system_prefix
            .iter()
            .filter(|m| !is_injected_summary(m))
            .cloned()
            .collect();
        assert_eq!(stripped.len(), 1);
        assert_eq!(stripped[0]["content"], "sys");
    }

    #[test]
    fn accumulate_source_ids_unions_previous_and_new_range() {
        use crate::chat::types::ConversationContextState;
        let messages: Vec<ChatMessage> = ["m0", "m1", "m2", "m3", "m4"]
            .iter()
            .map(|id| chat_msg(id, "user", "x"))
            .collect();
        let mut conversation = test_conversation(messages);
        // 已有 S1 覆盖 m0..=m1（source_until = m1，ids = [m0, m1]）。
        conversation.context_state = ConversationContextState {
            summary: Some(ConversationContextSummary {
                id: "ctxsum_prev".to_string(),
                content: "prev".to_string(),
                source_message_ids: vec!["m0".to_string(), "m1".to_string()],
                source_until_message_id: "m1".to_string(),
                token_estimate_before: 0,
                token_estimate_after: 0,
                created_at: 0,
                provider_id: "p".to_string(),
                model: "m".to_string(),
                stale: false,
            }),
            ..Default::default()
        };
        // 新压缩覆盖到 m3 → ids = S1.ids ∪ (m2, m3)。
        let ids = accumulate_source_ids(&conversation, "m3");
        assert_eq!(ids, vec!["m0", "m1", "m2", "m3"]);
        // until_id 未找到 → 仅旧 ids。
        assert_eq!(
            accumulate_source_ids(&conversation, "nope"),
            vec!["m0", "m1"]
        );
    }

    #[test]
    fn accumulate_source_ids_no_previous_summary() {
        let messages: Vec<ChatMessage> = ["a", "b", "c"]
            .iter()
            .map(|id| chat_msg(id, "user", "x"))
            .collect();
        let conversation = test_conversation(messages);
        // 无旧 summary → 从头累积到 until_id。
        assert_eq!(accumulate_source_ids(&conversation, "b"), vec!["a", "b"]);
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
