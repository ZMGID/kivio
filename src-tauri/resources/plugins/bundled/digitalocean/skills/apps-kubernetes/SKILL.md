---
name: apps-kubernetes
description: "检查 DigitalOcean App Platform 与托管 Kubernetes 的状态。"
---

# 应用与集群

## 工作流

先 `doctl apps list` 或 `doctl kubernetes cluster list`，按 ID 获取部署/集群详情。下载 kubeconfig 会修改本地配置，需明确目的和目标文件；不默认覆盖。重新部署、扩缩容及销毁都需先展示目标与影响，完成后回读服务状态。

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
