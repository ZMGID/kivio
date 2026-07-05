# PRD — 修复「工具记录缺失」孤立工具分段

## Goal / User Value

会话里出现「工具记录缺失 · <tool_call_id>」(GitHub issue #14),看起来像 bug、令人困惑。目标:消除这种孤立工具分段的困惑呈现,让被中断/异常的工具调用以诚实、可理解的方式显示,并在落库层做双向对账兜底。

## Background / Confirmed Facts

- 提示来源:前端 `src/chat/MessageBubble.tsx:253` `MissingToolSegment`(第 257 行文案「工具记录缺失{ · id}」)。触发点 `TimelineToolSegment`(269-273):工具**分段**按 `tool_call_id` 在 `toolCalls` 里找**记录**,找不到就渲染该占位。
- id 提取无归一化:`segments.ts:4 segmentToolCallId`、`segments.ts:13 toolRecordId`(纯读字段)。所以是真·缺记录,不是前端匹配 bug。
- 分段与记录在不同阶段生成、但都用同一 `call.id`:
  - 分段:planning 阶段解析出工具调用即创建(`agent/finalize.rs:227 append_tool_calls`,`tool_call_id = call.id`),并立即流式推给前端。
  - 记录:execution 阶段执行工具时创建(`agent/rounds.rs:547` / `agent/execute.rs:95/123/154`,`id = call.id`)。
- 后端对账只有单向:`chat/commands.rs:2837 normalize_assistant_segments` 给「有记录没分段」补分段(2891-2916,`tool_segment_for_record` at 2978);**「有分段没记录」不处理**,孤立分段永久残留。
- issue 的 id `fc_call_function_4agzr50pp9go_1` 是 OpenAI Responses 风格 / 转售网关**原样返回**的 id(代码不会给 id 加 `_1` 后缀;openai 流式仅在 id 为空时合成 `call_<index>`,见 `model/openai.rs:864`)。
- 该库已知转售网关会中途 400 / 掐流(memory:opus48 thinking.enabled 400、promptCacheKey 400;侧栏见 `Chat stream Error: 400`)。

## 根因(两个候选)

1. **回合中断(最可能)**:工具分段已流式发出并落库,但那一轮在**创建执行记录前被中断**(网关掐流 / 400 / 取消 / 超时)。存下的 assistant 消息有分段无记录 → 永久「缺失」。
2. **流式 id 捕获错位**(若该回合其实跑完了):分段拿到的 id 与记录 id 不一致——同 [[streaming-toolcall-id-no-empty-overwrite]] 一类,但该网关 `fc_..._<index>` 后缀格式踩了新坑。

哪个是真凶需看报告者的会话存档 JSON(对比 `messages[].segments` tool 段 `tool_call_id` vs `messages[].tool_calls[].id`)。**候选 2 无法盲修**——需要复现的 provider。

## Requirements

- R1 落库层双向对账:`normalize_assistant_segments` 增加「孤立工具分段」(segment 有 `tool_call_id` 但无同 id 记录)的处理,行为见 Decision。
- R2 呈现诚实:被中断/异常的工具调用不再以「工具记录缺失」这种像故障的文案出现。
- R3 不回归现有正向对账(有记录补分段)与正常多轮时间线顺序。
- R4 覆盖测试:构造「有分段无记录」的消息,验证对账结果符合 Decision。

## Acceptance Criteria

- AC1 一条带孤立工具分段的 assistant 消息经 `normalize_assistant_segments` 后,前端不再显示「工具记录缺失」占位(按 Decision 呈现)。
- AC2 现有 `normalize_assistant_segments` 正向补分段行为与顺序不变(既有测试绿)。
- AC3 `cargo test` chat::commands 相关 + 前端 `MessageBubble` 相关测试全绿。
- AC4(若纳入候选 2)有该网关复现:请求调试面板 + chat-probe 抓一次,确认分段/记录 id 一致。

## Decision(已定）

- **Q1 呈现 = 合成中断态记录**:落库层对孤立工具分段合成一条 `ToolCallRecord{status: Cancelled}`,前端渲染成正常工具卡片但标记「已中断/未完成」。从 `api_messages` 里按 id 回捞该工具调用的 name/arguments 丰富占位记录;捞不到则用通用名。保留「模型确实发起过该工具」的痕迹。
- **Q2 = 只做防御对账,候选 2(流式 id 错位)defer**:本次只做防御性双向对账,覆盖两种根因的用户可见症状。候选 2 需复现的 provider/会话 JSON,列 follow-up,无复现不盲改。

## Out of Scope

- 不改 provider/网关本身的稳定性;不新增 provider 适配。
- 候选 2(流式 id 错位)的根因修复(无复现,defer)。
