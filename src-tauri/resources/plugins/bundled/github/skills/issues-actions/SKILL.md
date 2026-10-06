---
name: issues-actions
description: "查找 Issue、排查 Actions 失败并跟踪运行结果。"
---

# Issue 与 Actions

## 工作流

用 `gh issue list` / `gh issue view <number>` 缩小问题，用 `gh run list`、`gh run view <run-id> --log-failed` 分析失败步骤。日志可能含私有数据，仅摘取相关报错。重跑 workflow 或关闭 Issue 是写操作，不能当成诊断动作；执行后重新读取运行或 Issue 状态。

## 执行约定

- 使用 Kivio 的现有终端/文件工具执行，按实际任务加载此技能；不要假定整个插件只有一个固定入口。
- 首次使用或依赖、账号、项目变化时加载 `github:setup`；同一任务已验证环境可复用，失败时只修复具体阻塞。
- GitHub Enterprise 必须沿用目标 host；不要将仓库名相同视为同一仓库。
- 命令示例中的尖括号是待确认参数，不能原样执行。先用当前版本帮助核对未知子命令、参数与返回结构，不编造选项。
- 用户已授权的操作继续完成；需要额外授权时先准备可审阅的具体目标和变更。操作失败保留错误，不扩大权限或范围重试。
- 以真实退出状态、响应内容和回读结果验收，给出实际产物路径/资源 ID；区分已完成与未验证项。

## 依据

- [项目仓库](https://github.com/cli/cli)
- [使用与安装文档](https://cli.github.com/manual/)

核对日期：2026-10-06。本技能是 Kivio 的独立适配说明，不代表上游官方插件或支持承诺。
