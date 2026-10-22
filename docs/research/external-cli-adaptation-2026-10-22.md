# 外部 CLI 适配核验（2026-10-22）

本轮重点为 Claude Code、Codex 与 Pi，逐项对照上游协议和 T3 Code 当前实现。版本信息为本次查询快照；官方最新版与本机实际运行版本分别记录，不把源码对照当成新版实机验收。

## 版本与近期变化

| CLI | 本机 | 查询到的最新稳定版 | 与适配有关的近期变化 |
| --- | --- | --- | --- |
| Claude Code | 2.1.295 | 2.1.308 | 2.1.304–307 修复无头模式 MCP 连接恢复、非幂等调用重复发送、模型切换等待、工具解析失败后的恢复；2.1.308 只公布可靠性修复。 |
| Codex | 终端 0.162.0；桌面 PATH 0.160.0 | 0.163.0 | 运行中消息引导、中断 Code Mode 与嵌套调用、续聊权限恢复、只读工具并行。app-server 继续提供 thread/turn、模型目录和审批协议。 |
| Pi | 1.3.0 | 1.4.0 | 1.2 扩展启动事件与 busy 状态变化；1.3 保留思考等级切换时的缓存、恢复会话的活动工具；1.4 修复压缩输出限制、截断工具调用后的会话恢复、重试分类，并让 codemode 等待可用 worker。 |

Pi 的这些内部修复应通过更新 Pi 本身获得，Kivio 不复制其工具执行、缓存或重试循环。现有 RPC 仍使用 `data.disposition` 判断输入是否启动运行，以 `agent_settled` 判断真正结束，并处理 `aborted`，本轮未发现这部分需要另加兼容分支。

## T3 Code 对照与本轮改动

| 对照点 | Kivio 原行为 | 本轮结果 |
| --- | --- | --- |
| Codex 模型目录 | 一次 `model/list`，优先用目录行 `id` | 跟随 `nextCursor` 读取完整目录，优先使用可发送的 `model` 标识；完整合并后再过滤、去重、选择默认模型与思考档位。重复游标判失败，整个探测共用原有时间预算。 |
| Codex 轮次归属 | 只在更新活跃轮次 ID 时检查线程；仍会渲染子线程正文，并接受子线程的完成事件 | 在当前会话入口隔离其他线程的通知；解析 `turn/started` 和启动回执中的原生轮次 ID，忽略已知不匹配轮次。子代理展示继续由主线程的协作工具事件承担。 |
| Codex 原生中断 | `interrupted` 与成功完成共用出口 | 返回既有取消语义，保留已有回答，不把中断标记为成功。 |
| Claude 终止原因 | 已有原因文案，但仅在 `is_error` 或失败 subtype 时使用 | 显式失败 `terminal_reason` 和 429 同样决定失败，即使外壳为 success；保留具体错误信息，避免把部分回答当成错误说明。 |
| Claude 工具阶段中断 | 只识别 `aborted_streaming` | 同时识别 `aborted_tools`；无论成功/失败外壳，都走已有取消路径。 |

T3 Code 的服务等级选择、多账号管理、额度展示、目标持续执行等是独立产品能力，不是上述协议修复已经完成的内容。本轮不搬入其 Effect 编排框架，也不改 Kivio 的会话所有权。

原路径仍为聊天发送 → 外部 CLI 会话负责人 → 原生协议；模型探测仍由 `detect_codex_models` 持有，Claude 结果语义仍由 `ClaudeStreamState` 持有。没有新增跨领域状态或第二套运行时。导入、续聊、回退遵守 ADR-0001/0002/0003/0006。

## 验证

- 5 个回归场景在修复前均失败，修复后均通过：目录分页/模型标识、子线程提前结束主线程、Codex 中断、Claude 成功外壳下的失败、工具阶段中断。
- 最终外部 CLI 领域测试：808 通过，47 忽略；忽略项不计为已验证。
- 架构边界检查、`npx tsc --noEmit`、定向 Rust 格式检查和差异检查通过。
- `npx tauri build --debug --bundles app --config src-tauri/tauri.dev.conf.json` 构建成功；重启了该路径下的新 `Kivio Dev.app`，不是旧包或 Vite 页面。
- 在原生窗口选用 Codex，模型菜单正常列出 GPT-6.1 Sol、GPT-6 Astra 等模型；从窗口实际发送验收消息，最终显示 `CLI_ADAPTER_OK`，恢复可发送状态。
- 实际 CLI 原生调用：Codex 与 Pi 的 baseline、multi-turn 场景各 2 项通过；多轮记忆正确，同一会话的子进程 PID 相同，服务轮数递增。命令为 `node scripts/probe-e2e.mjs baseline multi-turn --agent codex` 和对应的 `--agent pi`。
- Claude 同命令的 baseline 遇到上游连续 529 过载，程序返回错误并结束；baseline、multi-turn 共 2 项按环境问题跳过，不能宣称正常对话通过。
- 桌面进程实际启动的是 fnm 安装下的 Codex 0.160.0，终端 `command -v codex` 指向 Homebrew 的 0.162.0。两者分别核验；没有更改用户 PATH、CLI 安装或供应商配置。最新版三个 CLI 均为官方资料与源码层面对照，未替换全局安装做新版实机验收。

本次验证日志保存在 `/tmp/kivio-cli-reference/`（临时目录，不作为长期验收记录）：`regressions-before.log`、`regressions-after.log`、`external-agents-tests-final.log`、`native-build.log`、`native-codex-probe.log`、`native-pi-probe.log`、`native-claude-probe.log`。

## 来源

- [Claude Code changelog](https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md)、[v2.1.308](https://github.com/anthropics/claude-code/releases/tag/v2.1.308)
- [Codex 0.163.0](https://github.com/openai/codex/releases/tag/rust-v0.163.0)、[官方 app-server 协议](https://developers.openai.com/codex/app-server/)
- [Pi changelog](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/CHANGELOG.md)、[RPC 协议](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/rpc.md)
- [T3 CodexProvider](https://github.com/pingdotgg/t3code/blob/main/apps/server/src/provider/CodexProvider.ts)，本次文件 SHA `4a60f12891ee551a6a8436f0980eea7a88330370`
- [T3 CodexAdapterV2](https://github.com/pingdotgg/t3code/blob/main/apps/server/src/orchestration-v2/Adapters/CodexAdapterV2.ts)，本次文件 SHA `067551ff9a912a13d4e877ff77faa745b286ef6a`
- [T3 ClaudeAdapterV2](https://github.com/pingdotgg/t3code/blob/main/apps/server/src/orchestration-v2/Adapters/ClaudeAdapterV2.ts)，本次文件 SHA `49b344961b3fb7563512929ff64df07bf2bf4165`

## 第二轮：继续对照 T3 Code 的运行生命周期

对照 T3 `ProviderTextDeltaCoalescer`（文件 SHA `3729cb7073f9e1ef27d00468bb4fd0ba55eb4dca`）的原生 item 正文归并，以及 CodexAdapterV2 的待决反向请求实现，继续修复：

- Codex 的 `item/completed.agentMessage.text` 作为该 item 的最终正文。没有 delta 时补全正文；有 delta 时去重、补尾，若最终正文改写先前草稿，只替换对应时间线段，保留其他正文与思考顺序。通过现有生成协议增加 `text_segment_replaced`，实时呈现与断线快照一致。
- 等待审批或问答期间持续读取 stdout，支持并行询问和乱序答复。处理 `serverRequest/resolved`、问答 `autoResolutionMs`；已撤回请求不再接收迟到答复，自动问答超时不会授予工具权限。缺少交互宿主或队列不可用时明确拒绝。
- 原生验证额外发现底部问答面板残留：原先 `resolve_user_prompt` 只改快照，运行未结束时前端队列不知道已经撤回。现在经同一协议发送 `user_prompt_resolved`，清理主窗口及独立窗口的对应轮次问题，重复结束幂等处理。
- Codex 模型探测同步排空 stderr，避免日志填满管道后卡住模型列表。回归替身在首个协议响应前写入 512 KiB stderr。
- Claude `set_model` 等待确认由 3 秒改为 20 秒，容纳官方 2.1.307 改为最长 15 秒的确认窗口；4 秒延迟确认验证复用原会话进程，不错误重连。

正文最终态、阻塞式问答、stderr 管道和慢速切模确认四类回归均已在修复前复现失败，修复后通过；独立窗口的问题撤回回归同样先失败后通过。

第二轮验收记录：

- Rust 外部 CLI 与聊天协议领域合计 846 通过、47 忽略；前端正文、协议、交互队列及独立窗口定向测试合计 57 通过。类型检查、定向 Lint、架构检查通过。
- 重建并启动实际 `Kivio Dev.app`。在可控 app-server 替身下，运行尚未结束时 `DRAFT_R2` 已被 `CORRECTED_R2` 替换，存储内容也为最终正文。
- 同一原生窗口并行展示两个问题；上游发送 `serverRequest/resolved` 后，两条过程卡显示已取消，底部作答面板消失，此时仍可见“停止生成”按钮。协议记录没有向已撤回请求发送迟到答复。
- 退出 Dev 后恢复原 Codex 配置，再启动真实 CLI：baseline 与 multi-turn 全绿，第二轮沿用同一 PID，`turnsServed` 从 1 递增至 2。测试替身仅用于本次 Dev 验收，没有替换 CLI 安装。
- 可控协议场景不等于最新 CLI 实机覆盖；Claude 慢速切模仅完成协议替身回归，上游 529 限制仍见第一轮记录。未验收 Windows/WSL、每模型图像能力、Claude 运行中引导、服务等级或完整账号额度界面。

第二轮日志目录 `/tmp/kivio-cli-round2/`：`before.log`、`before-interactions.log`、`after.log`、`frontend-before.log`、`prompt-before.log`、`prompt-after.log`、`owner-tests-final3.log`、`ordered-final-before.log`、`native-build-final2.log`、`native-real-codex.log`。T3 正文归并参考：[ProviderTextDeltaCoalescer](https://github.com/pingdotgg/t3code/blob/main/apps/server/src/orchestration-v2/Adapters/ProviderTextDeltaCoalescer.ts)。

补充边界：前一原生 item 的最终补尾晚于后一段正文时，改为校正原时间线段及完整正文，不能把补尾拼在后一段之后；交错 item 的增量也按原段位置校正。该最终补尾回归在修复前失败，修复后纳入上述 846 项全绿。验收期间磁盘不足通过清理本项目可重建的 Rust incremental 缓存解决，未清理用户资料。

最终工作区版本再次构建成功并从 bundle 路径重启；原生窗口新建真实 Codex 会话 `conv_9901d01c-6933-476a-b345-6ecdb4e4e504`，发送 `Reply exactly T3_ADAPTER_OK. Do not use tools.`，窗口显示 `T3_ADAPTER_OK` 并恢复输入状态。所有更改保留在工作区，未提交、未推送。

## 第三轮：补齐能力元数据、运行中引导、服务等级与账户额度

本轮继续使用现有外部 CLI 会话负责人，未引入第二套 Agent 运行时。对照 T3 的模型目录、Codex 服务等级和 Claude 优先输入生命周期，以及官方协议，补齐以下功能：

| 能力 | 实现与边界 |
| --- | --- |
| 每模型图片能力 | Codex 读取 `inputModalities`，Pi 读取模型 `input`，Auto 继承原生默认模型；仅文本模型不发送原生图片块，保留附件路径回退。旧目录缺少字段时保留兼容行为。模型菜单明确标出“仅文本”。 |
| Codex 服务等级 | 读取原生 `serviceTiers` / `additionalSpeedTiers`，主窗口与独立窗口共用现有选择控件；会话配置、最近运行时持久化、启动配置及 `turn/start.serviceTier` 全链路传递。切换模型清除旧模型的显式等级；等级改变沿用现有重连/原生 resume 机制。 |
| Claude 运行中引导 | 优先输入使用 `priority: now` 和 UUID，等原生 user echo 后才确认送达；被替代片段的 aborted result 不得结束当前引导轮。普通下一轮继续复用进程。 |
| Claude 前台子 Agent | 引导前将仍在运行的 Agent/Task 经 `background_tasks` 转入后台，并继续排空 stdout 等待确认。失败或 5 秒超时保留排队输入，不通过强制优先输入杀掉子 Agent；撤回的旧审批经同一交互清理协议结束。 |
| 账户与额度 | 按需读取 Codex `account/read`、`account/rateLimits/read` 和 Claude `get_usage`；无用户对话或模型推理。展示账户、套餐、多额度窗口、剩余比例、重置时间与可用 credits；缺失数据明确未知。白名单提取，不返回凭据。 |
| 多窗口与迟到结果 | 主窗口/独立窗口共用账户组件与设置入口；切换 CLI、会话或卸载会使旧请求失效。账户浮层经现有定位 hook + portal 避免侧栏裁切。 |

新增 `account.rs` 是短生命周期只读探测的资源负责人：初始化、排空输出、拒绝意外反向请求、超时及子进程树清理在同一处完成。其他 Rust 构造器中的 `capabilities: None` / 新配置字段默认值仅用于类型及旧数据兼容。

### 第三轮验证

- 模型能力丢失、Claude 优先输入以及前台 Agent 未转后台的回归均先失败后修复通过。Claude 回归通过真实子进程管道交换初始化、背景化回执、优先输入回显、旧轮终态及下一轮消息；直接运行测试可执行文件，移除 Unix shell 依赖，供现有 Windows Rust CI 同样执行。
- Rust 外部 CLI 与聊天协议定向测试：850 通过、47 忽略；前端相关 10 个文件共 120 通过。TypeScript、定向 Lint、架构检查与原生构建通过。Windows CI 本轮未触发，不能将跨平台可运行测试记作 Windows 已通过。
- 在临时隔离的官方 npm 安装中验收 Codex **0.163.0** 与 Pi **1.4.0**：baseline、multi-turn 各 2 项通过，多轮复用同一 PID。Claude **2.1.308** 遇到上游连续 529，2 项按环境限制跳过；Claude 正常对话与优先输入的真实上游验收仍未通过。
- 实际重建并启动 bundle 中的 `Kivio Dev.app`。可控 Codex app-server 下，Fast 选择得到 `TIER_OK`，协议记录包含 `serviceTier: priority`；切换模型后该显式等级清除。相同图片在纯文本模型下无原生图片块，在图片模型下发送 `localImage` 并显示 `VISION_BLOCK_OK`。
- 原生账户面板完整显示模拟账号、5h/7d 剩余比例和重置时间，无侧栏裁切。此前也成功读取真实 Codex 账号；不在本记录保存个人账户值。
- 退出 Dev 后将 Claude/Codex/Pi 的三项 CLI 配置逐项恢复备份并核对一致，再启动最终构建。真实 GPT-6.1-Sol 对话显示 `CLI_CAPABILITIES_OK` 并恢复输入。
- 用户确认暂无 Windows 环境，本轮完成实现及自动化覆盖，Windows/WSL 实机验收明确待测。所有更改未提交、未推送；保留其他聊天的 Git 状态栏修改。

本轮临时日志在 `/tmp/kivio-cli-round3/`，包括 `owner-tests-attachments.log`、`frontend-final.log`、`native-build-attachments.log`、`latest-codex.log`、`latest-pi-retry.log`、`latest-claude.log`。版本安装与替身均未覆盖全局 CLI。

补充来源：[T3 codexModelOptions](https://github.com/pingdotgg/t3code/blob/main/apps/server/src/codexModelOptions.ts)，本次文件 SHA `43b8a3c184f5a6548763b9067a3178565009cb35`；上述 T3 ClaudeAdapterV2；[官方 Claude Agent SDK](https://www.npmjs.com/package/@anthropic-ai/claude-agent-sdk) 0.3.308 的 `SDKUserMessage.priority`、`background_tasks`、`get_usage` 类型及协议实现。本项目未新增 SDK 依赖。

原生协议检查额外定位并修复：Codex/ACP 持久会话复用轮原先只发送最新用户文本，导致降级图片及普通文件路径说明丢失。现在只追加本轮附件说明，保留会话指令仅首轮注入的约定；Claude/Pi 的完整 per-turn 提示词不重复追加。回归在修复前断言只收到 `look` 而非图片路径，修复后通过并纳入上述 850 项。

最终附件修复包再次构建成功并重启。在既有原生会话第 4 轮附加同一 PNG，纯文本模型显示 `TEXT_ONLY_PATH_OK`；独立检查实际 `turn/start` 记录确认 `native_images = 0`、`attachment_path_present = true`，没有重放会话指令。之后再次退出、恢复三项原始 CLI 配置并逐项核对一致，重启最终包，新建页已切回真实 GPT-6.1-Sol。

## 2026-10-23 追加审查与修复

复查三轮未提交适配 diff 的协议读写、终态归并、交互撤回、模型能力、设置持久化、账户查询及跨平台进程路径，定位并修复两项问题：

1. Claude 优先输入等待前台 Agent 转后台时，新出现的并行 Agent 不在最初的快照中，可能仍在前台就收到 `priority: now`。现在由同一会话读循环在每次发送前对齐最新前台集合，新增 Agent 也必须等待背景化回执；保留原有总截止时间与失败后排队行为，没有新增状态负责人。
2. Codex `rateLimitsByLimitId` 中，后一个额度项的 `credits: null` 会覆盖前面有效的 credits。现在只归一化对象类型的 credits，空项保留已读到的有效余额。

扩展原有两个测试：子进程管道替身在第一条背景化请求尚未确认时发出第二个 Agent，要求第二次背景化先于优先输入；账户载荷同时包含有效余额与后续空 credits。两项在修复前均失败，修复后纳入 Rust 850 通过、47 忽略。未重复增加各转发层的相同断言。

前端关联 10 文件 120 项通过，TypeScript、架构与 diff 检查通过。第一次修复后编译遇到本机磁盘耗尽；仅清理项目生成的增量缓存及已结束测试遗留的对象文件，以 `CARGO_INCREMENTAL=0` 重跑完成，未删除用户资料。日志：`/tmp/kivio-cli-review/before.log`、`after-no-incremental.log`、`frontend.log`、`typecheck.log`、`architecture.log`。

本轮没有改动 CLI 路径或全局安装。Claude 优先引导的真实上游验收和 Windows/WSL 实机验收仍不能用协议替身结果替代。

审查修复版已完成 `npx tauri build --debug --bundles app --config src-tauri/tauri.dev.conf.json`，退出旧 Dev 并从 bundle 路径重启。原生窗口真实 Codex 会话 `conv_523cad1d-df7d-47b5-9abf-e6f84fc4827d` 显示 `CLI_REVIEW_OK` 并恢复输入状态；账户面板读取到真实额度窗口及更新时间，查询结束，无需临时替换 CLI。账号个人值未写入报告。构建日志 `/tmp/kivio-cli-review/native-build.log`。修复仍未提交、未推送。
