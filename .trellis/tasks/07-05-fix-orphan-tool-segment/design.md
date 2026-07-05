# Design — 修复孤立工具分段(工具记录缺失)

## 根因回顾

工具**分段**(planning 阶段按解析出的 `call.id` 创建并流式推送)与工具**记录**(execution 阶段按 `call.id` 创建)分两阶段产生。当一轮在「分段已发出」与「记录已创建」之间被中断(网关掐流/400/取消/超时),落库的 assistant 消息就有分段无记录。前端 `MessageBubble.tsx` 找不到匹配记录 → 渲染「工具记录缺失」。后端对账 `normalize_assistant_segments` 只补「有记录没分段」,不补反向。

## 方案:落库层合成中断态占位记录(后端为主)

在 assistant 消息**组装处**做一次「孤立工具分段 → 合成 Cancelled 记录」的反向对账。

### 插入点
`chat/commands.rs` 的 assistant 消息组装函数(围绕 `2600` 行,调用 `normalize_assistant_segments` 之前),它同时持有 `tool_calls: Vec<ToolCallRecord>`、`api_messages: Vec<Value>`、`segments`。

**同时覆盖中断草稿路径**:`persist_partial_assistant_snapshot`(`commands.rs:2790`)——中断且永不完成的 run,草稿就是最终存档,最容易出孤立分段。把对账抽成共享 helper,两处都调。

### 新增 helper(commands.rs)
```
fn reconcile_orphan_tool_segments(
    tool_calls: &mut Vec<ToolCallRecord>,
    segments: &[ChatMessageSegment],
    api_messages: &[Value],
)
```
逻辑:
1. `record_ids` = `tool_calls` 里所有 `id` 的集合。
2. 遍历 `segments`,取 `kind == Tool && tool_call_id` 且**不在** `record_ids` 的 id(孤立)。去重、保序。
3. 每个孤立 id:先从 `api_messages` 里扫 assistant 消息的 `tool_calls[]`(OpenAI 线格式 `{id, function:{name, arguments}}`)按 id 回捞 name/arguments;捞到则用真名+真参,捞不到则 `name = ""`(前端兜底显示「工具调用」)、`arguments = ""`。
4. 合成 `ToolCallRecord`:`status = ToolCallStatus::Cancelled`,`error = Some("工具调用未完成(会话中断)")`,`round`/时间戳尽力填(`started_at/completed_at = now`,`duration_ms = 0`),其余默认。追加进 `tool_calls`。

追加后:`normalize_assistant_segments` 的既有「记录→分段」pass 会把新记录与原分段自然对上(id 相同不会重复补段);前端 `toolRecordId === segmentToolCallId` 命中 → 渲染正常工具卡(Cancelled 态)。

### 契约/兼容
- 只**新增**记录,不动既有分段顺序、不删数据。对无孤立分段的消息零影响(helper 空转)。
- 合成记录 `status=Cancelled` 复用既有 `ToolCallStatus::Cancelled`(前端 `tool_card.rs status_symbol` 已有 `⊗`,前端 ToolCallBlock 已识别)。
- 历史已存坏消息:下次该会话被重新组装/保存时才修;不做一次性迁移扫描(避免全量改写存档)。可接受——新会话不再出现,老会话打开仍显示旧占位直到被再次落库。若要更彻底,可在 `load_conversation` 读取时对每条 assistant 跑一次 reconcile(只读期修正,不写盘)——**列为可选增强**。

## 前端(小改,确保 Cancelled 空名卡片可读)
- `src/chat/ToolCallBlock.tsx`:确认 `status=cancelled` 且 `name` 为空时,标题回退显示「工具调用 · 已中断」而非空白;有 name 时正常显示名 + 中断徽标。
- 若既有渲染已足够,则前端零改动(以实测为准)。

## 测试
- Rust:`chat::commands` 新增测——构造 `segments`(含一个 tool 段 id=X)+ `tool_calls`(不含 X)+ `api_messages`(含 X 的 name/args),断言组装后 `tool_calls` 出现 id=X、`status=Cancelled`、name 回捞正确;另测 `api_messages` 无 X 时 name 为空兜底;再测无孤立分段时 `tool_calls` 不变。
- 前端:`MessageBubble.test.tsx` 已有 orphan-tools 用例(136 行附近)——补一条:tool 段有匹配的 cancelled 记录时渲染工具卡而非 MissingToolSegment。
- 回归:`normalize_assistant_segments` 既有正向补段测保持绿。

## 取舍
- 选「合成记录」而非「剔除分段」:保留可观测性(用户能看到模型发起过什么工具、在哪中断),契合调试诉求;比纯前端改文案更彻底(修的是存档数据,所有客户端一致)。
- 不盲修候选 2(流式 id 错位):无复现;防御对账已消除其用户可见症状。
