# Verification

## 执行结果

- `rustfmt --edition 2021 --check --config skip_children=true src-tauri/src/chat/mcp_image_feedback.rs src-tauri/src/chat/model_call.rs src-tauri/src/chat/vision.rs`：通过。
- `./scripts/win-cargo-test.ps1 --lib chat::vision::tests`：3/3 通过。
- `./scripts/win-cargo-test.ps1 --lib chat::mcp_image_feedback::tests`：7/7 通过。
- `./scripts/win-cargo-test.ps1 --lib chat::commands::tests::auxiliary_vision_result_becomes_text_for_main_chat_model`：1/1 通过。
- `cargo check --manifest-path src-tauri/Cargo.toml`：通过。
- `git diff --check`：通过。

## 变更边界审查

- `mcp/native_registry.rs` 未修改，仍调用 `crate::chat::commands::read_image_as_tool_result`。
- `mcp/registry.rs` 未修改，仍调用 `crate::chat::commands::attach_image_artifacts_for_model`。
- `commands.rs` 保留上述两个 `pub(crate)` 薄代理，实际实现迁入 `chat/vision.rs`。
- `vision.rs` 与 `model_call.rs` 均未引用 `commands`，不存在反向依赖。
- 最终依赖边界为：`commands -> model_call`、`commands -> vision`、`vision -> model_call/storage/mcp_image_feedback/model_metadata`。
- `commands.rs` 从 9,320 行降至 8,458 行；新增 `mcp_image_feedback.rs` 232 行、`model_call.rs` 94 行、`vision.rs` 672 行。
- 未修改视觉策略、图片护栏、提示词、OCR 降级、临时文件生命周期、provider request shape、MCP JSON、follow-up message 顺序或 Tauri 注册。

## 已知基线项

`cargo check` 仍报告既有警告：

- `src-tauri/src/shortcuts.rs` 未使用参数
- `src-tauri/src/capture_geometry.rs` dead code
- `src-tauri/src/plugins/install.rs` dead code

本任务未混入修复。

仓库当前整体并非由本机 rustfmt 版本完全格式化；因此仅对三个新模块执行定向 rustfmt，避免全仓无关格式 churn。

## 第二轮：catalog / CRUD 提取（提交前）

- `rustfmt --edition 2021 --check --config skip_children=true src-tauri/src/chat/commands/catalog.rs`：通过。
- `cargo check --manifest-path src-tauri/Cargo.toml`：通过；仅保留既有基线警告。
- `./scripts/win-cargo-test.ps1 --lib builder_args`：2/2 通过。
- `./scripts/win-cargo-test.ps1 --lib reconcile_orphan_tool_segments`：3/3 通过。
- `./scripts/win-cargo-test.ps1 --lib strip_transcripts_for_frontend`：1/1 通过。
- `git diff --check`：通过。
- Tauri command 集合迁移前后均为 46 个，无缺失、无新增、无重复；本轮 20 个 command 仅调整 Rust 注册路径，IPC 名称未变化。
- `commands.rs` 从 8,458 行降至 7,629 行；新增 `commands/catalog.rs` 868 行。
- 本轮是行为中立的模块移动，不新增可复用业务契约，因此 `.trellis/spec/` 无需新增行为规范。


## Round 3: context / compaction extraction (pre-commit)

- `src-tauri/src/chat/commands/context.rs`: 1,037 lines extracted.
- `src-tauri/src/chat/commands.rs`: 7,629 -> 6,642 lines.
- Tauri chat command basenames are unchanged compared with `b789f5a`; no missing, added, or duplicate command registrations.
- `rustfmt --edition 2021 --check --config skip_children=true src-tauri/src/chat/commands/context.rs`: passed.
- `cargo check --manifest-path src-tauri/Cargo.toml`: passed with only the existing baseline warnings.
- `./scripts/win-cargo-test.ps1 --lib chat::commands::tests`: 72/72 passed.
- `./scripts/win-cargo-test.ps1 --lib chat::agent::compaction::tests`: 59/59 passed.
- `git diff --check`: passed.
- The recompression test now appends enough content to exceed `RECENT_KEEP_TOKENS`; the previous short fixture did not satisfy the behavior's prerequisite.
- Compaction implementation references now point to `commands/context.rs` where appropriate, while L2 summary writeback references remain on `commands.rs`.


## Round 3: post-commit verification

- Commit: `75c9407 refactor(chat): extract context commands`.
- Post-commit `cargo check`: passed with only existing baseline warnings.
- Post-commit `chat::commands::tests`: 72/72 passed.
- Post-commit `chat::agent::compaction::tests`: 59/59 passed.
- Working tree was clean before round 4 began.

## Round 4: session interaction extraction (pre-commit)

- `src-tauri/src/chat/commands/interaction.rs`: 637 lines extracted.
- `src-tauri/src/chat/commands.rs`: 6,642 -> 6,032 lines.
- Moved 12 Tauri command registrations; chat command basenames remain 50/50 with no missing, added, or duplicate entries compared with `75c9407`.
- Preserved the crate-visible parent paths for stream/tool event emitters used by `external_agents::run` through re-exports.
- `rustfmt --edition 2021 --check --config skip_children=true src-tauri/src/chat/commands/interaction.rs`: passed.
- `cargo check --manifest-path src-tauri/Cargo.toml`: passed with only existing baseline warnings.
- `./scripts/win-cargo-test.ps1 --lib chat::commands::tests`: 72/72 passed.
- `git diff --check`: passed.
- This round is a behavior-neutral extraction; no new reusable runtime contract required a `.trellis/spec/` update.


## Round 4: post-commit verification

- Commit: `1ea93a4 refactor(chat): extract interaction commands`.
- Post-commit `cargo check`: passed with only existing baseline warnings.
- Post-commit `chat::commands::tests`: 72/72 passed.
- Working tree was clean before round 5 began.

## Round 5: attachment command extraction (pre-commit)

- `src-tauri/src/chat/commands/attachments.rs`: 136 lines extracted.
- `src-tauri/src/chat/commands.rs`: 6,032 -> 5,903 lines.
- Moved 7 Tauri command registrations; chat command basenames remain 50/50 with no missing, added, or duplicate entries compared with `1ea93a4`.
- The formatted new module exactly matches the functions extracted from `1ea93a4`; only module imports and registration paths changed.
- `rustfmt --edition 2021 --check --config skip_children=true src-tauri/src/chat/commands/attachments.rs`: passed.
- `cargo check --manifest-path src-tauri/Cargo.toml`: passed with only existing baseline warnings.
- `./scripts/win-cargo-test.ps1 --lib chat::commands::tests`: 72/72 passed.
- `git diff --check`: passed.
- This round is a behavior-neutral extraction; no `.trellis/spec/` behavior contract changed.

## Round 5: post-commit verification

- Commit: `372c09b refactor(chat): extract attachment commands`.
- Post-commit `cargo check`: passed with only existing baseline warnings.
- Post-commit `chat::commands::tests`: 72/72 passed.
- Working tree was clean before round 6 began.

## Round 6: title generation extraction (pre-commit)

- `src-tauri/src/chat/commands/title.rs`: 158 lines extracted.
- `src-tauri/src/chat/commands.rs`: 5,903 -> 5,765 lines.
- Moved title resolution, model-backed generation, prompt construction, output sanitization, and deterministic fallback generation.
- This round contains no Tauri command, so command registration paths and IPC names are unchanged.
- The formatted new module exactly matches the title functions extracted from `372c09b`; only visibility, imports, and module formatting changed.
- `rustfmt --edition 2021 --check --config skip_children=true src-tauri/src/chat/commands/title.rs`: passed.
- `cargo check --manifest-path src-tauri/Cargo.toml`: passed with only existing baseline warnings.
- `./scripts/win-cargo-test.ps1 --lib chat::commands::tests`: 72/72 passed, including all four title-specific tests.
- `git diff --check`: passed.
- This round is a behavior-neutral extraction; no `.trellis/spec/` behavior contract changed.

## Round 6: post-commit verification

- Commit: `4bee8b1 refactor(chat): extract title generation`.
- Post-commit `cargo check`: passed with only existing baseline warnings.
- Post-commit `chat::commands::tests`: 72/72 passed.
- Working tree was clean before round 7 began.

## Round 7: chat tooling and filters extraction (pre-commit)

- `src-tauri/src/chat/commands/tooling.rs`: 242 lines extracted.
- `src-tauri/src/chat/commands.rs`: 5,765 -> 5,535 lines.
- Moved 11 functions covering slash-skill trigger/pinning resolution, tool listing and injection, plan-mode filtering, and inline-code write filtering.
- Connector prerequisites, assistant allow-list checks, skill argument substitution, image-tool injection, and read-only classification remain byte-for-byte equivalent after formatting.
- This round contains no Tauri command, so command registration paths and IPC names are unchanged.
- The formatted new module exactly matches the tooling functions extracted from `4bee8b1`; only visibility, imports, and module formatting changed.
- `rustfmt --edition 2021 --check --config skip_children=true src-tauri/src/chat/commands/tooling.rs`: passed.
- `cargo check --manifest-path src-tauri/Cargo.toml`: passed with only existing baseline warnings.
- `./scripts/win-cargo-test.ps1 --lib chat::commands::tests`: 72/72 passed, including slash-trigger, plan-filter, and inline-code-filter tests.
- `git diff --check`: passed.
- This round is a behavior-neutral extraction; existing skill runtime and connector-gating contracts remain unchanged, so no `.trellis/spec/` update is required.

## Round 7: post-commit verification

- Commit: `ce718cd refactor(chat): extract tool filters`.
- Post-commit `cargo check`: passed with only existing baseline warnings.
- Post-commit `chat::commands::tests`: 72/72 passed.
- Working tree was clean before round 8 began.

## Round 8: assistant message lifecycle extraction (pre-commit)

- `src-tauri/src/chat/commands/messages.rs`: 809 lines extracted across 23 functions.
- `src-tauri/src/chat/commands.rs`: 5,535 -> 4,760 lines.
- Moved assistant message construction, orphan tool reconciliation, segment normalization/synthesis, partial snapshots, final persistence, edit replay rebuilding, and agent todo/plan state capture.
- Preserved `chat::commands::push_assistant_message` through a crate-visible re-export for `external_agents::run`.
- Updated `catalog.rs` to depend directly on `commands::messages::reconcile_orphan_tool_segments`; behavior and visibility remain unchanged.
- This round contains no Tauri command, so command registration paths and IPC names are unchanged.
- The formatted new module exactly matches the message-lifecycle block extracted from `ce718cd`; only visibility, imports, module formatting, and the owning import path changed.
- `rustfmt --edition 2021 --check --config skip_children=true src-tauri/src/chat/commands/messages.rs`: passed.
- `cargo check --manifest-path src-tauri/Cargo.toml`: passed with only existing baseline warnings.
- `./scripts/win-cargo-test.ps1 --lib chat::commands::tests`: 72/72 passed, including message construction, segment normalization, edit replay, and orphan reconciliation tests.
- `git diff --check`: passed.
- This round is a behavior-neutral extraction; no reusable runtime contract changed, so no `.trellis/spec/` update is required.


## Round 8: post-commit verification

- Commit: `9dcad89 refactor(chat): extract message lifecycle`.
- Post-commit `cargo check`: passed with only existing baseline warnings.
- Post-commit `chat::commands::tests`: 72/72 passed.
- Working tree was clean before round 9 began.

## Round 9: conversation mutations extraction (pre-commit)

- `src-tauri/src/chat/commands/mutations.rs`: 626 lines extracted across 7 Tauri commands and 4 owning helpers.
- `src-tauri/src/chat/commands.rs`: 4,760 -> 4,162 lines.
- Moved message update/regeneration/deletion, conversation fork/deletion/update, group selection, and their mutation-specific helpers.
- Updated 7 Tauri registration paths to `chat::commands::mutations::*`; the command basename set remains exactly 50/50 with no missing, added, or duplicate registrations.
- Existing parent tests retain access to `apply_regenerate_truncation` and `build_fork_messages` through test-only imports.
- The formatted new module exactly matches the mutation block extracted from `9dcad89`; only imports, the two test-helper visibilities, and module formatting changed.
- `rustfmt --edition 2021 --check --config skip_children=true src-tauri/src/chat/commands/mutations.rs`: passed.
- `cargo check --manifest-path src-tauri/Cargo.toml`: passed with only existing baseline warnings.
- `./scripts/win-cargo-test.ps1 --lib chat::commands::tests`: 72/72 passed, including regeneration, fork, and group-selection coverage.
- `git diff --check`: passed.
- This round is a behavior-neutral extraction; the existing conversation-fork contract remains unchanged, so no `.trellis/spec/` update is required.


## Round 9: post-commit verification

- Commit: `bb982ca refactor(chat): extract conversation mutations`.
- Post-commit `cargo check`: passed with only existing baseline warnings.
- Post-commit `chat::commands::tests`: 72/72 passed.
- Working tree was clean before round 10 began.

## Round 10: agent runtime adapters extraction (pre-commit)

- `src-tauri/src/chat/commands/agent_host.rs`: 285 lines extracted.
- `src-tauri/src/chat/commands.rs`: 4,162 -> 3,888 lines.
- Moved `ChatAgentHost`, debug-only `ProbeAgentHost`, `RegistryToolExecutor`, and all three trait implementations; `run_chat_probe` remains in the parent for a later orchestration boundary.
- Adapter visibility is limited to the parent module, preserving existing struct construction and runtime behavior.
- This round contains no Tauri command, so registration paths and IPC names are unchanged.
- The formatted new module exactly matches the adapter blocks extracted from `bb982ca`; only imports, parent-only visibility, and module formatting changed.
- `rustfmt --edition 2021 --check --config skip_children=true src-tauri/src/chat/commands/agent_host.rs`: passed.
- `cargo check --manifest-path src-tauri/Cargo.toml`: passed with only existing baseline warnings.
- `./scripts/win-cargo-test.ps1 --lib chat::commands::tests`: 72/72 passed.
- `git diff --check`: passed.
- This round is a behavior-neutral extraction; no reusable runtime contract changed, so no `.trellis/spec/` update is required.


## Round 10: post-commit verification

- Commit: `7381884 refactor(chat): extract agent runtime adapters`.
- Post-commit `cargo check`: passed with only existing baseline warnings.
- Post-commit `chat::commands::tests`: 72/72 passed.
- Working tree was clean before round 11 began.

## Round 11: model-input sanitization extraction (pre-commit)

- `src-tauri/src/chat/commands/sanitization.rs`: 86 lines extracted across 5 functions.
- `src-tauri/src/chat/commands.rs`: 3,888 -> 3,805 lines.
- Moved API-message/content sanitization, image data-URL stripping, raw image-base64 detection, and replacement text generation.
- Updated `context.rs` to import `sanitize_api_message_for_model` and `sanitize_image_payloads_for_model` directly from the owning module.
- Existing parent tests retain sanitizer access through a test-only import.
- This round contains no Tauri command, so registration paths and IPC names are unchanged.
- The formatted new module exactly matches the sanitization block extracted from `7381884`; only imports, sibling visibility, and module formatting changed.
- `rustfmt --edition 2021 --check --config skip_children=true src-tauri/src/chat/commands/sanitization.rs`: passed.
- `cargo check --manifest-path src-tauri/Cargo.toml`: passed with only existing baseline warnings.
- `./scripts/win-cargo-test.ps1 --lib chat::commands::tests`: 72/72 passed, including both image-payload sanitizer tests.
- `git diff --check`: passed.
- This round is a behavior-neutral extraction; no reusable runtime contract changed, so no `.trellis/spec/` update is required.


## Round 11: post-commit verification

- Commit: `0101271 refactor(chat): extract model input sanitization`.
- Post-commit `cargo check`: passed with only existing baseline warnings.
- Post-commit `chat::commands::tests`: 72/72 passed.
- Working tree was clean before round 12 began.

## Round 12: debug probe runtime extraction (pre-commit)

- `src-tauri/src/chat/commands/probe_runtime.rs`: 141 lines extracted.
- `src-tauri/src/chat/commands.rs`: 3,805 -> 3,682 lines.
- Moved the debug-only `run_chat_probe` orchestration and preserved `chat::commands::run_chat_probe` through a debug-only compatibility re-export.
- Moved the complete probe documentation from its accidental attachment in `agent_host.rs` back to the owning probe function.
- Removed probe-only storage and catalog imports from the parent module.
- This round contains no Tauri command, so registration paths and IPC names are unchanged.
- The formatted extraction exactly matches the original probe block from `bb982ca`: expected and actual SHA-256 are both `7748d69c3b7d729f20648be7f267c61f7529fe2379aa718fb0357ee8584575cb`.
- `rustfmt --edition 2021 --check --config skip_children=true src-tauri/src/chat/commands/agent_host.rs src-tauri/src/chat/commands/probe_runtime.rs`: passed.
- `cargo check --manifest-path src-tauri/Cargo.toml`: passed with only existing baseline warnings.
- `./scripts/win-cargo-test.ps1 --lib chat::commands::tests`: 72/72 passed.
- `git diff --check`: passed.
- This round is a behavior-neutral extraction; no reusable runtime contract changed, so no `.trellis/spec/` update is required.


## Round 12: post-commit verification

- Commit: `df7a3bb refactor(chat): extract debug probe runtime`.
- Post-commit `cargo check`: passed with only existing baseline warnings.
- Post-commit `chat::commands::tests`: 72/72 passed.
- Working tree was clean before round 13 began.

## Round 13: direct image generation extraction (pre-commit)

- `src-tauri/src/chat/commands/direct_image.rs`: 161 lines extracted.
- `src-tauri/src/chat/commands.rs`: 3,682 -> 3,535 lines.
- Moved direct image generation orchestration, the pending marker constant, artifact Markdown rendering, and prompt selection/truncation.
- Kept `agent_run_entry_label` in the parent because normal reply completion and multi-model fan-out still use it; the new module accesses it as a descendant-only parent helper.
- Removed direct-image-only `ChatToolArtifact` and production `ModelProvider` imports from the parent while retaining `ModelProvider` for existing tests.
- This round contains no Tauri command, so registration paths and IPC names are unchanged.
- All three moved function bodies exactly match `df7a3bb` after normalizing the entry function's `pub(super)` visibility. SHA-256: `25a616a8d4dfe41f846ea3ece83c90877ce6afa297c278cabb2402b83bc417a8`, `548357fb1027b1f1b3247136b052e7e80e628a23decfaf9c1f6c6cce425faf14`, and `dc2ebb9b2de1131621c6ea359ede2c5bfd7b1b5e6c0e7d4deb6f3fcd7ea8f037`.
- `rustfmt --edition 2021 --check --config skip_children=true src-tauri/src/chat/commands/direct_image.rs`: passed.
- `cargo check --manifest-path src-tauri/Cargo.toml`: passed with only existing baseline warnings.
- `./scripts/win-cargo-test.ps1 --lib chat::commands::tests`: 72/72 passed, including the shared run-entry label coverage.
- `git diff --check`: passed.
- This round is a behavior-neutral extraction; no provider/image-generation contract changed, so no `.trellis/spec/` update is required.


## Round 13: post-commit verification

- Commit: `44ec5ce refactor(chat): extract direct image generation`.
- Post-commit `cargo check`: passed with only existing baseline warnings.
- Post-commit `chat::commands::tests`: 72/72 passed.
- Working tree was clean before round 14 began.

## Round 14: reply runtime primitives extraction (pre-commit)

- `src-tauri/src/chat/commands/reply_runtime.rs`: 134 lines extracted.
- `src-tauri/src/chat/commands.rs`: 3,535 -> 3,417 lines.
- Moved the reply-model cap and resolver, TOCTOU-safe send reservation, per-run reply/generation RAII guard, reply-arm configuration, and arm outcome type.
- Parent-level aliases preserve existing callers in `commands.rs` and `mutations.rs`; only the owning module receives `pub(super)` visibility needed by those callers.
- The existing `resolve_reply_arms_dedups_filters_and_caps` test remains in the parent and continues to cover filtering, stable deduplication, provider validation, and the four-model cap.
- This round contains no Tauri command, so registration paths and IPC names are unchanged.
- Formatting the block extracted from `44ec5ce` with the same imports and visibility adjustments exactly matches the new module; expected and actual SHA-256 are both `3172cb0b9c674dc0364b813585a719b7eb94cf21426e86270427718be7d2101b`.
- `rustfmt --edition 2021 --check --config skip_children=true src-tauri/src/chat/commands/reply_runtime.rs`: passed.
- `cargo check --manifest-path src-tauri/Cargo.toml`: passed with only existing baseline warnings.
- `./scripts/win-cargo-test.ps1 --lib chat::commands::tests`: 72/72 passed.
- `git diff --check`: passed.
- This round is a behavior-neutral extraction; reply-slot, generation, and fan-out contracts are unchanged, so no `.trellis/spec/` update is required.
