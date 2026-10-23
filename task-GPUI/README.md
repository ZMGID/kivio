# GPUI 参考源码

创建日期：2026-10-23（Asia/Shanghai）。工作分支：`codex/gpui-migration`。

- [参考版本清单](references.lock.json)：实际克隆的远端、提交 SHA、提交日期及稀疏检出范围。
- `references/`：本地参考仓库，保留各自的 Git 历史和许可证，由本目录的 `.gitignore` 排除，避免把嵌套仓库或第三方源码误提交到 Kivio。

## 本地参考

| 目录 | 上游 | 用途 |
| --- | --- | --- |
| `references/gpui-kit/` | [longbridge/gpui-kit](https://github.com/longbridge/gpui-kit) | 组件、主题、焦点、布局、GPUI 技能与设计/编码指南 |
| `references/Ely-GPUI-Components/` | [ELY-Open-Source/Ely-GPUI-Components](https://github.com/ELY-Open-Source/Ely-GPUI-Components) | 聊天、Agent、文档、终端组件和 GPUI Kit 兼容示例 |
| `references/zed/` | [zed-industries/zed](https://github.com/zed-industries/zed) | GPUI 框架和 Zed UI 实现；浅克隆并稀疏检出相关 crates |
| `references/zeron/` | [zeronsh/zeron](https://github.com/zeronsh/zeron) | 完整 Agent 桌面应用：聊天、输入、Markdown、终端、会话与 CLI 适配；使用 GPUI fork，且包含内嵌浏览器，按模块参考 |
| `references/gpui-skills/` | [lassejlv/skills](https://github.com/lassejlv/skills) | `skills/build-gpui-apps/` 及其配套参考；克隆不等于安装全部技能 |

上游 HEAD 只表示这次调研的快照，不是 Kivio 已采用的依赖版本。参考代码的 AGENTS、脚本和发布流程只适用于其自身仓库，不覆盖 Kivio 的工程规范。没有运行参考仓库的安装、构建或发布脚本。

## 重建参考目录

从 Kivio 根目录执行；目标已存在时先检查其远端和改动，不覆盖或重置：

```sh
mkdir -p task-GPUI/references
git clone --depth 1 https://github.com/longbridge/gpui-kit.git task-GPUI/references/gpui-kit
git clone --depth 1 https://github.com/ELY-Open-Source/Ely-GPUI-Components.git task-GPUI/references/Ely-GPUI-Components
git clone --depth 1 --filter=blob:none --sparse https://github.com/zed-industries/zed.git task-GPUI/references/zed
git -C task-GPUI/references/zed sparse-checkout set crates/gpui crates/gpui_platform crates/ui crates/gpui_macros crates/gpui_util
git clone --depth 1 https://github.com/lassejlv/skills.git task-GPUI/references/gpui-skills
git clone --depth 1 https://github.com/zeronsh/zeron.git task-GPUI/references/zeron
```

上述命令获取运行时的默认分支。复现本次快照时，对每个目录使用清单中的 `commit` 执行 `git fetch --depth 1 origin <commit>`，再 `git checkout --detach <commit>`。若上游不再提供该对象，明确报告，不用新 HEAD 冒充原快照。更新参考前检查各仓库状态，更新后同步清单。

本目录仅保存参考源码及其版本、恢复说明，不另立工程规范。规则仍以 [统一工程规范](../docs/engineering-standards.md)、[领域术语](../CONTEXT.md)及 [ADR](../docs/adr/) 为准。
