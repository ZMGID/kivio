---
name: setup
description: "检查 Kivio 的 Azure CLI 插件接入、运行依赖与最小只读验证；仅在缺少配置、环境变化或用户要求时使用。"
---

# 配置 Azure CLI

## 接入检查

先通过 Kivio 的 `kivio_inspect` 查询 plugins 和 skills，确认 `azure` 包已启用、没有 diagnostics，且 `azure:setup` 及业务技能已进入当前对话。组件文件由插件市场统一管理；损坏时走市场的“重新配置”。不手工复制到 `~/.agents/skills`，不编辑插件记录或 settings；本包没有 MCP，不创建空 MCP 服务。

## 依赖与配置

1. 在 Kivio 实际工具环境检查可执行文件路径（Unix 用 command -v，PowerShell 用 Get-Command），然后验证：az version。PATH 不一致时先定位已有安装，不重复安装。
2. 缺失时：按 Microsoft 安装文档安装 az；macOS 可用 `brew install azure-cli`，Windows 使用官方 MSI 或文档列出的 winget 包。 安装会修改本机或项目；已有任务授权涵盖安装时继续，否则说明具体命令和影响后取得授权。不因再次配置而自动升级可用版本。
3. `az account show --query "{tenant:tenantId,subscription:id,name:name}"` 核对当前上下文；未登录时由用户执行 `az login --tenant <tenant>`。按指定 subscription 查询 `az group list --subscription <id>`。
4. 不主动 az account set 更改默认订阅；后续显式传 --subscription，避免租户与订阅混用。

## 验收

分别报告插件组件、实际工具版本、目标账号/上下文或文件范围，以及本次真实验证结果。帮助、版本和缓存状态不等于业务权限通过。凭证只通过用户本机登录或安全配置提供，不索取或回显 token、Cookie、密码；网络错误和权限不足按错误处理，不无限重登。配置过程不发送消息、不部署、不修改业务数据。

## 依据

- [项目仓库](https://github.com/Azure/azure-cli)
- [使用与安装文档](https://learn.microsoft.com/cli/azure/)

核对日期：2026-10-06。本技能是 Kivio 的独立适配说明，不代表上游官方插件或支持承诺。
