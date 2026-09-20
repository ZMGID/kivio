# GitHub MCP 授权

GitHub 官方 MCP 的默认入口使用 Kivio 注册的 OAuth App 和 GitHub Device Flow。
普通用户点击授权，在浏览器输入 Kivio 显示的短码，再确认公开仓库权限即可。
用户无需注册应用、填写 Client ID、Client Secret 或 PAT。

## 维护者配置

- 应用：`Kivio Dev`，公开 Client ID：`Ov23liilcX7ps76sGn1r`。
- 在 GitHub OAuth App 设置中开启 **Enable Device Flow** 并保存，这是发布前置条件。
- 当前默认申请 `public_repo offline_access`：公开仓库读写及可刷新的令牌。
- Device Flow 取令牌和刷新令牌都不需要 Client Secret。不要把密钥加入源码、构建变量或安装包。
- 本机之前手动配置的 OAuth/PAT 继续有效。重新走内置授权成功后，用个人设备令牌替换旧认证，并去掉旧应用密钥。
- 如果更换应用，更新 `src-tauri/src/connectors/github_device.rs` 的公开 Client ID；不要复用第三方客户端身份。

GitHub 主机及 MCP 路径由后端严格匹配。自定义 MCP 服务仍使用原来的 DCR 或高级自定义应用配置。
授权等待遵守 GitHub 轮询间隔，处理限速、拒绝、到期和取消；离开页面或销毁窗口会取消等待，迟到结果不会保存。
MCP 页面和连接器目录共用同一个授权生命周期，令牌通过原设置保存入口持久化，刷新复用 MCP manager。

## 发布验收

1. 用没有 GitHub 认证配置的用户配置连接，确认不要求应用密钥。
2. 浏览器授权后，确认获取 MCP 工具列表；保存的认证不包含 Client Secret。
3. 取消、拒绝或等待短码到期后可以重试；取消后的结果不能覆盖现有认证。
4. 在令牌过期时确认无密钥刷新成功。

参考：[GitHub Device Flow](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps#device-flow)。
