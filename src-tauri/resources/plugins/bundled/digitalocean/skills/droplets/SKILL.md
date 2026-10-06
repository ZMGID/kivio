---
name: droplets
description: "列出和诊断指定账号的 DigitalOcean Droplet。"
---

# Droplet 查询

## 工作流

使用 `doctl compute droplet list`，按实际 ID 执行 get，核对 region、size、status。只摘取任务必需的 IP 和状态；SSH/控制台动作与 API 查询分开。分页和过滤按当前帮助执行。

## 执行约定

- 使用 Kivio 的现有终端/文件工具执行，按实际任务加载此技能；不要假定整个插件只有一个固定入口。
- 首次使用或依赖、账号、项目变化时加载 `digitalocean:setup`；同一任务已验证环境可复用，失败时只修复具体阻塞。
- 保留 context；Droplet 创建、扩容、备份及带宽可能计费，不能用于 setup 验证。
- 命令示例中的尖括号是待确认参数，不能原样执行。先用当前版本帮助核对未知子命令、参数与返回结构，不编造选项。
- 用户已授权的操作继续完成；需要额外授权时先准备可审阅的具体目标和变更。操作失败保留错误，不扩大权限或范围重试。
- 以真实退出状态、响应内容和回读结果验收，给出实际产物路径/资源 ID；区分已完成与未验证项。

## 依据

- [项目仓库](https://github.com/digitalocean/doctl)
- [使用与安装文档](https://docs.digitalocean.com/reference/doctl/)

核对日期：2026-10-06。本技能是 Kivio 的独立适配说明，不代表上游官方插件或支持承诺。
