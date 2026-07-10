# Chat 压缩（Compaction）契约

> 任务来源：`07-02-fix-compaction-stuck-and-boundary-mapping`（2026-07-02）。
> 相关代码：`src-tauri/src/chat/agent/compaction.rs`、`src-tauri/src/chat/commands/context.rs`（`build_chat_api_messages` / `tag_ui_message_id`）、`src/chat/compactionBoundary.ts`、`src/chat/Chat.tsx`。

## Scenario: chat-compaction 事件与 boundary 落盘

### 1. Scope / Trigger

跨层契约：Rust 后端发 Tauri 事件 + 落盘 `context_state`，前端据此渲染时间线 divider / 压缩动画 / 压缩中状态。改动压缩路径、事件 payload、boundary 记录字段时必须遵守本契约。

### 2. Signatures

```rust
// src-tauri/src/chat/types.rs
pub struct CompactionBoundaryRecord {
    pub id: String,                                  // "ctxbd_<uuid>"
    pub source_until_message_id: String,             // 上下文切分点（被摘要段末尾的 UI 消息）
    #[serde(default)]
    pub display_after_message_id: Option<String>,    // 时间线锚点（触发压缩时的最后一条消息）
    pub token_estimate_before: usize,
    pub token_estimate_after: usize,
    pub summary_content: String,
    pub trigger: String,                             // "manual" | "auto" | "agent_loop"
    pub created_at: i64,
}
```

```ts
// src/chat/api.ts (payload) / src/chat/types.ts (record)
type ChatCompactionPayload = {
  conversationId: string
  phase: 'started' | 'completed' | 'microcompacted' | 'failed' | string
  trigger?: 'manual' | 'auto' | 'agent_loop' | string
  boundary?: CompactionBoundaryRecord | null
}
```

### 3. Contracts

**事件配对（硬约束）**：`started` 一旦发出，同一次压缩**必须**跟一个终止事件（`completed` / `microcompacted` / `failed`）。前端 `Chat.tsx` 收到 `started` 置 `agentLoopCompacting=true`，只有终止事件能清——漏发 = UI 永久卡"压缩中"。实现上 `compact_conversation` 是单出口结构（外层 wrapper 发 started，`Err` 统一发 failed），新增提前 return 不需要单独补事件；`maybe_compact_send_view`（agent_loop）的每个分支都要显式发终止事件。`completed` 允许 `boundary: null`（压缩生效但无法映射 UI boundary）。

**双锚点语义**：
- `source_until_message_id` = **上下文真相**：该消息（含）之前已被摘要覆盖，`build_chat_api_messages` 从它之后 replay 原文。写错会静默丢上下文。
- `display_after_message_id` = **时间线显示**：divider 永远渲染在"压缩触发时刻"的位置（用户心智模型），与 token 切分点无关。前端 `readDisplayAfterId` 缺失时回退 `source_until_message_id`（旧记录兼容）；锚点消息被 regenerate/delete 删掉时同样回退到切分点（divider 不消失）。

**runtime→UI 映射（`_ui_message_id` 标注）**：`build_chat_api_messages` 给每条来自 UI 消息的 runtime 消息注入 `"_ui_message_id"`（一条 UI 消息展开出的多条共享同一 id；system prompt / summary 注入消息不标注）。该字段发给 provider 前被 `model_message_from_openai_message` 剥离（只抽已知字段），**不进任何 wire 请求**。`source_until_message_id_for_split` 按标注映射，UI 消息横跨切分边界时回退到上一个完整落入旧段的 id；映射失败返回 `None`（不落盘 boundary，但仍发终止事件）。

### 4. Validation & Error Matrix

| 条件 | 行为 |
|---|---|
| 摘要失败 / 质量闸拒绝（空、<200 字、截断、劣化） | `Err` → 发 `failed`，不覆盖旧 summary |
| 无可摘要旧段（token 尾窗覆盖全部）且 trigger≠manual | `Err("没有足够的旧消息可以压缩")` → `failed` |
| 同上但 trigger==manual 且区间消息数 > 4 | `manual_fallback_split` 保底：切到最后一条 user 之前 |
| agent_loop 压缩成功但 UI 映射失败 | 压缩视图生效，发 `completed`（boundary: null），不落盘 |
| display 锚点消息已被删除（前端） | divider 回退渲染在 `source_until_message_id` 位置 |

### 5. Good/Base/Bad Cases

- **Good**：手动压缩 6 条小消息对话 → started → completed(boundary)，divider 固定在触发时刻，摘要覆盖最后一条 user 之前的消息。
- **Base**：旧版本落盘的 boundary（无 `display_after_message_id`）→ serde default 反序列化为 None，前端回退切分点渲染。
- **Bad（禁止）**：在压缩路径新增 `Err` 提前 return 却绕过 `compact_conversation` 的单出口 wrapper；按"条数"推算 runtime↔UI 对应关系（工具展开/多答剔除/摘要锚点都会错位）。

### 6. Tests Required

- Rust（`compaction.rs` tests + 独立 harness，本机 cargo test 有 0xC0000139 环境问题）：映射四场景（工具展开 / 跨边界回退 / 仅摘要锚点→None / 无旧段→None）、`manual_fallback_split` 两场景、display 锚点取"最后一条带标注消息"。
- Vitest `compactionBoundary.test.ts`：display 锚点优先、无锚点回退切分点、锚点被删回退切分点、legacy summary 回退、动画槽位 = 最后一条消息。

### 7. Wrong vs Correct

#### Wrong

```rust
// 按条数推算 UI boundary —— runtime 与 UI 不是 1:1，必然错位丢上下文
let ui_consumed = old_segment.iter().filter(|m| is_user_or_assistant(m)).count();
ui_message_order.get(ui_consumed - 1)
```

#### Correct

```rust
// 构造时标注来源 id，切分后按标注映射（跨边界回退）
old_segment.iter().rev()
    .filter_map(|m| m.get(UI_MESSAGE_ID_KEY).and_then(Value::as_str))
    .find(|id| !ids_in_recent.contains(id))
```

---

## Scenario: 链式摘要合并、估算口径、多答排除（07-06-compaction-correctness-fixes）

> 2026-07-06 补。相关代码：`compaction.rs`、`commands/context.rs`（`summary_message` / `count_tokens_in_value` / `build_chat_api_messages`）、`commands.rs`（L2 写回块）、`prepare.rs`（`estimate_value_tokens`）。

### 1. 注入摘要识别（防跨轮丢上下文，硬约束）

`build_chat_api_messages` 把落盘 `context_state.summary` 注入为一条 **system** 消息，前缀 `PERSISTED_SUMMARY_PREFIX`（`"Previous conversation summary:"`）。该前缀是 `compaction::PERSISTED_SUMMARY_PREFIX` 常量，**生成方 `commands/context.rs::summary_message` 与识别方 `compaction::extract_previous_summary` 共用**——禁止任一侧硬编码字符串（格式漂移会让 L2 认不出旧摘要，进而整体覆盖、静默丢早期上下文）。

`extract_previous_summary(system_prefix, old_segment)` 识别两种上一份摘要，**锚点优先**：
- 锚点摘要：old_segment 内 user + `SUMMARY_MARKER_PREFIX`（同 run 内更晚的 L2 产物）；
- 注入摘要：system 前缀内 system + `PERSISTED_SUMMARY_PREFIX`（跨轮的落盘 summary）。

识别到的旧摘要作为 `previous_summary` 走链式合并；注入摘要额外从压缩后视图的 system 前缀剔除（`is_injected_summary`），锚点摘要及其配对 ack（`SUMMARY_ACK_TEXT`，`is_summary_ack`）从摘要输入 head 剔除。

### 2. source_message_ids 累积（两路径同口径）

`accumulate_source_ids(conversation, until_id)` = 旧未过期 summary 的 ids ∪ 旧 boundary 之后至 `until_id`（含）的**全部**原始消息 id（含多答排除臂）。落盘路径 `compact_conversation_inner` 与 **L2 run 结束写回**（commands.rs）都调用它——L2 产出的 summary `source_message_ids` 初始为空，写回时**必须在替换 summary 之前**用该 helper 填充，否则 `compressed_message_count` 归零且下一轮无法定位覆盖范围。

### 3. token 估算口径（防 base64 打爆 / preview 低估）

- `prepare::estimate_value_tokens` 是唯一多模态估算口径：**图片部件记 0**（按 provider tile 计费，非 base64 体积）、文本按文本、对象递归。`commands/context.rs::count_tokens_in_value` 委托它。
- `compaction::estimate_message_tokens` 对非字符串 content 走 `estimate_value_tokens`（不再 `to_string()` 整体估算），并计入 `reasoning_content`。
- `compaction::estimate_chat_message_tokens` 优先按展开形态（`model_messages` → `api_messages`）估算，分支顺序与 `build_chat_api_messages` 一致；无展开数据才回退 `result_preview` 口径。
- `serialize_message` 多模态 user：文本全文 + 图片占位 `[image attachment omitted]`，绝不含 `;base64,`。

### 4. 多答排除臂过滤（落盘路径）

落盘 old_segment 的 token 切分与序列化都跳过 `commands::context::group_answer_excluded_from_context`（pub(crate)，与 `build_chat_api_messages` 同谓词）为 true 的消息——经 `context_included_indices` + `token_split_over_indices` / `manual_fallback_split_over_indices`（升序原始下标，boundary 映射回原始下标）。被排除臂**内容不进摘要**，但其 id **仍计入** source_message_ids（被 boundary 覆盖、不再 replay）。L2 路径不受影响（runtime 天然已排除）。

### 5. 取消 ≠ 失败

`compact_with_summary_model` 返回 `CompactAttempt { Summary | Cancelled | Failed }`；`summarize_history` 透传 `CompactOutcome`。`maybe_compact_send_view` 的 `Cancelled` 分支**不递增** `compaction_unresolved_rounds`（用户主动取消非压缩无能为力），但仍发 `failed` 终止事件（UI 归位，不新增 phase 值）。`force_compact` / `compact_conversation_inner`（cancel=None）下 `Cancelled` 不可达。

### 6. Bad（禁止）

- `summary_message` 或 `extract_previous_summary` 任一侧硬编码 `"Previous conversation summary:"` 而非引用常量。
- L2 写回直接搬运 summary（`source_message_ids` 留空）而不累积。
- 压缩估算对多模态 content 走 `to_string()`（base64 打爆）或对工具消息只算 `result_preview`（低估真实 replay）。
- 落盘 old_segment 用连续 `messages[start..=split]` 而不过滤多答排除臂。

---

## Scenario: 真实用量锚点回喂上下文计量（07-06-context-token-ground-truth）

> 2026-07-06 补。相关代码：`chat/agent/context_estimate.rs`（新）、`compaction.rs::maybe_compact_send_view`、
> `loop_.rs`（`RunState.last_step_usage`/`merge_usage`/`attach_usage`）、`commands/context.rs::resolve_usage_anchor` +
> `compute_context_state` + `build_assistant_message`、`types.rs::ChatMessage.anchor_usage`、
> `external_agents/context.rs::TOKEN_COUNT_PROVIDER_REPORTED`、`src/chat/ContextIndicator.tsx`。

### 1. 口径（唯一，对齐 pi 源码）

上下文占用 = **provider 实报锚点（整个 prompt + 该次响应）+ 锚点响应之后新增消息的字符估算**，
逐字对齐 pi 的 `contextTokens = calculateContextTokens(lastUsage) + Σ estimate(其后消息)`。
统一函数 `context_estimate::effective_context_tokens(anchor_total, trailing, estimate_full)`：

- **保守下限（硬约束）**：返回 `max(anchor_total + trailing, estimate_full)`，且**仅当**锚点值 ≥ 纯估算时才
  标记 `anchored=true`。锚点绝不能让 footer / 压缩触发比改动前的纯字符估算**更乐观**（锚点偏小 → 退回纯估算）。
- 无锚点（新对话 / 旧数据无 usage / provider 切换 / 压缩后）→ `(estimate_full, false)`，行为 == 改动前。

### 2. 锚点求和 = 「prompt + 响应」总数，按 provider 家族消歧（防双算，硬约束）

`context_estimate::anchor_total_tokens(usage, api_format)`（对齐 pi `totalTokens || input+output+cacheRead+cacheWrite`）：
- `anthropic_messages`：`input_tokens` 是**非缓存**部分 → 全量 = `input + output + cache_read + cache_creation`（四者不相交）。
  **不**用 Kivio 的 `total_tokens`（对 Anthropic 只填 `input+output`、漏 cache）。
- 其它（`openai_*` / responses）：优先 `total_tokens`（= prompt+completion，prompt 已含 cached，无双算）；
  缺失则 `input(=prompt,含cached) + output`。**叠加 cached 就是双算**（opencode #4416 踩过）。
  口径须与 `usage.rs::model_usage_from_{openai,anthropic}_value` 的映射一致。
- **含 output**：锚点响应已成为下一次请求的历史，其 token 必须计入；trailing 只覆盖响应**之后**的消息（§3）。

### 3. 锚点 = 单次调用 usage，不是累计；trailing 严格「响应之后」（硬约束）

`ChatMessage.anchor_usage` / `RunState.last_step_usage` / `AgentRunResult.last_step_usage` 存**最后一次**模型调用的 usage，
与累计字段 `ChatMessage.usage`（多步 prompt 之和、会虚高数倍）**必须分开**——累计值当锚点会严重高估。

trailing 必须是锚点**响应之后**新增的消息（响应本身用真实 output 计入锚点 §2，不重复估，否则双算）：
- **loop**：`merge_usage` 只记 `last_step_usage`；`runtime_len_at_last_call` 在 `rounds.rs` push 完该次规划**响应之后**才设，
  使 `runtime_messages[start..]` = 响应之后的工具结果。
- **footer**：`resolve_usage_anchor` 的 trailing 用 `messages[anchor_idx + 1 ..]`（排除锚点消息本身）。

### 4. 锚点失效（R4，硬约束）

`resolve_usage_anchor(conversation, provider)` 解析 footer/首轮 config 锚点，以下情形**必须**失效（回落纯估算）：
- **provider 切换**：锚点消息 `provider_id`（多模型自带 / 单模型回退 `conversation.provider_id`）≠ 当前 provider（计数口径不可比）。
- **压缩边界在锚点之后**：`max(compaction_boundaries.created_at) > 锚点消息.timestamp`。手动 `/compact` 后不产生新 assistant，
  旧 assistant 的 `anchor_usage` 仍是**压缩前**的巨值——不失效会让 footer 虚高 + 下轮 `initial_anchor` 触发对已压缩对话的
  二次压缩。run 内自动压缩不受影响（最终 assistant 的 `timestamp` 在 run 结束时置，≥ 边界时间）。
`maybe_compact_send_view` 里压缩成功（micro/LLM 两分支）后必须 `last_step_usage=None` + `initial_anchor_valid=false`。

### 5. 跨层口径标记

内置路径锚定时 `context_state.token_count_source = TOKEN_COUNT_PROVIDER_REPORTED`（`"provider_reported"`，与外部 CLI 的
`cli_reported` 并列，定义在 `external_agents::context`）+ 填 `session_input_tokens`。前端 `ContextIndicator` 据此显示**精确值**
（不带 `~`）+「模型实报」标签；`types.ts` 的 `token_count_source` 联合类型须含该值。禁止两侧硬编码该字符串。

### 6. 兜底估算（`estimate_full`）必须含工具 schema（对齐 pi / footer）

无锚点时（首轮 / 刚压缩）用的纯字符估算 `estimate_full`，必须涵盖 **消息 + 工具 schema**（pi 兜底 = system + 每工具 + 消息；
footer 也含 `estimate_tool_segments`）。工具定义随每次请求发送、provider 会计入——漏算会让首轮低估数千 token、压缩过晚。
- **loop**：`maybe_compact_send_view` 的 `estimate_full = estimate_messages_tokens(runtime) + Σ estimate_value_tokens(tool.to_openai_tool())`。
- **footer**：`compute_context_state` 的 segments 已含 `estimate_tool_segments`（同口径 `count_tokens_in_value(tool.to_openai_tool())`）。

### 7. Bad（禁止）

- 用累计 `ChatMessage.usage` 当锚点（多步虚高）。
- 锚点 OpenAI 家族把 `input + cached` 相加、或不加 `output`（前者双算、后者漏响应）。
- trailing 把锚点响应本身也估进去（响应已用真实 output 计入锚点 → 双算）。
- `effective_context_tokens` 去掉 `max(estimate_full)` 下限（锚点偏小时比现状更乐观 → 压缩过晚超窗）。
- `resolve_usage_anchor` 只查 provider 不查压缩边界（stale 锚点泄漏 → footer 虚高 + 已压缩对话被二次压缩）。
- loop 的 `estimate_full` 漏算工具 schema（首轮低估、压缩过晚）。

