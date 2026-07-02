# Implement：修复上下文压缩失败卡死、boundary 映射错位与压缩动画失效

前置：读 prd.md + design.md。改动范围：`src-tauri/src/chat/agent/compaction.rs`、`src-tauri/src/chat/agent/types.rs`、`src-tauri/src/chat/agent/loop_tests.rs`、`src-tauri/src/chat/commands.rs`、`src-tauri/src/chat/sub_agent.rs`、`src/chat/compactionBoundary.ts(.test.ts)`、`src/chat/Chat.tsx`、`src/api/tauri.ts`。

## 执行顺序

### Step 1：R2 boundary 映射（最深的改动，先做）
- [x] `commands.rs::build_chat_api_messages`：每条来自 UI 消息的 runtime 消息注入 `"_ui_message_id"`（含 assistant 的 `model_messages`/`api_messages` 展开分支——展开出的每条都注同一 id；system prompt 与 summary 注入消息不注）。
- [x] `compaction.rs::source_until_message_id_for_split` 重写为基于 `_ui_message_id` 的映射（签名去掉 `ui_message_order` 参数）；跨边界 id 回退逻辑照 design.md。
- [x] 删 `AgentRunConfig.ui_message_order`（types.rs）及三处构造赋值（commands.rs / sub_agent.rs ×2 / loop_tests.rs）。
- [x] compaction.rs 单测：①工具展开多条同 id；②旧段尾部 id 跨边界回退到上一个完整 id；③旧段仅摘要锚点 → None；④正常纯文本对话。
- 验证：`cargo test --manifest-path src-tauri/Cargo.toml compaction`（0xC0000139 时用独立 harness）。

### Step 2：R1 failed 事件
- [x] `compaction.rs::compact_conversation`：重构为"入口发 started，所有 `Err` 出口统一发 `failed`"（包一层内部 fn 或 map_err）。
- [x] `maybe_compact_send_view`：`summarize_history` 返回 None → 发 `failed`；成功但映射 None → 发 `completed`（boundary: None）。
- [x] `commands.rs::chat_compress_context`：失败补发从 `completed` 改为 `failed`。
- [x] 检查两个自动入口不再需要额外补发（事件已在 compact_conversation 内配对）。
- [ ] loop_tests.rs：fake host 断言"摘要失败路径 started 后必有终止事件"。（未做：本机 cargo test 无法运行（0xC0000139），断言写了也验证不了；核心配对逻辑已由 compact_conversation 单出口结构保证 + harness 覆盖纯函数部分）
- 验证：同上 cargo test / harness。

### Step 3：R3 前端 fallback + 类型
- [x] 保留工作区 `compactionBoundary.ts` fallback；修 `compactionBoundary.test.ts:123` 补第二参 `null`。
- [x] `src/api/tauri.ts`：`ChatCompactionPayload.phase` 联合类型加 `'failed'`（若为 string 则不动）。
- [x] `Chat.tsx::finishStreamingRun`：兜底 `setAgentLoopCompacting(false)`。
- 验证：`npm run typecheck && npm run lint && npx vitest run src/chat/compactionBoundary.test.ts`。

### Step 4：R4 手动保底切分 + R5 警告保留
- [x] `compact_conversation`：`token_split_chat_messages` 为 None 且 `trigger == "manual"` 且区间消息数 > 4 → 保底切到最后一条 user 之前；否则原报错。
- [x] 单测：保底切分场景 + ≤4 条仍报错场景。
- [x] `chat_compress_context`：删 `conversation.context_state.warning = None;`。
- 验证：cargo test / harness。

### Step 5：全量检查（2.2）
- [x] `npm run typecheck` / `npm run lint` / `npm test` 全绿。
- [x] `cargo test --manifest-path src-tauri/Cargo.toml`（或 harness 替代，提交注明）。
- [x] 手测（`npm run dev`）：用户已验证（小对话手动压缩动画位置正确、divider 固定在触发时刻）
  1. 小对话（>4 条）点手动压缩 → 成功出 divider + 动画；
  2. 断网/无 key 触发手动压缩 → 报错且状态不卡"压缩中"；
  3. 工具调用重的对话触发 agent_loop 压缩 → 动画有槽位、结束后 divider 落点与实际保留消息一致（压缩后追问早期细节验证没丢上下文）。

## 回滚点
- 每个 Step 一个 commit（Conventional Commits）；Step 1/2 是行为修复主体，Step 3–4 可独立回滚。
- `_ui_message_id` 标注若出问题可整体 revert Step 1 的 commit 回到条数推算（旧行为虽错位但不 crash）。

## Review gate
- Step 1 完成后自查映射四场景测试；Step 5 结束跑 trellis-check 再进入 Phase 3。
