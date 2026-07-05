# 工具分段 ↔ 记录 双向对账契约

> 来源:07-05-fix-orphan-tool-segment(GitHub issue #14「工具记录缺失」)。

## 背景

Chat 时间线里,工具**分段**(`ChatMessageSegment{kind: Tool, tool_call_id}`)与工具**记录**(`ToolCallRecord{id}`)分两阶段生成:
- 分段:planning 阶段解析出工具调用即创建(`agent/finalize.rs append_tool_calls`),`tool_call_id = call.id`,并立即流式推送。
- 记录:execution 阶段工具执行时创建(`agent/rounds.rs`/`execute.rs`),`id = call.id`。

前端 `MessageBubble.tsx` 按 `segment.tool_call_id == record.id` 配对渲染工具卡;配不上就显示占位。

## 契约:两个方向都要对账

`chat/commands.rs` 落库/读取时必须保证分段与记录**双向**齐全:

1. **记录→分段**(`normalize_assistant_segments`):有记录没分段 → 合成分段(既有,`tool_segment_for_record`)。
2. **分段→记录**(`reconcile_orphan_tool_segments`,本次新增):有分段没记录(孤立分段)→ 合成 `ToolCallRecord{status: Cancelled, error:"工具调用未完成（会话中断）"}`。name/arguments 从 `api_messages`(OpenAI 线格式 assistant `tool_calls[]`)按 id 回捞,捞不到留空(前端 `getToolName` 兜底 "Tool")。

**孤立分段成因**:一轮在「分段已流式发出」与「记录已创建」之间被中断(网关掐流/400/取消/超时),落库消息就有分段无记录。转售网关(id 形如 `fc_call_function_<hash>_<index>`)尤其常见。

## 必接的三处

- `build_assistant_message`(正常完成落库):`reconcile_orphan_tool_segments` 在 `normalize_assistant_segments` **之前**跑。
- `persist_partial_assistant_snapshot`(中断草稿落库):中断草稿是「永不完成」run 的最终存档,最易出孤立分段,必接。
- `chat_get_conversation` → `reconcile_conversation_orphan_tool_segments`(读取时只读修正,不写盘):让**存量**坏会话打开即正常。必须在 `strip_transcripts_for_frontend` **之前**跑——strip 会清空已完成消息的 `api_messages`,而回捞 name 依赖它;中断草稿的 `api_messages` 不被 strip,故最常见的中断场景仍能拿到真名。

## 红线

- `reconcile_orphan_tool_segments` 只**新增**记录,不删分段、不改顺序;无孤立分段时空转,对正常消息零副作用。
- `ToolCallRecord` 未派生 `Default`,合成时必须显式填全字段(含 `structured_content`)。
- 未做的:候选 2(流式 tool_call id 在分段与记录间捕获错位,形如 `fc_..._<index>`)的根因修复——无复现,defer,见 [[streaming-toolcall-id-no-empty-overwrite]] 同类。防御对账已消除其用户可见症状。
