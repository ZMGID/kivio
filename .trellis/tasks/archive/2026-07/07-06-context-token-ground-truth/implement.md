# Implement: 真实用量锚点回喂

前置:阅读 design.md。所有 Rust 测试经 `powershell -File scripts/win-cargo-test.ps1`(裸 cargo test 二进制启动失败 0xC0000139);对照 HEAD 既有 --lib 失败基线(~14 个,env/locale/path 类)。

## 步骤

### 1. 锚点数据结构 + loop 内记录
- [ ] `chat/model/types.rs` 或 `chat/agent/` 新增 `UsageAnchor { prompt_total: u64, output: u64 }` + `From<&ModelUsage>`(prompt_total = input+cached+cache_creation,均 unwrap_or(0);input_tokens 为 None 时整体返回 None)。
- [ ] `RunState` 增加 `last_step_usage: Option<ModelUsage>` 与 `runtime_len_at_last_call: usize`。
- [ ] planning.rs(3 处)/ synthesis.rs(4 处)`merge_usage` 调用点同步更新 `last_step_usage` + `runtime_len_at_last_call = state.runtime_messages.len()`(调用完成、追加消息之前的长度语义,以 design D2 为准)。
- 验证:`win-cargo-test.ps1 --lib` 编译过 + 既有 loop_tests 不回归。

### 2. effective_context_tokens 纯函数 + 压缩触发接入(消费点 A)
- [ ] 实现 `effective_context_tokens(anchor, estimate_full, estimate_after_anchor) -> (usize, bool)`,含 `max()` 保守规则;单测:无锚点=纯估算;锚点大于估算时采用锚点口径;锚点+增量 < 纯估算时仍取纯估算。
- [ ] `AgentRunConfig` 增加 `initial_anchor: Option<UsageAnchor>`(+ `initial_anchor_estimate_after: usize`),commands.rs 组装处从会话最后一条 assistant 的 `anchor_usage` 填充;找不到 → None。
- [ ] `maybe_compact_send_view`:`estimated` 改为 effective 口径(首步用 config 锚点,后续用 `last_step_usage` + `messages[runtime_len_at_last_call..]` 估算);压缩成功(micro/LLM 两分支)后清空 `last_step_usage` 与 config 锚点。
- [ ] R5:`budget` 减 `output_reserve = min(chat_max_output_tokens_for_model(...), window/5)`。
- 验证(loop_tests.rs 新增):
  - fake 步注入 `usage{input:240k}`、窗口 256k、字符估算 <10k → 下一步触发压缩。
  - 压缩成功后锚点清空:再下一步用纯估算、不再触发。
  - R5:锚点 200k + max_output 128k(reserve 封顶 51.2k)+ 窗口 256k → budget≈179k < 200k → 触发。

### 3. 落盘锚点(run → ChatMessage)
- [ ] `AgentRunResult` 增加 `last_step_usage: Option<ModelUsage>`,finalize/attach_usage 带出。
- [ ] `ChatMessage` 增加 `anchor_usage: Option<ModelUsage>`(serde default + skip_serializing_if none);commands.rs `build_assistant_message` 链路写入。
- 验证:带锚点消息序列化→反序列化 round-trip;旧 JSON(无字段)反序列化 OK。

### 4. footer 接入(消费点 B)
- [ ] `compute_context_state`:尾部扫最近带 `anchor_usage` 的 assistant;R4 失效判定(其后有压缩边界 / provider 变更);锚定时填 `session_input_tokens`、`token_count_source: Some("provider_reported")`、`estimated_input_tokens = effective`。
- [ ] 常量与 external_agents/context.rs 的 `TOKEN_COUNT_*` 并列定义(避免魔法串)。
- 验证:commands.rs 现有 context-state 测试风格补 3 例(锚定 / 无 usage 回落 / provider 切换失效)。

### 5. 前端
- [ ] `src/chat/types.ts`:注释补充 `token_count_source` 取值。
- [ ] `ContextIndicator.tsx`:`provider_reported` 与 `cli_reported` 同等展示精确值(去 `~`);i18n zh/en 补「模型实报」。
- [ ] 相关 .test.tsx 补断言。
- 验证:`npm run lint && npm run typecheck && npm test`。

### 6. 全量检查 + chat-probe 实测
- [ ] `win-cargo-test.ps1` 全量对照基线;lint/typecheck/vitest 全绿。
- [ ] chat_probe/request.json 驱动运行中 GUI 真实对话一轮,确认 footer 显示「模型实报」值且与 usage 面板一致。

## 回滚点
- 步骤 1-2 纯 loop 内,revert 即回滚。
- 步骤 3 引入落盘字段(向后兼容,新版本写、旧版本忽略),回滚无数据风险。
