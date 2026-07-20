# Implement — 消息链路正确性

前置阅读：本任务 `prd.md`、`design.md`；父任务 `research/kivio-audit.md`（缺陷 1、2、N7 小节）、`research/paseo-reference.md`（C、D 节）。

## 执行清单（按序）

### Step 1 — 缺陷 1：transcript 排除末条 user
- [ ] `src-tauri/src/external_agents/prompt.rs`：`build_transcript` 按 `rposition(|m| m.role == "user")` 定位末条 user，遍历时跳过该索引。
- [ ] 新增单测：非空会话（2 user + 1 assistant）+ `skip_transcript=false`，断言末条文本在 `full_prompt` 中恰好出现 1 次、历史消息仍在；regenerate 形态（末位是 assistant）单独一组。
- [ ] 验证：`cargo test --manifest-path src-tauri/Cargo.toml prompt`
- 回滚点：单文件 commit。

### Step 2 — 缺陷 2：AcpTextAssembler
- [ ] `src-tauri/src/external_agents/session/acp.rs`：新增 `AcpTextAssembler`（见 design §2），纯逻辑 + 单测三组语义（增量/按消息累积/整轮累积）。
- [ ] 改 `run_acp_session`：删 `emitted_text`，text/thought 各持一个 assembler；`tool_call`/`tool_call_update`/`agent_thought_chunk`（对 text assembler）处调 `on_boundary()`。
- [ ] 改 `acp_apply_turn_update` + `AcpSession::run_turn`：同上，去重逻辑只保留 assembler 一份。
- [ ] 驱动级测试：update 序列（msg1 chunks → tool_call → msg2 累积 chunks）断言 TextDelta 拼接无重复。
- [ ] 验证：`cargo test --manifest-path src-tauri/Cargo.toml acp`
- 回滚点：单文件 commit。

### Step 3 — N7：claude parser text_streamed 按消息复位
- [ ] `src-tauri/src/external_agents/stream/claude.rs`：`text_streamed` 在新 assistant message 开始时复位；补两条整块 assistant 消息的测试。
- [ ] 验证：`cargo test --manifest-path src-tauri/Cargo.toml claude`
- 回滚点：单文件 commit。

### Step 4 — 全量回归
- [ ] `cargo test --manifest-path src-tauri/Cargo.toml`（对比基线：Windows 下 --lib 有 ~14 个既有环境类失败，macOS 应全绿）
- [ ] `npm run lint && npm run typecheck && npm test`（前端无改动，应直接绿）

## 审查门（主会话执行，非子代理自证）

1. Diff review：确认无 UI 契约（chat-stream payload）改动、无 UnifiedAgentEvent 改动。
2. 实测：`npm run dev` 起 app，grok 会话发 "123" 验证不再"发两遍"；带工具调用的提问验证正文只显示一次。
3. 满足 prd.md 全部 Acceptance Criteria 后 → `trellis-check` 子代理复检 → 归档。

## 风险

- AcpTextAssembler 对"整轮累积"上游的兼容依赖 starts_with 命中旧前缀——若某 CLI 在 boundary 后发的快照恰好以旧全文开头但语义是新消息，会被误裁。缓解：单测覆盖 + 实测 grok/cursor。
- pi/kimi 每轮全量回放路径无 ACP 边界事件，不受 Step 2 影响（其重复来自缺陷 1，Step 1 已修）。
