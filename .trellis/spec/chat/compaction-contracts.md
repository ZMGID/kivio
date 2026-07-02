# Chat 压缩（Compaction）契约

> 任务来源：`07-02-fix-compaction-stuck-and-boundary-mapping`（2026-07-02）。
> 相关代码：`src-tauri/src/chat/agent/compaction.rs`、`src-tauri/src/chat/commands.rs`（`build_chat_api_messages` / `tag_ui_message_id`）、`src/chat/compactionBoundary.ts`、`src/chat/Chat.tsx`。

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
