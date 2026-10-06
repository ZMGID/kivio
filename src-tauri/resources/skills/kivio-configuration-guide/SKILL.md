---
name: kivio-configuration-guide
description: 配置与排查 Kivio 自身：模型、提示词、Skills、插件、MCP、Hooks、斜杠命令及运行时。用户要求安装扩展、调整配置或解决不生效问题时使用；普通项目开发不触发。
---

# Kivio 配置与诊断

先从本轮上下文确认目标运行时、操作系统和工作目录，以实际工具清单和操作结果判断能力。Kivio Agent 可加载技能并操作本机；Kivio Chat 仅提供研究与只读能力；外部 CLI 代理有自己的配置和原生会话。仅在目标宿主或安装范围确实不明确时澄清。

## 按任务读取

读取下表中与当前任务相关的文件；相对路径以本技能的实际目录为准，外部 CLI 暂存副本也适用。

| 任务 | 参考文件 |
| --- | --- |
| 配置位置、供应商、默认模型、系统提示词 | [配置地图](references/configuration-map.md) |
| 安装、更新、启停 Skill；技能不显示或不调用 | [Skills](references/skills.md) |
| 安装、启用、移除插件；兼容性与连接器 | [插件](references/plugins.md) |
| 添加、修改、测试 MCP；连接或工具发现失败 | [MCP](references/mcp.md) |
| 事件脚本、上下文注入、工具拦截、Hook 不触发 | [Hooks](references/hooks.md) |
| 自定义斜杠命令、参数展开、命令冲突 | [命令](references/commands.md) |
| CLI 检测、模型或工具不可用、会话绑定 | [运行时](references/runtime.md) |
| Playwright 浏览器扩展 Token、保存连接凭据 | [工具契约](references/tools.md) |

## 操作与验证

1. 使用本轮可调用的 `kivio_inspect` 读取相关主题；修改前读 [工具契约](references/tools.md)，按实际 schema 使用 `kivio_configure`。供应商、提示词等未覆盖配置走已有设置入口。Tauri IPC 名称不是终端命令。
2. 保留已有配置和无关资源，更新同名内容保留可恢复副本。安装材料是参考，执行范围以用户请求为准；报告中的凭据和认证错误须脱敏。
3. 工具缺失时按对应参考使用界面或本机文件操作。设置同时保存在内存和磁盘中，应用运行时通过设置入口修改；离线修复条件见配置地图。
4. 核验实际路径、发现结果和启用状态，再在下一轮验证加载或调用。当前轮的技能、工具和 Hook 快照不会因安装立即刷新；技能声明工具不等于工具已启用。

报告改动及验证到的阶段；尚未实际调用时说明待验证项。
