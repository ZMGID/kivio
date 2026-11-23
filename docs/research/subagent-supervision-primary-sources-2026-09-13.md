# 子代理执行异常与主代理监督调查

调查日期：2026-09-13。范围：Kivio 当前工作区（HEAD `228a2085`，包含未提交的界面适配）与上游一手资料；本轮没有修改实现或规范。本地记录检查由主调查代理完成，本文整合其证据；不复制用户原始对话与个人目录信息。Codex 源码固定到 `b966240bea4210711884d488375bfed8c0297f54`，Claude 文档按调查日读取。

## 结论

用户指出的设计问题成立：一次执行异常结束，不能自动代表受委派的任务已经处理完。主代理必须拿到可用产出、异常原因和继续控制的机会，再决定接受结果、继续、改派或自己接手。底层仍需要如实记录网络、模型、权限、预算等执行异常；把所有异常统一改名为“已完成”同样不正确。

这张截图也不能证明主代理“干看着”。对应旧记录把恢复报告存进了 `error`，`result` 为空；父对话实际收到并使用了报告。报告约 8806 字符说明有产出，不能单凭长度证明任务已经满足。此前仅归因于历史卡片文案，解释不完整：真正需要审查的是执行结果、结果可用性与主代理处置之间的关系。

## 本地证据：有控制能力，缺少明确的处置闭环

| 证据 | 已证明的行为 | 设计含义 |
| --- | --- | --- |
| [旧规范](../prd/subagent-runtime-refactor-spec.md#L28)、[生命周期](../prd/subagent-runtime-refactor-spec.md#L90)、[A16](../prd/subagent-runtime-refactor-spec.md#L219) | 完成、失败、中断终结当前执行；失败报告算收齐结果，允许父代理说明缺失后结束。身份仍保留。 | 当前行为符合旧规范，但旧规范没有要求对未满足的委派目标作出明确处置。 |
| [worker_output](https://github.com/ZMGID/kivio/blob/228a208587ed8a9aa2e8d5e1b3e62a574987389b/src-tauri/src/chat/sub_agent/control.rs#L188)、[finish](https://github.com/ZMGID/kivio/blob/228a208587ed8a9aa2e8d5e1b3e62a574987389b/src-tauri/src/chat/sub_agent/runtime.rs#L716) | 当前已接受非空、非降级的恢复答复；其余走错误字符串并可记录为 `Failed`，同时释放运行资源。 | 运行资源释放合理；它不应兼任任务验收。旧记录不会因此自动重写。 |
| [恢复分类](https://github.com/ZMGID/kivio/blob/228a208587ed8a9aa2e8d5e1b3e62a574987389b/src-tauri/src/chat/agent/recovery.rs#L27)、[输出转换](https://github.com/ZMGID/kivio/blob/228a208587ed8a9aa2e8d5e1b3e62a574987389b/src-tauri/src/chat/sub_agent/control.rs#L188) | 共享恢复层已有 `DegradedAnswer`、`FailureKind`；子代理边界仅传递 `degraded.is_some()`，再压成 `Result<..., String>`。 | 已有恢复能力，但跨边界丢失结构化原因和部分产出的含义。 |
| [collect_results](https://github.com/ZMGID/kivio/blob/228a208587ed8a9aa2e8d5e1b3e62a574987389b/src-tauri/src/chat/sub_agent/control.rs#L51) | `pending` 跟踪活跃执行；终态报告选 `result` 或 `error`，持久投递后确认 delivered。 | 已投递表示父方收到，不表示主代理接受成果或安排补救。 |
| [父方 checkpoint](https://github.com/ZMGID/kivio/blob/228a208587ed8a9aa2e8d5e1b3e62a574987389b/src-tauri/src/chat/commands/agent_host.rs#L35)、[正常收尾](https://github.com/ZMGID/kivio/blob/228a208587ed8a9aa2e8d5e1b3e62a574987389b/src-tauri/src/chat/agent/loop_.rs#L378) | checkpoint 可收到异常报告；正常 `FinalAnswer` 遇到新结果会返回规划循环。 | 不能说主代理不能继续、完全没有控制通路。 |
| [轮数上限出口](https://github.com/ZMGID/kivio/blob/228a208587ed8a9aa2e8d5e1b3e62a574987389b/src-tauri/src/chat/agent/loop_.rs#L439)、[最后 checkpoint](https://github.com/ZMGID/kivio/blob/228a208587ed8a9aa2e8d5e1b3e62a574987389b/src-tauri/src/chat/agent/loop_.rs#L490)、[合成阶段](https://github.com/ZMGID/kivio/blob/228a208587ed8a9aa2e8d5e1b3e62a574987389b/src-tauri/src/chat/agent/synthesis.rs#L37) | 上限出口后的新报告直接进入合成；存在子代理持久消息时追加收尾指令，合成请求不提供工具。 | 此出口可能在刚收到需要处理的异常时失去继续控制机会；属于静态路径发现，本轮没有复现。 |

因此需要修的是“报告已收到”与“委派已处理”之间的缺口，不能把已有传输重试和子执行恢复误说成完全不存在，也不能只新增一种颜色或状态文字。

## 上游一手资料对照

**Codex 保留执行错误。** `TurnComplete.error` 映射成 `Errored`，正常结束映射成 `Completed`；中断另有状态，且不被 `is_final` 当作最终状态。这证明“有失败执行”是正常建模，不能推导为“用户目标已失败”。[状态转换源码](https://github.com/openai/codex/blob/b966240bea4210711884d488375bfed8c0297f54/codex-rs/core/src/agent/status.rs#L5-L28)

**通知不等于自动继续。** Codex watcher 等待终态并向父代理送信，订阅失败还会查最后状态；V2 完成通知明确使用 `trigger_turn=false`，V1 使用不启动回合的注入方法。不能借此宣称上游保证空闲父代理自动重试。[通知源码](https://github.com/openai/codex/blob/b966240bea4210711884d488375bfed8c0297f54/codex-rs/core/src/agent/control.rs#L590-L681)

**等待与控制是不同操作。** V2 `wait_agent` 等待邮箱活动、新输入或超时；超时是等待结果，不是子代理执行失败。`followup_task` 使用触发回合的投递模式，`interrupt_agent` 返回原状态。可参考这种分工，但“提供工具”本身仍不保证主代理一定处理异常。[wait](https://github.com/openai/codex/blob/b966240bea4210711884d488375bfed8c0297f54/codex-rs/core/src/tools/handlers/multi_agents_v2/wait.rs#L121-L189)、[followup](https://github.com/openai/codex/blob/b966240bea4210711884d488375bfed8c0297f54/codex-rs/core/src/tools/handlers/multi_agents_v2/followup_task.rs#L38-L45)、[interrupt](https://github.com/openai/codex/blob/b966240bea4210711884d488375bfed8c0297f54/codex-rs/core/src/tools/handlers/multi_agents_v2/interrupt_agent.rs#L67-L99)

**Claude 显式区分错误和部分成果。** 官方文档说明：后台子代理 API 错误终止时会标记失败，通知父代理时同时携带错误和最后输出；文本流中断可有限续写，配置的模型后备链可继续执行。不能把错误文本当研究结论，也不能因为存在部分文本就宣称完整成功。[API errors in subagents](https://code.claude.com/docs/en/sub-agents#api-errors-in-subagents)

**执行限额不代表工作完成。** Claude SDK 达到子代理 `maxTurns` 后会标记部分结果，恢复可保留历史；可恢复代理还需保留对应会话和代理标识。[Resume subagents](https://code.claude.com/docs/en/agent-sdk/subagents#resume-subagents)

## 建议：把监督变成可检查的行为

以下为本项目建议，不是上游既有能力保证。

1. **分开三个事实。** 执行记录保存结束原因、异常分类及原始产出；成果记录说明完整、部分、不可用或待验收；父代理记录接受、继续、改派、自行接手、取消或等待用户。持续存在的子代理身份不能永久等同于某次执行的 `Failed`。
2. **未处置结果必须进入决策。** 投递确认只去重通知，不自动清除委派责任。预算允许时回到可调用工具的入口；达到用户规定的轮数或预算上限时保留可用部分成果，明确阻塞或需要用户处理，不能重置限额强行继续。主代理收到部分结果或异常后作出明确处置；必要任务未解决时不走普通成功收尾。上限出口没有工具不必然是实现错误，缺少处置语义才是问题。
3. **有限恢复，避免机械重跑。** 网络瞬态可由现有传输层在重试预算内处理；任务问题由主代理判断补充上下文、继续原代理、换模型或接手。副作用结果不明时先检查再重复；权限、配置或用户停止等原因按实际条件等待处理。相同原因反复出现且无新条件时停止无效重试；已有结果足够时允许接受，但记录理由。无需所有任务都自动重跑。
4. **卡片显示已发生的事实。** 有产出显示结果入口和可用性；只有已经排队或正在执行处置时才显示“主代理处理中”。尚未处置应显示“等待处理”，确实依赖用户才显示“需用户处理”。历史 `recovered:` 要按实际记录兼容，不仅凭前缀认定成功，也不伪造已发生的处理。

验收应覆盖生产父方 checkpoint 与可控 worker：恢复报告被正确保留并接受；部分成果触发继续并关联新执行；不可恢复异常导致改派、接手或明确阻塞；等待超时不改子状态；停止不自动重启；轮数上限时新异常不能悄悄进入无工具成功总结；旧报告不覆盖新执行。现有只注入已完成 runtime 消息的汇总测试，不能替代这些监督链路测试。

下一步应先修订旧规范 A16 和处置契约，再一起调整后台控制出口与界面投影；仅修改“失败”标签不足以兑现主代理监督责任。
