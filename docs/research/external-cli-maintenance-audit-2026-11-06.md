# 外部 CLI 适配维护审计

本次目标是让 Kivio 的外部 CLI 接入具备明确的 provider 归属、可结束的进程生命周期、可靠的协议边界，以及一致的安装和账号身份。本文记录调查结论和迁移建议，不增加工程规范。后续实现继续遵守[统一工程规范](../engineering-standards.md)。

调查基线为 Kivio `7a7e10b81b37b35758dff840a32d2e07138bdee6` 的工作区，以及 T3 Code `6577c6b6185167c0e502fa01f9e8e8f2dee6ff25`。日期为 2026-11-06。本轮不修改产品代码，也不把源码分析当成当前已安装 CLI 的实机验收。

## 结论

Kivio 已有 provider 定义表、共享协议 actor、常驻会话注册表、原生会话绑定、回退和多项协议回归。问题集中在不同身份和生命周期之间的连接。新增一个 CLI 的定义可以集中填写，但原生会话操作、账号配置、错误和进程复用仍会进入多个共享文件。现有统一聊天事件可以保留，不能把所有 CLI 都能做同样的事情作为前提。

优先修复会造成审批错配、上下文错位、重复执行和旧账号继续运行的路径。之后按 provider 收拢已有实现。单纯把 `session/*.rs` 搬进几个目录，不会修复这些行为。

## 已有能力必须保留

- [registry.rs](../../src-tauri/src/external_agents/registry.rs) 注册 10 个代理。[types.rs](../../src-tauri/src/external_agents/types.rs) 的 `RuntimeAgentDef` 已包含安装、导入、运行、引导等策略。不存在完全没有能力建模的情况。
- [live.rs](../../src-tauri/src/external_agents/session/live.rs) 已让同一会话跨轮复用 actor。[spawn.rs](../../src-tauri/src/external_agents/spawn.rs) 的 `AgentChild` 已管理 Unix 进程组，Windows 已做存活主进程下的 `taskkill /T` 清理。Kivio 并非每轮都重新 spawn。
- [ADR-0006](../adr/0006-rewind-realigns-external-cli-sessions.md) 已定义 Pi fork、Claude 截断分叉、Codex revert，以及其他情况的新会话补历史。它明确保留回退按钮，且回退不撤销工作目录里的文件修改。
- [provider_profile.rs](../../src-tauri/src/external_agents/provider_profile.rs) 已有 Claude 私有设置、Codex 私有 `CODEX_HOME`、dsh profile，以及原生配置备份和原子写入。Codex 链接原生会话目录是为了支持终端续聊。
- [2026-10-22 核验记录](external-cli-adaptation-2026-10-22.md) 已记录线程与轮次过滤、并行审批、正文最终态、Claude 引导、账号额度和多轮实测。那些是历史验证记录，本轮没有重新验证其中的全部结论。

## 对 T3 八项对照的核验

| 对照项 | 已核验的 T3 实现 | 对 Kivio 的结论 |
| --- | --- | --- |
| provider 包与能力 | [接入说明][t3-add]、[统一接口][t3-adapter]、[能力模型][t3-capabilities]。OpenCode 等有独立包，Codex 和 Claude 在该提交仍位于服务端 Adapters。 | 按 provider 收拢是正确方向。先使用 Rust 模块边界，不需要立即新增十个 crate。共享协议与统一 UI 事件仍有价值。 |
| 进程生命周期 | [AgentScope][t3-scope]、[Linux scope 实现][t3-linux]、[Claude watcher][t3-claude]。Claude POSIX watcher 在宿主管道 EOF 后终止进程组。 | Windows 主进程退出后的后代、WSL 内部进程树和宿主崩溃需要独立验收。Linux cgroup 不应成为当前 Windows/macOS 桌面的前置工程。T3 的 PID 复用保护未在本轮逐路径核验。 |
| 协议边界 | [严格 ACP parser][t3-acp]、[ACP client][t3-acp-client]、[Codex adapter][t3-codex]。确认坏 JSON 行会失败、存在 v2 与 v1 兼容路径、notification 问答的 message response，以及 goal 多原生 turn 管理。 | Kivio 需统一坏帧、投递确认和运行终态。不能把每个 notification 当请求，也不能把一次 Kivio run 等同于一个 native turn。重启恢复问答的完整链路本轮未跑。 |
| 恢复、回退、分支 | T3 接口明确区分 snapshot、fork、rollback 和 handoff。 | Kivio 已有回退，但普通分支没有交付历史。不能照搬 T3 的拒绝策略覆盖 ADR-0006，也不能让导入会话自动跨 provider 切换。 |
| replay | [测试策略][t3-testing]、[transcript][t3-replay]，记录真实协议并通过真实编排与投影重放。 | Kivio 有大量纯逻辑和可控子进程测试，也有 live probe。缺的是按 provider 版本保存、可持续重放的共同场景，不是从零搭测试。 |
| 安装与隔离 | [maintenanceResolver][t3-maintenance] 检查包管理来源，包含 mise、Chocolatey、Homebrew 处理。[Claude home][t3-claude-home] 与 [Codex shadow home][t3-codex-home] 分离配置和共享状态。 | Kivio 已做部分来源判断和隔离，但执行路径、配置代次和账号查询没有统一身份。T3 的 Antigravity release lease、ACP Registry SHA 校验和所有安装器组合未逐项核验。 |
| 诊断日志 | [EventNdjsonLogger][t3-log] 在编码后按字节检查 64 KiB 预算并回退到结构摘要，[nativeProtocolLogging][t3-log-summary] 限制 payload 内容。 | Kivio 的统一聊天事件调试与 CLI 原生协议诊断不是同一层。需要有界、脱敏、保留请求归属的原生诊断，不能复制完整凭据或用户正文进日志。 |
| 远程与多端 | T3 的 provider 包区分 client/server，接入指南声明客户端需要跨 web/mobile 使用。[接入说明][t3-add] | 本轮只核验此结构，没有逐条验证远程 OAuth 和 primary handoff。远程执行属于新的产品范围，排在本地正确性之后。 |

OpenCode 还需要单独决策。T3 的 [OpenCode 2 server][t3-oc-server] 按实例共享进程，其 [v2 adapter][t3-oc-adapter] 为每个 thread 生成 MCP 名称，并用先拒绝其他 thread、再允许本 thread 的规则隔离。Kivio 目前按 ADR-0003 使用 ACP。可先把 OpenCode 自身策略收进独立模块，继续共享 ACP 传输。只有实测证明 ACP 不能满足产品行为时，再增加原生 HTTP 路径。无需为了目录形式同时维护两种传输。

## 需要先修复的行为

以下 P1 表示应先于结构迁移修复的用户正确性问题。除明确标出的隔离测试外，结论来自调用链与源码，尚未做桌面故障注入。

| 优先级 | 触发条件与影响 | 证据 | 必须验证的结果 |
| --- | --- | --- | --- |
| P1 | 两个 CLI 进程给出相同审批 ID，宿主索引覆盖前一个待决请求。一个窗口的回答可能落到另一条对话。后台任务同样只用原生 task ID 做 key。 | [interaction_state.rs](../../src-tauri/src/chat/interaction_state.rs) 115、156；[codex_app_server.rs](../../src-tauri/src/external_agents/session/codex_app_server.rs) 331、2232；[run.rs](../../src-tauri/src/external_agents/run.rs) 2458；[background_tasks.rs](../../src-tauri/src/external_agents/background_tasks.rs) 21、37。后台任务覆盖已用实际模块的隔离测试复现。 | 宿主交互 ID 带会话实例与请求归属。不同对话的同名 ID、旧进程迟到回答和重复回答互不影响。给 CLI 的原生 ID 原样保留。 |
| P1 | 冷启动握手期间按 Stop，连接完成后仍无条件发送 `RunTurn`，随后才检查取消。升级的 Close 也发到同一 actor channel，不能保证打断正在阻塞的写管道。 | [run.rs](../../src-tauri/src/external_agents/run.rs) 1500、1590、2313、2386；[claude_stream.rs](../../src-tauri/src/external_agents/session/claude_stream.rs) 928。 | 启动前取消不发送 prompt。启动后取消等待原生确认，超时由独立资源控制路径结束进程。满 channel、不读 stdin 的 CLI 也能退出。 |
| P1 | 外部回复登记 generation 后没有配对退役。正常完成也会在活跃 generation 集合中留下旧记录。 | [run.rs](../../src-tauri/src/external_agents/run.rs) 669；[reply.rs](../../src-tauri/src/chat/commands/reply.rs) 193、308；[runtime_state.rs](../../src-tauri/src/chat/runtime_state.rs) 95、116、480。协议 run guard 不负责这个集合。 | 成功、失败、取消、早返回均退役自己的一代。空闲时不再接受属于旧运行的 steering，后续轮次不受清理影响。 |
| P1 | CLI 已执行工具后断线，通用 `RetryFresh` 仍可能重连并重发同一 prompt。当前策略只对部分错误和 Antigravity 禁止重放，没有投递阶段约束。 | [run.rs](../../src-tauri/src/external_agents/run.rs) 1649、1735、2103。 | 区分未发送、已确认、结果未知。只在可证明安全时自动重试。副作用已执行但结果丢失时不自动发第二次。 |
| P1 | 外部 CLI 对话创建分支，UI 复制原消息，但新原生会话首轮只得到新提问。 | [mutations.rs](../../src-tauri/src/chat/commands/mutations.rs) 792、825、858；[run.rs](../../src-tauri/src/external_agents/run.rs) 380、400、1616。 | 分支首轮能引用保留历史。原生 fork 或显式文本 handoff 必须生成独立绑定，第二轮不重复注入，源会话保持可用。 |
| P1 | 导入提交没有重复验证列表的工作目录过滤。移项目或改项目根后，发送优先取新项目根，而绑定恢复只比较 agent/protocol。 | [import.rs](../../src-tauri/src/external_agents/import.rs) 886、990；[workspace.rs](../../src-tauri/src/external_agents/workspace.rs) 12；[session/mod.rs](../../src-tauri/src/external_agents/session/mod.rs) 424。 | 导入提交与续聊均校验规范化后的原目录。junction、盘符和 WSL 同一路径表示不误拒绝，实际改目录不能静默写错位置。 |
| P1 | 回退成空消息或 ACP 空快照后，仍有原生绑定，但更换运行时只按消息数判断。 | [commands.rs](../../src-tauri/src/external_agents/commands.rs) 375、399；[Chat.tsx](../../src/chat/Chat.tsx) 2626。 | 运行时切换由绑定状态决定，遵守 ADR-0001。空可见历史不等于没有原生会话。 |
| P1 | 改供应商、凭据或 binary 后，常驻复用条件没有包含完整启动身份。账号查询起新进程，旧对话可能仍用旧账号。 | [run.rs](../../src-tauri/src/external_agents/run.rs) 808、1400、1883；[live.rs](../../src-tauri/src/external_agents/session/live.rs) 308；[account.rs](../../src-tauri/src/external_agents/account.rs) 67。dsh 和 Antigravity 已有部分指纹，其他 provider 不能依赖这些特例。 | 探测、账号查询和执行解析同一启动身份。配置变化使旧实例有序退役，下次发送不再使用旧 home 或 binary。 |
| P1 | 删除供应商时，前端设置 patch 与清理 IPC 独立发出。保存失败仍可能先删私有配置，清理错误被吞掉。 | [ExternalAgentsSettings.tsx](../../src/settings/ExternalAgentsSettings.tsx) 975；[commands.rs](../../src-tauri/src/external_agents/commands.rs) 129；[provider_profile.rs](../../src-tauri/src/external_agents/provider_profile.rs) 2023。 | 已提交的设置变化决定清理。失败保留旧有效配置，清理可重试，运行实例仍引用的配置不得提前回收。保留 Codex 会话 junction 保护。 |
| P1 | 更新按路径猜安装器。Windows `.cmd` 可被视为 npm 安装，WSL Kimi 路径可被分配宿主安装计划。预览和执行分别重新解析，目标不固定。 | [installer.rs](../../src-tauri/src/external_agents/installer.rs) 613、679、967、1112。 | 计划固定执行环境、安装来源、目标版本与安装锁。无法证明所有权时手动更新。安装后验证实际选中的同一 binary，而非另一个 PATH 候选。 |

### 其他必须纳入验收的缺口

1. 导入创建聊天、保存绑定、保存来源是分开的写入。后两步失败会留下部分结果。原生会话防重在列表与当前前端窗口，提交没有同等保护。见 [import.rs](../../src-tauri/src/external_agents/import.rs) 1056、1171。
2. 回退 marker 先于会话 CAS 写入，成功运行后又先于助手消息保存清除。失败与崩溃窗口需要可恢复的提交语义。见 [mutations.rs](../../src-tauri/src/chat/commands/mutations.rs) 676 和 [run.rs](../../src-tauri/src/external_agents/run.rs) 1018、1072。
3. ACP 运行中坏 JSON 被当作 Idle，握手和导入也有静默跳行。应区分未知但合法 notification、坏 JSON、必填字段缺失与正常 EOF。未知可选事件可以兼容忽略，坏必需消息不能被当作仍在健康运行。见 [acp.rs](../../src-tauri/src/external_agents/session/acp.rs) 2130、2147、5052。
4. Windows `AgentChild` 在主进程被回收后不再 `taskkill`，这是防 PID 复用误杀的合理限制，却没有覆盖存活后代。可复用 [browser_windows.rs](../../src-tauri/src/html_render/browser_windows.rs) 179、245 的 suspended spawn、Job Object、恢复线程做法。WSL 要验证发行版内的进程归属，不能将杀掉 `wsl.exe` 记作 Linux 后代已退出。
5. 删除或退出只处理已登记的会话。连接中、重连中和从注册表移除但仍由 actor 持有的进程，也要进入关闭流程。删除后的迟到连接不能重建磁盘绑定。见 [live.rs](../../src-tauri/src/external_agents/session/live.rs) 447 和 [mutations.rs](../../src-tauri/src/chat/commands/mutations.rs) 1226。
6. Kimi 能加载但不重放时，目前可导入空快照。ADR-0003 记录过需要额外历史来源。最新安装版本是否仍如此必须实测，不能把旧记录当新版协议事实。文件导入的 mtime 过期提示也会把 Kivio 自己写入算作变化。见 [acp.rs](../../src-tauri/src/external_agents/session/acp.rs) 4984、5072 和 [import.rs](../../src-tauri/src/external_agents/import.rs) 1297。
7. WSL 的自定义 Linux 路径被转成 UNC 后，按原始路径反查覆盖配置可能失配。普通环境变量是否真正进入 Linux 子进程还需实测。见 [wsl.rs](../../src-tauri/src/external_agents/wsl.rs) 216、[overrides.rs](../../src-tauri/src/external_agents/overrides.rs) 95、115。微软的 [WSL 互操作说明](https://learn.microsoft.com/en-us/windows/wsl/interop) 说明了 `WSLENV` 的桥接用途。
8. 限额已有分类与额度面板，但错误展示与重试门控不是一套类型。需要把原生错误转成带可选 reset time 的失败结果，并由该结果决定是否重试。见 [errors.rs](../../src-tauri/src/external_agents/errors.rs) 和 `persistent_failure_action`。
9. cc-switch 候选扫描把 `env`、`configToml` 和 `authJson` 原文传到前端。可改为先列非敏感元数据，选中后由后端导入。安装输出也需要有界脱敏。本轮没有发现或声称真实凭据已泄露。见 [cc_switch.rs](../../src-tauri/src/external_agents/cc_switch.rs) 21、139 和 [installer.rs](../../src-tauri/src/external_agents/installer.rs) 1134。
10. ACP 消费 `session/update` 时没有按 `params.sessionId` 过滤，prompt 的无 error 回复也没有完整验证 result 与 stopReason。Codex 通用 RPC 错误路径需核对请求归属，且数值和字符串 RPC ID 不应折叠为同一个值。见 [acp.rs](../../src-tauri/src/external_agents/session/acp.rs) 1999、2053 和 [codex_app_server.rs](../../src-tauri/src/external_agents/session/codex_app_server.rs) 188、2136。验收覆盖外来 session、旧请求错误、旧终态及正常的先通知后回执。
11. Claude 和 dsh 已能处理部分主轮结束后的后台事件，应保留这些路径。任务、唤醒和闲时问答还需要携带来源实例。dsh 闲时问答没有同时等待会话关闭，CLI 退出后可留下等待用户的卡片。见 [run.rs](../../src-tauri/src/external_agents/run.rs) 2761、2837、2860 和 [dsh_jsonrpc.rs](../../src-tauri/src/external_agents/session/dsh_jsonrpc.rs) 1044。

## 目标归属

先区分四个概念，避免把不同的 provider 混为一谈。

- CLI 种类，例如 Codex、Claude、OpenCode。现有 `RuntimeAgentDef` 继续作为静态定义入口。
- 运行实例，指某个 binary、宿主或 WSL 发行版、配置目录与凭据代次的组合。它不等于中转供应商 ID。
- 会话绑定，记录 Kivio 对话、原生会话、原工作目录与可兼容的运行身份。进程退出不删除有效绑定。
- 一次运行，持有本轮 generation、投递阶段、原生 turn 集合和待决交互。一次运行结束不等于后台任务或会话进程必须结束。

建议在现有 crate 内形成以下职责。目录名字可随实现调整，职责不能复制。

```text
external_agents/
  providers/
    codex/       定义、原生操作、账号与安装策略、协议测试
    claude/      定义、原生操作、账号与安装策略、协议测试
    opencode/    定义与 OpenCode 策略，第一步复用 ACP
  session/       会话绑定、运行与进程资源的现有负责人
  transport/     确有多个 provider 复用的 ACP、JSON-RPC、stdio 边界
  registry.rs    单一注册入口
```

这是归属建议，不要求为每种角色新增 Manager、Controller 或转发层。一次 Codex 协议变更主要落在 Codex 实现及其直接契约。共享层只处理资源、交付、持久化和通用事件，不继续增长 provider 名称分支。

能力沿现有类型扩展。对分支、回退、恢复问答等操作表达原生支持、文本降级和不可用原因。会话握手可以收紧静态声明，UI 和命令入口使用同一有效结果。模型输入能力、用户权限策略与 CLI 功能能力各自保留语义。不能将它们折叠成一个 `supported` 布尔值。

账号隔离要保留现有“CLI 自身配置”的共享模式。新增隔离实例时明确哪些配置和凭据是私有的，哪些历史有意共享。旧绑定不能在迁移时默认为另一个账号。CLI 类型隔离、凭据隔离和原生会话历史隔离不是同一个开关。

## 可验收的迁移顺序

| 单元 | 最小完整变更 | 删除或替换的旧路径 | 验收 |
| --- | --- | --- | --- |
| 1. 固定当前故障 | 用现有可控子进程和真实模块测试补齐双会话 ID、generation、冷启动 Stop、执行后断线。修复这些具体缺陷。 | 未配对的运行登记、裸原生 ID 全局索引、无法证明安全的重发。 | 旧代码先失败，新代码通过。检查真实输出、待决状态和 PID，不只检查 Cancel 被调用。 |
| 2. 统一启动与资源身份 | 让探测、账号、聊天使用同一解析结果。把 binary、执行环境、配置代次接入复用判据。进程从启动起到回收始终受管理。 | binary 名称反查配置、各处不完整指纹、仅从已连接注册表寻找待关闭进程。 | 切供应商与 binary 后下一轮使用新身份。启动中 Stop、删除、退出和 channel 堵塞后无受管进程残留。Windows 与 WSL 分开验收。 |
| 3. 修复会话操作 | 收拢导入提交、防重、原目录校验、分支交付和回退提交状态。 | 仅 UI 过滤、按消息数推断绑定、分支只复制显示历史。 | IPC 并发导入和失败重试没有半成品。分支有上下文，回退失败可恢复，源原生会话保留。 |
| 4. 按 provider 迁移 | 先 Codex，再 Claude，再 OpenCode。每个迁移完整带走定义、原生操作、错误、账号与安装策略，以及测试。 | 已迁移 provider 在共享文件中的专属分支和旧入口。 | 每个 provider 单独通过协议回归、生成契约和架构检查。变更一项规则不再跨多个共享分派表。 |
| 5. 安装与配置提交 | 后端拥有删除清理和安装任务。固定安装来源、环境和精确版本，检查运行占用。无法确定来源时保留手动操作。 | 前端独立 cleanup IPC、仅当前 webview 的去重、执行时重新选择未固定目标。 | 多窗口更新不重叠。保存失败保留旧配置。校验失败不激活，旧进程引用的版本和配置仍在。 |
| 6. 保留真实回放 | 为支持的 CLI 版本录制脱敏 transcript，用真实会话与事件投影重放。复用现有 fake transport、probe 和 CI。 | 只靠手写 happy path 帧判断协议兼容的做法。 | 覆盖 simple、multi-turn、resume、stop、steering、并行审批、断线、fork、rollback、后台迟到事件和 quota。真实 binary smoke 单独标记环境结果。 |

单元 1 就补回归，不等待单元 6 才验证。单元 6 是把真实协议兼容性检查长期维护起来。能力发布随各 provider 的完整迁移同步更新，不保留新旧两个权威。

没有跨 provider 导入切换、文件 checkpoint 或远程执行的实现计划。它们会改变现有产品语义，应在本地行为稳定后单独决定。尤其不能把 ADR-0006 的会话历史回退描述为文件系统回滚。

## 本轮验证与限制

- 6 个相关前端测试文件共 84 项通过。安装与账号相关 23 项，导入、消息操作、消息队列和上下文菜单 61 项。这些测试不证明上述 Rust 缺陷不存在。
- 临时 Rust 检查直接引用当前 `background_tasks.rs`。模块已有测试通过，新增的“两条对话各有同名任务”断言失败，实际只保留一条任务。它证明索引冲突，不证明真实 CLI 在本轮恰好产生过同名任务。
- 审批隔离检查直接引入当前 `interaction_state.rs`，只替换未调用的问答模块。两个对话注册 `codex-approve-1` 后，第一个接收端关闭，回答和“总是允许”记录落到第二条对话。初次链接库问题已解决，主代理重新编译运行并复现。此测试不证明真实 CLI 已产生重复 ID，也没有模拟前端事件顺序。
- 本轮未运行完整 Rust 测试、桌面实机、真实 CLI 安装更新、WSL 环境注入或账号切换。没有重跑历史研究记录中的测试。
- 现有 CI 已有 macOS 质量门和 Windows Rust job，见 [.github/workflows/ci.yml](../../.github/workflows/ci.yml)。应将回放接入现有 CI，不再建立平行测试流程。
- 本轮仅新增研究记录。原有 schema 改动、`.impeccable/questions/` 和 `task-GPUI/` 不属于本轮。

本次建议受三个原则影响。Model the Domain 使运行实例、会话绑定、单次运行分别建模。Sequence Work into Verifiable Units 使每个 provider 完整迁移后再进入下一个。Laziness Protocol 使方案优先复用注册表、协议 actor、Job Object 经验和已有测试，不先建多 crate 框架。

[t3-add]: https://github.com/pingdotgg/t3code/blob/6577c6b6185167c0e502fa01f9e8e8f2dee6ff25/docs/internals/adding-a-provider.md
[t3-adapter]: https://github.com/pingdotgg/t3code/blob/6577c6b6185167c0e502fa01f9e8e8f2dee6ff25/packages/provider-core/src/server/ProviderAdapter.ts
[t3-capabilities]: https://github.com/pingdotgg/t3code/blob/6577c6b6185167c0e502fa01f9e8e8f2dee6ff25/docs/orchestration-v2/provider-capability-system.md
[t3-scope]: https://github.com/pingdotgg/t3code/blob/6577c6b6185167c0e502fa01f9e8e8f2dee6ff25/packages/shared/src/AgentScope.ts
[t3-linux]: https://github.com/pingdotgg/t3code/blob/6577c6b6185167c0e502fa01f9e8e8f2dee6ff25/apps/server/src/process/agentScope.ts
[t3-claude]: https://github.com/pingdotgg/t3code/blob/6577c6b6185167c0e502fa01f9e8e8f2dee6ff25/apps/server/src/orchestration-v2/Adapters/ClaudeAdapterV2.ts#L359
[t3-acp]: https://github.com/pingdotgg/t3code/blob/6577c6b6185167c0e502fa01f9e8e8f2dee6ff25/packages/effect-acp/src/protocol.ts#L163
[t3-acp-client]: https://github.com/pingdotgg/t3code/blob/6577c6b6185167c0e502fa01f9e8e8f2dee6ff25/packages/effect-acp/src/client.ts
[t3-codex]: https://github.com/pingdotgg/t3code/blob/6577c6b6185167c0e502fa01f9e8e8f2dee6ff25/apps/server/src/orchestration-v2/Adapters/CodexAdapterV2.ts
[t3-testing]: https://github.com/pingdotgg/t3code/blob/6577c6b6185167c0e502fa01f9e8e8f2dee6ff25/docs/orchestration-v2/testing-strategy.md
[t3-replay]: https://github.com/pingdotgg/t3code/blob/6577c6b6185167c0e502fa01f9e8e8f2dee6ff25/packages/provider-testing/src/replayTranscript.ts
[t3-maintenance]: https://github.com/pingdotgg/t3code/blob/6577c6b6185167c0e502fa01f9e8e8f2dee6ff25/packages/provider-core/src/server/maintenanceResolver.ts
[t3-claude-home]: https://github.com/pingdotgg/t3code/blob/6577c6b6185167c0e502fa01f9e8e8f2dee6ff25/apps/server/src/provider/Drivers/ClaudeHome.ts
[t3-codex-home]: https://github.com/pingdotgg/t3code/blob/6577c6b6185167c0e502fa01f9e8e8f2dee6ff25/apps/server/src/provider/Drivers/CodexHomeLayout.ts
[t3-log]: https://github.com/pingdotgg/t3code/blob/6577c6b6185167c0e502fa01f9e8e8f2dee6ff25/apps/server/src/provider/EventNdjsonLogger.ts#L723
[t3-log-summary]: https://github.com/pingdotgg/t3code/blob/6577c6b6185167c0e502fa01f9e8e8f2dee6ff25/packages/provider-core/src/server/nativeProtocolLogging.ts
[t3-oc-server]: https://github.com/pingdotgg/t3code/blob/6577c6b6185167c0e502fa01f9e8e8f2dee6ff25/packages/provider-opencode/src/server/v2/OpenCode2Server.ts
[t3-oc-adapter]: https://github.com/pingdotgg/t3code/blob/6577c6b6185167c0e502fa01f9e8e8f2dee6ff25/packages/provider-opencode/src/server/v2/adapter.ts#L355
