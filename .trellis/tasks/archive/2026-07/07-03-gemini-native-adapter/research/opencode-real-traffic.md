# Research: opencode 真实 Gemini 流量（ground truth）

- **来源**：用户提供的 opencode→Gemini 抓包（`trace_f6a6fd11.html` + 完整 request/response JSON），模型 `gemini-3.1-flash-lite`。
- **Date**: 2026-07-03

## 端点 + 头（确认）

```
POST /v1beta/models/{model}:streamGenerateContent?alt=sse   (流式)
POST /v1beta/models/{model}:generateContent                  (非流式)
Host: generativelanguage.googleapis.com
Content-Type: application/json
x-goog-api-key: <API_KEY>          ← 认证走这个头（确认，非 ?key=）
```
- opencode 还发了 `x-session-affinity` / `x-session-id` 头，Gemini **接受**（200）——说明 Gemini 只对未知 **body 字段**严格（`promptCacheKey` 400 是 body 字段），**头不影响**。我们的 gemini adapter 不需要这些头（对 Gemini 无意义），保持干净即可。

## 请求 body（确认的确切形状）

```jsonc
{
  "generationConfig": {
    "maxOutputTokens": 32000,
    "topK": 64,          // opencode 发了；可选
    "topP": 0.95         // opencode 发了；可选
    // 注意：opencode 这条 NO temperature。我方可放 temperature（Gemini 接受），但非必需。
    // thinkingConfig 见下（本 trace 未出现，模型侧默认思维）
  },
  "contents": [
    { "role": "user", "parts": [ { "text": "…" } ] }
    // assistant 轮 role:"model"；tool 结果轮见下
  ],
  "systemInstruction": { "parts": [ { "text": "<系统提示词整段>" } ] },
  "tools": [
    { "functionDeclarations": [ { "name": "...", "description": "...", "parameters": { <JSON schema> } } ] }
  ],
  "toolConfig": { "functionCallingConfig": { "mode": "AUTO" } }   // opencode 显式发；我方也发（明确）
}
```
- **不含**：`promptCacheKey`/`prompt_cache_key`、`tool_choice`、`stream_options`、顶层 `temperature`、`Authorization: Bearer`。确认这批是撞 400 的元凶。
- `functionDeclarations[].parameters` 就是标准 JSON schema（opencode 的 bash/edit/… 直接放 `{type:"object", properties, required}`，未见 `$schema`/`additionalProperties`）。→ 我方仍做防御性 `normalize_gemini_schema`（剥 `$schema`/`additionalProperties`/nullable anyOf）。

## 工具往返映射（据 opencode 自身 gemini 解析逻辑确认）

trace 内嵌的 claude-tap viewer JS 暴露了 opencode 的 gemini part 解析：
- **assistant functionCall**：`part.functionCall = { name, args }`，`args` 是**对象**（非字符串），**无 id** → 解析成 `{type:'tool_use', name, input: args}`。我方合成 `id = call_<uuid>`。
- **tool functionResponse**：`part.functionResponse`，按 `response.id || response.name` 关联 → `{type:'tool_result', tool_use_id: id||name, content}`。确认 **Gemini 按函数名关联**（无 call id），我方装配 contents 时建 `tool_call_id→name` 映射还原 name；`response` 为对象。
- 请求体里放 functionCall/functionResponse 的**多轮 tool 往返本 trace 未抓到**（都是纯文本轮）→ 该映射靠单测 + chat-probe 实测验证。

## 响应（streamGenerateContent SSE，200，确认）

```jsonc
{
  "candidates": [
    { "content": { "role": "model", "parts": [
        { "text": "收到，测试消息已收到。" },
        { "text": "", "thoughtSignature": "EjQK…" }   // ← 思维签名挂在 part 上
      ] },
      "index": 0, "finishReason": "STOP" }
  ],
  "modelVersion": "gemini-3.1-flash-lite",
  "responseId": "…",
  "usageMetadata": {
    "promptTokenCount": 9449, "candidatesTokenCount": 7, "totalTokenCount": 9456,
    "promptTokensDetails": [ { "modality": "TEXT", "tokenCount": 9449 } ]
    // 思维时另有 thoughtsTokenCount / cachedContentTokenCount
  }
}
```
> 注：贴来的 JSON 里 `response.body.usage` 和 `response.body.content` 是 claude-tap 归一化加的，**不是** Gemini 原生字段。Gemini 原生就是 `candidates` / `usageMetadata` / `modelVersion` / `responseId`。

- **finishReason `STOP`**（即便可能带 functionCall 也常是 STOP）→ 我方 finish_reason **由 part 是否含 functionCall 推导** `tool_calls`。
- **`thoughtSignature`**：Gemini 思维时在 part 上返回签名，多轮应回传以保持思维连续。**MVP 可忽略**（思维仍工作，只是不回传签名）；后续可在 assistant 回放的 part 上带回 `thoughtSignature` 优化。usageMetadata → ModelUsage：`promptTokenCount→input`、`candidatesTokenCount→output`、`totalTokenCount→total`、`cachedContentTokenCount→cached`、`thoughtsTokenCount→reasoning`。

## 对 design 的净修正
1. **显式发 `toolConfig.functionCallingConfig.mode:"AUTO"`**（原 design 说省略；改为显式，对齐 opencode）。
2. `generationConfig`：`maxOutputTokens` 必发；`topK`/`topP`/`temperature` 可选（opencode 发 topK/topP、不发 temperature）——我方沿用现有 `GenerateOptions.temperature` 放进去无妨，或省略跟随 opencode。实现期二选一，倾向发 temperature（我们有该选项）。
3. 响应 part 的 `thoughtSignature` 归到 reasoning 处理路径（MVP 可只读文本、忽略签名）。
4. 认证头 `x-goog-api-key` 确认。
