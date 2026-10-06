---
name: plan-apply
description: "生成并审阅 Terraform plan，执行已批准的精确计划。"
---

# 计划与变更

## 工作流

确认 workspace、backend 与目标账号后生成 plan，逐项说明 create/update/replace/destroy。保存计划到任务专用受限路径，不上传可能含敏感值的文件。用户批准具体计划后 apply 该文件；过期计划重新生成与核对。不要默认 -auto-approve、-lock=false、force-unlock 或 state rm。

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
