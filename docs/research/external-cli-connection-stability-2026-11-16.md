# 本地 CLI 连接稳定性核验

日期：2026-11-16。T3 Code 参考提交：`ddb7fb3f50c5c8ddb6d495e3cee04ac677ca589e`。

## 依据与范围

截图错误 `external CLI launch configuration changed; retry` 来自 Kivio 的启动身份检查，历史日志没有保留具体差异，不能据此认定当时登录失效。本次确认了一个可复现的误判：同一 Codex 账号的 ID token 仅改变订阅检查时间、组织显示信息，旧启动身份也会改变。

参考 T3 的实例生命周期规则：稳定配置继续复用；替换前关闭旧实例；凭据刷新只重建实际发生变化的实例。

- [实例配置协调与关闭旧 scope](https://github.com/pingdotgg/t3code/blob/ddb7fb3f50c5c8ddb6d495e3cee04ac677ca589e/apps/server/src/provider/ProviderInstanceRegistry.ts#L360)
- [凭据刷新保留未改变的实例](https://github.com/pingdotgg/t3code/blob/ddb7fb3f50c5c8ddb6d495e3cee04ac677ca589e/apps/server/src/provider/ProviderInstanceRegistry.ts#L567)
- [Codex app-server 接入](https://github.com/pingdotgg/t3code/blob/ddb7fb3f50c5c8ddb6d495e3cee04ac677ca589e/apps/server/src/orchestration-v2/Adapters/CodexAdapterV2.ts#L1760)

本次保持 Kivio 按对话持有进程，不引入多线程共用 app-server，也不改变导入、工作目录、审批或 MCP 凭据的隔离规则。

## 路径与归属

原路径：捕获身份 → 连接 → 原生配置变化使注册失败；投递前校验失败还可能带旧身份重连。前端同一模型目录的并发读取分别提交快照和刷新请求。

新路径：Codex 在连接前更新原生配置快照 → 握手 → 校验；仅原生配置变化时，关闭旧连接、撤销旧凭据，带新快照续接同一原生线程一次。再次变化就停止。首次创建的空线程在内部续接后仍接收完整首条提示；已有线程保持续聊。设置代次、可执行文件或环境变化不能通过此重试绕过检查。消息投递后不自动重放。

- `provider_profile.rs` 唯一负责文件和凭据的语义指纹。保留账号、用户、API Key 和未知凭据字段的变化检查；忽略订阅、组织显示信息及 token 轮转。
- `overrides.rs` 唯一负责启动身份比较，错误仅返回差异类别，不输出环境、配置或凭据内容。
- `run.rs` 负责连接、关闭、恢复、发布与投递；删除对失效启动身份的通用传输重试。
- `externalCliSettings.ts` 只合并当前窗口相同范围的在途模型请求，完成或失败后释放；缓存时效仍由后端发现模块决定。强制刷新与不同对话不合并。

## 验证

- 扩展已有账号身份测试：修复前因订阅信息轮转失败，修复后通过；不同用户、账号和无效凭据仍使身份变化。
- 新增并发模型读取、失败后重试、刷新范围隔离用例；修复前重复请求断言失败，修复后通过。相关前端测试 31 项通过。
- CLI Rust 回归 978 项通过、47 项既有用例跳过。首轮沙箱阻止两项端口/进程测试；使用同一已编译测试程序在沙箱外通过。
- 类型、完整 lint、协议生成校验、架构检查、Rust tests 编译检查和差异空白检查通过。
- Kivio Dev + 隔离协议进程：一次配置变化产生 `thread/start` → `thread/resume`，原生线程相同、首条提示只投递一次且包含完整指令；下一轮复用 PID。持续变化时只打开两次连接，没有投递本轮提示。
- 正常环境的 Kivio Dev + 真实 Codex 0.162.0：`npm run probe:e2e -- --agent codex multi-turn` 通过，两轮保留原生上下文、复用同一 PID，`turnsServed` 从 1 增至 2。两轮分别为 5352 ms、2206 ms；这不是稳定的性能基准。

隔离协议进程用于控制配置变化时机，不能替代真实 CLI 验证；临时核验材料位于 `/private/tmp/kivio-cli-stability-20261116/`，未含真实凭据。

## 2026-11-20：完成后的过程文字折叠

用户确认工具记录在展开后可见，缺陷是收起后过程文字仍留在正文。受影响消息保留了 67 条工具记录、44 段思考、12 段过程文字和一段明确的答复。过程文字使用 `plain` / `synthesis`，旧展示规则只收起 `tool_loop` / `auxiliary`，所以进度汇报没有进入 Work。

对照同一 T3 提交的 [deriveTurnFolds](https://github.com/pingdotgg/t3code/blob/ddb7fb3f50c5c8ddb6d495e3cee04ac677ca589e/apps/web/src/components/chat/MessagesTimeline.logic.ts#L1145)：流式生成结束后保留终态答复，将此前的过程条目纳入折叠，同时保留资源卡及明确交付的答复。

Kivio 的唯一分组入口仍为 `segments.ts::groupTimelineSegments`：有明确 `answer` 段时，将其之前未标记为答复的文字与工具一起收起；明确交付的阶段性答复、交付卡和未知尾部正文继续可见。没有明确答复标记的旧消息沿用原规则，不靠工具顺序猜测正文性质。不修改存储、复制内容或原生历史。

新增渲染回归在修复前失败，覆盖生成结束、手动展开、再次收起和重开消息，并确认阶段性与最终答复始终可见。相关 157 项测试通过；受影响消息的脱敏副本验证仅一段答复留在正文，12 段过程文字、67 条工具和 44 段思考进入折叠区。浏览器访问 localhost 未获授权，未做浏览器或原生窗口目测。

## 2026-11-21：消息阶段、发送与插入复查

继续对照同一提交的 `CodexAdapterV2.ts`、`MessagesTimeline.logic.ts`、`QueuedRunsControl.tsx` 和 `threadWorkflows.ts`。T3 保留原生消息阶段，排队消息使用用户气泡展示，另以 Send now 将所选条目提升到当前运行；是否可提升由运行状态和能力决定。

本次确认并修复四处问题：

- 运行中发送会自动 steering，未受理后才回退到输入框上方的压缩队列，且没有手动提升入口。现在发送只入队，消息在滚动时间线显示，每条下方提供立即插入、编辑和移除；未提升的消息仍在轮末逐条发送。选择空闲队列中的某条发送时，不误发队首。唯一可变队列仍归 `useMessageQueue`，页面仅提交动作。
- 图片和磁盘附件被前端 steering 拦截，IPC 也没有传磁盘路径。现在 steering 与 follow-up 共用后端 `inject_running_message` 的附件处理、能力检查及 actor 回执。原生图片走 ImageBlock，普通文件沿用路径说明；内置文本运行时对磁盘附件仍返回未受理。恢复草稿同时恢复附件。
- Codex 的 commentary 在转换时仅变成 `is_answer=false`，丢掉明确的过程语义。现在完成事件携带可选消息阶段，commentary 映射 ToolLoop，partial/final_answer 保留 Answer；未知阶段保持原分类。即使正文没有变化，也发送阶段更新，避免重复追加正文。
- 只有过程文字的历史可能带有一个将原始 delta 连起来的 synthesis 回退副本。展示投影现在同时识别原始拼接与分段拼接的已知副本，避免重开后重复显示；不改写原记录与原生历史。

验证：发送自动插入、气泡入口、IPC 附件、消息阶段和历史重复的回归均观察到修复前失败、修复后通过。相关队列、输入框、消息列表、折叠、展示投影与流式性能用例通过；TypeScript、涉及文件 ESLint、架构检查、协议校验、UI 构建和差异空白检查通过。Rust 外部 CLI 978 项通过、47 项跳过；新增原生附件投递/接受及拒绝回执测试通过。沙箱内的端口和子进程两项失败，在沙箱外用同一测试程序重跑通过。

边界：待发队列仍沿用原来的窗口内存生命周期，没有引入 T3 的服务端持久队列，关闭窗口后的待发恢复未实现。浏览器 localhost 访问此前未获授权，未换通道绕过；本轮没有原生窗口目测或真实 CLI 插入图片验收。正式版安装包没有替换。
