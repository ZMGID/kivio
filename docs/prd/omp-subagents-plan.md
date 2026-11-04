# 子代理改用 OMP 实现：三步计划

日期：2026-11-05。分支：`feat/omp-subagents`。决定见 [ADR-0007](../adr/0007-subagents-follow-omp.md)。

## 目标与边界

把内置运行时的子代理整体替换为 oh-my-pi（OMP）的子代理设计，行为以 OMP 源码为准：`can1357/oh-my-pi` commit `5dec65ed5f500ec456c2c7f91f7e517f99d92eec`（MIT，见 `docs/licenses/oh-my-pi-MIT.txt`）。下文 `omp:` 前缀路径均相对其 `packages/coding-agent/src/`，`omp-tui:` 相对 `packages/tui/src/`。

- 旧实现（`agent` / `agent_control`、持久 runtime、`chat_subagent_control`、`SubAgentPanel` 一套）直接删除，不迁移、不兼容。历史对话里的旧工具卡按普通工具卡显示。
- 外部 CLI 代理（`external_agents`）不受影响。
- 不做（OMP 默认关闭或与子代理无关）：worktree 隔离、structured/`yield`/outputSchema、per-call model、seance、workpool、prewalk、advisor、speculative launch、eval 工具共享、collab。
- 提示词原文从 OMP `prompts/` 拷贝，只删去上面未实现功能的分支；不改写措辞。

## 三套状态（不得合并）

| 状态 | 取值 | 归属 |
|---|---|---|
| 运行 run | pending / running / completed / failed / aborted | 进度事件、task 卡片 |
| 代理 registry | running / idle / parked / aborted | 代理注册表、Agent Hub |
| 作业 job | running / completed / failed / cancelled | 作业管理器、`wait`、送达 |

## 第一步：派工、执行、结果送达、wait

删除 `src-tauri/src/chat/sub_agent.rs`、`sub_agent/control.rs`、`sub_agent/runtime.rs`、`chat_subagent_control`、编排模式子代理叠加段（`chat/plan.rs`）、旧内置角色，以及前端对它们的调用（前端显示的重做在第三步；本步只保证编译和不报错）。

新模块放在 `src-tauri/src/chat/sub_agent/`：

| 模块 | 行为（OMP 依据） |
|---|---|
| `registry` | `AgentRef { id, generation, parent_id, kind: main/sub, status, session_file, last_activity, activity }`；`list_visible_to`。(`omp:registry/agent-registry.ts`) |
| `jobs` | owner 归属作业、送达队列、`watch`/`consume` 去重；未消费保留 5 分钟，已消费 30 秒。(`omp:async/job-manager.ts`、`job-control.ts`) |
| `task` 工具 | 入参 `{context, tasks[{name?, agent?, task, solutionSpace}]}`；整批预检，任一失败整批拒绝；同批名字 trim 后大小写不敏感去重；id = name 或 AdjectiveNoun，冲突加 `-2`，嵌套 `父.子`；会话级信号量默认 32 排队；`maxRecursionDepth` 默认 2。返回 `task-spawn-feedback.md`。(`omp:task/index.ts`、`types.ts`、`output-manager.ts`、`parallel.ts`、`spawn-policy.ts`) |
| `executor` | 子代理系统提示词 = Kivio 默认 Agent 提示词 + `subagent-system-prompt.md`（Role/Context/Coop/Completion，非 structured 分支）；首条 user 为 `subagent-user-prompt.md`；去 todo；工具含 task/bash 时加 wait；深度到顶去 task；审批强制放行；最后一条无工具调用的回复为结果；软请求预算 200（scout/sonic 100），1.5× 强制收尾一轮（禁用工具），1.5×+5 硬中止；`AgentProgress` 150ms 合帧；完成度探针 2/5/10/30/60 分钟；产物 `<id>.jsonl`、`<id>.md`。(`omp:task/executor.ts`、`completion-probe.ts`、`structured-subagent.ts`) |
| `delivery` | `task-summary.md`（11500 字全文 / 5000 字预览 + `agent://`）+ `task-follow-up.md`，外包 `async-result.md`；父在跑：步骤边界作为旁注注入（复用 steering/follow-up 通道）；父空闲：开新一轮（复用定时任务/语音编排的空闲发送路径）；用户手动中断后只记入上下文。(`omp:session/async-job-delivery.ts`、`yield-queue.ts`、`prompts/tools/async-result.md`) |
| `wait` 工具 | 无参数；梯度 5/10/30/60/300 秒，60 秒无调用重置；一个作业完成后再收集 1.5 秒；Completed/Still Running 文本；返回的结果视为已消费。本步只等作业，等回复在第二步。(`omp:tools/wait.ts`) |
| 角色 | 内置 `task`、`scout`、`reviewer`、`sonic`，正文来自 `omp:prompts/agents/*.md`；frontmatter 字段 `name/description/tools/spawns/model/thinking`；三层加载（内置 → 用户 → 项目 `.kivio/agents`）保留；`@task`/`@smol`/`@slow` 接现有子代理模型设置。(`omp:task/agents.ts`、`discovery.ts`) |
| `agent://` / `history://` 读取 | 输出未发布时返回状态与最新助手文本；已发布读 `<id>.md`；`history://` 简明对话记录。(`omp:internal-urls/agent-protocol.ts`、`history-protocol.ts`) |

设置：删除 `sub_agent_concurrency`，新增 `task_max_concurrency=32`、`task_max_recursion_depth=2`、`task_soft_request_budget=200`（后端默认与规范化；`McpCenter` 同步）。

验收：可控 provider 的 Rust 测试覆盖批量派工、并发排队、深度上限、预算收尾、父在跑注入、父空闲开轮、wait 消费后不重复送达；实机派 3 个 scout，主代理继续工作，结果自动回来。

## 第二步：IRC 与生命周期

- **IRC 总线**（`omp:irc/bus.ts`、`messaging.ts`、`omp-tui:tools/irc.ts`）：`IrcMessage {id, from, to, body, ts, reply_to?, wake_relay?, broadcast?, progress?}`；回执 injected/woken/revived/failed；投递顺序 parked/aborted 先复活（失败直接 failed，不入邮箱）→ 匹配的 wait 等待者 → 会话桥 → 会话抛错才进邮箱（上限 100，丢最旧）；回复账本按发送序号；progress 之后只有正式消息算回答；非 main 端点间通信向主会话推只显示的 `irc:relay`。
- **会话桥**（`omp:session/irc-bridge.ts`）：父消息在跑时以 `parent-irc.md` steer；其他放中断队列，广播/progress 为 quiet 不打断 wait；空闲开唤醒回合；plan 模式只记入上下文；正文转义 harness 标签；派工驱动的一轮持有 `hold_irc_wakes`，peer 消息延到结果交出后另开一轮。
- **唤醒回合**（`omp:task/executor.ts` `attachIrcWakeTurnMonitor`、`relayWakeTurnOutput`、`buildWakeRelayBody`）：父发起则登记 owner=父的 job；其他发送者自动转发本轮输出，成功且已回复过则跳过，失败一定通知；wakeRelay/广播/progress 不作唤醒来源。
- **`write agent://<id>`**：`?progress`；`agent://all` 仅 main；子代理提示词 peer 名单上限 32，running 优先，parked 只计数（`collectIrcPeerRoster`）。
- **wait 等回复**：`pending_replies` 分 from/blocked/onUser/stopped；`AgentAwaits` 死锁检测，用户为 `@user`；无人可答时提前返回并说明原因。(`omp:tools/wait.ts`、`registry/agent-awaits.ts`)
- **生命周期**（`omp:registry/agent-lifecycle.ts`、`persisted-agents.ts`、`task/persisted-revive.ts`）：一轮结束 idle；空闲 7 分钟 parked（先解绑再释放，保留 jsonl）；`ensure_live` 复活并合并并发请求，generation 防迟到；kill = 中止 + tombstone → aborted，再发消息复活为新一代；重启扫描产物目录恢复名单；父对话停止只取消其作业，子代理保持可复活。

验收：测试覆盖运行中收父消息、空闲唤醒、parked 复活、relay 不循环、互等时 wait 提前返回、重启恢复名单；实机两个子代理互相提问，parked 后主代理发消息能唤醒。

## 第三步：前端

删除 `SubAgentPanel`、`SubAgentConversation`、`SubAgentIndicator`、`SubAgentAvatar`、`SubAgentElapsed`、`useSubAgents`、`subAgentMessages`、`subAgentStatus` 及 `MessageList` 的 `subagent-result-*` 过滤。按工程规范第 1 节复用全局组件与 token：

- task 卡片：运行中最后 4 行（已完成按耗时在前），行 `状态 · id · 描述 · % · 工具 · 请求 · 上下文 · 费用`，下行当前工具与 intent，>5 秒显示耗时；展开任务说明 ≤20 行与最近输出；结束后问题优先 ≤4 行，输出折叠 3 行 / 展开 12 行，底部汇总。(`omp-tui:tools/task.ts`、`agent-tree.ts`)
- wait 卡片、IRC 卡片（收到的消息、`A → B` 通信）。(`omp-tui:tools/wait.ts`)
- Agent Hub（右侧栏，取代任务面板）：列 Cost/Time/Req/Tools/Tok/Ctx；首次出现定序不重排；平铺/树、过滤；详情面板；活动日志 全部/错误/回复/工具；打开（复活）、发消息、revive、kill。(`omp-tui:overlays/agent-hub*.ts`、`agent-activity.ts`)
- 对话记录查看器（增量读 jsonl）、输入栏上方钉住的 Subagents 列表（仅运行中，>3 折叠）。(`omp-tui:overlays/agent-transcript-viewer.ts`，`omp:modes/interactive-mode.ts`)
- 高频进度沿用 `Chat.tsx` 的 rAF 合帧。

验收：实机逐项检查各状态卡片、Hub 四个操作、明暗主题。

## 风险

- 父空闲自动开轮会与用户输入争同一对话：必须复用现有"等空闲后预留"路径，不另写。
- 子代理审批强制放行：需要审批的工具不再询问用户（ADR-0007 已接受）。
- 第二步触及主循环边界注入：先写失败用例再改。
