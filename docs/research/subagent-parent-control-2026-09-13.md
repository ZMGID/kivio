# 子代理协作：把控制权交还主代理

调查日期：2026-09-13。本地依据为 HEAD `7c5b3f56` 加当前工作区；源码行号随修改可能变化。本次只写研究报告，没有修改生产代码。问题范围是日常主代理—子代理协作，不是要求每行固定结构、统一评分的批处理系统。

## 结论

用户指出的核心问题成立：**子代理输出是交给主代理使用的信息，框架不应再要求主代理为每次输出登记“接受／补齐／阻塞／改派”，以登记结果决定主对话能否继续或结束。** 调查没有发现一手资料能够证明这种强制验收台账是实现协作的必要条件。

这里的“输出就是结果”不应被实现为“任何执行都自动成功”。应直接取消日常协作中的框架任务判决：保存输出；同时如实保存 API 中断、用户停止、预算耗尽等运行事实；由主代理决定这些信息如何影响下一步。输出包含“没有找到文件”，仍然是调查结果；模型接口没有返回内容，是运行事件，不能伪造研究结果。两者都不应自动终止仍能工作的主代理。

## 一手资料确实说了什么

**OpenAI 官方文档：主代理负责要求、决策和最终输出。** 子代理返回摘要；Codex 编排包括启动、后续指令、等待和关闭；可以要求它调整或停止正在运行的子代理。无法交互申请新权限时，错误返回父工作流。文档并未规定“结果必须先通过框架验收”。这足以支持职责分工，不能据此宣称它的所有退出和自动唤醒路径都已验证。[Subagents：职责与控制](https://learn.chatgpt.com/docs/agent-configuration/subagents#orchestration-and-thread-controls)、[权限](https://learn.chatgpt.com/docs/agent-configuration/subagents#approvals-and-sandbox-controls)。

**Claude Code：可纠偏、可继续，并保留真实异常。** 官方文档说明 `SendMessage` 支持原代理保留历史继续，启动者的途中消息属于任务方向调整；后台 API 异常通知带上最后输出；达到 `maxTurns` 可继续。用户亲自停止的代理不能由其他代理自动恢复。这里既有运行状态，也有返回给主代理的内容，不能把运行标签当作任务价值判决。[Resume subagents](https://code.claude.com/docs/en/sub-agents#resume-subagents)、[API errors in subagents](https://code.claude.com/docs/en/sub-agents#api-errors-in-subagents)。

**Anthropic Research 的经验支持让模型处理变化，但不适合整套照抄。** 2025-06-13 的工程文章描述主代理综合发现并决定是否继续研究，工具异常交给模型调整，并配合重试和检查点。文章同时明确：当时采用同步批次，主代理不能途中调整子代理，单个慢代理可能挡住整体进度；这是文章承认的局限，并不是今天要照搬的协作方案。文章提到的 pass/fail 属于评估实验，也不能直接变成线上每次子输出的结束门槛。[How we built our multi-agent research system](https://www.anthropic.com/engineering/multi-agent-research-system)。

以上均实际打开官方页面核对。本轮按 OpenAI Docs 技能限定使用其官方文档域名，未重新核验 Codex GitHub 固定提交；旧研究报告中的源码描述不作为此次已复核事实。Claude 页面为调查日版本，不保证旧安装版本具备全部行为。

## 当前 Kivio 的问题不是缺几个控制工具

本地已经能启动、留言、继续、停止、读取和等待。真正的问题是新增了一层强制任务判决，并把它连接到了主轮退出。以下为源码检查，不是本轮真实接口复现。

| 环节 | 当前代码事实 | 后果 |
| --- | --- | --- |
| 创建执行 | 每次执行设置 `requires_review: true`，结束且无 `resolution` 即 `needs_review`。[runtime.rs](https://github.com/ZMGID/kivio/blob/7c5b3f5629e6c13076aa739210d7c6a7e5103ab8/src-tauri/src/chat/sub_agent/runtime.rs#L129)、[初始化](https://github.com/ZMGID/kivio/blob/7c5b3f5629e6c13076aa739210d7c6a7e5103ab8/src-tauri/src/chat/sub_agent/runtime.rs#L1130) | 普通输出默认变成待验收事项。 |
| 主代理收到结果 | `supervision_message` 注入 system 要求，指定 `resolve` 并禁止先给最终回答。[runtime.rs](https://github.com/ZMGID/kivio/blob/7c5b3f5629e6c13076aa739210d7c6a7e5103ab8/src-tauri/src/chat/sub_agent/runtime.rs#L391) | 接收信息被改造成强制走业务流程。 |
| 登记处置 | `resolve` 限定四类判决及理由，还校验继任执行和是否存在输出。[runtime.rs](https://github.com/ZMGID/kivio/blob/7c5b3f5629e6c13076aa739210d7c6a7e5103ab8/src-tauri/src/chat/sub_agent/runtime.rs#L411) | 主代理即使已经使用结果，也要满足额外台账格式。 |
| 等待其他分支 | `wait` 将未登记结果作为 `needs_review` 返回，独立于报告是否已经投递、游标是否推进。[control.rs](https://github.com/ZMGID/kivio/blob/7c5b3f5629e6c13076aa739210d7c6a7e5103ab8/src-tauri/src/chat/sub_agent/control.rs#L385) | 已收到的 A 可能反复干扰等待 B；传输与验收耦合。 |
| 尝试结束 | 检测到监督消息累计超过两次，或达到工具轮数上限，退出规划。[loop_.rs](https://github.com/ZMGID/kivio/blob/7c5b3f5629e6c13076aa739210d7c6a7e5103ab8/src-tauri/src/chat/agent/loop_.rs#L390) | 纠偏次数成为结束触发器，未必与用户目标有关。 |
| 替代最终答复 | `finalize_supervision` 自动给无处置记录填 `blocked`，包括仍在运行的执行；`collaboration_blocked_result` 生成“协作尚未完成”。[runtime.rs](https://github.com/ZMGID/kivio/blob/7c5b3f5629e6c13076aa739210d7c6a7e5103ab8/src-tauri/src/chat/sub_agent/runtime.rs#L477)、[loop_.rs](https://github.com/ZMGID/kivio/blob/7c5b3f5629e6c13076aa739210d7c6a7e5103ab8/src-tauri/src/chat/agent/loop_.rs#L555) | 框架代替主代理决定并书写结论。 |
| 清理 | 生产 `ChatAgentHost::run_ended` 不区分退出原因，调用 `stop_parent`。[agent_host.rs](https://github.com/ZMGID/kivio/blob/7c5b3f5629e6c13076aa739210d7c6a7e5103ab8/src-tauri/src/chat/commands/agent_host.rs#L27) | 上述主轮退出会继续停止其他分支。 |
| 界面 | `requiresReview` 投影成“等待主代理验收／处理”。[subAgentStatus.ts](https://github.com/ZMGID/kivio/blob/7c5b3f5629e6c13076aa739210d7c6a7e5103ab8/src/chat/subAgentStatus.ts#L13) | 即使输出已可用，仍展示人为的验收关卡。 |

因此准确因果不是“任何子代理报错就立即停止主代理”，而是：**子输出进入强制验收 → 未登记可能触发主轮收尾 → 统一退出清理停止其他子代理。** 此前那次实际 HTTP 400 是主代理模型请求出错，属于另一条触发主轮退出的路径，不应混为同一个问题。

`worker_output` 也把带降级标记的输出转换为 `Err`，但 `partial` 会保存正文，不能误称所有这些输出已经丢失。[输出转换](https://github.com/ZMGID/kivio/blob/7c5b3f5629e6c13076aa739210d7c6a7e5103ab8/src-tauri/src/chat/sub_agent/control.rs#L253)、[保存结果](https://github.com/ZMGID/kivio/blob/7c5b3f5629e6c13076aa739210d7c6a7e5103ab8/src-tauri/src/chat/sub_agent/runtime.rs#L931)。需要调整的是跨层表达和控制后果，不只是改一个颜色。

## 对上次研究和规范的修正

此前[监督研究](subagent-supervision-primary-sources-2026-09-13.md)把“主代理应当处理结果”进一步推成“成果可用性状态＋必须处置的台账”，[规范 5.1](../prd/subagent-runtime-refactor-spec.md#L132)随后固化了这套门槛。**前者是合理的代理行为要求，后者不是从引用资料必然推出的设计，而且与用户当前明确要求相违背。** 本报告明确修正这项建议；不是在原门槛上继续增加自动补救规则。

保留早先[身份与历史 ADR](../adr/0005-subagents-retain-identity-across-tasks.md)的有效部分：子代理可继续、执行结束释放资源、用户单停只影响对应分支、主停止覆盖当前协作、重启保留历史但不自动运行。该 ADR 默认要求主代理收齐所需结果后结束，没有授权任意自动后台重启；新设计也不应暗改此约定。

## 建议的控制流程（项目方案，不是上游能力保证）

1. 主代理拆分工作、派出子代理，同时继续自己能做的工作。
2. 子代理报告或运行事件进入父方收件箱。父代理下一个可决策边界收到正文、来源、执行 ID、结束原因；投递确认只负责去重。
3. 主代理根据用户目标自由选择使用结果、追问、改方向、继续原代理、让其他代理接手、自己处理或停止某分支。不需要额外 `resolve` 才能往下走。
4. 仍需要某个运行中结果时，主代理等待；新报告或新用户消息唤醒当前等待。等待超时表示暂时没有新消息，不改变子执行状态。
5. 主代理形成最终答复。对于仍在运行的分支，主代理可以继续等待，也可以主动停止已无必要的工作。这是调度行为建议，不是新的答复前置条件：框架不得因为存在活跃子代理、没有调用某个管理工具而拦截回答；也不能生成“未完成”替代回答或悄悄清理其他分支。

建议保留三个互不代替的数据概念：`output`（内容和来源）、`run_state/end_reason`（运行事实）、`delivery_cursor`（送达事实）。继续原代理的关系使用执行 lineage；它不需要表达“前一结果被接受还是失败”。历史 `resolution` 可仅供兼容读取，不再驱动新执行。

应删除／退役的对象包括强制 `requires_review`、`resolve` 业务判决、监督提醒计数退出、自动填写 `blocked`、替代最终回答以及“待验收”界面分组。普通自然语言结果可直接展示“有新结果／已返回”；接口中断显示事实和已保存内容。不要把旧的“未完成”统一替换成“成功”，也不要把已处理信息重新包装成必须处理的任务。

## 主轮与整个协作的生命周期必须拆开

“不因子代理事件全停”不能只改一个异常分支：`run_ended` 当前把主模型回合结束和整个协作取消视为同一件事。建议使用明确的退出原因和明确的协作关闭动作；用户停止、应用退出、不可恢复的主请求中断、正常最终答复分别处理。

当前模式优先保持同一主协作在运行：主模型有待结果时等待，报告回到同一控制循环。一般子运行结束或报错不触发整体取消。主模型 API 暂时不可用时，界面如实显示主代理请求中断并保存收到的报告；不能声称没有可用主模型仍能持续自主决策，也不能将它改写为所有子任务失败。

若希望主轮先返回、子代理继续后台运行，再由报告自动开启父轮，必须另做明确产品模式，定义唤醒、去重、归属和预算。当前默认约定不采用它。父方暂停期间到达的报告可先持久排队，用户明确继续后处理；用户停止或应用重启后的存量消息不得偷偷唤醒执行。另一方面，保留子代理历史并不等于保留执行进程；界面新版化也不要求删除用户历史。

## 验证应测控制权而不是验收格式

- A 返回“未找到文件”，B 仍工作：主代理能使用 A 的信息或追问，继续等 B；整个过程不调用 `resolve`。
- A 返回“需要额外信息”：正文正常投递，主代理根据已有上下文补充方向；不自动判定总体阻塞。
- A 的 API 中断且有部分正文：正文和异常原因同时抵达父方，B 不受影响；空正文只报告运行事件。
- A 的结果已送达后等待 B：不能再次因为 A 没有“验收”立即返回同一旧报告。
- 主代理直接使用已收到结果给出有用答复、没有调用 `resolve`：答复必须保留，省略登记不能触发取消，也不能被“协作尚未完成”替换。
- 原子代理结束后可携带历史继续；运行途中可补充／调节；任一操作都不要求事先声明前次任务成功或失败。
- 用户停止单个分支不影响其他分支；用户停止整个协作仍覆盖全部；重启不会隐式恢复执行。
- 主模型 API 出错单独显示来源和详情；不能用“子任务未完成”隐藏原因。正常主答复不被框架文字替换。

这些测试应走生产 `ChatAgentHost` 的 checkpoint 和退出路径。`ProbeAgentHost` 缺少对应完整生命周期实现；此前接口 200 或报告角色测试通过，只能证明消息格式相关路径，不能证明整个调度流程。[生产与探针宿主](https://github.com/ZMGID/kivio/blob/7c5b3f5629e6c13076aa739210d7c6a7e5103ab8/src-tauri/src/chat/commands/agent_host.rs#L268)。已有模拟监督测试即使通过，若断言的就是“未登记则合成未完成”，也只是在验证被用户否定的旧策略；这些预期必须随行为一起修正。

本次结论来自官方文档和当前源码的静态对照，没有在桌面执行完整协作回归。后续实施应同步删除规范中的强制验收要求，再调整运行时、工具、主循环、界面和相应测试；仅修改提示词不能解决已经编码的退出链路。
