---
name: container-diagnostics
description: "检查容器健康、日志和 Compose 状态。"
---

# 容器诊断

## 工作流

用 `docker ps -a` 定位目标；按需 `docker logs --tail 100 <container>` 和针对性 `docker inspect --format ...`。不要整份输出可能含凭证的 Env。Compose 项目先确定目录和配置文件，用 `docker compose ps`。给出状态、错误证据与下一步，默认不 restart。

## 执行约定

- 使用 Kivio 的现有终端/文件工具执行，按实际任务加载此技能；不要假定整个插件只有一个固定入口。
- 首次使用或依赖、账号、项目变化时加载 `docker:setup`；同一任务已验证环境可复用，失败时只修复具体阻塞。
- Docker Desktop 的许可条件与 Engine 不同；遵循用户已有部署方式，不自动替换 Docker context。
- 命令示例中的尖括号是待确认参数，不能原样执行。先用当前版本帮助核对未知子命令、参数与返回结构，不编造选项。
- 用户已授权的操作继续完成；需要额外授权时先准备可审阅的具体目标和变更。操作失败保留错误，不扩大权限或范围重试。
- 以真实退出状态、响应内容和回读结果验收，给出实际产物路径/资源 ID；区分已完成与未验证项。

## 依据

- [项目仓库](https://github.com/docker/cli)
- [使用与安装文档](https://docs.docker.com/reference/cli/docker/)

核对日期：2026-10-06。本技能是 Kivio 的独立适配说明，不代表上游官方插件或支持承诺。
