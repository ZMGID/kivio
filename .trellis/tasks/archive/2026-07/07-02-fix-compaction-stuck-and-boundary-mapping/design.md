# Design：修复上下文压缩失败卡死、boundary 映射错位与压缩动画失效

## 1. 事件契约扩展（R1）

### 现状
`chat-compaction` payload：`{ conversationId, phase, trigger, boundary }`，phase ∈ `started | completed | microcompacted`。发射点：
- `compaction.rs::emit_compaction_event`（`compact_conversation` 内部，started/completed）
- `compaction.rs` 内 `env.host.emit_compaction_status`（agent_loop：started/microcompacted/completed）
- `commands.rs::emit_chat_compaction_state`（手动入口失败补发 completed）

### 变更
新增 phase `"failed"`（payload 同形，`boundary: null`）：

| 发射点 | 时机 |
|---|---|
| `compact_conversation` | 所有 `Err` 提前返回前（无旧段 / 无 provider / 无 key / 无模型 / 摘要质量不达标）。用内部辅助包装：入口发 started，`Err` 统一发 failed 后返回。 |
| `maybe_compact_send_view` | `summarize_history` 返回 `None` 时；成功但 `source_until_message_id_for_split` 返回 `None` 时发 `completed`（boundary: null）——压缩视图已生效，只是无落盘 boundary，不能报 failed。 |
| `chat_compress_context` 手动入口 | 把现有失败补发的 `completed` 改为 `failed`（语义修正）。 |
| 自动落盘入口 ×2 | `compress_conversation_context` 返回 `Err` 已在 `compact_conversation` 内发过 failed，无需再补（验证一遍调用链即可）。 |

**约束**：`compact_conversation` 内保证 started 与终止事件一一配对——所有 return 路径经过单一出口（或 helper），杜绝再次遗漏。

### 前端
`Chat.tsx` 的 `onChatCompaction` 监听：`phase !== 'started'`（含 `failed`、boundary 为 null 的 `completed`）即清 `agentLoopCompacting`（现逻辑已如此，新 phase 自然吸收；确认 `payload.boundary` 为空时不进 boundary 写回分支——现有 `if (boundary?.id)` 已安全）。
另在 `finishStreamingRun` 里兜底 `setAgentLoopCompacting(false)`（AC6，防御未来遗漏）。

`ChatCompactionPayload` 类型（`src/api/tauri.ts`）的 `phase` 联合类型加 `'failed'`。

## 2. runtime→UI boundary 映射（R2）

### 根因
`source_until_message_id_for_split` 用"runtime 旧段中 user|assistant 条数"当 `ui_message_order` 下标。runtime 与 UI 不是 1:1：
- UI assistant 带工具调用 → runtime 展开为多条 assistant(tool_calls)+tool+最终 assistant（多算）；
- 多答组未选中答案被 `group_answer_excluded_from_context` 剔除（少算）；
- 上次压缩的摘要锚点 user/assistant 对（`SUMMARY_MARKER_PREFIX`）被计入（多算 2）。

### 方案：构造时标注来源，弃用条数推算
`build_chat_api_messages`（commands.rs）为每条 push 进结果的 runtime 消息加注 `"_ui_message_id": "<UI message id>"`（一条 UI 消息展开出的多条 runtime 消息共享同一 id；系统 prompt、summary 注入消息不加注）。

**安全性依据（已验证）**：runtime 消息发给 provider 前经 `generate_request_from_openai_messages` → `model_message_from_openai_message`，只抽取已知字段（role/content/tool_calls/tool_call_id/reasoning_content），未知字段天然被剥离，不会泄漏到任何 provider wire 格式（openai.rs / anthropic.rs / responses.rs 全部走 ModelMessage）。

`source_until_message_id_for_split` 重写：
```text
输入：runtime_messages, keep_tokens
1. select_recent_by_tokens 切出 old_segment（逻辑不变）
2. 取 old_segment 里最后一条带 _ui_message_id 的消息，得候选 id
3. 若 recent（含）之后还存在同 id 的 runtime 消息 → 该 UI 消息横跨边界，
   回退到 old_segment 内上一个"完整落入旧段"的不同 id
4. 找不到（旧段只有摘要锚点/系统注入）→ None
```
不再需要 `ui_message_order` 参数（`AgentRunConfig.ui_message_order` 字段与三处构造一并删除：commands.rs、sub_agent.rs ×2、loop_tests.rs）。

**降级行为**：映射返回 `None` 时不写 summary/boundary（与现状一致），但发 `completed`（boundary: null）事件；运行时压缩视图照常生效。

### 附带修正
`estimate_message_tokens` 的 fallback 分支（非字符串 content 时 `message.to_string()`）会把 `_ui_message_id` 计入几个 token——可接受（估算本就是启发式），不做特殊处理；microcompact / replace_with_summary 均不受影响（前者只改 content，后者原样保留 recent）。

## 3. 前端动画槽位 fallback（R3）

沿用工作区已有方向：`resolvePendingCompactionAfterIndex` 在 `estimatePendingCompactionAfterIndex` 返回 null 时，回退到摘要边界之后最后一条 assistant 消息的 index；再无 assistant → null（不渲染）。
- 修 `compactionBoundary.test.ts:123`：补第二参 `null`。
- 触发条件不变：仅 `compactionInProgress`（`contextCompressing || agentLoopCompacting`）为 true 时才渲染，R1 修复后该 flag 不会卡死，fallback 不会造成永久转圈。

## 4. 手动压缩小对话保底（R4）

`compact_conversation` 中 `token_split_chat_messages(...)` 返回 `None` 且 `trigger == "manual"` 时：
- 若 `summary_start..len` 区间 UI 消息数 > 4：保底切分——保留最后一对（最后一条 user 及其后所有消息，即 `messages.iter().rposition(|m| m.role == "user")`），其余进 old_segment；
- 否则维持原报错"没有足够的旧消息可以压缩"。

auto / agent_loop 路径不变（`has_compressible_old_segment` / `maybe_compact_send_view` 触发条件照旧）。

## 5. 衰减警告保留（R5）

`chat_compress_context` 成功路径删除 `conversation.context_state.warning = None;`。`compact_conversation` 已在末尾用 `decay_warning_for` 统一写入/清空，语义完整。
注意 `compute_context_state` 的 `warning: memory_warning.or_else(|| conversation.context_state.warning.clone())` 会保留写入值——无需其他改动。

## 兼容性
- 事件新增 phase 对旧监听者向后兼容（未知 phase 走"非 started"分支清 flag，行为正确）。
- `_ui_message_id` 只存在于运行期 runtime 视图；`generated_api_messages`（持久化镜像）来自 provider 输出，不带标注；落盘格式不变。
- `AgentRunConfig.ui_message_order` 是 crate 内部结构，删除无外部影响。

## 测试策略
- Rust 单测（compaction.rs tests mod）：映射四场景（工具展开 / 多答剔除 / 摘要锚点 / 跨边界回退）、手动保底切分、事件配对（用 loop_tests.rs 的 fake host 断言 started/failed 配对）。本机 cargo test 若 0xC0000139 则用独立 harness（沿用 ffff7c3 先例）并在提交注明。
- Vitest：fallback 槽位（已有），`failed` phase 清 flag（Chat.tsx 逻辑简单，靠类型 + 手测）。
- 手测清单见 implement.md。
