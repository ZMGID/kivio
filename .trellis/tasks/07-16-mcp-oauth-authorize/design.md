# Design — MCP 服务器页 OAuth 授权入口

## 架构原则
不动 OAuth 内核（`connectors/oauth.rs`）与刷新链（`manager.rs::refresh_oauth_if_needed`，按 `auth.expires_at` 主动刷新，不依赖 `connector_id`）。只做**接线**：把已有能力接到 MCP 服务器页 + 让 401 可读 + 补 catalog URL 回退。

## #1 MCP 服务器页授权入口
- 位置：`SettingsShell.tsx` mcp tab，`isHttpTransport` 分支 Headers 字段下方（`:3812` 后）加按钮「OAuth 授权 / 重新授权」。
- 行为：点按 → `api.connectorOauthConnect({ url: server.url, name: server.name })`（复用现成命令，浏览器授权）→ 从返回的 server 中**只取** `auth` 对象和 `headers.Authorization`，`updateMcpServer(server.id, { auth, headers: { ...server.headers, Authorization } })` 并回**现有条目**。
  - 关键：保留用户在 MCP 页的 `id`/`name`/其它 headers，不引入 `connector-<slug>` 新条目、不 id 漂移。授权成功后顺手 `handleTestMcpServer` 填工具数（镜像 ConnectorsPanel `:233-242`）。
- 可见性：仅 `transport==='streamable_http'` 显示。URL 为空时禁用。已授权（`server.auth?.kind==='oauth'`）时按钮文案切「重新授权」。
- 状态：busy/error 局部状态，错误就地显示（镜像 ConnectorsPanel `oauthError`/`oauthBusyFor`）。

## #2 401 提示式重授权（仅设置页，不自动弹浏览器）
- 后端 `client.rs`：`StreamableHttpMcpClient::request`/`notify` 非成功分支（`:319`、`:357`），当 `status==401` 时读 `WWW-Authenticate` 头，若含 `Bearer`（尤其带 `resource_metadata=`），把错误信息改成结构化可识别前缀，如 `OAUTH_REQUIRED: <www-authenticate>`。保持 Result<_,String> 契约不变，仅让字符串可被前端识别。
- 前端 `SettingsShell.tsx`：`handleTestMcpServer`/`handleReloadMcpServer` 捕获到 `OAUTH_REQUIRED` 前缀时，把该服务器标为「需要授权」状态并高亮 #1 的授权按钮（不自动跑 OAuth）。
- 不改 chat 运行时事件链（聊天中工具调用 401 只会得到更清晰的错误串，不做界面重授权）。

## #3 catalog OAuth URL 回退
- 后端 `connectors/mod.rs::connector_oauth_connect`：`catalog_id` 命中 `builtin_oauth_url` 用其 URL；**未命中但传了 `url` 参数**时用 `url`，`connector_id` 仍取 `catalog_id`（保图标/展示）。
- 前端 `ConnectorsPanel.tsx::connectOauthConnector`（`:231`）：oauth catalog 项改为 `connectorOauthConnect({ catalogId: entry.id, url: entry.url })`。修好 Linear/Sentry/Atlassian（catalog 已带各自 `url`）。Notion 行为不变。

## 契约 / 兼容
- `ChatMcpServer.auth = ConnectorAuth`（`settings.rs:708`）已在前端镜像（`tauri.ts:367`），无需改结构。
- token 明文存 settings.json —— 与既有 connector 一致，不新增风险面。
- 无迁移：老服务器无 `auth` 字段时按钮走首次授权。

## 风险 / 回滚
- 风险文件：`client.rs`（401 分支改动，勿影响其它状态码路径）、`SettingsShell.tsx`（大文件，改动局限 mcp tab）。
- 回滚：三处独立，可分别 revert（#1 前端、#2 前后端、#3 前后端）。
</content>
