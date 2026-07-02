# Microcompact 增量降级（compaction-microcompact）

> 从 `07-02-compaction-optimize` 拆出的 R-1。本任务**暂缓**，待父任务 R-3/R-4 落地并验证后再规划。

## Goal
在触发重型 LLM 摘要**之前**，先把旧段的工具结果降级成预览/占位，重估预算；若已降回预算内则跳过摘要调用。对齐 Claude Code 的 Microcompact（"defer as long as possible, keep it cheap, escalate in stages"）。

## Background
- 现状：Kivio 压缩是全有或全无的重型 LLM 摘要；runtime_messages 里保留工具结果全文，只有摘要序列化时才截 2000 字。
- Claude Code Microcompact：发送前按 `tool_use_id` 清旧工具结果，不废前缀缓存，分级升级。
- 插入点：`compaction.rs::maybe_compact_send_view`（L2，runtime_messages 上工作），在 `summarize_history` 之前；`select_recent_by_tokens` 已分出 old_segment/recent 并护 tool 配对。

## Requirements（待细化）
- 触发压缩时，先对 old_segment 里的 `role=="tool"` 结果做降级（截断/占位），重算 `estimate_messages_tokens`。
- 降级后若 `≤ budget`：跳过 LLM 摘要，直接用降级后的视图。
- 只降级 old_segment，近期窗口原样保留；不拆 tool_call↔tool 配对。

## Risks
- 触碰刚重构完的 L2 路径，回归面大。
- 与现有 thrash 熔断 / 质量兜底 / 链式锚点的交互需想清楚。
- provider 不稳定性敏感（降级视图仍可能超窗）。

## Acceptance criteria（待细化）
- mock host：构造"旧段工具结果占多数"的 runtime_messages，降级后回到预算内时**断言无摘要请求发出**。

## 已决（见 design.md）
- 降级策略：old 段 `role=="tool"` 结果内容整体替换为短标记（先最小化，丢上下文再加 head）。
- 保留最近 N 条：不需要——近期 20k 尾窗已保护，只降级 old_segment。
- 降级后仍超窗：纯函数"仅当降级足以回预算内才 Some"，否则 None 落到既有摘要（摘要序列化本就截工具输出 2000 字）。
