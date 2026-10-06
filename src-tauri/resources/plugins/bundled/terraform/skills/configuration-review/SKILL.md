---
name: configuration-review
description: "检查 Terraform 配置、格式和验证结果。"
---

# 配置审阅

## 工作流

先 `terraform fmt -check -diff`，检查 module 来源和 provider 锁文件。在依赖已准备后 `terraform validate`；缺依赖先列明 init 影响。输出精确文件位置和诊断，区分语法验证与云端可用性。

## 执行约定

- 使用 Kivio 的现有终端/文件工具执行，按实际任务加载此技能；不要假定整个插件只有一个固定入口。
- 首次使用或依赖、账号、项目变化时加载 `terraform:setup`；同一任务已验证环境可复用，失败时只修复具体阻塞。
- plan 可能访问云 API、持有状态锁并包含敏感值；不读取或展示完整 state、plan JSON。
- 命令示例中的尖括号是待确认参数，不能原样执行。先用当前版本帮助核对未知子命令、参数与返回结构，不编造选项。
- 用户已授权的操作继续完成；需要额外授权时先准备可审阅的具体目标和变更。操作失败保留错误，不扩大权限或范围重试。
- 以真实退出状态、响应内容和回读结果验收，给出实际产物路径/资源 ID；区分已完成与未验证项。

## 依据

- [项目仓库](https://github.com/hashicorp/terraform)
- [使用与安装文档](https://developer.hashicorp.com/terraform/cli)

核对日期：2026-10-06。本技能是 Kivio 的独立适配说明，不代表上游官方插件或支持承诺。
