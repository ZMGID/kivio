# Implement：Gemini 原生接口协议适配

前置：读 prd.md + design.md + research/。以 `chat/model/anthropic.rs` 为模板。

## 执行顺序

> **全部完成 + chat-probe 端到端实测通过**（真实 Gemini 3.1-flash-lite）。
> 实现要点/与规划差异：
> - Step 1-5 按计划：`gemini.rs` peer adapter；`ProviderApiFormat::Gemini`（`from_raw`/`as_str`）；5 处分派 + image_gen 兜底臂；前端 `normalizeProviderApiFormat` + 格式下拉 `Gemini`；`x-goog-api-key` 头；URL 从 model+stream 拼；不发 promptCacheKey/tool_choice/stream_options；显式 `toolConfig.mode=AUTO`；`normalize_gemini_schema`；finishReason 由 functionCall 推导；usageMetadata 映射；6 个单测。
> - **thoughtSignature 回传（chat-probe 实测发现为必需，非规划的"MVP 忽略"）**：Gemini 3.x 回放 functionCall 必须带回响应给的 `thoughtSignature`，否则 synthesis 400。为此给 `MessagePart::ToolCall` + `PendingToolCall` 各加可选 `signature` 字段，贯穿 解析→流累加器/provider_messages(`thought_signature` 键)→存储→回放(`pending_tool_calls_from_openai_message` 读回)→gemini contents 带回。每 chunk 预扫签名兜底（签名可能在兄弟 part）。修复前 `streamOutcome=recovered`+synthesis 400×2；修复后 `completed`+400×0。
> - 验证：`cargo check --lib --tests` 干净；前端 typecheck/lint 绿；chat-probe：单轮工具往返 success + 多轮 synthesis completed、无 promptCacheKey/thought_signature 400。


### Step 1：enum 变体 + 分派接线（先让编译器点全）
- [ ] `settings.rs`：`ProviderApiFormat` 加 `Gemini`；`from_raw` 收 `"gemini"|"google"|"gemini_generate"`；`as_str` 返回 `"gemini"`。
- [ ] `chat/model/mod.rs`：`mod gemini;` + 导出 `GeminiProvider`（先建空壳 struct + `new` 占位，`generate`/`stream`/`capabilities` 可 `todo!()` 临时占位以过编译分派）。
- [ ] 补齐 5 处非穷尽 match 的 Gemini 臂：`chat/agent/planning.rs`(740/766)、`chat/commands.rs`(5303)、`kivio_code/vision.rs`(114)、`chat/image_generation.rs`(146/197 加兜底/透传臂——Gemini 图像本次范围外)。
- 验证：`cargo check --lib` 干净（占位后无非穷尽 match 报错）。

### Step 2：请求体 + 消息映射
- [ ] `gemini.rs`：`GeminiProvider { state, provider, retry_attempts }` + `new`。
- [ ] `url(model, stream)` → `{base}/models/{model}:generateContent` / `:streamGenerateContent?alt=sse`。
- [ ] `request_body(&request, stream)`：`systemInstruction`(system 非空)、`contents`(见下)、`tools.functionDeclarations`(有工具时)、`generationConfig`(temperature/maxOutputTokens/thinkingConfig)；末尾 merge `provider_options`。**不含** promptCacheKey/tool_choice/stream_options 等。
- [ ] `gemini_contents_from_generate_request`：角色映射(User/Tool→user、Assistant→model)、part 映射(text/inlineData/functionCall/functionResponse)、**tool_call_id→name 映射**还原 functionResponse.name、response 包成对象、同角色相邻合并。
- [ ] `normalize_gemini_schema`（仿 normalize_anthropic_schema）：剥 `$schema`/`additionalProperties`、nullable anyOf 等。
- [ ] 单测：request_body 断言字段 + 断言无 promptCacheKey/tool_choice；contents/functionResponse 映射。
- 验证：cargo check --lib --tests。

### Step 3：响应解析（非流式）
- [ ] `parse generateContent 响应 → GenerateOutput`：parts(text/thought→reasoning、functionCall→PendingToolCall 合成 id)、finish_reason(**由 functionCall 存在推导 tool_calls**，否则映射 STOP/MAX_TOKENS…)、usageMetadata→ModelUsage、provider_messages 产 OpenAI 形状。
- [ ] `generate()` 实现：build body → send_with_failover(x-goog-api-key 头) → parse。
- [ ] 单测：candidates 响应 → GenerateOutput（text/tool_calls/finish/usage）。
- 验证：cargo check --lib --tests。

### Step 4：流式解析
- [ ] `stream()`：`?alt=sse`，逐 `data:` 片段(完整 GenerateContentResponse)→ StreamPart(text→TextDelta、thought→ReasoningDelta、functionCall→Start+Delta(full args)+Done)；结束一条 Finish{reason,full}；流读错误 `stream_read_error`。
- [ ] `capabilities()`：tool_calling=supports_tools，其余 true。
- [ ] 单测：alt=sse 片段解析 → 事件 + 组装的 tool call。
- 验证：cargo check --lib --tests。

### Step 5：thinking + 前端接线
- [ ] thinking：`thinking_enabled`/`thinking_level` → `generationConfig.thinkingConfig`（拿不准仅置 includeThoughts:true，budget 待 probe 校准）。
- [ ] 前端：`tauri.ts::normalizeProviderApiFormat` 加 gemini 分支；`SettingsShell.tsx` 格式下拉加 `{value:'gemini',label:'Gemini'}`。
- [ ] （可选）thinking 档若暴露：`model_metadata.rs::reasoning_efforts_for_model` + `ThinkingLevelSelector` 加 gemini 分支；否则不动。
- 验证：cargo check；`npm run typecheck && lint`。

### Step 6：全量检查 + probe E2E
- [ ] `cargo check --lib --tests` 干净；`--release` 编译过；`npm run typecheck && lint && test` 绿。
- [ ] **chat-probe E2E**（`docs/chat-probe.md`）：把某 Gemini provider 的 `apiFormat` 改成 `gemini`、base_url 设原生根，`npm run dev` 起 app，写 request.json 指定该 provider + gemini 模型 + 强制工具的 prompt：
  - ① 不再 `400 Unknown name "promptCacheKey"`；
  - ② `toolCalls` 里工具被调用且 success（如 glob/read）；answer 正确；
  - ③ 用带 thinking 的模型试 reasoning 是否回传。
- [ ] 手测：切回 openai_chat 的 provider 不回归。

## 回滚点
- Step 1 占位后各 Step 独立；出问题优先 revert gemini.rs 内对应部分。
- 整体回滚：删 gemini.rs + 各 Gemini match 臂 + enum 变体 + 前端选项。
- 每 Step 一 commit。

## Review gate
- Step 2 后自查：request_body **绝不含** OpenAI 专有字段（promptCacheKey/tool_choice/stream_options/Bearer 头）。
- Step 3/4 后自查：finish_reason 在有 functionCall 时为 tool_calls；tool_call_id↔name 往返正确（否则多工具/多轮会串）。
- Step 6：probe 实证不再 400 且工具往返成功，才算达成 AC1/AC2。

## 已知实现风险
- thinkingConfig 字段跨 Gemini 版本漂移 → 靠 probe 对目标模型实测校准（design §6）。
- Gemini schema 只收 OpenAPI 子集 → normalize_gemini_schema 的剥离范围靠 probe 撞出边界。
- functionResponse 按名关联 + 无 call id → id↔name 映射是多工具正确性的关键，重点测。
