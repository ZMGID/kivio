# Design — 消息链路正确性

## 1. Prompt 末条重复（缺陷 1）

### 现状
`compose_external_prompt`（prompt.rs:20）：`build_transcript` 遍历 `conversation.messages` 全量（调用时末条 user 已入列，见 reply.rs:95），随后 :65 又追加 `latest_user_message`。

### 方案：transcript 排除末条 user
`build_transcript` 增加语义：跳过**最后一条 user 消息**（按索引定位 `rposition(role=="user")`，而非按文本匹配——避免用户两轮发相同文本被误跳）。`latest_user_message` 继续由尾部承载，成为唯一事实源（Paseo D.2 原则）。

不选备选方案「compose 不再追加 latest」的原因：latest 尾置是所有协议（含 slash passthrough、resume 轮 skip_transcript）的统一出口，改它牵动面大；transcript 只在非 resume 路径参与，改它影响面最小。

### 边界
- 消息列表末尾可能有 assistant（regenerate 场景）：`rposition` 精确找最后一条 user，不假设它在列表末位。
- transcript 为空（首轮）→ 现有 `if !transcript.is_empty()` 分支自然处理。

## 2. ACP 消息边界去重（缺陷 2 + N7）

### 现状
两份相同逻辑（run_acp_session :695 / acp_apply_turn_update :1132）：全局 `emitted_text` 前缀裁剪。跨消息边界（工具调用后新消息的累积快照）失效 → 重复。

### 方案：共享 `AcpTextAssembler`
新结构（acp.rs 内）：

```rust
struct AcpTextAssembler {
    current: String,      // 当前消息已发出的累积文本
    boundary_pending: bool, // 见到 tool_call/thought 等边界事件后置位
}
impl AcpTextAssembler {
    fn on_boundary(&mut self)              // tool_call / tool_call_update / agent_thought_chunk → boundary_pending = true
    fn push_chunk(&mut self, text: &str) -> Option<String>  // 返回应发出的 delta
}
```

`push_chunk` 逻辑：
1. `boundary_pending` 且 `!text.starts_with(&self.current)` → 视为新消息起点：`current.clear()`。
2. 前缀匹配裁剪（累积快照）或整段追加（增量 delta）——与现逻辑一致，但作用域是"当前消息"。
3. 追加后 `boundary_pending = false`。

关键点：边界事件只**置位**不立刻清零——若上游是"整轮累积"语义（快照仍以全轮文本开头），步骤 1 的 starts_with 检查会命中旧前缀、不清零，行为与现状完全一致（向后兼容）。只有"按消息累积/纯增量"语义才触发新消息重置。这使三种上游语义统一在一个分支里。

两处驱动（一次性 + 持久）删除各自的 `emitted_text`，共用 `AcpTextAssembler`；`agent_thought_chunk`/`tool_call*` 处理处调 `on_boundary()`。ThinkingDelta 同样受益（thought 也可能按消息累积，用第二个 assembler 实例）。

### stream/claude.rs（N7）
`text_streamed` 全局 bool 改为跟随 message 生命周期：解析到新 assistant message 开始（message_start / 新 message id）时复位。改动限于该 parser 内部状态。

## 3. 测试设计（R3）

- `prompt.rs`：构造 2 user + 1 assistant 会话，`skip_transcript=false`，断言 `full_prompt.matches(latest_text).count() == 1`；再断言历史消息仍在。
- `acp.rs`：`AcpTextAssembler` 纯单元测试三组——纯增量、按消息累积（夹 boundary）、整轮累积（夹 boundary，验证不重复亦不丢字）。另加一组驱动级测试：喂 update 序列断言 sink 收到的 TextDelta 拼接结果。
- `stream/claude.rs`：两条 assistant 消息（中间 tool_use）均整块交付，断言两条都发出。

## 4. 影响面

- 触碰文件：`prompt.rs`、`session/acp.rs`、`stream/claude.rs`；`run.rs` 不改（emitted 状态在 acp.rs 内部）。
- UI 契约（chat-stream payload）不变；`UnifiedAgentEvent` 不变。
- 回滚：三个文件改动彼此独立，可按 commit 单独 revert。
