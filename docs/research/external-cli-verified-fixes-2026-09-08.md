# 外部 CLI 代理适配复核与修复（2026-09-08）

本次复核 Claude Code 2.1.263、Codex 0.153.4、Pi 0.85.1、Grok Build 1.0.13。本机安装版本依次为 2.1.260、0.149.1、0.84.4、1.0.13；未升级用户全局 CLI。

## 已确认并修复

- **Codex 异步问题**：0.153 的 `agentMessage.questions` 是 `{title, options: string[] | null}[]`，不是反向 RPC。完成帧生成非阻塞问题卡，重复完成帧去重；用户通过选项或自由文本作答，答案进入普通消息队列。生成中可继续运行，空闲时发送。卡片不抢输入框焦点；跳过只收起卡片；后续用户消息使旧问题失效。旧版没有该字段时行为不变。问题随工具记录保存，答案作为用户消息保存；本地队列及仅收起状态沿用现有内存生命周期。
- **Pi 思考等级**：加入 `max`，优先用只读 RPC `get_available_models` / `get_state` 获取每模型 `thinkingLevelMap` 与当前模型。映射规则与上游 `getSupportedThinkingLevels` 一致：`null` 禁用，`xhigh/max` 要求显式映射，非推理模型隐藏档位。`--no-session` 不创建会话文件，不调用会修改设置的 `set_model`。RPC 不可用时保留旧 `--list-models` 路径。
- **Grok 权限**：增加“工具请求时确认”和“完全放行（默认）”。询问档不传 `--always-approve`，运行中的 ACP `session/request_permission` 交给现有审批宿主，批准只回 `allow_once`，拒绝/取消/宿主缺失回 `cancelled`。保留原有完全放行默认值及 `--no-leader`。选项不完整时也必须回复，避免挂起。
- **Claude 模型目录**：更新 Fable 5.1，并同时更新正常模型目录、备用目录、配置别名映射及供应商覆盖测试。正常模型目录实际来自本地四档目录与 settings/env 覆盖，并非 `system/init` 自动发现，因此仅改备用表不够。

## 修正上一轮判断

- Grok 的 `agent` 子命令在 1.0.13 **没有** `--permission-mode` 或 `--sandbox`。上一轮建议直接传这些参数不成立。只提供实测支持的询问/放行能力，不把交互式 CLI 的模式冒充成可用启动参数。
- Codex 新增 thread `model` / `reasoningEffort` 是可用于展示的元数据。当前续聊只发 `threadId`，单轮明确选择才在 `turn/start` 覆盖，未读取新元数据不等于丢失原生会话配置。本次没有把可选展示增强当协议错误修改。
- Claude `--append-subagent-system-prompt-file` 是 2.1.261 新增的可选控制，本机 2.1.260 尚不支持。没有确认现有子代理规则丢失，不无条件添加新参数。

## 验证证据

- 最终外部代理 Rust 回归：721 passed、0 failed、43 ignored；前端相关回归：86 passed。`npm run typecheck`（含协议一致性检查）、改动前端文件的 ESLint 与 `git diff --check` 均通过。
- Pi 本机只读 RPC：`get_available_models` 成功，5 个模型中 4 个包含 `thinkingLevelMap`；`get_state` 成功，`sessionFile` 为 null。
- Rust 回归覆盖 Codex 完成帧去重/旧协议兼容、Pi 档位空洞/扩展档位、Grok 批准/拒绝/取消/无宿主及请求 id 回显。
- 前端回归覆盖异步问题内联作答、不抢焦点、纯文本、跳过及失效卡片，并运行原有工具卡、权限菜单、消息队列测试。
- 未执行需要模型推理的四 CLI 完整实时对话；协议样本测试不等同于新版 Codex 真机端到端测试。

## 上游依据

- [Claude Code changelog](https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md)
- [Codex 0.153.4](https://github.com/openai/codex/releases/tag/rust-v0.153.4)；[AsyncUserInputQuestion schema](https://github.com/openai/codex/blob/rust-v0.153.4/codex-rs/protocol/src/items.rs)
- [Pi 0.85.1 thinking-level rules](https://github.com/earendil-works/pi-mono/blob/v0.85.1/packages/ai/src/models.ts)；[RPC implementation](https://github.com/earendil-works/pi-mono/blob/v0.85.1/packages/coding-agent/src/modes/rpc/rpc-mode.ts)
- [Grok Build changelog](https://x.ai/build/changelog)；本机 `grok agent --help`（1.0.13）。Grok Build 与非官方 `grok-dev` 是不同产品。
