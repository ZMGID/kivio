# design.md — Microcompact 增量降级（R-1）

## 核心思路
在 L2 `maybe_compact_send_view` 触发重型 LLM 摘要**之前**插一层轻量降级：把 old_segment 里的工具结果内容换成短标记，重估预算；若已回到预算内则**跳过摘要**。对齐 Claude Code "能拖就拖、便宜优先、分级升级"。

## 边界与不变量
- 只动 **old_segment**（`select_recent_by_tokens` 切出的、近期 20k 尾窗**之前**的段）。近期窗口原样保留 → 天然满足"最近的工具结果不降级"。
- 不拆 `tool_call↔tool` 配对（复用 `select_recent_by_tokens` 已有的配对保护）。
- 只降级 `role=="tool"` 消息；user/assistant/reasoning/工具入参不动。
- `generated_api_messages`（持久化镜像）不碰，与摘要路径一致。
- 幂等：已是标记的内容再降级 = 无变化（估算稳定，正确落到摘要）。

## 纯函数（可单测，不依赖 model/host）
```
fn microcompact_send_view(messages: &[Value], keep_tokens: usize, budget: usize) -> Option<Vec<Value>>
```
1. `(system, old, recent) = select_recent_by_tokens(messages, keep_tokens)`；`old` 空 → None。
2. 遍历 `old`：`role=="tool"` 且 content 非标记 → content 替换为 `MICROCOMPACT_TOOL_MARKER`。
3. 组回 `system + degraded_old + recent`；`estimate_messages_tokens <= budget` → `Some(view)`，否则 `None`。
   - 语义：**仅当降级足以回到预算内才 Some（=跳过摘要）**；不够则 None，落到既有摘要路径。

## 接线（`maybe_compact_send_view`）
在 `estimated <= budget` 早退之后、`summarize_history` 之前：
```
if let Some(degraded) = microcompact_send_view(&state.runtime_messages, keep_tokens, budget) {
    state.runtime_messages = degraded.clone();
    state.compacted = true;                       // 让 finalize 转发降级后历史（永久回收）
    state.compaction_unresolved_rounds = 0;       // 已回到预算内，重置 thrash
    env.host.emit_compaction_status(&id, "microcompacted", Some("agent_loop"), None);
    return degraded;
}
// 否则继续走 summarize_history（此时 runtime 未变，摘要看原文；序列化本就截工具输出到 2000 字）
```

## 分级升级效果
- 第 1 次超预算：降级旧工具结果 → 多半够 → 跳过昂贵摘要。
- 后续轮：新工具结果逐渐进入 old_segment，继续被降级。
- 直到降级不够（旧段以 user/assistant 文本为主）→ 才回落到 LLM 摘要。

## 与现有机制的交互
- **thrash 熔断**：microcompact 成功→重置计数（同摘要成功分支）。
- **质量兜底 / 链式锚点**：不受影响——microcompact 不产 summary、不动 boundary/summary 记录。
- **摘要路径**：microcompact 返回 None 时零行为变化。

## 风险 / 回滚
- 仅动 `compaction.rs::maybe_compact_send_view` + 新增一个纯函数 + 一个常量。回归面局限于 L2 发送视图。
- 回滚：`git checkout src-tauri/src/chat/agent/compaction.rs`。
- 最小化选择：先整体替换标记（不留 head）。若实测发现模型丢上下文，再改为保留 ~200 字 head（`ponytail:` 标注升级路径）。

## 前端
- 新增 `"microcompacted"` phase 事件；前端时间线可选渲染（不阻塞后端；不加也不报错）。本任务后端为主，前端仅确保不崩。
