---
status: accepted
supersedes: 0005
---

# 子代理改用 OMP 设计

2026-11-05，用户确认：内置运行时的子代理整体改为 oh-my-pi（OMP，`can1357/oh-my-pi` commit `5dec65e`，MIT）的设计，原有实现放弃，不迁移、不兼容。实施计划见 [OMP 子代理完整实现计划](../prd/omp-subagents-plan.md)。2026-11-06 用户要求先完成源码调查和文档，再完整实现与测试，取代按三个阶段分别交付的安排。

源码契约与证据见 [OMP 子代理实现与运行调查](../research/omp-subagents-implementation.md)。本文是目标决定，不表示当前实现已通过对齐验收。

本决定取代 [ADR-0005](0005-subagents-retain-identity-across-tasks.md)。主要变化：

- 派工为批量 `task`，默认异步返回 id、结果自动送达父代理；保留 blocking 项、同步 fallback 与 speculative launch 的上游语义。父代理空闲时，送达结果会开启新的一轮；用户手动中断后抑制自动开轮，后续正常用户回合解除抑制。
- 子代理之间、父子之间通过 `write agent://<id>` 互发消息（IRC）。发送不等待回复；运行中的收件方在步骤边界接收，空闲的被唤醒开一轮，parked 的先复活。普通消息因此会启动空闲子代理。
- 子代理状态为 running / idle / parked / aborted；空闲 7 分钟释放会话并保留记录，发消息或打开时复活；kill 写入 tombstone。
- 子代理使用 OMP 的 unattended `tools.approvalMode: yolo`，但用户显式 `tools.approval` 策略仍生效；不是无条件绕过授权。
- `wait` 无参数，等待自己名下的作业或自己发出消息的回复。

## 取舍

- 接受"父空闲时自动开轮"：避免主代理为等子代理而阻塞整轮；代价是可能出现无用户输入的回合，需与用户输入共用同一条空闲预留路径。
- 接受无交互审批模式，但不移除用户策略：工具授权、只读限制与显式审批规则必须在子会话新建和复活时保留。
- 放弃 ADR-0005 的执行契约与“留言不唤醒”语义，换取 OMP 的身份保留、冷复活及恢复执行语义；不能把只恢复名单称为恢复执行。

## 后续影响

代理状态、单次运行状态、后台作业状态、task 调用聚合状态与投递消费标记分别表达，不得合并。提示词使用 OMP 模板及能力开关渲染，不能把不存在的工具写进静态文案。

允许符合 spawns 与深度限制的递归派工，替代旧实现“子代理永不递归”；默认 maxRecursionDepth=2。内置角色和旧工具名直接切换，不保留兼容别名；现行插件层、资源快照与外部 CLI 集成不因此删除。

2026-11-11 补充（用户确认）：

- 主代理 Act 模式按 OMP 系统提示词的 `# Delegation` 节渲染派工说明，力度取自 OMP 目录的模型 `delegation-bias`（`rules/classes/openai.kdl`）：GPT-6 及以上为 restrained（先自己做、不开局派侦察、不派单个切片），GPT-5.6 为 gated（未经要求不派子代理），其余为 eager。模板在 `sub_agent/prompts/delegation.md`，按本轮模型判定档位。不在 `task` 工具说明里另加文字。
- 删除 Orchestrate 模式。OMP 没有常驻的编排模式，派工力度由上述档位决定；模式只保留 Act 与 Plan。已保存为 `orchestrate` 的会话读取为 Act。
- 子代理的开始与结束事件都显示在派工所在的回复中，不并入之后无关的回合。
