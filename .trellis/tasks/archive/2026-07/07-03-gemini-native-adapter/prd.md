# Gemini 原生接口协议适配（GenerateContent peer adapter）

> **状态：占位（未启动）**。PRD 骨架，记录背景与方向，以后正式做时再补 design.md / implement.md 并 `task.py start`。

## 背景 / 动机

Google Gemini 的 OpenAI-compat 端点对未知字段**严格校验**：Kivio（及 opencode 等 OpenAI 风格客户端）下发 `promptCacheKey`/`prompt_cache_key`（会话亲和缓存键，见 `.trellis/spec/chat/request-shape-contracts.md` C2/C4）时，整个请求 `400 Unknown name "promptCacheKey": Cannot find field`。

- **实证**：chat-probe 通道 + opencode 用同样 url+key 均复现完全相同报错 → 是 Gemini shim 的协议差异，**非 Kivio bug**。
- 走 OpenAI 适配层"门控/删字段"只能治标，且 Gemini 原生协议（`generateContent`/`streamGenerateContent`）在工具调用、system_instruction、多模态、思维（thinking）、usage 口径上与 OpenAI Chat 差异较大，长期应有**原生 adapter**。

## Goal

为 Gemini 增加一个**原生协议 adapter**，作为 `chat/model/openai.rs`、`chat/model/anthropic.rs` 的 **peer**（实现同一 `LanguageModelProvider` 契约，见 `chat/model/README.md`）：运行时代码只构造 `GenerateRequest`、消费 `GenerateOutput` + `StreamPart`，不感知 Gemini 线格式。provider 配置里可把某 provider 标为 Gemini 原生格式（`apiFormat`），走该 adapter。

## Requirements（草拟，正式规划时细化）

- **R1 新 adapter** `chat/model/gemini.rs`（peer of openai/anthropic）：`GenerateRequest` → Gemini `generateContent` 请求体；SSE `streamGenerateContent` → `StreamPart`；`GenerateOutput`（text/tool_calls/finish/usage）从 Gemini 响应映射。
- **R2 provider 选路**：`apiFormat` 增加 `gemini`（或 `google_genai`）枚举值；provider 预设/设置 UI 可选；runtime 按 `apiFormat` 分派到 gemini adapter。
- **R3 能力映射**：system_instruction、`tools`/`functionDeclarations` + `functionCall`/`functionResponse`（工具调用往返）、多模态（inlineData/fileData）、thinking（如支持）、safety settings、usageMetadata → ModelUsage。
- **R4 不再发 OpenAI 专有字段**：gemini adapter 天然不带 `promptCacheKey`/`tool_choice`/`stream_options` 等 OpenAI 专有项；会话亲和/缓存按 Gemini 的等价机制（若有）或省略。
- **R5 兼容**：现有把 Gemini 当 OpenAI-compat 用的 provider 配置继续可用（不强制迁移）；仅新增一条原生路径。

## Acceptance Criteria（草拟）

- [ ] AC1：标为 gemini 原生格式的 provider，chat 多轮 + 工具调用往返正常（chat-probe 端到端验证：`glob`/`read` 等被调用且 success，无 400）。
- [ ] AC2：流式增量、finish_reason、usage 正确映射。
- [ ] AC3：不发 OpenAI 专有字段；原 OpenAI-compat 路径不回归。
- [ ] AC4：`cargo check --lib --tests` 干净；adapter 有单测（请求体映射、响应/SSE 解析、工具往返）。

## Notes

- 参考：`chat/model/README.md`（adapter 契约）、`openai.rs`/`anthropic.rs`（peer 实现范式）、`request-shape-contracts.md`（OpenAI 侧请求形状契约，勿把 Gemini 专有逻辑漏进去）。
- 验证利器：**chat-probe 通道**（`docs/chat-probe.md`）——写 request.json 指定 gemini provider/model，真实跑并看 result.json 的 toolCalls，判断适配是否成功。
- 范围外（正式规划再定）：embeddings（Gemini 有独立 embed 端点，另说）、image generation 走 Mixer。
