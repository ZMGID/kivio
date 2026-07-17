# 支持 OpenCode 自定义模型探测

## Goal

让 Kivio 展示 OpenCode 配置中实际可用的自定义 provider/model，而不是在 ACP 探测失败时只展示四个硬编码模型。

## Background

- GitHub issue: https://github.com/ZMGID/kivio/issues/16
- 截图中的 `default`、Anthropic、OpenAI、Google 四项与 `src-tauri/src/external_agents/defs/acp.rs:88-93` 的静态 fallback 完全一致，说明动态探测没有产生有效模型。
- `src-tauri/src/external_agents/detection.rs:139-143` 当前只通过 ACP `session/new` 探测，并以系统临时目录作为 cwd。
- OpenCode 提供 `opencode models` 命令，由 OpenCode 自己加载、合并和解析 `opencode.jsonc`/`opencode.json` 后输出 `provider/model`。

## Requirements

- R1. 不在 Kivio 中复制 OpenCode 的 JSONC 配置解析与 provider 合并规则。
- R2. 使用 OpenCode 原生模型列表命令发现自定义模型，解析非空 `provider/model` 行并去重。
- R3. 原生命令失败、超时或无有效输出时继续使用 ACP 探测。
- R4. 所有动态探测失败时保留当前静态 fallback，避免模型选择器完全不可用。
- R5. 探测必须有超时且不弹出控制台窗口。
- R6. 模型探测必须使用当前会话的有效工作目录，使全局配置和项目目录中的 `opencode.jsonc` 都由 OpenCode 加载。
- R7. 检测结果与模型元数据缓存必须按工作目录隔离，避免不同项目之间串用模型列表。

## Acceptance Criteria

- [x] AC1. 全局或当前项目的配置产生自定义渠道时，`opencode models` 输出中的相应 `provider/model` 会进入检测结果。
- [x] AC2. 重复、空行或无效行不会生成重复/空模型。
- [x] AC3. 原生命令非零退出、超时或空输出时会回退到 ACP。
- [x] AC4. ACP 同样失败时仍返回当前静态 fallback。
- [x] AC5. 模型选择和实际启动仍使用完整 `provider/model` ID。
- [x] AC6. 解析、空结果、external_agents 全范围及全仓自动化测试通过。
- [x] AC7. 不同项目目录使用不同缓存键，UI 会清空旧结果并忽略过期异步响应。

## Out of Scope

- Kivio 自行解析 OpenCode JSONC schema。
- 修改 OpenCode 的配置文件或认证信息。

## Technical Notes

- 用户已确认同时支持全局和项目级配置。
- 设置页没有会话上下文时使用进程当前目录；OpenCode 仍会加载用户全局配置。
