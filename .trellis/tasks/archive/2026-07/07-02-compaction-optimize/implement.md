# implement.md — 优化上下文压缩（R-3 + R-4）

## 执行清单（按序）

### R-3 摘要输出上限
1. `compaction.rs:31` — `SUMMARY_OUTPUT_TOKENS: u32 = 4_096;` → `8_192;`，更新注释（不再"对齐 OpenCode 4096"，改为"容纳 9 段 analysis+summary"）。
2. `compaction.rs:897-909`（`maybe_compact_send_view` 调 `summarize_history`）— 第 904 行 `config.max_output_tokens` 改为
   `chat_max_output_tokens_for_model(Some(&config.provider), &config.model, config.max_output_tokens)`（`chat_max_output_tokens_for_model` 已 import）。
3. 单测：`summary_output_tokens(20_000)==8192`、`summary_output_tokens(4_096)==4096`、`summary_output_tokens(200_000)==8192`。

### R-4 链式衰减告警
4. `compaction.rs` — 新增常量 `const DECAY_WARNING_COMPRESSION_COUNT: u32 = 3;` + 纯函数
   `fn decay_warning_for(compression_count: u32) -> Option<String>`（`>=` 阈值返回含次数的英文串，否则 None）。
5. `compaction.rs:1151`（`compact_conversation` 内，`compression_count` bump 之后）— `warning = None` →
   `warning = decay_warning_for(conversation.context_state.compression_count)`。
6. `commands.rs` L2 写回块（随 `result.compaction_summary` 落盘处，`compression_count` bump 之后的 `warning = None`）— 同样改为 `decay_warning_for(...)`。用 `crate::chat::agent::compaction::decay_warning_for`（需 `pub(crate)`）。
7. 单测（compaction.rs）：`decay_warning_for(2)==None`、`decay_warning_for(3)` 是 Some 且含 `"3"`、`decay_warning_for(5)` 含 `"5"`。

### 验证
8. `cargo check --manifest-path src-tauri/Cargo.toml --lib` 通过。
9. `cargo check --manifest-path src-tauri/Cargo.toml --tests` 通过（新单测编译）。
10. 读代码确认两条压缩路径 max-output 与 warning 口径一致（acceptance R-3/R-4）。

## 风险 / 回滚点
- 改动集中在 `compaction.rs`（3 处）+ `commands.rs`（1 处）。全部为局部改动，无结构调整。
- `decay_warning_for` 设 `pub(crate)` 供 commands.rs 复用——唯一跨模块暴露面。
- 回滚：`git checkout src-tauri/src/chat/agent/compaction.rs src-tauri/src/chat/commands.rs`。

## 备注
- 本机 `cargo test` 受 onnxruntime.dll 缺失阻塞；断言靠纯函数单测 + `cargo check --tests` 保证编译。
