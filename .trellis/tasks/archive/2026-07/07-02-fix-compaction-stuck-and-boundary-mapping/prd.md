# 修复上下文压缩失败卡死、boundary 映射错位与压缩动画失效

## Goal

`97dfb8a`（统一压缩路径 + token 尾窗）与 `ffff7c3`（microcompact）合入后，上下文压缩与压缩动画出现多处回归：失败路径卡死前端状态、boundary 映射错位可静默丢上下文、动画无渲染槽位、小对话不可手动压缩。本任务修复这组缺陷。

## Requirements

### R1（P0）：压缩失败/无 boundary 时必须发终止事件
- `chat-compaction` 事件当前只有 `started` / `completed` / `microcompacted`；`started` 无条件发，但失败路径不发任何终止事件。
- 覆盖点：
  - `maybe_compact_send_view`（agent_loop）：摘要失败/质量闸拒绝返回 `None`；成功但 `source_until_message_id_for_split` 返回 `None`。
  - `compact_conversation`：发 `started` 之后的所有 `Err` 提前返回（无旧段、无 API key、无模型、摘要过短/质量闸拒绝）。
  - 两个自动落盘入口（发送前 auto / `try_auto_compress_context_after_update`）失败时（手动入口已有补发，需统一口径）。
- 新增 `failed` phase（payload 与既有事件同形，`boundary: null`），前端据此清 `agentLoopCompacting`。

### R2（P0）：修正 agent_loop 压缩的 runtime→UI boundary 映射
- `source_until_message_id_for_split` 现按 runtime 旧段中 `user|assistant` 条数当下标查 `ui_message_order`，与 UI 消息不是 1:1（工具调用多轮展开多算；多答组剔除少算；摘要锚点对多算 2）。
- 要求：映射必须精确指向"其 runtime 展开完全落入旧段"的最后一条 UI 消息；无法可靠映射时返回 None（不写 boundary/summary，但仍发终止事件，压缩视图照常生效——运行时压缩不依赖落盘）。
- 推荐做法：构造 runtime 消息时携带来源 UI message id 标注，而非事后按条数推算（具体见 design.md）。

### R3（P1）：前端动画槽位 fallback + 修 typecheck
- 完成工作区已有的半成品改动：`resolvePendingCompactionAfterIndex` 在 token 估算无旧段时回退到摘要边界后最后一条 assistant 消息。
- 修复 `compactionBoundary.test.ts:123` 缺参导致的 typecheck 失败。
- 前端在 run 结束（`finishStreamingRun`）时兜底清 `agentLoopCompacting`（防御性，见 AC6）。

### R4（P1）：手动压缩小对话可用性
- token 尾窗无旧段时，手动压缩不再直接报"没有足够的旧消息可以压缩"：当对话 UI 消息数足够（> 4 条）时退化为"保底保留最后一对 user/assistant，其余进摘要"；确实太短时保持原报错文案。
- 仅手动路径放宽；auto / agent_loop 触发条件不变（它们本就要超 90% 窗口才触发）。

### R5（P2）：手动压缩不再抹掉衰减警告
- `chat_compress_context` 成功路径去掉无条件 `warning = None`，保留 `compact_conversation` 写入的 R-4 衰减警告（其内部已实现"未达阈值清空旧警告"）。

## Acceptance Criteria

- [x] AC1：手动 / auto / agent_loop 三条路径，压缩成功、失败、无旧段任一结局下，前端 `agentLoopCompacting` / `contextCompressing` 都会归位；失败路径发 `failed` 事件（Rust 侧发事件调用点齐全 + 前端监听处理）。
- [x] AC2：`source_until_message_id` 映射在以下场景单测全过：带多轮工具调用的 assistant、多答组（未选中答案剔除）、二次压缩（旧段含摘要锚点对）、映射失败返回 None。
- [x] AC3：小对话 / 工具输出重对话中压缩动画有渲染槽位；`npm run typecheck`、`npm run lint`、`npm test` 全绿。
- [x] AC4：> 4 条消息的小对话手动压缩成功（保底切分）；≤ 4 条时报错文案与现状一致。
- [x] AC5：手动压缩至衰减阈值（3 次）后 `context_state.warning` 含衰减提示。
- [x] AC6：`finishStreamingRun` 清 `agentLoopCompacting`。
- [x] AC7：Rust 侧新增/改动逻辑有单元测试（本机 cargo test 有 0xC0000139 环境问题时，用独立 harness 验证并在提交信息注明）；Vitest 既有套件不回归。

## Notes

- 范围外：prefix-cache 保留、外部 agent（`agent_runtime.is_external()`）压缩路径、摘要 prompt 与质量闸逻辑本身。
- 工作区现有未提交改动（`compactionBoundary.ts` fallback + 测试）并入 R3 完成，不单独提交。
