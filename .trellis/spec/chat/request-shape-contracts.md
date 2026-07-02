# Provider 请求形态契约（OpenAI-compatible adapter）

> 任务来源：`07-02-align-request-shape-and-tool-robustness`（2026-07-02/03）。
> 依据：opencode 真实流量（`trace_opencode_2026-07-02.json`）与 Kivio 实抓请求（`trace_kivio_2026-07-03.json`）逐字段比对。
> 相关代码：`src-tauri/src/chat/model/openai.rs`（`request_body` / `with_session_headers`）、`src-tauri/src/mcp/types.rs`（保留名别名）、`src-tauri/src/chat/agent/prepare.rs`（系统提示词）、`src-tauri/src/settings.rs`（`chat_current_datetime_context`）。

## Scenario: 发给聚合器/代理的 OpenAI-compatible 请求形态

### 1. Scope / Trigger

跨层契约：`request_body`/请求头形状 + 系统提示词内容共同决定 provider（尤其**会话亲和型逆向代理**，如 cursor2api 类）能否正确路由、命中缓存、执行工具。改这些时必须遵守本契约。

### 2. Signatures / 字段

`OpenAiChatProvider::request_body(request, stream) -> Value` 产出的 body 关键字段：

| 字段 | 取值 | 何时发 |
|---|---|---|
| `stream_options.include_usage` | `true` | `stream == true`（provider 随流报 usage，否则只能估算） |
| `tool_choice` | `"auto"` | `!tools.is_empty()` |
| `prompt_cache_key` + `promptCacheKey` | `conversation_id` | conversation_id 非空（双写法：前者 OpenAI 官方，后者 AI SDK/聚合器） |
| `x-session-id` + `x-session-affinity`（**请求头**） | `conversation_id` | conversation_id 非空；流式 + 非流式两条 send 路径都加（在 `send_with_failover` 闭包内、`bearer_auth` 之后） |
| `temperature` / `max_tokens` / `reasoning_effort` | 用户配置 | 既有行为，**有意保留**（与工具调用无关，二分实验已证） |

### 3. Contracts（硬约束）

**C1 — 系统提示词前缀一天内逐字节稳定。** system 消息是每轮请求的公共前缀；任何逐轮变化的内容（分钟级时钟、随机 id、计数器）都会打穿 provider 的 prefix-match prompt cache，并让会话亲和代理无法靠指纹续会话。`chat_current_datetime_context` 只到日期级（`2026年7月3日 星期五`，无 HH:MM）。验收：同对话相邻 agent 轮 system 消息 SHA 相同。

**C2 — 会话亲和三件套同值。** 同一对话每轮的 `x-session-id`/`x-session-affinity` 头 + `prompt_cache_key`/`promptCacheKey` body 字段全部等于 conversation_id。这让代理被**显式告知**"这些请求是同一会话"，不再靠前缀指纹猜（猜错 = 串台 / 复用脏会话）。

**C3 — 保留名 wire 别名。** 部分上游把特定工具名当内部保留工具**拦截消化**（实测 Cursor 系上游吞掉名为 `web_search` 的工具调用 → HTTP 200 空响应）。`RESERVED_WIRE_ALIASES`（`mcp/types.rs`）把内部名映射成无歧义 wire 别名（`web_search`→`search_web`）：
- **仅** `ChatToolDefinition::openai_tool_name()` / `ModelTool::openai_tool_name()`（native/skill/mixer 源）应用别名 → tools 声明 + 系统提示词渲染别名；
- 收到别名调用后 `match_tool_call`（匹配 `openai_tool_name()`）自然映射回内部工具；
- **内部逻辑全部仍见内部名**（`tool.name`）：`native_registry::find_entry`、`is_read_only_tool`、`parallel_safe`、`filter_tools_for_agent`、`agent_plan_allows_tool`、DSML 抽取。唯一收 wire 名的 `disabled_builtin_tool_feedback` 先 `resolve_reserved_wire_alias` 反解。
- MCP 工具不受影响（按前缀 id 命名，走 `_ => sanitize(&id)` 分支）。

### 4. Validation & Error Matrix

| 条件 | 行为 |
|---|---|
| conversation_id 为空（aux 任务如 vision） | 缓存键/会话头全部省略（不发空串，`.filter(|id| !id.is_empty())`） |
| provider 拒绝 `tool_choice` | `is_tools_unsupported_error`（stop.rs）识别并降级无工具重试 |
| 模型调用内部名（未经别名，如某些不读提示词的模型） | `match_tool_call` 精确匹配 `openai_tool_name()`==别名失败 → 大小写兜底 → 未知工具喂回（不崩） |
| 新发现的保留名撞车 | 往 `RESERVED_WIRE_ALIASES` 加一行即可，无需改逻辑 |

### 5. Good/Base/Bad Cases

- **Good**：冰供应商 grok-composer-2.5-fast 多轮搜索会话——`search_web` 被调用→映射回 `web_search` 执行→结果返回→`web_fetch` 续读，全程正常（实抓 `trace_kivio_2026-07-03.json` 验证）。
- **Base**：正经 provider（OpenAI/Anthropic）——别名同样是合法工具名，会话头/缓存键是已知或被忽略的字段，无副作用；`prompt_cache_key` 还提升官方缓存路由命中。
- **Bad（禁止）**：把别名泄漏进按内部名比对的逻辑；系统提示词渲染内部名而 tools 声明别名（不一致会诱发未知工具调用）；系统提示词注入分钟级时钟或其他逐轮变化内容。

### 6. Tests Required

- `openai.rs`：`request_body` 断言缓存键（双写法）/ `stream_options` / `tool_choice` 的有无门控。
- `mcp/types.rs`：别名双向映射；native `web_search` wire 名为 `search_web`；MCP 工具不受别名影响。
- `prepare.rs`：提示词渲染别名不含 `web_search`；`disabled_builtin_tool_feedback` 经别名反解识别内置工具。
- `settings.rs`：`chat_current_datetime_context` 同日字节一致、无 HH:MM。
- 本机 cargo test 有 0xC0000139 环境问题 → 纯函数用独立 harness 验证，提交注明。

### 7. Wrong vs Correct

#### Wrong
```rust
// 系统提示词硬编码内部名，与 tools 声明（别名）不一致 → 模型照提示词调 web_search → 被上游吞
"实时搜索必须优先用 web_search"
// 系统提示词注入分钟级时钟 → 每轮前缀都变 → 缓存全 miss + 代理无法续会话
format!("当前时间：{}:{:02}", now.hour(), now.minute())
```

#### Correct
```rust
// 提示词渲染 wire 别名，与 tools 声明一致
apply_reserved_wire_alias("web_search") // -> "search_web"
// 日期只到天，前缀稳定
format!("当前日期：{}年{}月{}日 {}", y, m, d, weekday)
// 会话亲和：同对话每轮同值
body["prompt_cache_key"] = conversation_id; // + x-session-id / x-session-affinity 头
```
