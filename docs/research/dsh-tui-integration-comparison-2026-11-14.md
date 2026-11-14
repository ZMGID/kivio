# Kivio 的 dsh 接入与 dsh-TUI 对照

核对日期：2026-11-14。本文记录实现证据与差异，不增加工程规范，也不把第三方产品的全部功能当成 Kivio 的验收要求。

## 范围与版本

- 参照是用户指定的第三方 [ccch1mneyyy/dsh-TUI](https://github.com/ccch1mneyyy/dsh-TUI/tree/9f3928c80da3c384eeae15a13159dcddb2475f54)，提交 `9f3928c80da3c384eeae15a13159dcddb2475f54`，不是已删除的官方 TUI。
- Kivio 基线为 `141390ba8a9d2252baf248f0a239871f1542041e`，加本次未提交的流式修复。
- 本机 Kivio profile 的 `@deepseek-ai/dsh-sdk-jsonrpc-server` 为 `0.2.2-rc.3`。协议形状同时核对了官方 `deepseek-harness` 提交 `39163dd` 的源码。
- 本次对照是源码与自动测试层面的检查；不等同于所有供应商、插件、权限档位和真实会话都已实测。

## 接入结构

dsh-TUI 在 Node 进程内持有 DSH Agent，直接订阅 Agent 与 Session 的事件，再转换到其通用界面事件。Kivio 启动独立的 `dsh --profile kivio` 进程，由 Kivio bridge 扩展官方 SDK JSON-RPC server；Rust 读取标准输出，转成 `UnifiedAgentEvent`，再进入 Kivio 聊天协议与界面。

两者都使用 DSH 原生 Agent、模型适配器、工具与持久化。Kivio 的额外职责是进程生命周期、跨进程协议和桌面显示投影，不需要复制 TUI 的终端组件或另建一套 Agent loop。

## 能力对照

“已接”表示找到了对应调用及事件处理，不表示已经通过全部实机场景。

| 能力 | dsh-TUI 参照 | Kivio 当前实现 | 判断 |
| --- | --- | --- | --- |
| 新建和续聊 | 包装原生 Agent/handle，读取原生 Session | `session/open` 调用 `agents.create/resume`，保留原生 id；应用历史负责显示 | 已接 |
| 正文和思考流 | 同订阅 `session/event`、`agent/assistant-stream` | 本次补实时订阅，片段复用现有正文/思考 delta；完整消息补遗漏后缀 | 本次修复 |
| 尝试开始、失败和重复帧 | 保留 attemptId/revision，丢弃失败尝试临时输出 | 本次按 Agent 去重 revision，转发 start/end；Rust 按 attemptId 撤回失败正文和思考 | 本次修复 |
| 完整消息校准 | 以完整消息校准临时 attempt 投影 | 只在完整文字以已流出文字为前缀时补后缀 | 正常 DSH 累积流可用；未实现非前缀改写的通用校准 |
| 子代理流与后台回复 | 独立子代理订阅、任务和输出投影 | 关联 parent/child/run id；空闲时继续读管道；本次补失败预览恢复和后台正文撤回 | 已接，需真实并发验证 |
| 模型与推理档位 | `agent.model.select` 原位更新下一次请求；Auto 保留续聊选择 | 初始化传 provider/model/effort；启动指纹改变时重连，resume 时 select；同路由 Auto 保留已有 effort | 能力已接，生命周期不同 |
| Agent preset | 按当前 Agent 的服务和命令判断能力 | 内置与自定义 preset 注册、挂载；续聊优先原生记录；旧 code 映射 ptc | 已接；Kivio 的能力声明更静态 |
| 权限、审批 | 原生 approval waterfall，按作用域排队并响应撤销 | 设置原生 permission preset；SDK `approval/request` → Kivio 审批；取消清理等待项 | 已接；不由 Kivio 重做工具权限决策 |
| 问用户和计划审阅 | scoped user-questions，含 plan review | bridge 转 `session/ask`；Rust 匹配 ask_user_question/exit_plan_mode 卡片并返回选择 | 已接 |
| steer / follow-up | `agent.steer/followup`，订阅 inbox 状态 | 分别映射 `session/steer` 与 `session/prompt`，按 RPC ACK 确认送达 | 发送已接，消费/丢弃状态未完整映射 |
| 取消与任务停止 | cancel 区分 user/interrupt/switch；单独撤回排队消息 | cancel 等 prompt ACK 后发原生取消并等 idle；超时结束进程；stop-job 校验所属会话 | 已接；队列保留语义与 TUI 不同 |
| 原生历史浏览、分叉、回退 | 读取原生日志，按 seq 截取 seed 创建分叉 | dsh 未开放原生会话导入；回退为新原生会话加可见历史 | 功能差距；回退遵守 ADR-0006 |
| 工具调用与结果 | 原生 presenter、结构化 content、图片、diff/终端卡片 | tool/call/result → Kivio 通用工具卡；适配常见工具名称；图片转占位文字 | 基础已接，富结果显示不足 |
| 输入图片 | 附件存储与输入策略共用，并支持命令图片 | base64 → `attachments.saveImage` → 原生引用；串行化图片物化与 prompt/steer；命令支持图片 | 已接；未逐供应商实测 |
| 用量与上下文 | request/context + settled usage；区分模型、缓存与计费窗口 | 累计 step、重试及压缩用量；cache 与 reasoning 避免重复计数；contextWindow 来自原生事件 | 核心已接；原生路由变更后的界面同步较少 |
| 压缩 | durable start/end，加 `llm/stream` purpose=compaction 的实时进度 | `/compact`、compaction start/summary/end、用量与边界 | 已接阶段事件，未接摘要生成中的字符进度 |
| Todo、Goal、Plan | todo/write、goal/change、工具 metadata 和计划交互 | todo/write、计划审批、原生命令透传；没有 goal/change 专用映射 | Todo/计划交互已接，Goal 状态展示缺口 |
| 插件和配置 | profile 插件安装/移除/启停，展示应用与重启状态 | 插件清单、部分原生设置/凭据/供应商编辑；Kivio 管理独立 profile | 配置已接，不等同完整插件管理器 |
| MCP 与宿主工具 | 复用 DSH profile/Agent 的插件装配 | 新建和恢复 Agent 都挂官方 MCP client；Kivio-server 凭据按会话注入 | 已接；凭据与工具授权仍归宿主 |
| 进程退出与重连 | Agent handle 和订阅 dispose；失效绑定停止更新 | Rust kill-on-drop、取消超时、shutdown；bridge 处理管道 EOF/父进程退出；profile 启动有进程内锁 | 已有保护；未做多应用实例并发启动实测 |

## 本次修改的边界

1. bridge 订阅实时 assistant stream，保留 start/chunk/end，按 Agent 对 revision 去重。新 Agent 的 revision 可以重新从头开始。
2. Rust 映射层保留当前 attemptId，失败结束只撤回对应尝试；旧尝试的迟到结束不清掉新尝试。
3. 外部代理显示层在开始时划分独立片段，复用已有 `StreamAttemptDiscarded` 前端协议，同时修正后端正文、思考和持久化片段。保留先前已完成输出与工具卡。
4. 子代理失败恢复尝试前的预览；后台自动回复撤回失败尝试文字。
5. 新增共享事件后，补齐 ACP/Codex 测试辅助函数的枚举分支。没有改变它们的运行协议。

## 尚未对齐的事项与优先级

### 优先核对：排队消息取消后的可观察状态

TUI 监听 `agent/inbox/claimed`、`discarded`，并提供 pendingRetraction；用户取消时可 `keepInbox: true`。Kivio bridge 直接 `agent.cancel({kind:'user'})`，Rust 在 ACK 后发 UserSteer/UserFollowUp，但没有对应原生领取/丢弃回传。

这说明 Kivio 缺少完整的队列状态闭环，不足以仅凭源码断言某条用户消息必然丢失。需要实测“生成中追加两条消息→立即取消→续聊”，确认界面记录、队列与模型收到的消息一致，再确定保留或退回输入框的产品语义。不能直接照搬 TUI 的 keepInbox，否则用户点击停止后可能自动继续执行下一条。

### 明确显示缺口：工具图片与富结果

`map_tool_results` 将 image 块变为 `[image attachment]`；TUI 保留图片引用并调用原生 presenter。DSH Agent 仍持有原生工具结果，缺的是 Kivio 卡片的显示能力，不能描述成模型丢失图片。后续应复用 Kivio 附件/工具结果能力，避免在 JSON 中持久化大图或照搬终端 presenter。

### 明确状态缺口：Goal 和原生模型选择

Kivio 没有 `goal/change` 专用投影，也没有 TUI 那样监听 `model/selection` 更新当前显示。原生命令能执行，不代表 Kivio 的目标卡、模型胶囊与原生状态完全同步。DSH Goal 与 Kivio 内置 Goal 的生命周期不同，需先明确展示归属，不能直接写进内置 Goal 状态。

### 既有选择：历史、分叉、回退

TUI 的原生 seq/seed 分叉比 Kivio 的可见历史补发保真。Kivio 当前方式由 [ADR-0006](../adr/0006-rewind-realigns-external-cli-sessions.md) 明确规定；[ADR-0001](../adr/0001-imported-cli-conversations-stay-on-their-cli.md) 和 [ADR-0002](../adr/0002-imported-history-is-a-snapshot.md) 也限制历史归属。若要实现原生分叉，应连同该决定、调用入口和真实恢复验收一起变更，不在流式修复里替换。

### 体验差距

模型切换需要重连、压缩缺少细粒度进度、插件管理仅覆盖清单与部分配置。完整消息校准沿用前缀补齐，尚未覆盖最终消息重写已显示前缀的情况，本次实测未遇到这种事件序列。这些不能与“完全没有流式”混为一类，也不能仅凭功能数量判定现有接入不可用。

## 代码证据

Kivio：

- [桥接与生命周期](../../src-tauri/resources/dsh/kivio-dsh-bridge.mjs)：create/resume、模型、preset、权限、MCP、command、steer、cancel、流式与退出。
- [Rust dsh 会话](../../src-tauri/src/external_agents/session/dsh_jsonrpc.rs)：连接、审批、取消、事件映射、子代理、用量和测试。
- [显示投影](../../src-tauri/src/external_agents/run.rs)：片段划分、撤回及最终持久化使用的正文。
- [运行时声明](../../src-tauri/src/external_agents/providers/dsh.rs)：能力、启动指纹、导入与回退策略。
- [Profile 装配](../../src-tauri/src/external_agents/dsh_profile.rs)、[插件与凭据](../../src-tauri/src/external_agents/dsh_plugins.rs)。
- [现有前端撤回逻辑](../../src/chat/streamPresentation.ts)、[测试](../../src/chat/streamPresentation.test.ts)。

dsh-TUI（均以上述固定提交为准）：

- `src/dsh-adapter/backend/session.ts`：流式、inbox、取消与订阅生命周期。
- `src/dsh-adapter/backend/translate.ts`：attemptId/revision、完整消息、工具图片、Goal 和用量。
- `src/dsh-adapter/channel/model-switch.ts`、`binding-events.ts`：原位模型选择和原生状态同步。
- `src/dsh-adapter/channel/session-rewind.ts`、`session-fork.ts`：原生 seed 分叉。
- `src/dsh-adapter/approvals.ts`、`questions-answerer.ts`：审批与问题的作用域和撤销。
- `src/dsh-adapter/plugin-install.ts`、`plugin-manage.ts`：完整插件生命周期。

## 验证记录

- bridge 自动测试：16 项通过；其中两项流式测试在已提交的旧桥接上失败。
- 前端 `streamPresentation.test.ts`：6 项通过。
- Rust `cargo test --manifest-path src-tauri/Cargo.toml --lib dsh -- --nocapture`：125 项通过、5 项按默认条件忽略；首次发现共享枚举新增后两个测试辅助 match 未穷尽，修正后编译和测试通过。`dsh` 是测试名筛选，也会匹配少量含该字串的其他测试名。
- `stream_attempt_discard_preserves_earlier_output_and_tools`：通过，确认撤回失败正文/思考后保留先前内容和工具卡，下一次尝试可以继续追加。
- 上述忽略项中的 `live_dsh_emits_tool_text_and_usage` 随后显式启用并通过：使用安装的 dsh，经 Kivio Rust `DshJsonRpcSession` 连接、运行 read-only 回合、执行输出固定标记的命令，验证工具调用、成功结果、正文、用量和正常关闭。
- 实际运行的 Kivio Dev 构建晚于本次生产代码改动；安装 profile 中 bridge 与源码 SHA-256 一致。
- 真实模型流式探测通过：使用本机 DSH Desktop 所带 `0.2.2-rc.3` CLI、现有 kivio profile、deepseek-official / deepseek-v4-flash / off / read-only，发送不调用工具的十行中文请求；在 `assistant/message` 之前收到 `assistant/chunk`，整轮共 143 个文字片段，正常 completed。该检查不包含桌面渲染。
- 尚未完成真实重试、取消队列、多实例启动及各权限档位的桌面验收。
