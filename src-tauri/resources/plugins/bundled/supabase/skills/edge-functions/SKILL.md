---
name: edge-functions
description: "开发、测试并部署指定 Supabase Edge Function。"
---

# Edge Functions

## 工作流

确认函数目录、导入依赖、JWT 验证与环境变量名称；不要读取密钥内容。按帮助使用 functions serve，在明确授权后部署到指定 project-ref。部署完成验证函数列表和最小请求，不向生产写入测试业务数据。

## 执行约定

- 使用 Kivio 的现有终端/文件工具执行，按实际任务加载此技能；不要假定整个插件只有一个固定入口。
- 首次使用或依赖、账号、项目变化时加载 `supabase:setup`；同一任务已验证环境可复用，失败时只修复具体阻塞。
- supabase status 可能输出本地 API Key；只读取必要字段。db reset/db push 不属于只读验证。
- 命令示例中的尖括号是待确认参数，不能原样执行。先用当前版本帮助核对未知子命令、参数与返回结构，不编造选项。
- 用户已授权的操作继续完成；需要额外授权时先准备可审阅的具体目标和变更。操作失败保留错误，不扩大权限或范围重试。
- 以真实退出状态、响应内容和回读结果验收，给出实际产物路径/资源 ID；区分已完成与未验证项。

## 依据

- [项目仓库](https://github.com/supabase/cli)
- [使用与安装文档](https://supabase.com/docs/reference/cli/)

核对日期：2026-10-06。本技能是 Kivio 的独立适配说明，不代表上游官方插件或支持承诺。
