# PRD — 优化上下文压缩（compaction-optimize）

## Goal
对照 Codex / Claude Code 的压缩实现，补齐 Kivio 聊天上下文压缩的两个低风险高性价比缺口：**摘要不再被输出上限截断（R-3）**、**多次链式重摘时给用户衰减告警（R-4）**。

## Scope（已锁定）
- 本任务：**R-3 + R-4**。
- R-1（Microcompact 增量降级）已拆到 `07-02-compaction-microcompact`，本期不做。
- #2 前缀缓存保留：out of scope，另行评估。

## Background（调研对照，confirmed）
- Kivio 摘要用 Claude Code 9 段 prompt（先 `<analysis>` 后 `<summary>`）；链式锚点合并；流式调用；质量兜底（200字/劣化<30%/截断拒绝，上一轮已加）；L2 thrash 熔断 `COMPACTION_THRASH_LIMIT=2`。
- Codex 多次压缩后发 WarningEvent 提示准确性下降、建议开新线程（Kivio 缺）。
- Claude Code 9 段 prompt 输出量大，Kivio 的 4096 输出上限与之矛盾。

## Confirmed facts（代码证据）
- `SUMMARY_OUTPUT_TOKENS=4096`；`summary_output_tokens(m)=m.min(4096)`（compaction.rs:31,479-480）。
- L2 `maybe_compact_send_view` 调 `summarize_history` 时传的是 **run 的 `config.max_output_tokens`**（compaction.rs:904），可能小于模型真实上限；手动/自动持久化路径 `compact_conversation` 传 `chat_max_output_tokens_for_model(...)`（真实上限）。→ 两路输出上限口径不一致。
- 9 段 prompt 先吐 `<analysis>` 再 `<summary>`；真实大旧段时二者合计易超 4096 → 截断（上一轮 Truncated 兜底正是防此，但根因是上限太小）。
- `context_state.warning: Option<String>`（types.rs:81 / 前端 types.ts:372）；现有告警都是**后端设的原始英文串**（commands.rs:1332 等）。→ 沿用此惯例，不引入 i18n 管线。
- 压缩成功后两路都把 `warning=None`：`compact_conversation`（compaction.rs:1151）、L2 写回块（commands.rs，随 `compaction_summary` 落盘）。→ R-4 在这两处改为条件告警。
- `compression_count` 在压缩成功时 `saturating_add(1)`，前端 ContextIndicator 已展示（ContextIndicator.tsx:156-158）。

## Requirements & 锁定决策
- **R-3 摘要输出上限**：
  - `SUMMARY_OUTPUT_TOKENS` 4096 → **8192**（容纳 analysis+summary；仍远小于窗口，安全）。
  - L2 调用点改传 `chat_max_output_tokens_for_model(Some(&config.provider), &config.model, config.max_output_tokens)`，与持久化路径口径统一。
  - `summary_output_tokens` 保持 `min()` 语义：真实上限 <8192 的模型不受影响，≥8192 的可用满 8192。
- **R-4 链式衰减告警**：
  - 新增纯函数 `decay_warning_for(compression_count) -> Option<String>`：`count >= DECAY_WARNING_COMPRESSION_COUNT`（=**3**）时返回英文告警串，否则 `None`。
  - 两处压缩成功后的 `warning = None` 改为 `warning = decay_warning_for(count)`（count 取本次 bump 后的值）。
  - 告警文案（英文，匹配现有惯例）：`"This conversation has been compressed N times; repeated compression can reduce accuracy. Consider starting a new conversation."`（N 用实际次数）。

## Out of scope
- R-1 Microcompact（拆子任务）、#2 前缀缓存、改动 9 段 prompt 本身、i18n 双语告警管线。

## Acceptance criteria
- [ ] R-3：`SUMMARY_OUTPUT_TOKENS == 8192`；`summary_output_tokens(20000) == 8192`、`summary_output_tokens(4096) == 4096`（单测）。
- [ ] R-3：L2 与持久化两路的 max-output 参数来源一致（读代码确认，均经 `chat_max_output_tokens_for_model`）。
- [ ] R-4：`decay_warning_for(2)==None`、`decay_warning_for(3)==Some(含 "3")`、`decay_warning_for(5)==Some(含 "5")`（单测，覆盖阈值边界）。
- [ ] R-4：两条压缩成功路径（`compact_conversation` + L2 写回）都调 `decay_warning_for`，未达阈值仍为 `None`（不误报）。
- [ ] `cargo check --lib` 与 `--tests` 通过（本机 `cargo test` 受 onnxruntime.dll 缺失阻塞，逻辑以单测+推演覆盖）。

## Notes
- 本机 `cargo test` 无法启动测试二进制：`STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139)`——某静态导入 DLL 版本不匹配/缺导出符号（非缺文件，那会是 0xc0000135）。把 ort 的 onnxruntime.dll、WebView2Loader.dll 加到 PATH 均无效；dev app 本身能跑，是测试二进制的链接环境问题。用 `cargo check --lib` + `--tests` 保证编译与类型正确，新增逻辑为纯函数、断言以代码审阅覆盖。
