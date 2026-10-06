---
name: resource-query
description: "查询指定订阅的资源组与资源状态。"
---

# Azure 资源查询

## 工作流

确认租户、subscription、resource group，先 `az group list`，再 `az resource list --resource-group <group> --subscription <id>`；用 --query 和 --output json 缩小输出。不要读取 Key Vault Secret 的值来验证连通性。

## 执行约定

- 使用 Kivio 的现有终端/文件工具执行，按实际任务加载此技能；不要假定整个插件只有一个固定入口。
- 首次使用或依赖、账号、项目变化时加载 `azure:setup`；同一任务已验证环境可复用，失败时只修复具体阻塞。
- 不主动 az account set 更改默认订阅；后续显式传 --subscription，避免租户与订阅混用。
- 命令示例中的尖括号是待确认参数，不能原样执行。先用当前版本帮助核对未知子命令、参数与返回结构，不编造选项。
- 用户已授权的操作继续完成；需要额外授权时先准备可审阅的具体目标和变更。操作失败保留错误，不扩大权限或范围重试。
- 以真实退出状态、响应内容和回读结果验收，给出实际产物路径/资源 ID；区分已完成与未验证项。

## 依据

- [项目仓库](https://github.com/Azure/azure-cli)
- [使用与安装文档](https://learn.microsoft.com/cli/azure/)

核对日期：2026-10-06。本技能是 Kivio 的独立适配说明，不代表上游官方插件或支持承诺。
