# Design: 真实用量锚点回喂上下文计量

## 核心思路

引入「**用量锚点(usage anchor)**」概念:provider 每次返回的 `usage` 描述了「那次调用实际发送的整个 prompt 有多少 token」。这是对**当时全部历史**的一次权威测量(自动吸收代码密度、中转按字符计费等一切口径差)。之后只需对「锚点之后新增的消息」做启发式估算,叠加即得当前占用:

```
effective = anchor_prompt_total + anchor_output + estimate(锚点之后新增消息)
anchor_prompt_total = input_tokens + cached_input_tokens + cache_creation_input_tokens  (缺省按 0)
最终取 max(effective, 纯估算)   // 保守:永不比现状乐观
```

**业界对标(2026-07 调研)**:此方案与 pi(pi-mono)完全同构——pi 的
`contextTokens = usage.input + usage.output + usage.cacheRead + usage.cacheWrite`(取最后一条非中止 assistant 的 usage),
仅新会话/刚压缩后无 usage 时回落 `ceil(chars/4)` 估算;opencode 同为"API usage 主口径 + tokenizer/chars÷4 兜底"。
⚠️ **反例教训(opencode #4416)**:opencode 把 `input`(已含全 prompt)与 `cache.read` 相加导致**双算**、长会话压缩过早。
Anthropic 口径里 `input_tokens` 是「未命中缓存的输入」,与 cacheRead/cacheWrite 相加才是全 prompt(pi 口径,本方案采用);
OpenAI 口径里 `prompt_tokens` **已含** cached tokens——适配层(openai.rs)映射 `cached_input_tokens` 时若来源是
`prompt_tokens_details.cached_tokens`(子集语义),锚点求和处**不得再叠加**。实现时按 provider 家族区分:
`anthropic_messages` → input+cacheRead+cacheWrite;`openai_*` → 直接用 prompt_tokens(cached 是其子集,不再加)。

## 关键决策与坑

### D1 锚点取「最后一次调用」,不是 run 累计
`RunState.usage`(`merge_usage`)是**跨步累加**的总账(计费用),多步 run 的 `input_tokens` 是各步 prompt 之和,**远大于**单次 prompt——不能当锚点。需要单独记录**最后一步**的 usage:

- `RunState` 增加 `last_step_usage: Option<ModelUsage>`,在 planning/synthesis 现有 4+4 处 `merge_usage(...)` 调用点同步 `last_step_usage = usage.clone()`(仅当 `Some` 且 `input_tokens.is_some()`)。
- 持久化侧同理:`ChatMessage.usage` 是该次 run 的累计。**落盘锚点专用字段**:`AgentRunResult` 增加 `last_step_usage`,由 finalize 从 `RunState` 带出;`ChatMessage` 增加可选 `anchor_usage`(serde default,旧数据无字段 → None → 回落估算,天然兼容)。

### D2 两个消费点,共用一个纯函数
新模块 `chat/agent/context_estimate.rs`(或并入 compaction.rs 顶部)提供:

```rust
pub(crate) struct UsageAnchor { pub prompt_total: u64, pub output: u64 }
pub(crate) fn effective_context_tokens(
    anchor: Option<&UsageAnchor>,
    estimate_full: usize,          // 纯启发式全量估算(现状口径)
    estimate_after_anchor: usize,  // 锚点之后新增消息的估算
) -> (usize, bool)                 // (tokens, anchored?)
```

消费点 A——**loop 内压缩触发**(`maybe_compact_send_view`):
- run 首步:锚点由 `AgentRunConfig` 传入(commands.rs 从会话最后一条 assistant 的 `anchor_usage` 解析;`estimate_after_anchor` = 该 assistant 之后新增消息的估算,即本次 user 输入等)。
- run 中后续步:锚点 = `state.last_step_usage`;`estimate_after_anchor` = 上一步之后追加进 `runtime_messages` 的消息(assistant tool_calls + tool 结果)。实现上记录「上次调用时 runtime_messages.len()」,对 `messages[len..]` 求和即可。
- 压缩成功(micro 或 LLM)后:`last_step_usage = None`、config 锚点作废 → 回落纯估算(消息序列已变,锚点失真)。

消费点 B——**footer**(`compute_context_state`):
- 从会话尾部找最近一条带 `anchor_usage` 的 assistant;若其后存在压缩边界、或 `provider_id != conversation.provider_id`,锚点作废(R4)。
- 锚定时:`estimated_input_tokens = effective`、`session_input_tokens = Some(effective)`、`token_count_source = Some("provider_reported")`。
- 未锚定:现状不变(`token_count_source: None` → 前端显示「估算」)。

### D3 R5 输出预留只改一处
`maybe_compact_send_view` 的 `budget`,采用 opencode 形态的**绝对量预留**(pi 用 `窗口−16384`,opencode 用 `窗口 − min(输出上限,32k) − 20k buffer;纯百分比在 1M 窗口下预留过大、8k 窗口下预留过小):

```rust
const OUTPUT_RESERVE_CAP: usize = 32_000;   // 对齐 opencode OUTPUT_TOKEN_MAX 量级
let output_reserve = (chat_max_output_tokens_for_model(...) as usize).min(OUTPUT_RESERVE_CAP);
let budget = ((window as f32 * AUTO_COMPACT_RATIO) as usize).saturating_sub(output_reserve);
```

落盘路径 `should_auto_compress_context` 若有同类预算判断,用同一 helper 保持口径一致。注意 `keep_tokens = RECENT_KEEP_TOKENS.min(budget)` 的既有约束仍成立(budget 变小,小窗口模型自动收紧)。

### D4 前端
- `types.ts` 的 `token_count_source` 已是 string,加 `"provider_reported"` 值即可,无破坏。
- `ContextIndicator` 判断 `isCliReported` 处并列判断 `provider_reported`(显示精确值、不带 `~` 前缀);i18n zh/en 各补一条「模型实报」标签。

### D5 明确不做
- 不改 `estimate_tokens` 公式;不动 KB chunking;不做 tiktoken;不做手配倍率;5xx→压缩重试另开任务。

## 数据流(总览)

```
provider 响应 usage
  → planning/synthesis merge_usage(总账) + last_step_usage(锚点)     [loop 内]
  → finalize 带出 AgentRunResult.last_step_usage
  → commands.rs 写入 ChatMessage.anchor_usage(落盘)
      → 下次 run 的 AgentRunConfig 锚点(压缩触发,消费点 A)
      → compute_context_state 锚点(footer,消费点 B)
```

## 兼容 / 回滚

- 全部新字段 Option + serde default:旧会话文件无字段照常反序列化,行为=现状。
- 回滚 = revert 本任务提交;无迁移、无存储格式版本变更。
- 风险最大点是压缩提前触发过频(R5 预留 + 锚点偏大):`max()` 保守规则只会**更早**压,不会更晚;若实测过频,可把 `AUTO_COMPACT_RATIO` 与 reserve 系数调参,不动结构。
