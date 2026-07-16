# PRD — MCP 服务器页 OAuth 授权入口

## Goal / 用户价值
用户在"MCP 服务器"设置页添加需要 OAuth 的 remote (streamable_http) MCP（如 TinyFish `https://agent.tinyfish.ai/mcp`）时，能就地一键授权，而不是只能填静态 `Authorization=Bearer...` header 然后拿到 401。复用已有 OAuth 内核，不重造轮子。

## Background / 已确认事实（代码勘查）
- OAuth 内核已完整存在：`connectors/oauth.rs::run_oauth_connect` = protected-resource discovery → auth-server discovery → DCR → PKCE(S256) → loopback 回调 → token 交换，返回带 `Authorization: Bearer` 和 `auth` 的 `ChatMcpServer`。
- Tauri 命令 `connector_oauth_connect`（`connectors/mod.rs:36`）已支持自定义 URL：`connectorOauthConnect({ url, name })`（前端 wrapper `src/api/tauri.ts:1618`）。
- 主动刷新已接：`manager.rs:318` 连接前调 `refresh_oauth_if_needed`（按 `auth.expires_at`+leeway），成功后改写 header 并持久化；**只读 `auth` 字段（token_endpoint/refresh_token/client_id），不依赖 `connector_id`** → 拼进 MCP 页服务器的 `auth` 也能刷新。
- `auth` 结构 `ConnectorAuth`（`settings.rs:708`）：`kind, access_token, refresh_token, expires_at, token_endpoint, client_id, scopes, account`；前端镜像 `tauri.ts:367`。
- **缺口 #1**：MCP 服务器页 = `SettingsShell.tsx` mcp tab（HTTP 分支 `:3791-3813`），仅有 headers 文本框（`:3808` 占位符 `Authorization=Bearer ...`），无授权按钮、无 401 处理。相关：`newMcpServer()`(`:546`)、`updateMcpServer`(`:1412`)、`handleTestMcpServer`(`:1448`)、`handleReloadMcpServer`(`:1490`)。
- **缺口 #2**：`client.rs:357`（及 `notify` `:319`）把 401 连同 `WWW-Authenticate` 拍平成错误串，从不读挑战头。
- **缺口 #3**：`connectors/mod.rs:21 builtin_oauth_url` 只映射 `notion`；catalog（`connectorCatalog.ts`）里 Linear/Sentry/Atlassian 的 oauth 项会报 "Unknown OAuth connector"（各项已带 `url`）。
- 无需改代码的现成绕过：连接器页 → 自定义连接器 → auth=OAuth → 填 URL。本任务把该能力搬到 MCP 服务器页。

## Scope（已确认）
纳入全部三个缺口。#2 采用**提示式**：读 `WWW-Authenticate` 让 401 可识别，设置页测试/重载碰 401 时标「需要授权」并高亮授权按钮，**不自动弹浏览器**、**不覆盖聊天运行时**。

## Requirements
- **R1（#1 授权入口）**：streamable_http 服务器编辑器（Headers 下方）提供「OAuth 授权 / 重新授权」按钮；点按调 `connectorOauthConnect({ url: server.url, name: server.name })`，成功后**只把** `auth` + `headers.Authorization` 拼回**现有条目**（`updateMcpServer(server.id, …)`），保留原 `id`/`name`/其它 headers，不新增 `connector-*` 条目；随后自动测试连接填工具数。URL 空或 busy 时禁用；已授权时文案切「重新授权」。中英 i18n。
- **R2（#2 401 提示）**：`client.rs` request/notify 在 `401` 且 `WWW-Authenticate` 含 `bearer` 时，错误串加 `OAUTH_REQUIRED:` 前缀（保持 `Result<_,String>` 契约）；前端 `handleTestMcpServer`/`handleReloadMcpServer` 识别该前缀 → 标「需要授权」+ 高亮授权按钮。不动 chat 事件链。
- **R3（#3 catalog 回退）**：`connector_oauth_connect` 在 `catalog_id` 未命中 `builtin_oauth_url` 但传了 `url` 时用 `url`（connector_id 仍取 catalog_id）；`ConnectorsPanel.tsx` oauth catalog 项改传 `{ catalogId, url: entry.url }`。修好 Linear/Sentry/Atlassian，Notion 不变。

## Acceptance Criteria
- [x] AC1（用户手测通过）：MCP 页加 streamable_http 服务器、填 `https://agent.tinyfish.ai/mcp`、点「OAuth 授权」→ 浏览器授权 → 返回后该条目带上 Bearer token 与 `auth`（`oat_...`），测试连接列出工具。
- [x] AC2（依赖现有逻辑）：已授权服务器 token 过期且有 refresh_token 时，下次连接经 `refresh_oauth_if_needed` 自动续期。
- [x] AC3（用户手测通过）：401 + Bearer WWW-Authenticate → 「需要 OAuth 授权」提示 + 高亮按钮，非裸 401。
- [x] AC4（后端已修）：catalog id 未命中 builtin_oauth_url 时回退 `url`，Linear/Sentry/Atlassian 不再报 "Unknown OAuth connector"。
- [x] AC5：`client.rs` 401 分类纯函数 `classify_http_error` 单测通过；typecheck/lint 通过；`cargo test` 无新增失败。

## 补充修复（测试中发现）
- 协议版本白名单过旧：`SUPPORTED_PROTOCOL_VERSIONS` 停在 2025-06-18，服务器协商 `2025-11-25` 被拒。已把 2025-11-25 加入白名单（向后兼容，广播版本仍 2025-06-18），单测覆盖。commit `bc78956`。

## Out of Scope
- 聊天运行时工具调用 401 的界面重授权（仅得到更清晰错误串）。
- 401 自动弹浏览器重授权。
- 改 `ConnectorAuth` 结构或 token 存储方式（沿用明文 settings.json，与既有 connector 一致）。

## Open Questions
（无阻塞项）
</content>
