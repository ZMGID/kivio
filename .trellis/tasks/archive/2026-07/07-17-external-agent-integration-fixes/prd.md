# 修复外部 Agent 集成问题

## Goal

修复 GitHub issue #17 与 #16，使外部 Agent 在结束阶段和模型发现阶段都遵循其原生协议，避免把正常完成误报为失败，并让 OpenCode 自定义模型可被选择。

## Background

- #17: Pi 已输出有效结果并发送 `agent_end`，随后因 Kivio 提前释放 stdout 读取端而在收尾写入时触发 `EPIPE`。
- #16: OpenCode ACP 模型探测失败时，Kivio 静默回退到四个硬编码模型，因此自定义 provider/model 不会出现在选择器中。
- 两项工作可独立验证，分别由子任务 `07-17-pi-epipe-shutdown` 与 `07-17-opencode-custom-model-discovery` 负责。

## Requirements

- R1. Pi 正常发出 `agent_end` 后，Kivio 必须允许子进程完成 stdout 收尾，不得制造 `EPIPE`。
- R2. Pi 真正异常退出、取消或协议错误仍必须保留现有错误反馈，不能把所有非零退出都吞掉。
- R3. OpenCode 模型列表必须优先来自 OpenCode 自己解析后的配置结果，而不是依赖静态内置列表。
- R4. ACP 探测不可用时必须有兼容性 fallback，并保留最终静态列表作为最后兜底。
- R5. 两项修复都需要针对根因的自动化回归测试。
- R6. OpenCode 模型发现同时覆盖用户全局配置与当前项目目录配置；项目检测缓存不得污染其他项目。

## Acceptance Criteria

- [x] AC1. 模拟 Pi 在 `agent_end` 后继续写入收尾数据时，运行结果成功且子进程不发生断管错误。
- [x] AC2. Pi 取消、协议错误和真实非零退出仍保留错误状态。
- [x] AC3. OpenCode 原生命令输出中的自定义 `provider/model` 会出现在检测结果中并去重。
- [x] AC4. OpenCode 原生探测失败时会继续尝试 ACP，所有动态探测都失败时才使用静态模型。
- [x] AC5. Rust 全仓测试、前端测试、类型检查、lint 和本次文件格式检查通过。
- [x] AC6. 不同 cwd 使用独立检测/模型缓存，并有缓存隔离与 stale-response 防护。

## Out of Scope

- 不自动回复、关闭或修改 GitHub issue。
- 不自行解析 OpenCode JSONC/provider schema；配置合并与 JSONC 兼容性由 OpenCode CLI 负责。

## Technical Notes

- 用户已确认本轮同时支持全局和项目级 `opencode.jsonc`。
- 执行顺序为先修复 #17，再修复 #16；两个子任务独立激活、验证和归档，父任务负责最终集成检查。
