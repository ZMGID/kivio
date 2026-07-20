# PRD — 消息链路正确性：prompt 重复与 ACP 流去重

父任务：`07-20-external-cli-overhaul`。研究依据：父任务 `research/kivio-audit.md`（缺陷 1、2、C1、N7、N9、E 节测试缺口）与 `research/paseo-reference.md`（C、D、G 节）。

## Goal

外部 CLI 收到的用户消息恰好一次；UI 显示的助手正文恰好一次。覆盖全部协议（ACP 持久/一次性、claude stream-json、pi/kimi 全量回放）、全部轮次（首轮、复用轮、重连轮）、含工具调用的多消息轮。

## Requirements

### R1 消除 prompt 末条重复（缺陷 1 + C1 + N9）
- `prompt.rs::compose_external_prompt`：`build_transcript` 与 `latest_user_message` 不得同时包含最后一条 user 消息。采用 Paseo 原则：transcript 排除末条 user（末条由 `# User request` 尾部的 latest 唯一承载）。
- 覆盖面（audit 核实）：所有 agent 首轮、pi/kimi 每一轮、持久协议 fresh connect/掉线重连轮（`run.rs` `first_prompt` 路径）。
- kimi 的 argv 30KB 上限（N9）：修复后 transcript 少一条消息，属自然收益，不做额外裁剪（超限报错行为不变）。

### R2 ACP assistant 输出按消息边界去重（缺陷 2 + N7）
- 废除全局 `emitted_text` 前缀匹配。参考 Paseo：不做全局文本去重，按消息边界维护累积游标。
- ACP 两处驱动（`run_acp_session` 与 `acp_apply_turn_update`）的 chunk 处理合并为一个共享函数 + 一个显式的 per-message 去重状态结构（含"当前消息累积文本"），在以下边界重置：tool_call / tool_call_update / agent_thought_chunk 出现后遇到的下一条 agent_message_chunk 视为新消息起点。
- 兼容三种上游语义：纯增量 delta（直接透传）、按消息累积快照（前缀裁剪，边界重置后生效）、整轮累积快照（旧行为退化兼容——前缀匹配在无边界事件时行为不变）。
- `stream/claude.rs` 的 `text_streamed` 全局 bool（N7）同类问题：按 message 复位。

### R3 测试补齐（audit E 节对应项）
- `prompt.rs`：非空会话 + `skip_transcript=false`，断言 full_prompt 中末条 user 文本恰好出现一次。
- `acp.rs`：两段 assistant 消息中间夹 tool_call 的 chunk 序列（增量式与按消息累积式各一组），断言拼接输出无重复。
- `stream/claude.rs`：多 assistant 消息场景 text_streamed 复位。

## 非目标

- 不动错误处理/超时/stderr（属 `07-20-cli-session-lifecycle`）；不动模型探测（属 `07-20-cli-detection-models`）；不改前端。

## 依赖与顺序

- 本任务先行，无前置依赖。
- `07-20-cli-session-lifecycle` 与本任务同文件（acp.rs / run.rs），须待本任务归档后开工（顺序写入对方 prd）。

## Acceptance Criteria

- [x] 单测：上述 R3 三组测试存在且通过；复检代理推演确认旧逻辑下均失败（红→绿可证）。
- [x] `cargo test --manifest-path src-tauri/Cargo.toml` external_agents 全部非 live 测试通过（--lib 1007，commit ce76f60）。
- [ ] 【待用户真机验收】grok 会话发 "123" → CLI 不再回复"你发了两遍"；带工具调用的回复正文只显示一次。
- [ ] 【待用户真机验收】pi 或 kimi（如本机可用）多轮对话，第 2 轮 CLI 收到的 prompt 中第 1 轮消息只出现一次、末条只出现一次。
- [x] 一次性 `run_acp_session` 与持久路径的 chunk 去重逻辑为同一份代码（acp_apply_session_update，复检确认）。
