---
name: hosting
description: "检查 Firebase Hosting 构建目录与预览频道。"
---

# Hosting 发布

## 工作流

核对 public 目录、rewrites、headers 和 target。用当前帮助检查 hosting:channel:deploy，明确项目与频道并在授权后执行。检查返回 URL 和路由；生产 deploy 限定 --only hosting，不能顺带修改 Firestore rules 或 Functions。

## 执行约定

- 使用 Kivio 的现有终端/文件工具执行，按实际任务加载此技能；不要假定整个插件只有一个固定入口。
- 首次使用或依赖、账号、项目变化时加载 `firebase:setup`；同一任务已验证环境可复用，失败时只修复具体阻塞。
- 模拟器与生产端点必须区分；部署明确 --project 与 --only，避免发布不相关资源。
- 命令示例中的尖括号是待确认参数，不能原样执行。先用当前版本帮助核对未知子命令、参数与返回结构，不编造选项。
- 用户已授权的操作继续完成；需要额外授权时先准备可审阅的具体目标和变更。操作失败保留错误，不扩大权限或范围重试。
- 以真实退出状态、响应内容和回读结果验收，给出实际产物路径/资源 ID；区分已完成与未验证项。

## 依据

- [项目仓库](https://github.com/firebase/firebase-tools)
- [使用与安装文档](https://firebase.google.com/docs/cli)

核对日期：2026-10-06。本技能是 Kivio 的独立适配说明，不代表上游官方插件或支持承诺。
