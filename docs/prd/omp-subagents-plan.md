# 子代理改用 OMP：完整实现与验收计划

更新：2026-11-06。分支：`feat/omp-subagents`。产品决定：[ADR-0007](../adr/0007-subagents-follow-omp.md)。OMP 基线：[5dec65ed5f500ec456c2c7f91f7e517f99d92eec](https://github.com/can1357/oh-my-pi/tree/5dec65ed5f500ec456c2c7f91f7e517f99d92eec)，MIT 许可见 [许可文件](../licenses/oh-my-pi-MIT.txt)。

**先读 [OMP 实现与运行调查](../research/omp-subagents-implementation.md)。** 该文档记录源代码事实、调用链、状态/消息/持久化契约、提示词和运行证据；本文件只规定做什么、依赖什么、怎样算完成，不再用“按 OMP”代替接口定义。

## 1. 两步交付与当前状态

交付分两步。旧的三个阶段作为第二步内部工作范围保留。

1. **调查与文档：完成。** 对照固定版本 OMP 源码，核实基线、缺口、数据归属、调用点与验收标准，更新本计划和实现调查。
2. **实现与测试：完成。** Kivio 的派工、等待、IRC、身份和恢复、工具共享、工作池与桌面入口已接通。全量测试通过，真实模型及 Windows 桌面证据见第 15 节；该节也列出依赖 OMP 特有 provider 或平台能力的差异。

调查基线为 Kivio `672013832a8eb59dc230e528bd9fa861c69a9b89`。本地 OMP checkout 的 HEAD 已核对为上述完整 SHA。调查已完成，见[实现调查第 14 节](../research/omp-subagents-implementation.md#14-2026-11-06-完整交付调查)。实现、当前验收和宿主边界见[第 15 节](../research/omp-subagents-implementation.md#15-2026-11-06-实施落点与整合验收)。决策和验证检查点见[决策记录](../research/omp-subagents-decisions.tsv)。

| 阶段 | 调查前基线 | 完成条件 |
|---|---|---|
| 派工、执行、结果与 wait | 已提交 `67201383`；2026-11-06 子代理 84 项回归通过；**完整实机验收未完成** | 下文执行验收全部有证据，且无结果丢失、产物覆盖问题 |
| IRC 与身份生命周期 | 未实施 | 投递、回复、park/复活、kill、重启语义端到端通过 |
| 显示与操作 | 仅旧 UI 清理及简易 task 卡片；未完成 OMP 展示 | 批量卡片、Hub、通信与历史显示在真实界面验收 |

旧的 `agent` / `agent_control`、durable runtime 和旧面板放弃，不留兼容别名、不迁移旧运行记录。旧对话里的旧工具卡退化为普通工具卡。外部 CLI 代理不随本次更换；其共享进度协议不得误删。

“旧实现不兼容”不意味着可以随意删除新 OMP 能力。之前文档把尚未实现的功能写成永久“不做”，并将 speculativeLaunch 误归为默认关闭，本版撤销该混淆：

- OMP 能力与默认配置完整记录在调查第 3、9、11 节。
- 工作区已加入 blocking/async 混合、async disabled 同步执行、speculative adopt/discard；旧版本的全异步限制不再代表当前代码，但这些路径仍须完成本版运行验收。
- structured/yield、taskFile、effort、per-call model、isolation、seance/workpool/prewalk/advisor、eval 工具共享、proc 服务、恢复执行等未覆盖能力，要记录状态及共享字段/文案影响；默认关闭不能当作批准删除。
- Rust/Tauri、目录布局、UI 组件和工具名可适配；调度、授权、消息消费和用户可观察行为不能悄悄改成另一套。

### 1.1 完整能力与依赖

以下状态是实现前基线，不是完成声明。源码依据和具体缺口见实现调查第 14 节。

| 能力 | 基线状态 | 必须完成的可观察行为 |
|---|---|---|
| 批量、blocking、async、同步退化、speculative | 有实现及定向测试 | 整批预检；采用和撤销一致；真实混合批次、授权和 hooks 验收 |
| 结果与 wait | job 路径已有 | 单次消费；失败保存后可重投；IRC 与服务完成也能结束合适的等待 |
| 会话、消息与复活 | 只有一次执行及初始化记录 | 一个身份串行执行多轮；park 后冷复活；kill 失效旧代；恢复权限不扩大 |
| structured 与 yield | 未实现 | 调用方、角色、会话 schema 优先级；strict/permissive；增量结果；异步子结果使旧提交失效 |
| taskFile | 未实现 | 文件指令与内联指令互斥；4 MiB 上限；保存路径和 SHA-256；不预启动文件任务 |
| effort 与逐次模型 | 未实现 | lo/med/hi 映射与上限；确切模型标识；用户授权来源和收窄名单；恢复重新检查 |
| 角色模型列表与配置 | 单模型及三个 Mixer 角色 | 按顺序解析可用模型；不把 provider/model 整串发给父 provider |
| isolation | 未实现 | 隔离工作目录；真实 Git 差异、应用、冲突、取消和清理；保留失败恢复产物 |
| seance | 内置角色被标记不可用 | 历史会话只读 fork；源不改变；恢复模型；同名自定义角色不冒充内置特例 |
| workpool | 未实现 | 入队、状态、drain、close；复用或新建 worker；并发和逐项失败；可执行的模型入口 |
| eval 工具共享 | 没有对应 kernel | 注册与调用真实工具；名字冲突；内核失效后明确失败；不把它改成普通工具名单 |
| prewalk 与 advisor | 未实现 | 首次修改时模型交接；真实审阅会话和结束前收齐；各自取消和错误处理 |
| 服务与 proc URL | shell 有后台进程 | 按 agent owner 隔离；状态和日志；服务结束与 job wait 竞争 |
| 重启与恢复执行 | 只有磁盘产物 | 名单恢复、冷复活、继续中断执行分别验收；先注册子 job 再继续父会话 |
| task/wait 卡片、IRC、Hub、历史 | 只有基础 task/wait 卡片 | 批次逐项、完整指标、通信归属、Hub 操作、游标读取、主窗口与弹窗入口 |

### 1.2 数据与模块归属

先确认数据形状，再写调用逻辑。`AgentRecord` 维护身份和 generation。可保留的 worker session 维护实际授权、模型约束、资源、当前模型上下文与追加 journal。每轮 `Host` 单独持有 run、job、取消、deadline、预算、permit 和进度。IRC 消息、回复义务和等待依赖另有自己的标识，不能从 agent 的 running 状态推导。

保留 `chat/sub_agent` 和现有 JobManager。task、消息唤醒、恢复与池任务共用执行入口。模型工具与桌面命令共用生命周期入口。跨端 DTO 由 Rust 生成，前端只提交动作并展示快照。旧 `agent_control` 不复活。

实现依赖顺序为会话与复活基础、IRC 与 wait、结构化与派工参数、隔离及历史咨询、池和关联能力、界面整合、整体恢复与验收。独立文件可以并行处理，依赖同一个会话契约的工作必须等契约验证后接入。

### 1.3 新的基线检查

| 检查 | 2026-11-06 结果 | 证据 |
|---|---|---|
| `scripts/win-cargo-test.ps1 --lib chat::sub_agent` | 84 通过 | 本机 `.scratch/omp-completion/baseline-rust.log` |
| 全量 Rust `--lib` | 加入已安装 Python312 的进程 PATH 后 3041 通过、52 忽略 | `.scratch/omp-completion/baseline-rust-python.log` |
| 全量 Vitest | 初次 2934 通过、10 失败 | `.scratch/omp-completion/baseline-vitest.log` |
| 两个失败文件独立复跑 | readingLibrary 12 通过；pdfDocument 因 Windows 目录 symlink 权限失败 | `.scratch/omp-completion/baseline-vitest-recheck.log` |

Python PATH 只改测试子进程，不改用户系统设置。前端并发下超时与后续连锁失败已通过独立复跑收窄。PDF 测试需要可跨 Windows 运行的目录 fixture，不能忽略后声称全量通过。

## 2. 各能力共享的契约

### 2.1 身份、状态与数据所有权

| 数据 | 唯一负责人 | 约束 |
|---|---|---|
| agent id、generation、父子关系、session 绑定 | registry/lifecycle | 身份可跨多轮；旧代异步回调不得覆盖新代 |
| 单次 run 的结果和进度 | executor/monitor | pending/running/completed/failed/aborted 与身份状态分开 |
| job id、owner、claim/consume、expiry | jobs/delivery | 一次结果只有一个消费者；显示名不是持久执行去重键 |
| task 调用聚合详情 | task 调用记录 | inline results、async handles、各子项终态均可重载 |
| transcript 与 session_init | 子会话存储 | transcript 不因模型上下文压缩而丢失；复活不能扩大工具授权 |
| UI 选择/折叠 | 前端组件 | 后端状态为权威，不另维护永久代理状态 |

### 2.2 能力与提示词必须一致

以调查第 9 节提示词清单逐项登记：模板 → renderer → 参数 → 开关 → 输出位置。新增/关闭能力时同步 schema、执行检查和文案；不能只改 schema。

特别核对：spawns 缺省为不限角色，空字符串为禁用；tools null 为默认集，显式空数组为空集；深度负值为无限、0 为禁止；普通工具名单与 restrictToolNames 不同；MCP/skills/hooks 和插件层不能在新建/复活间悄悄丢失。

审批按 OMP 的 yolo 模式但保留用户显式 `tools.approval` 策略，不等于无条件允许一切。Kivio 适配保留根会话授权与显式确认策略，子代理继承有效策略；无人处理的确认不自动放行，speculative 不绕过审批与 hooks。

## 3. 实现范围 A：派工、执行、结果送达、wait

### 3.1 实施项

| 工作 | 具体契约 | Kivio 主要落点 |
|---|---|---|
| task 接口 | context/tasks/solutionSpace、角色发现、整批预检、明确修复提示；工具可见性符合模式/深度 | `chat/sub_agent/task.rs`、`agents/`、`mcp/native_registry.rs` |
| 执行模式 | 每项 blocking 与 async 可混合；无 manager/async disabled 同步退化；speculative adopt/discard 不与正式分配冲突 | task/executor；调查 3.2 |
| 名字与产物 | 名字分配扫描内存和磁盘已有产物；job 执行去重不复用永久消息 id；嵌套前缀和平台路径限制明确 | task、names、registry、output 路径 |
| 调度 | OMP session/TaskTool 并发作用域、FIFO、0 无限、acquire/release 更新限制、取消排队；不以 root 全局锁代替语义 | task admission |
| 子会话初始化 | 默认系统提示词 + Role/Context/Plan/Coop/Completion；首条任务单独输入；模型/thinking、工具/MCP/skills/审批策略一致 | executor、agent/filter、资源快照 |
| 计划模式 | 将角色改造成只读规划器，禁继续派工，传已批准计划全文；不是无声明沿用普通编辑工具 | `chat/plan.rs`、task preflight、executor |
| 预算/错误/探针 | stopReason、请求计数含恢复路径、软/强制/硬预算、错误归因；ephemeral 探针间隔/输入/费用按调查 5.2 | executor/monitor |
| 结果与历史 | 结果状态、UTF-16 阈值、全量产物/预览/内存尾部分开；追加 transcript 与 compaction journal；失败保留证据 | executor、urls、result formatter |
| 送达 | busy checkpoint、idle 新轮、中断抑制与手动恢复；投递失败重排；wait 与 sink 原子认领；实际 retention | jobs、delivery、commands 共享回合入口 |
| wait | 当前阶段 job 路径先对齐；补齐 service 能力时统一 owner；IRC 在第二步接入；未实现分支不宣传 | wait、后台服务适配、read URL |
| 角色/frontmatter | 对照所有内置角色及有效字段，不把四个角色当上游全集；插件→用户→项目覆盖与实例寻址分开 | agents/types、parse、mod |
| 显示桥接 | 批量各项进度不互盖；终态持久化，刷新/重开不退回 Scheduled | progress 事件、tool record、SubAgentToolCard |

### 3.2 当前对照审查的修复队列

完整双边源码引用见调查第 12 节。以下不是重新设计，也不是要求保留旧系统。

**先修数据丢失与用户可见错误：**

1. 重启同名 agent 覆盖旧 `.jsonl/.md`；分配时占住已有磁盘名字。
2. 新执行复用 `subagent-result-<jobId>`，被旧持久消息去重后整批结果丢失；给执行使用独立持久身份。
3. idle sink 持久化失败只恢复 result 没有重试；结果重排并有界退避。
4. Stop 后只有直接 send 清抑制，Regenerate/Retry/voice/scheduled 新轮仍被压制；统一正常新轮边界。
5. 活跃批次 resize 只在 enqueue 读设置，release 继续旧限制；与 session 作用域一并修正。
6. bare YAML `tools:` 被当空名单；修 null/empty 区分。
7. task 卡片终态只在内存，父消息重载就丢；持久化最终聚合状态。

**再核对行为差异与残留：**

- transcript 被压缩历史重写、恢复请求漏预算、显式工具名单丢 MCP、activity 心跳不更新。
- thinking auto、camelCase frontmatter、probe 流式工具草稿、UTF-16 截断及渲染后空白处理。
- task 错误校验顺序/通用 schema 抢先拦截、深度 0 的工具展示、read URL 的窗口与上限、job 到期驱逐。
- custom→模型角色映射、数值上限 256/8/10000、plan 工具门控、OMP 角色全集、部分移植的主系统委派规则。
- 新增死代码 `child_conversation_ids`、无调用的 `close_runtime_input`、仅测试使用的旧 pending-input 方法，以及旧 model README/MessageList 注释；删除时按调用证据，不凭编译 warning 全删。

### 3.3 调查前的派工验收记录

| 场景 | 通过标准 | 当前证据 |
|---|---|---|
| 一次 task 派 3 个 scout，wait 汇总 | 三项身份独立、输出正确、只消费一次 | 实机两次 wait 汇总成功；精确竞争需专门回归 |
| 父立即结束，子稍后完成 | 不增加伪造用户消息，自动启动父回复 | 实机成功 |
| 同 root 重启后重用显式名字 | 不覆盖旧产物、不丢新报告 | 已加入磁盘占名与执行 UUID；跨进程实机验收待跑 |
| sink 一次写失败 | 保留结果并自动恢复投递，不无限循环 | 已加入认领回滚、退避重投及故障回归；检查结果见 §3.4 |
| Stop→Regenerate/Send/voice/scheduled | 停止轮不被抢开，新正常轮恢复结果自动送达 | 已接统一 epoch/admission；全入口实机验收待跑 |
| 并发 1 的递归派工、批次 resize | 不死锁；调度范围和 release 行为与 OMP 一致 | 已改为 session 作用域、release 读取新限制；检查结果见 §3.4 |
| 预算穿过普通/恢复请求路径 | 计数、notice、禁工具收尾与硬停止一致 | 已接请求计数与 tool choice；真实模型预算边界待验收 |
| null/empty tools、spawns、MCP、plan | 工具表和真实执行授权一致 | 已修改解析、授权和模式传递；完整运行验收待跑 |
| mixed blocking/async、sync fallback、speculative | 回执/终态/取消/副作用一致 | 已加入执行路径及回归；混合批次实机验收待跑 |
| 大结果/压缩后历史/read 分页 | 完整证据可查，不把 preview 当全文 | 已接追加历史、压缩留档与读取窗口；检查结果见 §3.4 |
| 卡片完成后刷新/重开 | 不退回 Scheduled、不丢各项结果 | 已接终态 journal 与重载合并；组件测试通过，真实界面待验收 |

旧版本构建、类型检查、protocol 检查已跑过，不能代替本版验收。旧版本全套测试有失败，尚未与 main 同环境基线运行比较，归因未确定。本轮检查单独记在 §3.4。

### 3.4 调查前的派工收尾记录

本节保留旧版本的工作记录。当前代码的测试和实机证据见[实现调查第 15 节](../research/omp-subagents-implementation.md#15-2026-11-06-实施落点与整合验收)。

- **派工与执行**：混合 blocking/async、同步 fanout、流式 speculative 的采用/废弃；按派工 session 的 FIFO 并发、运行中 resize、递归不共享根信号量；磁盘名字预留及独立执行身份。
- **结果与生命周期边界**：认领失败回滚及退避重投，消费/未消费定时淘汰；Stop 与新轮 admission 的 epoch 保护；wait 可被 steering 打断。IRC、service wait、park/复活仍属于后续阶段。
- **子会话与授权**：追加 transcript、压缩前留档、精确 MCP 授权、有效审批策略、只读 plan 转换及已批准计划正文；请求预算、禁工具收尾、失败归因和探针参数接入。
- **显示桥接**：终态 journal、读写合并、逐子项状态与 async 聚合、speculative 发布门控、执行前 task checkpoint。多模型分支在首条消息保存前强退，journal 可能尚无可附着消息；普通 draft checkpoint 仍沿用现有非 fsync 写入，不能声称完整崩溃恢复。
- **frontmatter**：已知尚不支持的执行能力显式拒绝；不认识的元数据忽略，不能把它当成已生效能力。旧 `disallowed_tools` 等限制字段拒绝，避免静默丢授权限制。seance 历史 fork 不可用时不向模型宣传。
- **适配限制**：Anthropic/Gemini 无有序的中途 developer 角色，保留报告顺序而不伪称原生 developer 权限；URL 输出目前先格式化全文再分页；job id 进程内保留以阻止旧回调误写新执行。全局及内置角色预算已接入，不宣称有完整逐角色覆盖设置。

本轮收尾检查：

| 检查 | 结果 |
|---|---|
| `powershell -NoProfile -File scripts/win-cargo-test.ps1 --lib chat::sub_agent` | 84 项通过，包含认领/重投、取消、预算、speculative、历史与终态 journal |
| `powershell -NoProfile -File scripts/win-cargo-test.ps1 --lib agents::parse::tests` | 5 项通过 |
| `npx tsc --noEmit` | 通过 |
| 定向 Vitest：SubAgentToolCard、MessageList.subagents、chatToolsShared、runDisplayEvents、usePopoutSession | 5 个文件、50 项通过 |
| `npm run architecture:check` | 12 项检查器测试通过，架构边界无新增违规 |

集成检查修复了大型 `session_init` JSON 宏递归超限、错误日志依赖，以及不完整但已经非法的 Unicode escape 未立即撤销 speculative run 的缺陷；后者保留了失败前/修复后通过的回归。

## 4. 实现范围 B：IRC 与完整身份生命周期

### 4.1 实施顺序与依赖

1. **先定义/写入复活契约**：参考调查 8.1。新建时记录 systemPrompt、task、角色/模型/thinking、精确 tools/mountedTools、spawns/readOnly/restrict、cwd、parent 与产物归属。仅存当前消息数组不够。
2. **registry/lifecycle**：idle TTL、park、ensureLive、reopen/kill、generation/expected-ref 检查、并发复活合并；不把正在 dispose 的 session 交给发送方。
3. **IRC bus 与 session bridge**：write agent URL、direct/broadcast/progress、匹配 waiter 优先、父 steer、后代 in-run、peer deferred、plan idle、失败邮箱。
4. **wake monitor 与回复路由**：父 owner job 与 peer relay 分开；失败必回信；wakeRelay/broadcast/progress 不再次自动回信，但并非“不唤醒”。
5. **wait 扩展**：收信优先、progress 承诺、job-carried reply、directOnly、@user 和传递等待图；不会回答的对象及时解除等待。
6. **重启/退出**：名单恢复、冷复活、中断 job restore 三条路径分别实现/记录；明确未支持的恢复形式，不默默重开错误会话。

消息总线的状态与持久 transcript 不等价：队列/回复账本重启后不能凭 transcript 自动恢复成原来的活跃等待关系。用户 kill、父 stop、取消 job、进程退出也不得合成一个 `cancel_root` 后声称等价。

### 4.2 消息与生命周期验收矩阵

| 场景 | 可观察结果 |
|---|---|
| 父给运行中子发指令 | 当前任务边界读取，不开第二个并发 run |
| 后代问正在等它的祖先 | 问题能在当前 run 处理，不被误延期造成死锁 |
| peer 在派工结果收尾时发消息 | 原任务结果不被 peer 回答替换，问答另开 wake turn |
| idle 收 direct/broadcast/progress/wakeRelay | 按源码分别检查是否运行、是否打断、是否自动回信；无 ping-pong |
| progress 后 job settle | 不把“稍后回复”的承诺错误视为已回答 |
| 双方互等或等待用户 | wait 报明阻塞原因并退出，不盲等 300 秒 |
| park 前后同时发消息/开 Hub | 无 disposed session 使用，复活请求合并 |
| kill 后消息、旧回调迟到 | 新 generation 接收，旧回调无权改新状态 |
| 重启后冷复活 | 工具名单不扩大、模型/角色恢复、MCP/hooks 可用、产物归属正确 |
| 缺 contract/cwd、损坏 transcript | 明确失败，历史仍可读，不造空会话 |
| 切换/压缩/用户新输入与 wake 竞争 | 单一会话归属，不串消息，不越过被接受的用户输入 |

## 5. 实现范围 C：任务卡片、通信与 Agent Hub

Hub 和任务卡片依赖同一套身份、会话与终态数据。

- task 卡片同时支持异步回执、blocking 内联结果、同步 fanout、持续进度、失败/取消。数据 keyed by agent/run，不让一个 batch 多项互相覆盖。
- live progress/终态/持久历史分别有来源；UI 重载以持久终态恢复，再合并更新的 live 事件。按调查 10 的分支核对排序与截断，而不是统一硬编码 3/12 行。
- wait 展示实际等待对象、Completed/Still Running、投递状态和返回原因；IRC 展示 incoming 与 main-only A→B observation，不把只显示卡片注入模型。
- Agent Hub 展示身份名单、generation、父子树、活动和指标；打开、消息、revive、kill 复用同一个后端生命周期入口；状态变化不导致列表无理由跳行。
- transcript viewer 增量读取完整 JSONL 行，处理截断/替换；输入区入口区分“运行中的任务数”和“仍可查看的代理数”。
- React 复用全局 Button/IconButton、settings controls、ErrorNotice、scrollbar 与主题 token；不复制 TUI 的绘图代码，不绕开现有 IPC/协议入口。

验收必须在真实界面覆盖：多 agent 批次、long output、失败/取消、父轮结束后仍更新、刷新/重开、Hub 四项操作、消息/恢复失败、明暗主题、键盘与滚动。截图或静态组件测试不替代操作链验证。

## 6. 文档、范围与验收的维护方式

- OMP 事实改动：更新调查，并附固定提交的文件/符号/行号；Kivio 选择不同：明确写“适配/差异”，不修改事实迎合当前代码。
- 暂未实现能力：schema、提示词、URL、持久字段、UI 都要有一致的不可用边界，不暴露假入口。没有用户批准，不升级为永久删除范围。
- 旧 agent 配置失效只做说明，不加别名或双系统。插件命名空间和资源继承是现行功能，不能借“放弃旧子代理”误删。
- 修复审查项时先复现可观察错误，再验证修复，保留有效回归。文档调查阶段不顺手改功能代码。
- 阶段完成必须同时满足行为、调用点、持久化和验收；“已提交”“能编译”“有一次成功路径”均不足以单独关闭阶段。
