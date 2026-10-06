---
name: workload-diagnostics
description: "分析 Kubernetes Pod、事件与有限范围日志。"
---

# 工作负载诊断

## 工作流

先 get pods，再针对一个 Pod 使用 describe 和 logs --tail=100；多容器指定 -c，重启问题按需 --previous。区分 Pending、ImagePullBackOff、CrashLoopBackOff、探针失败与资源不足。避免 get secrets，最终给出对象、namespace、事件时间和证据。

## 执行约定

- 使用 Kivio 的现有终端/文件工具执行，按实际任务加载此技能；不要假定整个插件只有一个固定入口。
- 首次使用或依赖、账号、项目变化时加载 `kubernetes:setup`；同一任务已验证环境可复用，失败时只修复具体阻塞。
- 后续命令显式保留 context 和 namespace；只读权限失败不能通过切换生产集群规避。
- 命令示例中的尖括号是待确认参数，不能原样执行。先用当前版本帮助核对未知子命令、参数与返回结构，不编造选项。
- 用户已授权的操作继续完成；需要额外授权时先准备可审阅的具体目标和变更。操作失败保留错误，不扩大权限或范围重试。
- 以真实退出状态、响应内容和回读结果验收，给出实际产物路径/资源 ID；区分已完成与未验证项。

## 依据

- [项目仓库](https://github.com/kubernetes/kubectl)
- [使用与安装文档](https://kubernetes.io/docs/reference/kubectl/)

核对日期：2026-10-06。本技能是 Kivio 的独立适配说明，不代表上游官方插件或支持承诺。
