# Implementation Plan — 首个安全提取

## Step 1 — 建立模块

- [x] 在 `src-tauri/src/chat/mod.rs` 声明私有 `mcp_image_feedback` 模块。
- [x] 新建 `src-tauri/src/chat/mcp_image_feedback.rs`。

## Step 2 — 移动纯实现

- [x] 原样移动 `select_image_artifacts_for_attach`。
- [x] 原样移动 `append_tool_result_note`。
- [x] 原样移动 `image_extension_for_mime`。
- [x] 原样移动 `data_url_image_part`。
- [x] 使用 `pub(super)` 控制可见性。
- [x] 在 `commands.rs` 添加最小 import，并删除原定义。

## Step 3 — 下沉测试

- [x] 移动 MCP 图片辅助测试和 `image_artifact` fixture helper。
- [x] 清理 `commands.rs` 测试模块中只为这些测试存在的引用。
- [x] 保持测试数据和断言语义不变。

## Step 4 — 验证

- [x] `rustfmt --edition 2021 --check --config skip_children=true src-tauri/src/chat/mcp_image_feedback.rs`
- [x] 运行 7 个 MCP 图片辅助单元测试（名称过滤或逐项过滤）。
- [x] `cargo check --manifest-path src-tauri/Cargo.toml`
- [x] `git diff --check`
- [x] 审查 diff，确认 `mcp/registry.rs`、公开入口和业务编排无变化。

## Step 5 — 记录下一刀

- [x] 根据本次结果记录完整 MCP 图片编排迁移需要先建立的视觉服务边界，不在本提交中实施。
## Step 6 — 建立共享模型调用边界

- [x] 新建 `chat/model_call.rs`。
- [x] 迁移 `call_chat_completion_message` 与 provider dispatch。
- [x] 迁移会话模型和稳定错误 helper。
- [x] 调整 `commands.rs` import，不改变调用参数与请求形态。

## Step 7 — 提取完整视觉服务

- [x] 新建 `chat/vision.rs`。
- [x] 迁移辅助视觉选择、工具记录、分析提示词、图片 content part 与 MIME helper。
- [x] 迁移 `read_image_as_tool_result` 和 MCP artifact 编排实现。
- [x] 在 `commands.rs` 保留原签名薄代理。
- [x] 确认 `vision.rs`、`model_call.rs` 不 import `commands`。

## Step 8 — 下沉测试并完成最终验证

- [x] 将 3 个辅助视觉模型选择测试迁到 `vision.rs`。
- [x] 保留 commands message builder 集成测试在原模块。
- [x] 对三个新模块执行定向 rustfmt 检查。
- [x] 运行 `chat::vision::tests`（3/3）。
- [x] 运行 `chat::mcp_image_feedback::tests`（7/7）。
- [x] 运行 `chat::commands::tests::auxiliary_vision_result_becomes_text_for_main_chat_model`（1/1）。
- [x] 运行 `cargo check --manifest-path src-tauri/Cargo.toml`。
- [x] 运行 `git diff --check`。
- [x] 检查 MCP registry 兼容入口和新模块依赖方向。

## Step 9 — 文档与最终审查

- [x] 更新 `verification.md`。
- [x] 更新 `.trellis/spec/chat/mcp-image-feedback.md` 中的实现位置和模块边界。
- [x] 审查完整 diff，确认仅包含模块移动、最小 import/代理和任务文档。
- [x] 用户已确认继续拆分，已完成首轮 commit。

## Step 10 — 连续拆分：catalog / CRUD 模块（第二轮）

- [x] 新建 `src-tauri/src/chat/commands/catalog.rs`。
- [x] 迁移 conversation 查询/创建/导入、assistant CRUD/builder、project CRUD、set CRUD。
- [x] 迁移相关 prompt/context helper 与 `ExternalConversationMessage`。
- [x] `commands.rs` 仅保留后续编排所需的私有 import，以及 `create_assistant_via_builder` 兼容 re-export。
- [x] Tauri 注册改为 Rust 模块实际路径 `chat::commands::catalog::*`，前端 command 名保持不变。
- [x] 定向 rustfmt、`cargo check`、相关 6 个单元测试和 `git diff --check` 通过。
- [x] 提交本轮，并在提交后复跑验证。

## Step 11 — 后续模块边界

- [ ] 每轮只提取一个可独立验证的职责边界。
- [ ] 每轮执行：实现 → 定向检查/测试 → Git commit → 提交后复测。
- [ ] 继续拆分 context、session、attachments、reply/messages、tooling/agent host、conversation mutations 与 tests。
- [ ] 最终将 `commands.rs` 收敛为模块门面，并执行全范围验证。


## Step 12 ? Continuous split: context / compaction module (round 3)

- [x] Add `src-tauri/src/chat/commands/context.rs`.
- [x] Move context stats/compression commands, summary helpers, token/image estimation, usage-anchor handling, context-state computation, auto-compaction/rollback helpers, event emitters, and API-message replay construction.
- [x] Keep parent-module access narrow with `pub(super)`; retain `group_answer_excluded_from_context` as `pub(crate)` for compaction reuse.
- [x] Update Tauri registration paths for `chat_get_context_stats` and `chat_compress_context` without changing IPC command names.
- [x] Update compaction call sites/comments and `.trellis/spec/chat/compaction-contracts.md` implementation paths.
- [x] Fix the recompression test fixture so its old segment actually exceeds `RECENT_KEEP_TOKENS`; production behavior is unchanged.
- [x] Verify formatting, compilation, the full `chat::commands::tests` suite, and `chat::agent::compaction::tests`.
- [x] Commit round 3 and rerun post-commit verification before starting round 4.


## Step 13 ? Continuous split: session interaction module (round 4)

- [x] Add `src-tauri/src/chat/commands/interaction.rs`.
- [x] Move external-send intake, plan-mode controls, cancellation/approval/consent/user-choice bridges, background command controls, Python completion, and request-debug commands.
- [x] Move session prompt waiters and chat stream/tool/plan event emitters behind narrow visibility.
- [x] Preserve `chat::commands::{emit_chat_stream_delta, emit_chat_stream_done, emit_chat_tool_record}` through crate-visible re-exports for `external_agents`.
- [x] Move 12 Tauri registration paths to `chat::commands::interaction::*` without changing IPC command names.
- [x] Keep plan-approval and tool-approval-summary tests in the parent test module through test-only imports.
- [x] Verify formatting, compilation, the full `chat::commands::tests` suite, Tauri command parity, and diff cleanliness.
- [x] Commit round 4 and rerun post-commit verification before starting round 5.


## Step 14 ? Continuous split: attachment commands (round 5)

- [x] Add `src-tauri/src/chat/commands/attachments.rs`.
- [x] Move attachment preview/open commands, generated-artifact open/reveal commands, pasted file/image persistence commands, and clipboard file intake.
- [x] Remove command-only attachment and shell imports from the parent while retaining reply-orchestration attachment helpers there.
- [x] Move 7 Tauri registration paths to `chat::commands::attachments::*` without changing IPC command names.
- [x] Verify exact formatted extraction against `1ea93a4`, formatting, compilation, full commands tests, Tauri command parity, and diff cleanliness.
- [x] Commit round 5 and rerun post-commit verification before starting round 6.

## Step 15 - Continuous split: title generation (round 6)

- [x] Add `src-tauri/src/chat/commands/title.rs`.
- [x] Move conversation-title resolution, model-backed title generation, title prompts, sanitization, and fallback generation.
- [x] Keep only narrow parent imports, plus test-only imports for the existing parent-module title tests.
- [x] Confirm this round moves no Tauri command and requires no registration-path change.
- [x] Compare the formatted extraction exactly against the functions from `372c09b`; no business logic changed.
- [x] Verify formatting, compilation, the full `chat::commands::tests` suite, and diff cleanliness.
- [x] Commit round 6 and rerun post-commit verification before starting round 7.

## Step 16 - Continuous split: chat tooling and filters (round 7)

- [x] Add `src-tauri/src/chat/commands/tooling.rs`.
- [x] Move slash-skill trigger/pinning resolution, chat tool listing, agent state tool injection, plan-mode filtering, and inline-code write filtering.
- [x] Preserve connector and assistant allow-list gates by keeping the existing `agent_prepare::skill_allowed_for_conversation` calls unchanged.
- [x] Keep parent-module tests intact through narrow production and test-only imports.
- [x] Confirm this round moves no Tauri command and requires no registration-path change.
- [x] Compare the formatted extraction exactly against the functions from `4bee8b1`; no business logic changed.
- [x] Verify formatting, compilation, the full `chat::commands::tests` suite, and diff cleanliness.
- [x] Commit round 7 and rerun post-commit verification before starting round 8.

## Step 17 - Continuous split: assistant message lifecycle (round 8)

- [x] Add `src-tauri/src/chat/commands/messages.rs`.
- [x] Move assistant message construction, orphan tool reconciliation, segment normalization/synthesis, partial snapshot persistence, final push/upsert, edit replay rebuilding, and agent todo/plan state capture.
- [x] Preserve the crate-visible `chat::commands::push_assistant_message` compatibility path through a re-export for `external_agents`.
- [x] Update `catalog.rs` to import orphan-tool reconciliation from the owning `messages` module.
- [x] Keep existing parent-module tests intact through narrow test-only imports.
- [x] Confirm this round moves no Tauri command and requires no registration-path change.
- [x] Compare the formatted extraction exactly against the functions from `ce718cd`; no business logic changed.
- [x] Verify formatting, compilation, the full `chat::commands::tests` suite, and diff cleanliness.
- [x] Commit round 8 and rerun post-commit verification before starting round 9.


## Step 18 - Continuous split: conversation mutations (round 9)

- [x] Add `src-tauri/src/chat/commands/mutations.rs`.
- [x] Move 7 Tauri commands for message update/regeneration/deletion, conversation fork/deletion/update, and group selection.
- [x] Move the 4 owning helpers for message lookup, regeneration truncation, fork-message construction, and forked-file copying.
- [x] Update the 7 `src-tauri/src/lib.rs` registration paths without changing IPC command names.
- [x] Keep existing parent-module tests intact through narrow test-only imports.
- [x] Compare the formatted extraction exactly against the functions from `9dcad89`; no business logic changed.
- [x] Verify formatting, compilation, the full `chat::commands::tests` suite, Tauri command parity, and diff cleanliness.
- [x] Commit round 9 and rerun post-commit verification before starting round 10.


## Step 19 - Continuous split: agent runtime adapters (round 10)

- [x] Add `src-tauri/src/chat/commands/agent_host.rs`.
- [x] Move `ChatAgentHost`, debug-only `ProbeAgentHost`, and `RegistryToolExecutor` with their trait implementations.
- [x] Keep debug probe orchestration in the parent for a later dedicated boundary.
- [x] Expose the adapter structs and fields only to the parent module so existing construction semantics remain unchanged.
- [x] Remove adapter-only imports from `commands.rs` and keep test-only types behind `#[cfg(test)]`.
- [x] Confirm this round moves no Tauri command and requires no registration-path change.
- [x] Compare the formatted extraction exactly against the adapter blocks from `bb982ca`; no business logic changed.
- [x] Verify formatting, compilation, the full `chat::commands::tests` suite, and diff cleanliness.
- [ ] Commit round 10 and rerun post-commit verification before starting round 11.
