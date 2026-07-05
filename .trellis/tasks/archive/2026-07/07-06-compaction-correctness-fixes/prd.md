# PRD：修复上下文压缩（compaction.rs）审查发现的正确性缺陷

## 背景

2026-07-06 对 `src-tauri/src/chat/agent/compaction.rs` 及其调用链做了一次正确性审查，
交叉核对了：

- 运行时调用方：`chat/agent/planning.rs`（planning 前压缩 + anti-thrashing 收尾）、
  `chat/agent/synthesis.rs`（合成前压缩 + overflow 恢复重试）、`chat/agent/recovery.rs`、
  `chat/agent/loop_.rs`（`RunState.compacted / pending_compaction_boundary / pending_compaction_summary`）
- 落盘写回：`chat/commands.rs`（run 结束写回 `context_state.summary` + boundary、
  `should_auto_compress_context` / `try_auto_compress_context_after_update` 自动落盘压缩）
- runtime 消息构建：`commands.rs::build_chat_api_messages`（summary 注入 + boundary 之后 replay +
  `_ui_message_id` 标注 + 多答组排除）
- 前端事件消费：`src/chat/Chat.tsx` 的 `chat-compaction` 监听（压缩中状态 + divider 动画）

整体架构（三条路径共享 `compact_with_summary_model` 核心、事件 started/终止配对、
tool_call↔tool 配对保护、质量三重闸、anti-thrashing）是健全的，**不需要重构**。
本任务只修审查确认的缺陷，按严重度分级。

## 术语（下文引用）

- **L2 压缩**：agent loop 运行中 `maybe_compact_send_view` 触发的运行时摘要
  （写回 `state.runtime_messages`，run 结束经 `AgentRunResult.compaction_summary` 落盘）。
- **落盘压缩**：`compact_conversation`（手动 `/compact` 或 `try_auto_compress_context_after_update`
  自动触发），直接改 `conversation.context_state.summary`。
- **S1 / S2**：先后两份 summary。
- **锚点摘要**：`replace_with_summary` 插入 runtime 历史的 user 消息，
  content 以 `SUMMARY_MARKER_PREFIX`（`[context summary]`）开头。
- **注入摘要**：`build_chat_api_messages` 从 `context_state.summary` 注入的 **system** 消息，
  content 前缀为 `Previous conversation summary:`（`commands.rs::summary_message`）。

---

## 缺陷 1（高危 / 必修）：L2 写回覆盖旧落盘 summary，静默丢上下文

### 复现链条

1. 长会话触发过一次落盘压缩，产生 S1（覆盖消息 1–50），存于 `context_state.summary`。
2. 下一轮 run 开始：`build_chat_api_messages` 把 S1 作为**注入摘要（system role）**放在
   system prompt 之后，之后只 replay S1 boundary 之后的消息（51+）。
3. 本轮 agentic run 中上下文再次超预算，`maybe_compact_send_view` 触发 L2 摘要：
   - `select_recent_by_tokens` 把**开头连续 system 消息**整体划入受保护前缀——
     S1 的注入消息（index 1，system）落在前缀里，**不进 old_segment**；
   - `extract_previous_summary` 只识别 **user role + `[context summary]` 前缀**的锚点摘要，
     对 system role + `Previous conversation summary:` 前缀的注入摘要**匹配不到**；
   - 因此新摘要 S2 只覆盖本轮 old_segment（消息 51–X），**完全不含 S1 的内容**。
4. run 结束：`commands.rs`（arm 分支 return 之后的写回块，约 2299–2311 行）执行
   `conversation.context_state.summary = Some(S2)` ——**无条件整体替换** S1。
5. 之后每一轮 `build_chat_api_messages` 只注入 S2、只 replay S2 boundary 之后的原文：
   **消息 1–50 的信息从此在任何发给模型的上下文中都不存在**（S1 没了，原文也不 replay）。

丢失发生在**跨轮**且完全静默：触发当轮运行时视图仍有 S1（在 system 前缀里），
用户看到的时间线原文也都在（落库消息未删），但模型再也看不到早期内容。

### 触发面

不是理论路径：长会话到裸窗口 90% 会先走 auto 落盘压缩生成 S1；此后任何一次长 agentic turn
（多工具轮把 runtime 再撑过 90%）触发 L2 即中招。会话越长、agent 工具越重越容易发生。

### 同源不一致（一并修）

L2 产出的 `ConversationContextSummary`：

- `source_message_ids` 恒为 `Vec::new()`（compaction.rs 构造 summary_record 处），
  写回时 `compressed_message_count = summary.source_message_ids.len() = 0`；
- 而落盘路径 `compact_conversation_inner` 是**累积合并**旧 summary 的 ids 的。

两条路径落的是同一个字段，口径必须一致，否则统计与「上一份 summary 覆盖到哪」判定失真。

### 需求

- **R1.1**：L2 压缩必须感知「本轮 runtime 历史里由 `context_state.summary` 注入的旧摘要」，
  新摘要必须**合并**其内容（作为 `previous_summary` 走既有 anchored 链式合并通道）。
  实现方向任选其一（design 阶段定）：
  a. `extract_previous_summary` 同时识别注入摘要形态（system role + `Previous conversation summary:` 前缀）。
     注意它在 system 前缀里而非 old_segment，探测范围需要覆盖 system 前缀；识别后要把它
     从下一轮的发送视图/序列化 head 中妥善处理（同锚点摘要的现有剔除逻辑对齐）。
  b. run config 显式携带落盘 summary 内容，`maybe_compact_send_view` 直接作为 previous_summary 传入。
  c. commands.rs 写回时检测 L2 summary 未含旧 summary（如 boundary 不衔接）则拼接合并而非替换。
  倾向 a 或 b（在摘要生成时由模型合并，语义最干净）；c 只能机械拼接文本、质量最差，
  除非 a/b 有结构性障碍否则不选。
- **R1.2**：修复后，S2 的 `source_until_message_id` 语义要能支撑 `build_chat_api_messages`
  的「boundary 之后 replay」不漏不重：S2 覆盖范围 = S1 覆盖范围 ∪ 本轮 old_segment。
- **R1.3**：L2 写回的 `source_message_ids` / `compressed_message_count` 与落盘路径口径一致
  （累积合并，不再是空 Vec / 0）。若 runtime 侧拿不到完整 UI 消息 id 列表，可由 commands.rs
  写回时基于 `source_until_message_id` 从 conversation.messages 推导，但必须与落盘路径产出等价。
- **R1.4**：识别注入摘要后，链式质量闸（`summary_quality_guard` 的 Degraded 30% 门槛）
  应以合并前的旧 summary 长度为基准生效——不能因为改了识别方式而绕过
  「新摘要显著短于旧摘要 → 拒绝覆盖」保护。

### 验收标准

- AC1.1：单测：构造「system prompt + 注入摘要(system) + 若干旧消息 + 近期消息」的 runtime 序列，
  L2 压缩路径能把注入摘要识别为 previous_summary（断言 `build_summary_user_content` 收到
  `<previous-summary>` 且内容来自注入摘要），且注入摘要文本不重复出现在序列化 head 中。
- AC1.2：单测：模拟 run 结束写回——已有 S1 的 conversation 接受 L2 产出的 S2 后，
  `context_state.summary.source_message_ids` 包含 S1 的 ids（累积语义），
  `compressed_message_count` > 0，`source_until_message_id` 为本轮 old_segment 末尾对应 UI 消息。
- AC1.3：回归：无旧 summary 的全新会话 L2 压缩行为不变（现有 loop_tests / compaction tests 全绿）。
- AC1.4：`cargo test`（经 `scripts/win-cargo-test.ps1`）compaction / loop / commands 相关测试通过。

---

## 缺陷 2（中危）：图片 base64 打爆 token 估算 → 徒劳压缩 + anti-thrashing 误收尾

### 现状

- 最后一条 user 消息带图片时，`commands.rs::image_content_part` 把完整 base64 data URL
  嵌进 content parts（数组 content）。
- `compaction.rs::estimate_message_tokens` 对非字符串 content 走
  `estimate_tokens(&message.to_string())` 整体 JSON 估算：1MB 图片 ≈ 1.4M base64 字符
  ≈ 35 万「token」，远超任何窗口 90% 预算。
- 后果链：每个 planning step 都触发压缩 → 图片消息在受保护近期窗口里压不掉 →
  `after > budget` → `compaction_unresolved_rounds` 递增 → 两轮后 planning 的
  anti-thrashing 分支（`COMPACTION_THRASH_LIMIT = 2`）**用已收集的工具结果提前收尾整个 turn**
  ——尽管真实 token（provider 按图片 tile 计费，如 OpenAI ~85–1105 tok/图）完全没超窗。
- 对照组：`commands.rs::count_tokens_in_value`（上下文用量条）**刻意对
  `image_url`/`input_image`/`image` 部件返回 0**——两处口径本该一致，压缩侧漏了。
- 次生问题：`serialize_message` 的 user 非字符串分支把整段 content JSON（含 base64）
  灌进摘要序列化；虽然会被 head-tail 裁剪，但裁剪保留的头尾可能全是 base64 垃圾，
  污染摘要质量。

### 需求

- **R2.1**：`estimate_message_tokens` 对数组/对象 content 的估算与
  `count_tokens_in_value` 口径对齐：image 部件按固定成本计（0 或一个小常量，
  与 `count_tokens_in_value` 保持同一选择），text 部件按文本估算；不再对整个 JSON 串估算。
  实现上优先复用/下沉 `count_tokens_in_value` 的逻辑而非复制一份（两处将来要一起演化；
  注意它现在在 commands.rs、依赖 `agent_prepare::estimate_tokens`，下沉方向 design 定）。
- **R2.2**：`serialize_message` 的 user 非字符串 content 分支：提取 text 部件全文 +
  对 image 部件输出短占位（如 `[image attachment omitted]`），不再整体 JSON 序列化。
- **R2.3**：修复后带图长会话不再出现「每步触发压缩 / anti-thrashing 提前收尾」；
  真实超窗时行为不变。

### 验收标准

- AC2.1：单测：带一条大 base64 图片 parts 的 user 消息，`estimate_message_tokens`
  结果与同文本的纯文字消息同数量级（不含 base64 体积）。
- AC2.2：单测：`serialize_message` 对图片 parts user 消息，输出含 text 部件全文与图片占位符，
  不含 base64 前缀（断言不含 `;base64,`）。
- AC2.3：回归：`estimate_messages_tokens` 现有测试全绿；纯文本消息估算值不变。

---

## 缺陷 3（中危）：落盘摘要不过滤多答排除臂，被排除内容经 summary 回灌模型

### 现状

- `build_chat_api_messages` 用 `group_answer_excluded_from_context` 把多模型一问多答里
  未选中的 assistant 臂**排除出发给模型的历史**（R6/AC4 语义：仅选中臂进上下文）。
- 但落盘路径 `compact_conversation_inner` 的
  `old_segment = &conversation.messages[summary_start..=split]` 不做该过滤：
  - 被排除臂的内容进入摘要输入，摘要产出后经注入摘要**回灌给模型**，违背排除语义
    （失败臂的错误文案 / 未选中模型的答案会作为「早前对话事实」存续）；
  - `token_split_chat_messages` 也把这些消息计入预算，token 切点与真实 replay 内容错位
    （boundary 偏早，实际发送内容比估算少）。

### 需求

- **R3.1**：落盘路径的 old_segment 序列化与 token 切分都跳过
  `group_answer_excluded_from_context` 为 true 的消息（与 `build_chat_api_messages` 同谓词，
  复用同一函数，不复制逻辑）。
- **R3.2**：`source_message_ids` **包含**被排除臂的 id（表示「已覆盖/不再 replay」——
  build_chat_api_messages 的 start_idx 按 boundary 下标整段跳过，被排除臂本来也不 replay；
  关键是**内容不进摘要**）。此语义在 prd 层面定死：ids 包含、内容不进。
- **R3.3**：L2 运行时路径不受影响（runtime_messages 由 build_chat_api_messages 构建，
  天然已排除），只改落盘路径。

### 验收标准

- AC3.1：单测：构造含多答组（选中臂 A、排除臂 B）的 conversation 走落盘序列化，
  断言摘要输入含 A 内容、不含 B 内容。
- AC3.2：单测：token 切分结果与「先滤除 B 再切」的序列一致。
- AC3.3：回归：无多答组的会话落盘压缩行为不变。

---

## 缺陷 4（中危）：两处 token 估算系统性偏低

### 现状

- (a) `estimate_message_tokens`（Value 版）在 content 为字符串时只计 content + tool_calls，
  **不计 `reasoning_content`**；而 `serialize_message` 是包含 reasoning 的。带长推理链的
  runtime 历史被低估 → 压缩触发过晚 / 「压完回预算内」误判。
- (b) 落盘路径 `estimate_chat_message_tokens` 用 `tool.result_preview`（截断预览）估算，
  但真实 replay 发给模型的是 `api_messages` / `model_messages` 里的**完整工具输出**。
  recent tail 名义 ≤ 20k tokens，实际可能大数倍。

### 需求

- **R4.1**：`estimate_message_tokens` 把 `reasoning_content` 计入（与 serialize 口径一致）。
- **R4.2**：`estimate_chat_message_tokens` 在消息带 `api_messages` / `model_messages` 时，
  改用展开后的 runtime 形态估算（可对每条展开消息调 Value 版 `estimate_message_tokens`
  求和——与 build_chat_api_messages 的展开路径同源；`model_messages` 优先，同该函数的
  分支顺序）；无展开数据时保持现状（preview 估算）。
- **R4.3**：R4.2 会让落盘 token 切点整体前移（recent tail 变小、old_segment 变大）——
  这是**修正**而非回归，但 `has_compressible_old_segment` / `should_auto_compress_context`
  的联动行为要在测试里明确固定下来。

### 验收标准

- AC4.1：单测：带 `reasoning_content` 的 assistant 消息估算值 > 去掉 reasoning 的同消息。
- AC4.2：单测：带大体量 `api_messages`（完整工具输出）的 ChatMessage，
  `estimate_chat_message_tokens` 结果与展开后 Value 版求和一致（远大于 preview 口径）。
- AC4.3：现有 token_split / has_compressible 测试按新口径更新后全绿。

---

## 缺陷 5（低危 / 整洁，随手修）

1. **`clip_serialized_to_budget` 注释措辞过强**：`char_budget <= 1` 退出分支上
   「硬保证 ≤ budget」不成立（budget 小于省略标记自身 token 时超出）。现实不可达
   （head_budget 有 `summary_input_budget/4` 下限）。修注释即可，或在极端分支返回
   纯截断（无标记）让保证真正成立——二选一，design 定。
2. **anchored 链式摘要漏剔 ack 消息**：`summarize_history` 剔除了旧锚点摘要的 user 消息，
   但没剔除配对的 assistant ack（「已了解早前对话的摘要，继续当前任务。」），
   它作为 `[Assistant]:` 噪声行进入摘要输入。识别方式：紧跟锚点摘要之后、content
   等于固定 ack 文案的 assistant 消息（该文案抽成常量供 `replace_with_summary` 与
   剔除逻辑两处引用）。
3. **陈旧注释/命名**：
   - `SUMMARY_INPUT_BUDGET_RATIO` 文档注释「近期 8k 窗口」→ 应为 20k
     （`RECENT_KEEP_TOKENS` 已改 20_000）；
   - `maybe_compact_send_view` 内注释「窗口比 8000 还小的模型」→ 同上；
   - 测试名 `summary_output_tokens_caps_at_4096` 实际断言 8192 → 改为与常量语义一致的名字。
4. **取消被计为压缩失败**：用户取消进行中 run 时 `compact_with_summary_model` cancel 分支
   返回 None → 计入 `compaction_unresolved_rounds` 并发 `failed` 事件。无实害
   （run 随后按取消收尾），但语义上取消 ≠ 失败。改法：cancel 分支返回可区分信号
   （enum 或专用包装），调用方对取消**不**递增 unresolved；`failed` 终止事件保留
   （前端只需要终止事件把「压缩中」归位）。或保持现状仅加注释说明——最小改动优先，
   design 定；不允许为此引入复杂状态机。

### 验收标准

- AC5.1：ack 剔除有单测（旧锚点 + ack + 后续消息 → 序列化 head 不含 ack 文案）。
- AC5.2：改名/注释项无行为变化（编译 + 现有测试全绿即可）。

---

## 范围与约束

- **只改** `src-tauri/src/chat/agent/compaction.rs`、`src-tauri/src/chat/commands.rs`
  （估算/过滤/写回相关函数）及配套测试；如 R1 选方案 b 需在 run config 加字段，
  允许最小侵入地触碰 `loop_.rs` / `chat/agent/types.rs`。
- **不改**：模型适配层（`chat/model/`）、AgentHost trait 签名（除非 R1 方案 b 必需且经
  design 确认）、前端 `chat-compaction` 事件 payload 形状（UI 契约）、
  `RECENT_KEEP_TOKENS` / `AUTO_COMPACT_RATIO` / `COMPACTION_THRASH_LIMIT` 等既有阈值。
- **一条循环多个宿主**：compaction 在 GUI chat、Kivio Code（`force_compact`）、sub-agent
  三处生效。改动必须对 kivio_code 的手动 `/compact` 路径回归（其 runtime_messages 没有
  注入摘要形态，R1 改动对它应是 no-op）。
- Windows 上跑 Rust 测试必须经 `scripts/win-cargo-test.ps1`（直接 cargo test 二进制
  0xC0000139 起不来）；对照基线：HEAD 上已有 ~14 个 --lib 环境相关失败，非本任务回归。
- 每个缺陷独立可验证、独立可提交；建议按 1 → 2 → 3 → 4 → 5 顺序实施
  （1 最高优先，5 可与任意一批同车）。

## 非目标

- 不重构三路径共享核心的结构；不引入新的压缩策略/阈值调参。
- 不处理注入摘要在 Gemini/Anthropic 适配层的 role 合并——适配层已有
  `merge_consecutive_*_roles` 处理，非本任务范围。
- 不做摘要质量的 prompt 调优。
- 不处理多模型臂（arm 模式）L2 boundary/summary 被丢弃的既有设计（臂不落盘是有意的）；
  臂内 `chat-compaction` 事件可能造成主时间线「压缩中」短暂闪烁的问题单独记录、不在本任务修。
