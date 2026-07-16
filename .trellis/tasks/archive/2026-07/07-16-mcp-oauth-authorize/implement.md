# Implement — MCP 服务器页 OAuth 授权入口

## 执行顺序（每步可独立编译/验证）

### 步骤 A — #3 catalog URL 回退（最小、无依赖，先做打通信心）
1. 后端 `src-tauri/src/connectors/mod.rs::connector_oauth_connect`：`catalog_id` 分支未命中 `builtin_oauth_url` 时，若 `url` 非空则用 `url` + `catalog_id` 作 connector_id 调 `run_oauth_connect`；仍缺则报原错误。
2. 前端 `src/settings/ConnectorsPanel.tsx::connectOauthConnector`（~`:231`）：改为 `connectorOauthConnect({ catalogId: entry.id, url: entry.url })`。
- 验证：`cargo test --manifest-path src-tauri/Cargo.toml -p <crate> connectors`（对照 CLAUDE.md 基线，勿把既有 env/locale 失败当回归）；`npm run typecheck`。

### 步骤 B — #1 MCP 页授权按钮
3. `src/settings/SettingsShell.tsx`：加 `handleOauthAuthorizeMcpServer(server)`：调 `api.connectorOauthConnect({ url: server.url, name: server.name })`，成功后 `updateMcpServer(server.id, { auth: result.auth, headers: { ...(server.headers||{}), Authorization: result.headers?.Authorization } })`，再 `handleTestMcpServer`。局部 busy/error 状态。
4. 在 `isHttpTransport` 分支 Headers 字段后（~`:3812`）渲染 `<Button>`「OAuth 授权 / 重新授权」，`disabled` 当 url 空或 busy；文案依 `server.auth?.kind==='oauth'` 切换。i18n 中英。
- 验证：`npm run typecheck` + `npm run lint`；dev 手测：加 streamable_http 服务器填 TinyFish URL → 点授权 → 浏览器 → 回来带 token → 测试出工具数。

### 步骤 C — #2 401 可读 + 提示
5. 后端 `src-tauri/src/mcp/client.rs`：`request`（`:357`）与 `notify`（`:319`）非成功分支，`status.as_u16()==401` 时读 `response.headers().get("www-authenticate")`，含 `bearer`（忽略大小写）则错误串前缀 `OAUTH_REQUIRED:`。加单测：构造 401+WWW-Authenticate 断言前缀（用现有 mock 风格；若无 HTTP mock，抽一个纯函数 `classify_http_error(status, www_auth, body)` 单测）。
6. 前端 `SettingsShell.tsx`：`handleTestMcpServer`/`handleReloadMcpServer` 错误含 `OAUTH_REQUIRED` 时设「需要授权」态并高亮授权按钮。
- 验证：`cargo test ... mcp::client`；typecheck/lint；手测被撤销 token 的服务器测试连接 → 出「需要授权」而非裸 401。

## 验证命令汇总
- `npm run typecheck && npm run lint`
- `cargo test --manifest-path src-tauri/Cargo.toml`（对照 HEAD 基线差异）
- 手测三条路径（TinyFish 首次授权 / 已授权重连 / 401 提示）

## 风险 / 回滚点
- `client.rs` 401 分支：勿改动其它状态码与超时/取消路径；抽纯函数便于测试与隔离。
- `SettingsShell.tsx` 体量大：改动限定 mcp tab 渲染块与两个 handler。
- 三步独立，可分别 revert。

## 测试策略（ponytail）
- 后端 401 分类：一个纯函数单测（最小可失败点）。
- 前端接线：无框架新增，靠 typecheck + 手测（UI 时序，手测更快）。
</content>
