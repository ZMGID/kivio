---
name: workers
description: "检查 Worker 配置、构建产物与部署版本。"
---

# Workers 开发与发布

## 工作流

先核对 wrangler.jsonc/toml、compatibility_date、环境和绑定。运行当前版本支持的 deploy --dry-run 检查构建；会写本地产物。发布前确认 account 与 env，发布后查 deployment/version 和 HTTP 响应；不把 dry-run 当成已部署。

## 执行约定

- 使用 Kivio 的现有终端/文件工具执行，按实际任务加载此技能；不要假定整个插件只有一个固定入口。
- 首次使用或依赖、账号、项目变化时加载 `cloudflare:setup`；同一任务已验证环境可复用，失败时只修复具体阻塞。
- dev 可能连接远端资源，明确 local/remote；Wrangler secret 与绑定值不回显。
- 命令示例中的尖括号是待确认参数，不能原样执行。先用当前版本帮助核对未知子命令、参数与返回结构，不编造选项。
- 用户已授权的操作继续完成；需要额外授权时先准备可审阅的具体目标和变更。操作失败保留错误，不扩大权限或范围重试。
- 以真实退出状态、响应内容和回读结果验收，给出实际产物路径/资源 ID；区分已完成与未验证项。

## 依据

- [项目仓库](https://github.com/cloudflare/workers-sdk)
- [使用与安装文档](https://developers.cloudflare.com/workers/wrangler/)

核对日期：2026-10-06。本技能是 Kivio 的独立适配说明，不代表上游官方插件或支持承诺。
