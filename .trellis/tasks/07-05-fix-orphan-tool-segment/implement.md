# Implement — 修复孤立工具分段

## 执行顺序

### 1. 新增反向对账 helper(`src-tauri/src/chat/commands.rs`)
- [ ] 写 `reconcile_orphan_tool_segments(tool_calls: &mut Vec<ToolCallRecord>, segments: &[ChatMessageSegment], api_messages: &[Value])`:
  - 收集 `tool_calls` 的 id 集合;遍历 `segments` 取 `kind==Tool` 且 `tool_call_id` 不在集合内的孤立 id(去重、保序)。
  - 从 `api_messages` 扫 assistant 消息 `tool_calls[]` 按 id 回捞 `function.name`/`function.arguments`(容错:字段缺失/非字符串)。
  - 合成 `ToolCallRecord{ status: Cancelled, error: Some("工具调用未完成（会话中断）"), started_at/completed_at=now, duration_ms:0, round: 尽力(从同 id 的 segment.round 取，否则 0), 其余默认 }`,push 进 `tool_calls`。
- [ ] 小工具:从 `api_messages` 取「id→(name,arguments)」的解析函数(可内联)。

### 2. 接入两条组装路径
- [ ] assistant 消息组装函数(`commands.rs:~2600`,调用 `normalize_assistant_segments` 之前):`let mut tool_calls = tool_calls;` 后 `reconcile_orphan_tool_segments(&mut tool_calls, &segments, &api_messages);`。注意后面 `tool_calls` 要用到两次(normalize + 存储 + model_messages),确保用的是增补后的。
- [ ] `persist_partial_assistant_snapshot`(`commands.rs:2790`,中断草稿):同样在构造 draft 前对 `tool_records`(clone 成 mut)跑 reconcile,用其 `api_messages`。保证中断草稿也不出孤立分段。

### 3. 前端确认/微调(`src/chat/ToolCallBlock.tsx`)
- [ ] 实测 `status=cancelled` 且 name 为空的记录渲染:若标题空白,加兜底「工具调用」;有 name 正常。有中断徽标即可。既有渲染够用则不改。

### 4. 测试
- [ ] Rust `chat::commands` 新增单测:
  - 孤立段 + api_messages 有该 id → tool_calls 增补 Cancelled 记录、name 回捞正确。
  - 孤立段 + api_messages 无该 id → 增补记录 name 空兜底。
  - 无孤立段 → tool_calls 不变(helper 空转)。
- [ ] 前端 `MessageBubble.test.tsx`:tool 段有匹配 cancelled 记录 → 渲染工具卡而非 `MissingToolSegment`。
- [ ] 回归:`normalize_assistant_segments` 既有正向补段测不变。

### 5. 验证
- [ ] `cargo check --manifest-path src-tauri/Cargo.toml` + `powershell.exe -File scripts/win-cargo-test.ps1 --lib chat::commands`(对照 14 失败基线)。
- [ ] `npm run lint && npm run typecheck && npm test`。
- [ ] 可选 E2E:用 chat-probe 造一次会调用工具的生成(如 glob),人为看正常路径不受影响;孤立态因需中断难自动造,靠单测覆盖。

## 风险 / 回滚点
- 改点集中在 `commands.rs` 消息组装 + 一处前端;纯新增记录,不删数据,零迁移。
- 风险:两条组装路径(正常 + 中断草稿)都要接,漏一条则中断场景仍复现——2 步务必都做。
- 回滚:独立 commit,`git revert` 即可。

## 验证命令
- `cargo check --manifest-path src-tauri/Cargo.toml`
- `powershell.exe -NoProfile -File scripts/win-cargo-test.ps1 --lib chat::commands`
- `npm run lint && npm run typecheck && npm test`
