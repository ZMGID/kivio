# PRD: 上下文 token 计量对齐业界标准(真实用量回喂)

## 背景 / 问题

一个 ~84 万(中转口径)tokens 的对话在 planning 阶段对任何 provider 都报 502,但 footer 上下文指示环仍是绿色。根因链:

1. **UI 与 API 计量脱节**:footer / 自动压缩触发只用字符启发式 `estimate_tokens`(ASCII/4 + 非ASCII 字数),对该对话估 ~287k;而中转按字符≈1:1 计数,实收 ~840k,差 ~3 倍。
2. 模型库窗口值虚高(gpt-5.x 已另行修正为 256k)放大了问题:压缩阈值 = 窗口×0.9,估算永远够不到 → 压缩永不触发 → 请求撞穿网关真实上限。
3. Cline #7383 / #4584、Roo #1908 都踩过同一坑(UI 低估 2~4 倍导致超限报错),业界共识修法是**分层计量**:① provider 返回的真实 `usage.prompt_tokens` 为 ground truth → ② 真实分词器 → ③ 字符启发式仅兜底。

Kivio 目前只有第 ③ 层。而第 ① 层的数据 Kivio **已经全量持有**:每条 assistant 消息持久化了 `ChatMessage.usage`(`ModelUsage.input_tokens` 等),agent loop 每步也在累计 usage——只是 `compute_context_state` 硬写 `session_input_tokens: None`,压缩触发只看估算。外部 CLI 路径(`external_agents/context.rs`)已实现 `token_count_source: "cli_reported"` 的真实值口径,内置路径缺同等能力。

## 目标

把 provider 上一次实际返回的 prompt 用量作为**锚点(anchor)**回喂给内置路径的上下文计量,使 footer 显示与自动压缩触发都基于「真实锚点 + 增量估算」,自动适配代码密度偏差与按字符计费的中转,无需用户配置。

## 需求

### R1 上下文占用计算引入真实锚点
- 会话存在可用锚点(最近一条带 `usage` 的 assistant 消息)时,估算口径变为:
  `effective_tokens = anchor_prompt_total + anchor_output + estimate(锚点之后新增的消息)`
  其中 `anchor_prompt_total = input_tokens + cached_input_tokens + cache_creation_input_tokens`(缺省字段按 0)。
- 无锚点(新对话 / 旧数据无 usage)→ 完全回落现有纯估算,行为不变。
- **保守规则**:最终取 `max(纯估算, 锚点口径)`,确保引入锚点永远不会比现状更乐观。

### R2 自动压缩触发采用同一口径
- `maybe_compact_send_view` 的 `estimated` 改用 R1 口径(loop 内锚点来自本 run 已累计的 usage;首步锚点由调用方从会话最后一条 assistant 消息传入)。
- 压缩成功后锚点失效(消息序列已变),回落纯估算,直到下一次模型调用产生新 usage。

### R3 footer 呈现真实口径
- `compute_context_state`(内置路径)在锚点可用时填 `session_input_tokens` / `token_count_source: "provider_reported"`,`estimated_input_tokens` 用 R1 口径。
- 前端 ContextIndicator 对 `provider_reported` 显示与 `cli_reported` 同级的「真实值」标注(i18n 补一条),无锚点时保持现有「估算」标注。

### R4 锚点失效规则
- 会话切换 provider 后,旧锚点作废(不同 provider 计数口径不可比,尤其按字符计费的中转)。
- 压缩(自动/手动)后、删除/编辑锚点之前的消息后,锚点作废。作废 = 回落纯估算,不是报错。

### R5 输出预算预留(对齐 Roo 的保守余量) —— ❌ 已 descoped(实现阶段移除)
- 原设计:压缩触发预算从 `窗口 × 0.9` 收紧为再减 `min(max_output_tokens, 窗口×0.2)`。
- **决策(2026-07-06)**:不做。既有压缩单测把预算硬编码在 `窗口×0.9`(多用 window=600 极小测试窗口),改预算会连带破坏 4 个压缩边界断言,且小窗口需额外兜底防预算归零;收益相对锚点(本任务核心)次要。预算保持 `窗口×0.9` 不变,R5 移出本任务,可作独立后续任务。

## 非目标(明确不做)

- 不内置 tiktoken / 各家 count_tokens 端点(第 ② 层)——体积与维护成本高,且对按字符计费的中转无效。
- 不提供用户手配「字符预算 / 倍率」——锚点方案自动校准,无需配置。
- 不改 `estimate_tokens` 启发式公式本身(业界标准兜底,直连场景已够用)。
- 不处理「5xx 网关错误识别为疑似超长并压缩重试」——独立缺陷,另开任务。
- 不动知识库 `chunking.rs` 的估算(用途不同,与上下文预算无关)。

## 验收标准

- [ ] 构造带 usage 的会话:footer 的 token 数 ≈ 最后一次 usage(input+cached+cache_creation+output)+ 新增消息估算,`token_count_source == "provider_reported"`;无 usage 的旧会话行为与现状一致(`estimated`)。
- [ ] loop 单测(loop_tests.rs 风格):fake host 注入一步大 usage(如 240k,窗口 256k)后,即使字符估算远低于阈值,下一步 planning 前也触发压缩;压缩后锚点失效、不再用旧锚点判断。
- [ ] 切换 provider 后 context state 回落 `estimated` 口径。
- [ ] ~~R5~~(已 descoped,见上;预算保持 `窗口×0.9`)。
- [ ] `npm run lint` / `npm run typecheck` / `npm test` / Rust 测试(经 `scripts/win-cargo-test.ps1`,对照 HEAD 既有失败基线)全绿。
