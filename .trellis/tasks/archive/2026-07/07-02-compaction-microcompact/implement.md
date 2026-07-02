# implement.md — Microcompact 增量降级（R-1）

## 执行清单
1. `compaction.rs` 常量：`const MICROCOMPACT_TOOL_MARKER: &str = "[earlier tool result omitted to save context]";`
2. `compaction.rs` 新增纯函数 `microcompact_send_view(messages, keep_tokens, budget) -> Option<Vec<Value>>`：
   - `select_recent_by_tokens` 切 (system, old, recent)；old 空→None。
   - old 内 `is_tool_result(&m)` 且 content != marker → 克隆并把 content 设为 marker。
   - 组回 `system+degraded_old+recent`；`estimate_messages_tokens<=budget`→Some 否则 None。
3. `compaction.rs::maybe_compact_send_view`：在 `estimated<=budget` 早退后、`summarize_history` 前插入 microcompact 分支（见 design.md 接线），成功则 return，设 `compacted=true` + 重置 thrash + emit `"microcompacted"`。
4. 单测（compaction.rs，纯函数，无需 model/host）：
   - `microcompact_reclaims_old_tool_results`：构造 old_segment 由大工具结果主导、budget 适中 → 返回 Some，且 `estimate_messages_tokens(view)<=budget`，且 recent 段逐字不变、old 的 tool content 变 marker。
   - `microcompact_returns_none_when_insufficient`：old 段以大 user/assistant 文本为主（降级工具结果也压不到 budget）→ None。
   - `microcompact_returns_none_when_no_old_segment`：全在近期窗口 → None。
   - `microcompact_preserves_tool_call_pairing`：old 末尾是 assistant(tool_calls) 时不把其 tool 结果拆到 recent（复用 select_recent_by_tokens 保护，断言 recent 首条非孤立 tool）。
5. 验证：`cargo check --lib` + `--tests`。可跑 `cargo test` 的机器上跑 `chat::agent::compaction`。

## 验收映射
- design"纯函数"→ 步骤 2 + 测试 1/2/3。
- design"只动 old_segment、护配对"→ 测试 1/4。
- design"跳过摘要"语义 → 测试 1（Some=足够）/2（None=落摘要）。

## 风险 / 回滚
- 单文件改动（compaction.rs）。回滚 `git checkout src-tauri/src/chat/agent/compaction.rs`。
- `cargo test` 本机受测试二进制 DLL 版本不匹配阻塞（0xc0000139），用 `cargo check --tests` + 纯函数单测覆盖，与父任务同。
