---
name: setup
description: "检查 Kivio 的 Obsidian 插件接入、运行依赖与最小只读验证；仅在缺少配置、环境变化或用户要求时使用。"
---

# 配置 Obsidian

## 接入检查

先通过 Kivio 的 `kivio_inspect` 查询 plugins 和 skills，确认 `obsidian` 包已启用、没有 diagnostics，且 `obsidian:setup` 及业务技能已进入当前对话。组件文件由插件市场统一管理；损坏时走市场的“重新配置”。不手工复制到 `~/.agents/skills`，不编辑插件记录或 settings；本包没有 MCP，不创建空 MCP 服务。

## 依赖与配置

1. 在 Kivio 实际工具环境检查可执行文件路径（Unix 用 command -v，PowerShell 用 Get-Command），然后验证：obsidian version。PATH 不一致时先定位已有安装，不重复安装。
2. 缺失时：官方 CLI 随 Obsidian 桌面应用提供，在设置 → 通用 → 命令行界面中启用；不要安装 npm 上无关的 obsidian-cli 包。 安装会修改本机或项目；已有任务授权涵盖安装时继续，否则说明具体命令和影响后取得授权。不因再次配置而自动升级可用版本。
3. 确认桌面应用运行，执行 `obsidian vaults`，由用户明确 vault，再用帮助确认 vault 参数和只读查询。这里只验证本次目标 vault，不创建测试笔记。
4. 保留双链、frontmatter、附件和 Canvas 格式；不要假设默认活跃笔记就是用户目标。

## 验收

分别报告插件组件、实际工具版本、目标账号/上下文或文件范围，以及本次真实验证结果。帮助、版本和缓存状态不等于业务权限通过。凭证只通过用户本机登录或安全配置提供，不索取或回显 token、Cookie、密码；网络错误和权限不足按错误处理，不无限重登。配置过程不发送消息、不部署、不修改业务数据。

## 依据

- [项目仓库](https://github.com/kepano/obsidian-skills)
- [使用与安装文档](https://help.obsidian.md/cli)

核对日期：2026-10-06。本技能是 Kivio 的独立适配说明，不代表上游官方插件或支持承诺。
