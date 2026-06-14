# Token 使用统计 PRD

## Goal

在设置/选项中新增一个 Token 使用统计页面，让用户能查看 Kivio 内各 AI 功能的请求日志、Token 消耗、成本估算和趋势，用来理解模型使用量、费用来源和不同 provider / model 的消耗分布。

## Product Context

Kivio 当前已经从轻量翻译工具扩展到 Chat、Lens、截图翻译、MCP/Skill、上下文压缩、标题总结、辅助视觉模型等多条 AI 调用路径。用户配置多个 OpenAI-compatible provider 和模型后，很难判断：

* 哪个功能最耗 Token。
* 哪个 provider / model 产生了主要费用。
* 流式 Chat、工具循环、上下文压缩、视觉分析是否在后台额外产生请求。
* provider 返回 usage 不完整时，统计是否可信。

本功能的核心价值是把“隐藏在后台的模型调用”变成可检查的本地账本，同时保持 Kivio 的隐私边界和轻量体积。

## What I Already Know

* 用户希望在选项中加入 Token 使用统计，参考截图包含总览卡片、30 天趋势图、请求日志、Provider 统计、模型统计。
* 统计页面目标语言应优先贴合现有设置页中文 UI。
* 当前项目是 React + TypeScript + Tauri v2，设置页入口为 `src/Settings.tsx`，实际 shell 在 `src/settings/SettingsShell.tsx`。
* 后端已经存在多 provider、多模型路由；不同功能分别使用翻译、截图翻译/OCR、Lens、Chat 的 provider/model。
* Chat 模型抽象中已经有 `ModelUsage` 类型，OpenAI/Anthropic 非流式响应路径可解析 provider 返回的 token usage。
* 项目没有现成图表库，依赖保持轻量；MVP 应优先使用 SVG/HTML 自绘趋势图，不引入重型 chart 依赖。
* 模型信息已包含定价字段：前端 `ModelDetailDrawer` 支持编辑 `pricing`，内置 `src/data/modelDatabase.json` 也包含 per 1M token 价格。

## Project Analysis

### Frontend

* 设置页由 `src/settings/SettingsShell.tsx` 统一承载，`SettingsTab` 当前包含 `general | translate | screenshot | lens | chat | memory | mixer | mcp | skill | webSearch | providers | about`。
* 导航项集中在 `navItems`，页面标题/副标题集中在 `pageMeta`，每个 tab 通过 `activeTab === '<tab>'` 条件渲染。
* 独立设置窗当前约 640 x 520；参考图里的大盘和日志表格较宽，MVP 需要响应式布局：独立设置窗展示紧凑版，Chat 内嵌设置或未来更大设置窗展示完整表格。
* 现有设置 UI 组件在 `src/settings/components.tsx`，可复用 `SettingsGroup`、`Select`、`Input`、按钮样式、面板样式和 lucide 图标。
* 前端 Tauri 调用集中在 `src/api/tauri.ts`，新增 usage 查询/清理类型和 `api.usageGetStats()` 等方法应放这里。

### Backend

* Tauri 命令集中注册在 `src-tauri/src/main.rs` 的 `invoke_handler`。
* 文本翻译入口：`commands::translate_text` -> `api::call_openai_text`。
* Lens / 截图翻译入口：`lens_commands.rs` 调用 `call_openai_ocr`、`call_openai_text`、`call_vision_api`、`stream_chat_call`、`stream_translate_combined`。
* Chat 主链路：`chat/commands.rs` 和 `chat/agent/loop_.rs` 通过 `OpenAiChatProvider` / `AnthropicMessagesProvider` 调用模型。
* Chat 模型层已有 `ModelUsage { input_tokens, output_tokens, total_tokens }`，OpenAI/Anthropic 非流式响应可解析 usage；流式路径目前多数返回 `usage: None`。
* 本地持久化已有 app data 目录使用经验，Chat 使用 `conversations/*.json` 和 atomic write；usage 应作为独立数据域存储，避免污染 `settings.json` 和对话文件。

### Existing Reusable Pieces

* `settings::ModelProvider` 和 `settings::ModelInfo` 已有 provider、model、定价、能力、用户覆盖字段。
* `chat/model_metadata.rs` 已经读取内置模型数据库，可新增价格解析工具，复用同一份数据源。
* `send_with_failover` 已知道 provider id 和实际请求结束状态，可作为成功/失败统计的近端参考，但 usage 解析仍应靠调用层，因为 response body 在调用层消费。
* `ModelUsage` 可扩展或新增更完整的 usage 类型，支持 cache / reasoning token，而不是只保存总量。

## Assumptions

* MVP 先统计应用内部发起的模型请求，不尝试读取第三方平台后台账单。
* 成本为本地估算值，基于用户配置或内置模型价格表，不保证等于 provider 真实账单。
* 统计数据应存放在本地应用数据目录，不放在 `settings.json` 中，避免设置文件膨胀。
* 如果 provider 没有返回 usage，MVP 可显示“未知”或使用估算值，并在日志中标记来源。
* 每一次模型 HTTP 调用记录为一条 usage record；一次用户发送消息可能产生多条记录，例如规划、工具调用、最终总结、标题总结、上下文压缩、辅助视觉分析。

## Open Questions

* 是否需要在 MVP 显示“缓存创建 / 缓存命中”两个指标。推荐答案：字段先支持，UI 中在没有数据时显示 0 或隐藏说明；不要为了截图一致而伪造。
* 是否需要把独立设置窗口尺寸放大。推荐答案：MVP 不强制改窗口尺寸，先做响应式滚动布局；如果实现后表格可用性太差，再单独调大 settings 窗口。

## Requirements

### MVP Requirements

* 在设置页新增“用量统计”入口，位置建议放在“模型 / Providers”附近，因为它与 provider/model 成本最相关。
* 页面顶部展示总览：
  * 总 Token。
  * 总请求数。
  * 总成本估算 USD。
  * 输入 Token。
  * 输出 Token。
  * 缓存命中 Token。
  * 缓存创建 Token。
  * 统计可信度，例如 provider-reported 占比 / missing usage 占比。
* 提供时间范围切换：7d / 30d / 90d / 全部。默认 30d。
* 提供趋势图：
  * X 轴为日期。
  * 至少展示总 Token 和成本。
  * 可选择性显示输入、输出、缓存命中、缓存创建。
  * 不引入外部图表库，优先使用 SVG polyline/path。
* 提供三种视图：
  * 请求日志。
  * Provider 统计。
  * 模型统计。
* 请求日志表格字段：
  * 时间。
  * 来源。
  * Provider。
  * 模型。
  * 输入。
  * 输出。
  * 总 Token。
  * 成本。
  * 耗时。
  * 状态。
  * usage 来源。
* 请求日志筛选：
  * 来源筛选：全部、Chat、文本翻译、截图翻译、Lens、标题总结、上下文压缩、辅助视觉、图片生成。
  * 状态筛选：全部、成功、失败、取消、无 usage。
  * Provider 搜索。
  * Model 搜索。
* Provider / 模型统计字段：
  * 请求数。
  * 成功率。
  * 总 Token。
  * 输入 / 输出 Token。
  * 缓存相关 Token。
  * 成本估算。
  * 平均耗时。
  * 最近使用时间。
* 后端统一记录 usage，前端只查询聚合结果，不从 UI 事件或响应文本反推。
* 数据记录不包含 prompt、response、截图、附件内容、API key。
* 提供“清空统计”操作，需要确认；MVP 可不做导出。

### Data Sources To Cover

MVP 推荐覆盖所有模型调用入口，但允许部分入口以 `usageSource = missing` 落账：

* `translator`: 主输入翻译。
* `screenshot_translation`: 云端 OCR+翻译、系统/RapidOCR 后的纯文本翻译。
* `lens`: Lens 视觉问答和纯文本问答。
* `chat`: 主 Chat 普通回复、Agent planning/tool loop/synthesis。
* `chat_title_summary`: 对话标题总结。
* `chat_compression`: 上下文压缩。
* `chat_aux_vision`: Chat 图片附件的辅助视觉分析。
* `chat_image_generation`: 文生图/生图模型调用，如果返回 usage 或成本信息则记录；没有 token usage 时只记录请求、耗时和状态。

不记录：

* provider 连接测试和拉取模型列表，因为不是 token usage。
* 本地工具调用、MCP 工具调用、文件读写、网页搜索本身，除非它们触发模型总结调用。

## Data Model

### Usage Record

建议新增后端模块 `src-tauri/src/usage.rs`，使用 app data 下独立文件保存。

```json
{
  "id": "usage_...",
  "created_at": 1780914277,
  "completed_at": 1780914279,
  "duration_ms": 1820,
  "source": "chat",
  "operation": "agent_synthesis",
  "provider_id": "openai",
  "provider_name": "OpenAI",
  "model": "gpt-4.1",
  "api_format": "openai_chat",
  "status": "success",
  "status_code": 200,
  "usage_source": "provider_reported",
  "input_tokens": 14246,
  "output_tokens": 2014,
  "total_tokens": 16260,
  "cached_input_tokens": 83840,
  "cache_creation_input_tokens": 0,
  "reasoning_tokens": 0,
  "cost_usd": 0.1736,
  "cost_source": "model_pricing",
  "conversation_id": "conv_...",
  "message_id": "msg_...",
  "error_kind": null
}
```

Field notes:

* `usage_source`: `provider_reported | estimated | missing`.
* `cost_source`: `model_pricing | user_override | unavailable`.
* `source` is user-facing feature area; `operation` is implementation-level subtask.
* Correlation ids are optional and must not include user text.
* Failed requests should still log status, provider, model, source, duration, and error kind, but token/cost fields may be null or 0.

### Storage

Recommended MVP storage:

* Directory: `{app_data_dir}/usage/`.
* Append-only monthly JSONL files: `usage-YYYY-MM.jsonl`.
* Optional small metadata file later: `usage/index.json`.
* Reads aggregate records by date range; writes append one line per model request.
* On malformed lines, skip and continue; do not break the stats page.
* Add clear command that deletes usage files after user confirmation from frontend.

Rationale:

* Avoids adding SQLite or a heavy dependency.
* Keeps writes cheap and robust.
* Works with the app's local-first privacy model.
* Monthly files make retention/export future-proof.

### Cost Calculation

Use existing model metadata:

* First check provider `model_overrides[model].pricing`.
* Then check `src/data/modelDatabase.json`.
* Price units are USD per 1M tokens.

Formula:

* `billable_uncached_input = input_tokens - cached_input_tokens`, clamped at 0.
* input cost = `billable_uncached_input * input_price / 1_000_000`.
* cached input cost = `cached_input_tokens * cached_input_price / 1_000_000` when available; otherwise fall back to input price or mark partial.
* output cost = `output_tokens * output_price / 1_000_000`.
* cache creation cost uses a dedicated field only if the model database later supports one; MVP may count it as normal input and keep the raw token field visible.

If pricing is missing, show cost as `--` for the row and exclude it from total cost, while still counting tokens.

## Backend API

Add Tauri commands:

* `usage_get_stats(query) -> UsageStatsResponse`
* `usage_clear() -> Result<(), String>`

Potential query shape:

```ts
type UsageStatsQuery = {
  range: '7d' | '30d' | '90d' | 'all'
  source?: string
  status?: string
  providerSearch?: string
  modelSearch?: string
  limit?: number
  offset?: number
}
```

Potential response shape:

```ts
type UsageStatsResponse = {
  summary: UsageSummary
  trend: UsageTrendPoint[]
  logs: UsageLogRow[]
  providerStats: UsageGroupStats[]
  modelStats: UsageGroupStats[]
  totalLogs: number
}
```

## Usage Capture Strategy

Implement a small helper API in Rust:

* Build a `UsageRecordBuilder` before the HTTP call with source/provider/model/operation.
* On success, enrich it with parsed usage and status.
* On error, record status/error kind without sensitive body content.
* Flush append-only JSONL after the response body is consumed, because usage is usually inside JSON/SSE body.

Capture points:

* `call_openai_text`: parse usage from non-stream OpenAI-compatible response.
* `call_openai_ocr`: parse usage from non-stream OpenAI-compatible response.
* `call_vision_api`: parse usage in non-stream; in stream parse usage chunks where provider sends them, otherwise mark missing.
* `stream_chat_call` and `stream_translate_combined`: parse stream usage chunks where available.
* `OpenAiChatProvider::generate_inner`: already parses `ModelUsage`; record after `output_from_chat_completion`.
* `OpenAiChatProvider::stream_inner`: extend stream parser to capture usage chunks where available; otherwise mark missing.
* `AnthropicMessagesProvider::generate_inner`: already parses `ModelUsage`; record after output.
* `AnthropicMessagesProvider::stream_inner`: capture final stream usage when available; otherwise mark missing.
* Chat auxiliary flows (`generate_title_with_model`, compression, auxiliary vision) should pass a distinct `operation` label through request metadata.

Important: avoid double counting. If usage is recorded at provider abstraction level for Chat, do not also record the same Chat request in the caller.

## UI Specification

### Navigation

Add `usage` to `SettingsTab`:

* Chinese label: `用量统计`
* English label: `Usage`
* Icon: lucide `BarChart3` or `Activity`

### Layout

For standalone settings window:

* Compact header summary cards in a 2-column grid.
* Trend chart below summary.
* Segmented control for `请求日志 / Provider 统计 / 模型统计`.
* Filters collapse into one or two rows depending on width.
* Table horizontally scrolls if needed; avoid text overlap.

For embedded settings:

* Wider grid with summary cards and full table.

### Copy

Use honest wording:

* Prefer `Token 消耗` over `真实消耗 Tokens` unless all selected records are provider-reported.
* Show `估算成本` instead of `总成本` when cost is local calculation.
* Show `usage 缺失` / `未返回 usage` for provider responses without usage.

### Empty / Error States

* No records: show a quiet empty state with no marketing copy.
* Corrupt usage file lines: stats page still loads; backend may include skipped count.
* Missing pricing: show token stats but cost as `--`.
* Missing usage: row status remains visible, tokens as `--`, summary includes missing count.

## Acceptance Criteria

* [ ] 设置页可进入 Token 使用统计页面。
* [ ] 成功模型请求会产生本地 usage 记录，包含时间、来源、provider、model、状态、token usage、耗时和成本估算字段。
* [ ] 统计页能读取并展示汇总数据、趋势数据和请求日志。
* [ ] 对没有 usage 的请求有明确状态，不会造成统计页崩溃。
* [ ] 统计数据不会泄露 API key 或请求正文敏感内容。
* [ ] 数据结构支持后续按 provider/model/source/date 查询和清理。
* [ ] Chat Agent 一轮多模型调用会按实际模型请求分别记录，不合并成单条用户消息。
* [ ] 流式请求没有 provider usage 时不会伪造“真实 token”，而是显示 missing 或 estimated。
* [ ] 成本计算优先使用用户模型覆盖价格，再回退内置模型数据库。
* [ ] 清空统计需要二次确认，并只删除 usage 数据，不影响 settings / conversations / history。

## Definition Of Done

* Tests added/updated where practical, especially backend aggregation / persistence logic.
* `npm run lint`, `npm run typecheck`, and relevant Rust tests pass when implementation begins.
* Docs/notes updated if persistent data schema or settings behavior changes.
* Rollout/rollback considered because this adds persistent local telemetry.

## Out Of Scope

* 不接入 provider 官方 billing API。
* 不上传任何使用统计到云端。
* 不把完整 prompt、图片、附件、模型响应正文写入统计日志。
* 不在 MVP 做团队/多设备同步。
* 不在 MVP 引入复杂账单对账、发票、预算告警。
* 不在 MVP 统计非模型工具的资源消耗。
* 不在 MVP 保证所有 OpenAI-compatible 代理都能返回流式 usage。

## Future Enhancements

* CSV / JSON 导出。
* 每日/每月预算提醒。
* 按会话 / 项目 / 助手统计。
* 可配置 retention，例如保留 30 / 90 / 180 / 365 天。
* 支持 provider 官方 billing API 对账。
* 更完整的 cache creation pricing 字段。
* 对 missing usage 的本地 tokenizer 估算，但必须清晰标记为 estimated。

## Technical Notes

* Initial files inspected: `src/Settings.tsx`, `src/api/tauri.ts`, `src-tauri/src/api.rs`, `src-tauri/src/chat/model/openai.rs`, `src-tauri/src/chat/model/anthropic.rs`, `src-tauri/src/chat/model/types.rs`.
* Additional files inspected: `src/settings/SettingsShell.tsx`, `src/settings/components.tsx`, `src-tauri/src/commands.rs`, `src-tauri/src/settings.rs`, `src-tauri/src/lens_commands.rs`, `src-tauri/src/chat/commands.rs`, `src-tauri/src/chat/agent/loop_.rs`, `src-tauri/src/chat/storage.rs`, `src-tauri/src/chat/model_metadata.rs`, `src-tauri/src/chat/types.rs`, `src/data/modelDatabase.json`, `package.json`.
* Implementation should read `.trellis/spec/guides/cross-layer-thinking-guide.md` because this crosses frontend, Tauri commands, model providers, and local persistence.
* Implementation should read `.trellis/spec/guides/code-reuse-thinking-guide.md` before adding helpers, especially around model metadata and storage.
