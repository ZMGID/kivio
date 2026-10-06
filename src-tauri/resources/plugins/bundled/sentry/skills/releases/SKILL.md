---
name: releases
description: "核对 Sentry release、commit 关联及部署记录。"
---

# Release 管理

## 工作流

明确 org、project、release version，用 releases list/info 读取。create、set-commits、finalize、deploys new 均是远端写操作，按授权执行；不能将多个项目的同名版本混同。最终回查 release 和部署记录。

## 执行约定

- 使用 Kivio 的现有终端/文件工具执行，按实际任务加载此技能；不要假定整个插件只有一个固定入口。
- 首次使用或依赖、账号、项目变化时加载 `sentry:setup`；同一任务已验证环境可复用，失败时只修复具体阻塞。
- 本插件面向发布与符号映射，不声称 sentry-cli 可以读取所有 Issue 或事件 API。
- 命令示例中的尖括号是待确认参数，不能原样执行。先用当前版本帮助核对未知子命令、参数与返回结构，不编造选项。
- 用户已授权的操作继续完成；需要额外授权时先准备可审阅的具体目标和变更。操作失败保留错误，不扩大权限或范围重试。
- 以真实退出状态、响应内容和回读结果验收，给出实际产物路径/资源 ID；区分已完成与未验证项。

## 依据

- [项目仓库](https://github.com/getsentry/sentry-cli)
- [使用与安装文档](https://docs.sentry.io/cli/)

核对日期：2026-10-06。本技能是 Kivio 的独立适配说明，不代表上游官方插件或支持承诺。
