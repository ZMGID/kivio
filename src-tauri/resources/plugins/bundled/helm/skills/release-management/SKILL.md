---
name: release-management
description: "跟踪 Helm release、升级差异与回滚历史。"
---

# Release 管理

## 工作流

先 helm list/status/history，记录 release、chart 版本、namespace 和 context。升级前按当前帮助预览并保护 Secret 输出；upgrade/install 需要明确目标与参数。结束后读取 helm status 和相关工作负载健康，失败时依据 history 选择回滚版本，保留失败证据。

## 执行约定

- 使用 Kivio 的现有终端/文件工具执行，按实际任务加载此技能；不要假定整个插件只有一个固定入口。
- 首次使用或依赖、账号、项目变化时加载 `helm:setup`；同一任务已验证环境可复用，失败时只修复具体阻塞。
- values 与渲染结果可能含 Secret；只输出必要字段，不默认跨所有命名空间枚举。
- 命令示例中的尖括号是待确认参数，不能原样执行。先用当前版本帮助核对未知子命令、参数与返回结构，不编造选项。
- 用户已授权的操作继续完成；需要额外授权时先准备可审阅的具体目标和变更。操作失败保留错误，不扩大权限或范围重试。
- 以真实退出状态、响应内容和回读结果验收，给出实际产物路径/资源 ID；区分已完成与未验证项。

## 依据

- [项目仓库](https://github.com/helm/helm)
- [使用与安装文档](https://helm.sh/docs/)

核对日期：2026-10-06。本技能是 Kivio 的独立适配说明，不代表上游官方插件或支持承诺。
