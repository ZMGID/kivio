# Implement：修复上下文压缩正确性缺陷

执行顺序按批次 A → E；每批次独立可编译、可测试、可提交（Conventional Commits）。
批次 C 依赖 A（复用 `accumulate_source_ids`）；B/D/E 无相互依赖但按序做以减少冲突。

**每批次收尾必跑**：

```powershell
powershell -File scripts/win-cargo-test.ps1   # 全量 Rust 测试（对照 HEAD ~14 个环境失败基线）
```

改动只涉及 Rust 后端，无需 `npm test` / `lint`（除非最终检查阶段顺手全跑）。

---

## 批次 A —— 缺陷 1：L2 感知注入摘要 + 写回累积（高危，最先做）

> 涉及：`compaction.rs`、`commands.rs`。design.md「缺陷 1」节。

- [ ] A1 `compaction.rs` 新增 `pub(crate) const PERSISTED_SUMMARY_PREFIX: &str = "Previous conversation summary:";`
      （带「生成/识别共用、防格式漂移」的文档注释）。
- [ ] A2 `commands.rs::summary_message` 改用该常量拼接（byte 级不变，
      现有测试 `assert!(serialized.contains("Previous conversation summary"))` 应原样通过）。
- [ ] A3 `extract_previous_summary` 扩展：入参改为能同时看到 system 前缀与 old_segment
      （建议签名 `fn extract_previous_summary(system_prefix: &[Value], old_segment: &[Value]) -> Option<PreviousSummary>`，
      其中 `PreviousSummary { text: String, from_injected: bool }` 或等价元组）。
      识别优先级：old_segment 锚点（user + `SUMMARY_MARKER_PREFIX`）优先，
      其次 system 前缀注入形态（system + `PERSISTED_SUMMARY_PREFIX`）；正文提取同现有
      `split_once('\n')` 逻辑。
- [ ] A4 `summarize_history`：
      - 调整 `extract_previous_summary` 调用点传入两段；
      - `from_injected` 为 true 时，拼装压缩后视图前从 `system_prefix` 剔除该注入消息
        （`replace_with_summary` 入参已是剔除后的 prefix，函数本身不用改）；
      - 锚点分支的 head 剔除逻辑保持不变。
- [ ] A5 `compaction.rs` 新增共享 helper：
      `pub(crate) fn accumulate_source_ids(conversation: &Conversation, until_id: &str) -> Vec<String>`
      —— 语义：旧 summary（未 stale）的 `source_message_ids` ∪ 旧 boundary 之后至 `until_id`
      （含）的全部消息 id；找不到 `until_id` 时返回仅旧 ids（防御，不 panic）。
      `compact_conversation_inner` 的现有累积代码改为调用它（行为等价重构）。
- [ ] A6 `commands.rs` run 结束写回块（~:2299）：接受 `result.compaction_summary` 时，
      用 `accumulate_source_ids(conversation, &summary.source_until_message_id)` 填充
      `source_message_ids`，`compressed_message_count` 取其 len（替换现在的直接搬运空 Vec）。
- [ ] A7 单测（compaction.rs tests 模块）：
      - `extract_previous_summary_detects_injected_system_summary`：AC1.1 前半
        （system 前缀注入形态被识别、正文正确）；
      - `extract_previous_summary_prefers_anchor_over_injected`：两者并存时取锚点；
      - `summarize_history` 层面验证注入消息被从视图剔除（可通过 `replace_with_summary`
        输出断言无 `PERSISTED_SUMMARY_PREFIX` 消息 + 有新锚点）——AC1.1 后半；
      - `accumulate_source_ids_unions_previous_and_new_range`：AC1.2 核心；
      - 全新会话（无注入/锚点）路径行为不变——AC1.3 由既有测试群覆盖，跑绿即可。
- [ ] A8 commands.rs 单测：模拟已有 S1 的 conversation 走写回块逻辑（可直接测新写回
      helper 或抽出的纯函数），断言累积 ids 与 count——AC1.2。
- [ ] A9 回归确认：kivio_code `force_compact` 路径（无注入形态 → no-op）；
      跑 `win-cargo-test.ps1` 全绿（对照基线）。
- [ ] A10 提交：`fix(chat): L2 compaction merges persisted summary instead of overwriting it`

**批次 A 回滚点**：单 commit revert；A5 的 helper 若已被批次 C 引用则先回滚 C。

---

## 批次 B —— 缺陷 2 + 4：token 估算统一（中危）

> 涉及：`prepare.rs`、`commands.rs`、`compaction.rs`。design.md「缺陷 2 + 4」节。

- [ ] B1 `prepare.rs` 新增 `pub(crate) fn estimate_value_tokens(value: &Value) -> usize`：
      从 `commands.rs::count_tokens_in_value` 原样搬移（image 部件 → 0；text/input_text →
      按 `text` 字段；对象 → key+value 递归；其余 → to_string 估算）。
      `commands.rs::count_tokens_in_value` 改为一行委托。
- [ ] B2 image/text part 类型判定抽共享（`prepare.rs` 内小 helper 或常量数组：
      `["image_url", "input_image", "image"]` / `["text", "input_text"]`），
      `estimate_value_tokens` 与 B4 的序列化占位共用。
- [ ] B3 `compaction.rs::estimate_message_tokens` 重写：
      - 字符串 content：`estimate_tokens(text) + tool_calls + reasoning_content + 4`
        （reasoning_content 为新增项，R4.1）；
      - 数组/对象 content：`estimate_value_tokens(content) + tool_calls + reasoning_content + 4`；
      - 无 content：`tool_calls + reasoning_content + 4`。
- [ ] B4 `compaction.rs::serialize_message` user 非字符串分支重写：遍历 parts，
      text 部件全文、image 部件输出 `[image attachment omitted]`（抽常量）、
      未知部件退回该 part JSON。
- [ ] B5 `compaction.rs::estimate_chat_message_tokens` 加展开分支：
      `model_messages` 非空 → `openai_messages_from_model_messages` 展开后逐条
      `estimate_message_tokens` 求和；否则 `api_messages` 非空 → 同法；
      否则现状 preview 口径。分支顺序与 `build_chat_api_messages` 一致（同源对齐注释）。
- [ ] B6 单测：
      - `estimate_message_tokens_ignores_image_base64`：大 base64 parts 消息估算与
        同文本纯文字消息同数量级——AC2.1；
      - `estimate_message_tokens_counts_reasoning`：带/不带 `reasoning_content` 对比——AC4.1；
      - `serialize_message_replaces_image_parts_with_placeholder`：输出含 text 全文 +
        占位符、断言不含 `;base64,`——AC2.2；
      - `estimate_chat_message_tokens_uses_expanded_api_messages`：带大 api_messages 的
        ChatMessage 估算 = 展开求和、远大于 preview 口径——AC4.2；
      - 既有 `estimate_counts_content_and_structured_fields` 等按新口径修正断言——AC2.3/AC4.3。
- [ ] B7 检查受影响既有测试：`token_split_chat_messages_*`（估算口径变化可能移动切点）、
      commands.rs `:7789/:7812` 两处测试——按新口径更新期望值，并在断言旁注明口径。
- [ ] B8 跑 `win-cargo-test.ps1` 全绿。
- [ ] B9 提交：`fix(chat): align compaction token estimates with real send content (images, reasoning, expanded tool outputs)`

---

## 批次 C —— 缺陷 3：落盘摘要过滤多答排除臂（中危，依赖 A5）

> 涉及：`commands.rs`（可见性）、`compaction.rs`。design.md「缺陷 3」节。

- [ ] C1 `commands.rs::group_answer_excluded_from_context` 改 `pub(crate)`（逻辑不动）。
- [ ] C2 `compaction.rs` 新增 `fn context_included_indices(conversation: &Conversation) -> Vec<usize>`
      （全量下标 minus 排除臂）。
- [ ] C3 `compact_conversation_inner` / `has_compressible_old_segment` 改为在过滤视图上
      切分与序列化：
      - token 切分走同一条实现（`token_split_chat_messages` 保持 `&[ChatMessage]` 签名的话，
        新增内部辅助接受过滤视图 + 下标映射；两个调用方共用）；
      - `manual_fallback_split` 同样在过滤视图上找最后一条 user；
      - boundary / `source_until_message_id` 映射回**原始下标**的消息 id；
      - 序列化只含过滤视图内消息。
- [ ] C4 `source_message_ids` 语义（R3.2）：沿用 A5 `accumulate_source_ids`
      ——按原始序列收集（含排除臂 id）。确认 helper 无需改动即满足；如 A 批次实现时
      范围参数不含 summary_start 语义则在此对齐。
- [ ] C5 单测：
      - `persisted_compaction_excludes_unselected_group_arms`：选中臂 A 内容进摘要输入、
        排除臂 B 不进——AC3.1；
      - `token_split_ignores_excluded_arms`：切点与「先滤 B 再切」一致——AC3.2；
      - `source_ids_include_excluded_arms`：ids 含 B（覆盖语义）；
      - 无多答组会话行为不变（既有测试回归）——AC3.3。
- [ ] C6 跑 `win-cargo-test.ps1` 全绿。
- [ ] C7 提交：`fix(chat): persisted compaction respects multi-answer group exclusion`

---

## 批次 D —— 缺陷 5.4：取消不计为压缩失败（低危中的行为项）

> 涉及：`compaction.rs`。design.md「缺陷 5」第 4 项。

- [ ] D1 `compact_with_summary_model` 返回类型改 `CompactAttempt { Summary(String), Cancelled, Failed }`
      （模块内私有 enum）；cancel 分支 → `Cancelled`，错误/空/质量闸 → `Failed`。
- [ ] D2 `summarize_history` 透传三态；`maybe_compact_send_view` 的 `Cancelled` 分支
      不递增 `compaction_unresolved_rounds`（仍发 `failed` 终止事件，payload/phase 不变）；
      `Failed` 分支行为同现状。
- [ ] D3 `force_compact` / `compact_conversation_inner`：`Cancelled` 不可达
      （cancel 传 None），`Failed` 映射回现有 None/Err；外部签名与行为不变。
- [ ] D4 单测：quality guard 各拒绝态映射 `Failed` 的既有测试保持；如 loop_tests 有
      压缩失败计数用例，确认取消路径不被误计（可加一个 host cancel 立即触发的用例，
      断言 `compaction_unresolved_rounds == 0`）。
- [ ] D5 跑 `win-cargo-test.ps1` 全绿。
- [ ] D6 提交：`fix(chat): user cancellation no longer counts as compaction failure`

---

## 批次 E —— 缺陷 5.1/5.2/5.3：整洁项（可与任意批次同车，单独列便于验收）

> 涉及：`compaction.rs`。design.md「缺陷 5」第 1–3 项。

- [ ] E1 ack 文案抽常量 `SUMMARY_ACK_TEXT`；`replace_with_summary` 引用；
      `summarize_history` 的 head 剔除 filter 追加 ack 条件（仅 previous_summary
      存在的分支）。
- [ ] E2 单测 `chained_summary_head_excludes_anchor_ack`：旧锚点 + ack + 后续消息 →
      序列化 head 不含 ack 文案——AC5.1。
- [ ] E3 `clip_serialized_to_budget` 注释弱化（「除不可达极端外恒成立」+ 说明
      head_budget 下限保证）；不改代码。
- [ ] E4 陈旧注释修正：`SUMMARY_INPUT_BUDGET_RATIO` 文档「近期 8k」→ 20k；
      `maybe_compact_send_view` 内「比 8000 还小」→ 同步；
      测试改名 `summary_output_tokens_caps_at_4096` → `summary_output_tokens_respects_model_cap`。
- [ ] E5 跑 `win-cargo-test.ps1` 全绿（AC5.2：无行为变化）。
- [ ] E6 提交：`chore(chat): compaction cleanups (ack dedup, stale comments, test rename)`

---

## 最终检查（全批次完成后）

- [ ] F1 `win-cargo-test.ps1` 全量对照基线（仅既有 ~14 个环境失败）。
- [ ] F2 `npm run lint && npm run typecheck && npm test`（前端未改，跑一遍确认无意外牵连）。
- [ ] F3 手动/半自动验证（可用 chat-probe 通道）：
      - 长会话触发落盘压缩 → 再触发 L2 → 检查 `context_state.summary` 内容含早期信息、
        `source_message_ids` 累积；
      - 带图会话多工具轮：不再出现每步压缩日志（`Chat context compaction: est ... over budget`）
        与 anti-thrashing 提前收尾；
      - kivio_code 交互模式 `/compact`：行为同修前。
- [ ] F4 review 汇总：五个 commit 逐一过 diff（或跑 `/code-review`）。
- [ ] F5 Trellis 3.3 spec 更新评估：`.trellis/spec/chat/` 若有 compaction 相关条目，
      补「PERSISTED_SUMMARY_PREFIX 生成/识别共用」「估算三函数同源（estimate_value_tokens）」
      两条契约；无对应文档则记入任务 notes。
- [ ] F6 `task.py` 收尾流程（archive 前确认 journal/commit 齐）。

## 风险与注意

- **B7 是最容易漏的点**：估算口径变化会移动既有测试的切点期望值，逐个核对而非批量放宽断言。
- A3 改 `extract_previous_summary` 签名时注意现有测试 `extract_previous_summary_detects_anchored_marker`
  需适配新签名（传空 system_prefix 即保持原语义）。
- C3 若 `token_split_chat_messages` 的公开签名被迫变化，commands.rs 的两处测试调用点
  （:7789/:7812）要同步。
- 任何一步发现 prd/design 假设与代码不符（例如写回块的实际行号/结构漂移），
  回 Plan 修正文档再继续（Trellis 阶段回滚规则）。
