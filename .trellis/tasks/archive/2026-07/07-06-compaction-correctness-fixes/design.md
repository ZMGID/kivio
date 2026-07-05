# Design：修复上下文压缩正确性缺陷

对应 `prd.md` 的缺陷 1–5。本文档定下每个缺陷的具体技术方案、被否方案的理由、
边界与数据流变化。**无架构重构**：三路径共享核心（`compact_with_summary_model`）、
事件契约（`chat-compaction`）、阈值常量全部不动。

## 现状事实（设计依据，已核实）

- `commands.rs` 私有函数：`group_answer_excluded_from_context`（:4621）、
  `count_tokens_in_value`（:3649）、`active_summary`（:3602）、`summary_message`（:3625，
  硬编码 `"Previous conversation summary:\n{content}"`，system role）。
- `openai_messages_from_model_messages` 已是 `chat/model/types.rs` 的 **pub** 函数（:547），
  R4.2 可直接复用，无需动可见性。
- `estimate_tokens` 在 `chat/agent/prepare.rs`（pub，ASCII÷4 + 非 ASCII×1）。
- `token_split_chat_messages` 的调用方：`compact_conversation_inner`、
  `has_compressible_old_segment`、commands.rs 两处**测试**（:7789 / :7812）。
- `compact_with_summary_model` 返回 `Option<String>`；cancel 分支与失败分支同样返回 None。

---

## 缺陷 1：L2 感知注入摘要（选方案 a，带共享常量）

### 决策：方案 a —— 按共享前缀常量识别注入摘要

**核心理由：方案 b（run config 携带 summary）解决了「合并」但解决不了「去重」。**
合并后的锚点摘要已含 S1 内容，而注入摘要（S1）仍留在压缩后视图的 system 前缀里——
同一信息在视图里出现两份。要把注入消息从前缀里摘掉，终究需要按形态识别它；
既然识别逻辑无论如何都要写，方案 a 一步到位，且不碰 `AgentRunConfig` / 宿主构造方
（sub_agent、kivio_code 的 config 全都不用改）。方案 c（写回时机械拼接）摘要质量最差，否。

「按字符串前缀嗅探脆弱」的顾虑用**共享常量**消解：前缀只在一个地方定义，
生成方与识别方引用同一常量，格式漂移在编译期就不可能发生。

### 改动明细

1. **共享常量**（compaction.rs 新增，commands.rs 改引用）：

   ```rust
   /// build_chat_api_messages 注入落盘 summary 的 system 消息前缀。
   /// 生成（commands.rs::summary_message）与识别（extract_previous_summary）共用，
   /// 防止格式漂移导致 L2 压缩认不出旧摘要（曾导致跨轮静默丢上下文）。
   pub(crate) const PERSISTED_SUMMARY_PREFIX: &str = "Previous conversation summary:";
   ```

   `commands.rs::summary_message` 改为 `format!("{PERSISTED_SUMMARY_PREFIX}\n{}", ...)`。
   wire 内容不变（byte 级一致），零行为变化。

2. **`extract_previous_summary` 扩展**：签名改为接收 `system_prefix: &[Value]` +
   `old_segment: &[Value]` 两段（或直接收整个 messages + split 信息，实施时取顺手的）。
   识别优先级：
   - **锚点摘要优先**（user role + `SUMMARY_MARKER_PREFIX`，在 old_segment 里）——
     它是同一 run 内更晚的 L2 产物，已含合并结果；
   - 无锚点时，在 **system 前缀**里找 system role + `PERSISTED_SUMMARY_PREFIX` 开头的消息，
     取正文（`split_once('\n')` 后段，同锚点处理）。
   两者都有时用锚点（更新）；这保证同一 run 内第二次 L2 压缩不回退到过期的 S1。

3. **`replace_with_summary` / `summarize_history` 去重**：识别到注入摘要时，
   从 `system_prefix` 中**剔除**该条注入消息再拼装压缩后视图（其内容已并入新锚点摘要）。
   锚点摘要的现有剔除逻辑（不进 head）保持，注入摘要同样不进 head——
   它在 system 前缀里本来就不进 old_segment，只需保证不重复出现在视图中。

4. **写回累积**（commands.rs，run 结束写回块 ~:2299）：接受 L2 `compaction_summary` 时：

   ```text
   new.source_message_ids =
       旧 summary（未 stale）的 source_message_ids
       ∪ conversation.messages[旧 boundary+1 ..= 新 source_until 下标] 的所有 id
   compressed_message_count = new.source_message_ids.len()
   ```

   由 commands.rs 在写回时基于 `source_until_message_id` 从 conversation 推导
   （L2 运行时侧拿不到完整 UI id 列表，且写回处有 conversation 的全部信息）。
   推导逻辑与 `compact_conversation_inner` 的累积语义等价——抽一个小的共享 helper
   （`fn accumulate_source_ids(conversation, prev_summary, until_id) -> Vec<String>`）
   放 compaction.rs，两条路径都调用，防止再次口径分叉。

5. **质量闸（R1.4）自然满足**：previous_summary（来自注入摘要）照常传入
   `compact_with_summary_model` → `summary_quality_guard` 的 Degraded 30% 门槛
   以它为基准生效，无需额外改动。

### 数据流（修复后）

```text
落盘 S1 ──build_chat_api_messages──▶ system 前缀内注入消息（PERSISTED_SUMMARY_PREFIX）
                                            │
              L2 触发：extract_previous_summary 识别（锚点优先，其次注入）
                                            ▼
        compact_with_summary_model(previous_summary = S1) → 模型合并产出 S2
                                            │
        replace_with_summary：system 前缀剔除注入消息 + 插入 [context summary] S2 锚点
                                            ▼
        run 结束写回：source_message_ids = S1.ids ∪ 本轮 old_segment ids（累积）
                                            ▼
        下一轮注入 S2（含 S1 信息）——不丢上下文
```

### 边界

- 全新会话（无落盘 summary）：system 前缀里没有匹配消息 → 行为与现状 bit 级一致。
- kivio_code `force_compact`：其 runtime_messages 无注入摘要形态 → no-op（回归点）。
- 同 run 多次 L2：第二次起走锚点分支（优先级规则），不重复合并 S1。

---

## 缺陷 2 + 4：token 估算统一（两处一起改，共享同一底层）

估算函数的关系（修复后）：

```text
prepare.rs::estimate_tokens(&str)              ← 唯一字符启发式（不动）
prepare.rs::estimate_value_tokens(&Value)      ← 新：count_tokens_in_value 下沉至此
        ├── commands.rs::count_tokens_in_value  → 改为薄委托（或直接改调用点）
        ├── compaction.rs::estimate_message_tokens（非字符串 content 分支）
        └── compaction.rs::estimate_chat_message_tokens（api/model_messages 展开分支）
```

### 决策

1. **`count_tokens_in_value` 下沉到 `prepare.rs`**，改名 `estimate_value_tokens`，
   `pub(crate)`。逻辑原样搬移（image 部件返回 **0**，text 部件按文本，对象按 key+value 递归）。
   commands.rs 原函数改为一行委托（保持现有调用点不动，减少 diff 面）。
   选 prepare.rs 而非 compaction.rs：它是 `estimate_tokens` 的家，估算家族聚一处。
2. **`estimate_message_tokens` 重写非字符串分支**：
   - content 为字符串：`estimate_tokens(content) + tool_calls(JSON) + reasoning_content + 4`
     （新增 reasoning_content 项，R4.1）；
   - content 为数组/对象：`estimate_value_tokens(content) + tool_calls + reasoning_content + 4`，
     **不再** `message.to_string()` 整体估算；
   - content 缺失/null：只计 tool_calls + reasoning + 4。
   注意：旧的 `to_string()` 兜底会把 role/tool_call_id 等结构字段也计进去（几个 token 的噪声），
   新实现不计——这是精度提升，测试按新口径断言。
3. **`serialize_message` user 非字符串分支重写**：遍历 parts，
   `type == "text"|"input_text"` 取 `text` 全文；`type == "image_url"|"input_image"|"image"`
   输出常量占位 `[image attachment omitted]`；未知 part 退回该 part 的 JSON（保守不丢信息）。
   类型字符串集合与 `estimate_value_tokens` 的 image 判定共用（同一 match 逻辑抽小 helper
   或至少常量数组共享），防止两处漂移。
4. **`estimate_chat_message_tokens` 展开分支（R4.2）**：

   ```rust
   if !message.model_messages.is_empty() {
       return openai_messages_from_model_messages(&message.model_messages)
           .iter().map(estimate_message_tokens).sum();
   }
   if !message.api_messages.is_empty() {
       return message.api_messages.iter().map(estimate_message_tokens).sum();
   }
   // 无展开数据：保持现状（content + reasoning + tool preview 口径）
   ```

   分支优先级 `model_messages` → `api_messages` → 现状，与
   `build_chat_api_messages` 的展开顺序**同源对齐**（这是「估算 = 真实发送内容」的关键）。

### 影响评估（R4.3）

落盘 token 切点前移（recent tail 按真实体量算，容纳的消息条数变少）。
`has_compressible_old_segment` 会更早返回 true → auto 落盘压缩更早触发。
这是修正：此前 preview 口径下「名义 20k 实际 100k+」的 recent tail 才是异常。
测试里用「带大 api_messages 的消息」显式固定新行为。

---

## 缺陷 3：落盘摘要过滤多答排除臂

### 决策：谓词提可见性 + 落盘路径过滤视图

1. `commands.rs::group_answer_excluded_from_context` 改 `pub(crate)`（逻辑零改动）。
2. compaction.rs 新增小 helper：

   ```rust
   /// 落盘路径的「进上下文」消息下标集：跳过多答组未选中臂
   ///（与 build_chat_api_messages 同谓词，直接复用，不复制逻辑）。
   fn context_included_indices(conversation: &Conversation) -> Vec<usize>
   ```

3. `token_split_chat_messages` **签名不动**（`&[ChatMessage]` 语义保持，测试兼容）。
   `compact_conversation_inner` / `has_compressible_old_segment` 改为：
   - 先取 `context_included_indices` 过滤出「参与压缩的消息视图」（`Vec<&ChatMessage>` +
     原始下标映射）；
   - 在过滤视图上做 token 切分（新增一个内部重载/泛型辅助，或把现有函数改成以
     `&[&ChatMessage]` 为输入、原调用点做一次 `iter().collect()` 适配——实施取 diff 小者，
     但**两个调用方必须走同一条切分实现**）；
   - `split` 映射回**原始下标**后取 boundary 消息 id。
4. 序列化：old_segment 只序列化过滤视图内的消息（排除臂内容不进摘要输入）。
5. `source_message_ids`（R3.2 定死语义）：按**原始序列** `summary_start..=boundary原始下标`
   收集全部 id（**含**排除臂）——它们同样被 boundary 覆盖、不再 replay。
   缺陷 1 的 `accumulate_source_ids` helper 天然按原始序列收集，语义吻合。

### 边界

- `manual_fallback_split` 同样要在过滤视图上找「最后一条 user」（否则手动小对话
  含排除臂时切点错位）——一并改。
- L2 路径零改动（runtime_messages 天然已过滤）。
- 无多答组会话：`context_included_indices` 返回全量下标，行为不变（回归点）。

---

## 缺陷 5：低危项决策

1. **`clip_serialized_to_budget`**：只改注释。把「硬保证」弱化为
   「除 budget 小于省略标记自身 token 的不可达极端外恒成立（head_budget 有
   `summary_input_budget/4` 下限）」。不改代码——为不可达分支加逻辑是负价值。
2. **ack 剔除**：`replace_with_summary` 里的 ack 文案抽常量
   `SUMMARY_ACK_TEXT: &str = "已了解早前对话的摘要，继续当前任务。"`。
   `summarize_history` 构造 head 时，在剔除锚点摘要的同一 filter 里追加条件：
   剔除 `role == "assistant" && content == SUMMARY_ACK_TEXT` 的消息
   （仅 previous_summary 存在时生效，与锚点剔除同分支）。按内容全等匹配足够：
   该文案是我们自己插入的固定串，assistant 自发产出同款全文的概率可忽略。
3. **改名/注释**：`summary_output_tokens_caps_at_4096` → `summary_output_tokens_respects_model_cap`；
   两处「8k/8000」注释改 20k。纯文本改动。
4. **取消 ≠ 失败**：`compact_with_summary_model` 返回类型改为内部 enum：

   ```rust
   enum CompactAttempt { Summary(String), Cancelled, Failed }
   ```

   - cancel 分支 → `Cancelled`；请求错误 / 空摘要 / 质量闸拒绝 → `Failed`。
   - `summarize_history` 透传（返回 `Option<(Vec<Value>, String)>` 改为携带同区分的形态，
     或返回 `Result`-风格三态——实施取最小 diff）。
   - `maybe_compact_send_view`：`Cancelled` 分支**不递增** `compaction_unresolved_rounds`，
     仍发 `failed` 终止事件（前端契约只需要终止事件归位，payload 形状不动，
     不新增 phase 值——新增 `cancelled` phase 会碰 UI 契约，超出本任务范围）。
   - `force_compact` / `compact_conversation_inner`（cancel 恒 None）：`Cancelled` 不可达，
     `Failed` 映射到现有 `None`/`Err`，外部行为不变。

---

## 兼容性与回滚

- 全部改动在 Rust 侧；`chat-compaction` payload、`ConversationContextSummary` /
  `CompactionBoundaryRecord` serde 形状不变（只是 `source_message_ids` 从空变为有值——
  字段本就存在，前端/存储无迁移）。
- 旧会话数据兼容：已丢失 S1 的历史会话无法追回（数据已不在），修复只保证不再发生。
- 回滚：五个缺陷各自独立 commit（见 implement.md 批次），任一批次可单独 revert；
  批次间无编译依赖（缺陷 1 的 `accumulate_source_ids` 被缺陷 3 复用，
  故批次 C 依赖批次 A 先落——已在实施顺序中体现）。

## 测试策略总览

- 单测为主（compaction.rs `#[cfg(test)]` + commands.rs 现有测试群扩展），
  覆盖 prd 各 AC；`select_recent_by_tokens` / 质量闸等既有测试作回归网。
- 运行：`scripts/win-cargo-test.ps1`（Windows 直跑 cargo test 二进制 0xC0000139）；
  基线：HEAD 已有 ~14 个 --lib 环境相关失败，对照排除。
- 可选 E2E：chat-probe GUI 通道（写 `chat_probe/request.json` 驱动运行中 GUI 跑真实
  agent 生成）人工验证带图会话不再触发 anti-thrashing 提前收尾。
