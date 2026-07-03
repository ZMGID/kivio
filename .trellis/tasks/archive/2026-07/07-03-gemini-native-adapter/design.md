# Design：Gemini 原生接口协议适配

前置：读 prd.md + research/（adapter-contract / anthropic-template / dispatch-and-provider-wiring / gemini-api-shape）。

## 1. 总体

新增 `src-tauri/src/chat/model/gemini.rs`，作为 `openai.rs`/`anthropic.rs`/`responses.rs` 的**第四个 peer adapter**，实现 `LanguageModelProvider`（`generate`/`stream`/`capabilities`）。运行时只给 `GenerateRequest`、收 `GenerateOutput`+`StreamPart`——不感知 Gemini 线格式。以 **anthropic.rs 为模板**（既有非-OpenAI peer，复用 `send_with_failover`+`state.http` 的方式与 gemini 一致）。

新增 `ProviderApiFormat::Gemini`；provider 的 `apiFormat` 选 `gemini` 时，5 处 `match api_format_kind()` 分派到 `GeminiProvider`。

## 2. 关键技术决策（已定）

| 项 | 决策 |
|----|------|
| 端点 | `POST {base_url}/models/{model}:generateContent`；流式 `:streamGenerateContent?alt=sse`。base_url 用户配 Gemini 原生根 `https://generativelanguage.googleapis.com/v1beta` |
| 认证 | **`x-goog-api-key: <key>` 请求头**（不用 `?key=` 查询参，避免泄进 debug 日志 URL，且契合 `send_with_failover` 的 per-key 闭包） |
| api_format 规范串 | `"gemini"`（`from_raw` 别名：`gemini` / `google` / `gemini_generate`） |
| 系统提示 | `GenerateRequest.system` → 顶层 `systemInstruction`（非 contents 消息） |
| **不携带** | `promptCacheKey`/`prompt_cache_key`、`stream_options`、`tool_choice`、会话亲和头、`reasoning_effort`、`Authorization: Bearer`、顶层 `temperature`（放 `generationConfig`）——即撞 400 的那批 OpenAI 专有项 |
| provider_messages | 仍产出 **OpenAI 形状** assistant 消息（runtime 回放靠它），复用 `openai_compatible_message` 模式 |

## 3. 请求体映射（`request_body(&request, stream)`）

> 形状已由 opencode 真实流量确认，见 `research/opencode-real-traffic.md`。

```jsonc
{
  "systemInstruction": { "parts": [{ "text": <system> }] },   // 仅 system 非空时
  "contents": [ /* 见 §4 消息映射 */ ],
  "tools": [{ "functionDeclarations": [{ name, description, parameters: <normalized schema> }] }], // 仅有工具时
  "toolConfig": { "functionCallingConfig": { "mode": "AUTO" } },  // 有工具时显式发（对齐 opencode）
  "generationConfig": {
    "temperature": <opts.temperature>,   // 可选（opencode 省略；我方有该选项，发无妨）
    "maxOutputTokens": <opts.max_tokens>,
    "thinkingConfig": { ... }   // 见 §6，thinking 开启时
  }
}
```
- 末尾 merge `options.provider_options`（通用逃生口，同 anthropic.rs:344）。
- `functionDeclarations[].name` 用 `ModelTool::openai_tool_name()`（与 loop 的工具名往返一致）。
- **schema 归一化** `normalize_gemini_schema`（仿 `normalize_anthropic_schema`）：Gemini 只收 OpenAPI 子集——剥 `$schema`/`additionalProperties`、处理 nullable `anyOf`、去不支持字段。实现期用 probe 验证边界。

## 4. 消息映射（canonical `ModelMessage[]` → Gemini `contents[]`）

- 角色：`User→"user"`、`Assistant→"model"`、`Tool→"user"`（functionResponse 载体）。
- part：
  - `Text→{text}`；`Image→{inlineData:{mimeType,data}}`；`ImageUrl→{fileData}` 或 inline。
  - `Assistant` 的 `ToolCall→{functionCall:{name, args:<Value>}}`（args 是对象，非字符串）。
  - `Reasoning→{text, thought:true}`（回放时可丢，同 responses）。
  - `Tool` 的 `ToolResult→{functionResponse:{name, response:<obj>}}`。**Gemini 按函数名而非 call id 关联**：装配 contents 时建 `tool_call_id→name` 映射（扫在前的 assistant functionCall），用它把 ToolResult 的 `tool_call_id` 还原成 name；`response` 必须是 JSON 对象（字符串输出包成 `{ "output": <str> }`）。
- **同角色相邻 contents 合并**（复用 anthropic 的 merge 逻辑）。

## 5. 响应解析

### 非流式 `generateContent` → `GenerateOutput`
- 遍历 `candidates[0].content.parts[]`：`{text}`→累加 text（`thought:true` 的 → reasoning）；`{functionCall:{name,args}}`→ `PendingToolCall`（合成 id `call_<uuid>`；`args` 对象 → `arguments` + `arguments_raw`=序列化）。
- `finish_reason`：**由 part 是否含 functionCall 推导** `"tool_calls"`（Gemini 常在有 functionCall 时仍返回 `STOP`）；否则映射 `finishReason`：`STOP→stop`、`MAX_TOKENS→length`、其余→原样小写。
- `usage`：`usageMetadata` → `ModelUsage`（promptTokenCount→input、candidatesTokenCount→output、totalTokenCount→total、cachedContentTokenCount→cached、thoughtsTokenCount→reasoning）。
- **`thoughtSignature`**（part 上的思维签名，真实流量确认）：MVP 忽略（只读 text/思维文本，思维仍工作）；后续可在 assistant 回放 part 带回签名优化思维连续性。
- `provider_messages`：产出 OpenAI 形状 assistant 消息（`openai_compatible_message`）。

### 流式 `streamGenerateContent?alt=sse` → `StreamPart`
- SSE `data:` 行是完整 `GenerateContentResponse` 片段（**无增量 arg deltas**）。逐片：`text` part → `TextDelta`（thought → `ReasoningDelta`）；functionCall part → `ToolCallStart{id,name}` + `ToolCallDelta{full args}` + `ToolCallDone{PendingToolCall}`。
- 结束一条 `Finish{reason, full}`（full=累计 text）。流读错误用 `stream_read_error(label,&e)` 分类可重试中断。

## 6. thinking + thoughtSignature 回传（实测必需）
- `thinking_enabled` + `thinking_level` → `generationConfig.thinkingConfig`（保守只置 `includeThoughts:true`）。
- **thoughtSignature 回传（关键，chat-probe 实测证明 Gemini 3.x + 工具必需）**：response 的 functionCall（或同轮兄弟 part）带 `thoughtSignature`，**回放该 functionCall 时必须原样带回，否则 synthesis 请求 400**（`Function call is missing a thought_signature`）。载体链路：Gemini 解析 → `PendingToolCall.signature` → 流累加器 / provider_messages 的 tool_call 自定义键 `thought_signature` → 存储 → 回放 `pending_tool_calls_from_openai_message` 读回 → `MessagePart::ToolCall.signature` → gemini contents 回放时 functionCall 带 `thoughtSignature`。`MessagePart::ToolCall` 与 `PendingToolCall` 各加一个可选 `signature` 字段（其他 provider 恒 None、忽略）。签名可能在 functionCall 兄弟 part 上且顺序不定 → 每 chunk/候选预扫取首个签名兜底。
- `capabilities()`：`{tool_calling: provider.supports_tools, vision:true, streaming:true, reasoning:true}`。

## 7. 接线（新增 `Gemini` 变体的传播）

加 `ProviderApiFormat::Gemini` 会让编译器点出全部非穷尽 match，逐个补 Gemini 臂：
1. `settings.rs`：enum 变体 + `from_raw`（别名）+ `as_str`（`"gemini"`）。`sanitize_settings` 靠这俩自动规范化。
2. 5 处分派：`chat/agent/planning.rs`(740/766)、`chat/commands.rs`(5303)、`kivio_code/vision.rs`(114)、`chat/image_generation.rs`(146/197 → 加**兜底/透传臂**，Gemini 图像生成本次范围外)。
3. `chat/model/mod.rs`：`mod gemini;` + 导出 `GeminiProvider`。
4. 前端：`tauri.ts::normalizeProviderApiFormat` 加 gemini 分支；`SettingsShell.tsx` 格式下拉加 `{value:'gemini',label:'Gemini'}`。
5. thinking UI：若暴露思维档，`model_metadata.rs::reasoning_efforts_for_model` + `ThinkingLevelSelector` 加 gemini 分支（否则本次不动，thinking 走默认）。

## 8. 凭证 / failover
- `GeminiProvider::new(state, provider, retry_attempts)`——读 `provider.base_url`/`api_keys`/`id`。
- 复用 `send_with_failover(state, label, attempts, &provider.id, &provider.api_keys, |key| ...)`；per-key 闭包里把 key 放 `x-goog-api-key` 头。body 用 `attach_json_body(builder,&body,provider.compress_request_body)`、`with_standard_request_timeout`。错误状态码 `extract_status_code`。

## 9. 测试
- Rust 单测（仿 anthropic.rs 四类）：① `request_body` 断言 `systemInstruction`/`generationConfig`/`functionDeclarations` **且断言无 `promptCacheKey`/`tool_choice`**；② 消息→contents + functionResponse（按名关联）映射；③ `candidates` 响应→GenerateOutput（text/tool_calls/finish/usage）；④ `alt=sse` 片段解析。ModelProvider 测试字面量补全所有字段。
- E2E：**chat-probe**——写 request.json 指定 gemini provider/model，确认①不再 400 `promptCacheKey`②工具往返 success（result.json 的 toolCalls）。
- cargo test 本机 0xC0000139 → 纯函数靠编译+复核+probe。

## 10. 兼容 / 回滚
- 纯新增 adapter + 一个 enum 变体；现有把 Gemini 当 OpenAI-compat 的 provider 配置不受影响（仍走 openai_chat）。用户改选 "Gemini" 才走新路径。
- 回滚：删 gemini.rs + 各 Gemini match 臂 + 前端选项 + enum 变体。

## 11. 范围外
Gemini 图像生成（image_generation.rs 只加兜底臂）、embeddings（Gemini 独立 embed 端点，另说）、Gemini 的 fileData 大文件上传（先支持 inlineData base64）。
