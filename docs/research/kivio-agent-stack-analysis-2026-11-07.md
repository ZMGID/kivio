# Kivio Agent 栈溢出问题分析

本次分析服务于统一运行架构，覆盖长对话、模型与工具执行、schema 和 JSON、子代理及运行清理。基线为 `36655fa18f8d63a35f11256714af807bdc299086`。OpenCode 是已经修复的同类案例，只用于检验归因，不作为待修项目。

结论分三层。已经发生的桌面发送故障支持有限异步组合耗尽栈余量；本次另外复现了 SDK 参数转换中的数据驱动递归溢出；其他解析、复制、销毁和缓冲入口还有需要统一约束的风险。二者不能用同一处 Box、线程栈配置或优化选项同时解决。

配套文档为[目标架构](../kivio-agent-runtime-architecture.md)与[实施方案](../prd/kivio-agent-runtime-redesign.md)。这些是提议，尚未实施，不替代[统一工程规范](../engineering-standards.md)。

## 已证实什么

### 真实发送路径存在有限的异步栈压力

桌面经 WebView IPC、send 预留、reply、应用 run_session、SDK loop、工具分派进入 MCP。SDK 模型和工具循环、Goal 续轮都是迭代；OMP 子运行有独立任务。没有证据表明普通轮数不断累积递归栈。

先前真实 IPC 记录出现 `0xC00000FD`。共享 reply 装箱后文本场景通过，原生 Exa 工具冷连接仍能崩溃。当前 `36655fa1` 把应用 dev 优化级别改为 1，已有工具回归通过。依赖此前已经按 opt-level=3 构建，不能把改善全部归因于 SDK。优化是有效缓解，但不建立以后每个新工具都满足栈预算的结构约束。

相邻 async Future 的状态可能内嵌；构造、移动和 poll 会产生机器栈临时值及调用帧。`Box` 限制内嵌状态，`spawn` 切开跨任务同步 poll，二者作用不同。调用层数、Future 静态大小、某个边界的剩余栈都不能单独证明峰值安全。[SDK owner](../../crates/kivio-agent-sdk/src/agent/loop_.rs)、[应用入口](../../src-tauri/src/chat/session_policy.rs)、[Tokio 调度语义](https://docs.rs/tokio/latest/tokio/task/fn.spawn.html)。

这组桌面结果来自此前诊断与当前修复记录，本轮未重新启动桌面。原产物指纹不完整，原始地址扫描也不是完整 unwind；不能据此指认某一个机器帧或首次致错提交。

### 本次复现了浅 JSON 的长引用链溢出

本轮用隔离诊断进程直接调用现有 SDK 的 `tools::schema::coerce_arguments`，没有复制其实现。schema 只有三层容器，输入值为字符串 `"7"`，但 `$defs` 中包含 1024 个不同的串联 `$ref`。编码仅 31,600 字节。

同一可执行文件、同一 2 MiB 工作线程、同一未优化构建下的结果如下。

| 场景 | 结果 |
|---|---|
| 32 引用，coerce_arguments | 成功，字符串转换为整数 |
| 1024 引用，validate_arguments | 成功 |
| 1024 引用，jsonschema build_map | 成功 |
| 1024 引用，coerce_arguments | 原生栈溢出，退出码 `0xC00000FD` |

[源程序、锁文件、产物身份与日志](kivio-agent-stack-evidence-2026-11-07/README.md)已保存。rustc 为 1.96.0。这个数量是复现样本，不是阈值承诺。它证明 SDK 的 coercion 路径存在另一类可独立触发的溢出，不证明此前桌面 Exa 闪退就是这份 schema 导致。

[Coercion::coerce](../../crates/kivio-agent-sdk/src/tools/schema.rs)直接递归跟随引用。`active` 防止同一路径重入，却不阻止很长的不同引用链。JSON 容器深度、引用图深度、算法总工作量是不同维度。该调用还发生在工具执行超时之前，审批和工具超时无法保护这里。

jsonschema 0.60.2 自身对较深引用编译使用工作表，并平铺保留引用目标以避免长析构链。本次两个控制场景也成功；不能把自有递归归罪于整个依赖。已核对锁定依赖的 `src/compiler.rs:206` 与 `src/validator.rs:541`，包内 VCS 记录为 `0b657fa991c9333908e8c51e8a582d602aac5391`、目录 `crates/jsonschema`，不依赖在线 latest 页面推断。

## 为什么旧版看起来没有这个问题

旧版不是另开线程的新架构。`0d007492^1` 的 reply 已直接 await loop；SDK 抽取前 `4084dc09^` 的应用适配器也直接 await owner/inner。普通工具循环当时就是迭代。

近期变化扩大了多处状态和路径。#74 拆分 policy 并抽出 SDK；#76 增加 deadline/select、停止后等待、inner 的异步 catch-unwind、typed partial result、SessionHistory，以及 provider/MCP 等能力。它们改变异步状态、生命周期和代码生成，但不能仅凭时间先后锁定某一个罪因。

更早的记录已经显示余量问题。[SDK v2 决策记录](kivio-agent-sdk-v2-decisions.tsv)的 T5c 阶段记载 external history 测试栈溢出，开发期间改为装箱 BuiltinHistory。主线 `92f18f50` 是 squash，直接新增了 boxed 字段；不能伪称主线存在单独的“内联改 Box”提交。该事件与后来 IPC 发送是不同实例，说明不止一个热路径受影响。

当前最有根据的解释是：原有深层组合没有明确资源边界，新增状态和路径暴露了过去的余量不足。这个解释仍需同条件历史重放才能进一步归因。旧安装包与今日 debug、不同 rustc 或锁文件不构成有效对照。本轮没有做历史二分，不能断言旧版所有路径都安全，也不能宣布已经找到首次致错提交。

OpenCode 曾在同类宿主链路出现有限栈压力，后经 owned reply 分配边界修复。它支持“同类问题不限于某个模型”的判断，但不能推出 Agent 与 CLI 是同一具体故障，也不产生新的 CLI 重构任务。

## Agent 内还缺哪些结构约束

| 部位 | 现有事实 | 需要解决的问题 |
|---|---|---|
| [schema coercion](../../crates/kivio-agent-sdk/src/tools/schema.rs) | 自有 `$ref`、组合 schema、对象与数组转换递归 | 显式帧遍历；分别约束引用、分支与总工作量 |
| [子代理 structured](../../src-tauri/src/chat/sub_agent/structured.rs) | 多套递归归一、anchor、rebase；部分函数有 64 层限制 | 局部限制未覆盖其他遍历和 clone；统一编译入口和受控表示 |
| [公开模型值](../../crates/kivio-agent-sdk/src/model/types.rs)与[历史载荷](../../crates/kivio-agent-sdk/src/agent/history.rs) | 接收程序构造的裸 Value，存在派生 Clone、序列化与释放 | 文本解析限制不覆盖程序输入及整个生命周期 |
| [SSE](../../crates/kivio-agent-sdk/src/model/sse.rs)与[OpenAI 适配](../../crates/kivio-agent-sdk/src/model/openai.rs) | line、event、tool arguments 可累计；工具序号限制不是字节限制 | 在累计之前限制字节、项数及单次工作量 |
| [MCP](../../crates/kivio-agent-sdk/src/mcp/mod.rs)与[结果映射](../../crates/kivio-agent-sdk/src/mcp/result.rs) | 目录分页有限，但结果先转 Value、clone/序列化后再截显示文本 | 组帧、目录总量、schema、图片和结果分别限额 |
| [SessionHistory](../../crates/kivio-agent-sdk/src/agent/session_history.rs) | 已是平面 entries/parent_id，路径和投影迭代；有全量 clone 与重复投影 | 保留平面结构，减少共享前缀重建和全档案复制 |
| [附件 GC](../../src-tauri/src/chat/gc.rs) | 对协议 Value 递归找引用 | 显式遍历；超预算必须停止删除，不能漏扫后继续回收 |

长消息主要增加堆内存，不能直接叫作栈溢出。真正危险的是数据进入递归转换、clone/format/serialize/drop，以及无界缓冲耗尽其他资源。serde_json 默认的反序列化深度保护不会自动保护已构造 Value 的后续操作。[serde_json 官方说明](https://docs.rs/serde_json/latest/serde_json/struct.Deserializer.html#method.disable_recursion_limit)。

横向抽样还找到自动化图校验的递归 DFS、技能目录递归复制；同时自动化执行队列、部分文件遍历和会话树已经使用迭代。它们验证相同的数据图原则，不在本方案中扩成全项目重写。本轮扫描 Rust 路径并重点审查 Agent 相关实现，不是全部文件、全部依赖、FFI 和前端的安全证明。

## 其他 Rust SDK 实际怎么做

比较固定源码，不以宣传页或“没有听说崩溃”作为证明。没有编译或压力测试这些第三方 SDK。

**Rig，`e1eabf97364b9e4e3bc32c27f721c951d65bd56b`。** `AgentRun` 是不持有 I/O 服务的状态机，输出 CallModel、CallTools、Done；driver 执行动作并交回结果。消息使用 Arc，driver 和工具阶段有明确装箱。普通执行仍可由调用方直接 await，工具使用有界流并发。值得借的是状态与执行分离及稳定能力边界，不能据此声称 Rig 的任意扩展都栈安全。[状态机](https://github.com/0xPlaygrounds/rig/blob/e1eabf97364b9e4e3bc32c27f721c951d65bd56b/crates/rig-agent/src/run/mod.rs#L73)、[驱动分配](https://github.com/0xPlaygrounds/rig/blob/e1eabf97364b9e4e3bc32c27f721c951d65bd56b/crates/rig-agent/src/agent/runner.rs#L557)。

**Swiftide，`e41a89fc18d8998c87f23bfcc4135f3a43518b9e`。** Agent 用 while 推进，模型和工具经动态 Future 接口，工具各自 spawn 后按原顺序收集结果。默认 context 持有历史，但读取会先 clone 全部消息再做摘要视图过滤；摘要不等于内存有界。可借鉴实际任务边界，不能照搬其 JoinHandle 清理方式来满足 Kivio 的 drain 契约。[主循环与工具](https://github.com/bosun-ai/swiftide/blob/e41a89fc18d8998c87f23bfcc4135f3a43518b9e/swiftide-agents/src/agent.rs#L440)、[默认历史复制](https://github.com/bosun-ai/swiftide/blob/e41a89fc18d8998c87f23bfcc4135f3a43518b9e/swiftide-core/src/agent_traits.rs#L339)。

**AutoAgents，`a85503b1e62b015f5bb774c1a5d90a775c0347bc`。** 每 run 的 Context、TurnState 与共享 memory 分开；ReAct 用 for，工具顺序 await。ProducerStream 拥有 producer，drop 能传播到自己持有的 Future，但它仍在 poll 内驱动 producer，并不是独立任务栈。滑窗限制消息数量，也不代替单条大小和 JSON 深度限制。[TurnState](https://github.com/liquidos-ai/AutoAgents/blob/a85503b1e62b015f5bb774c1a5d90a775c0347bc/crates/autoagents-core/src/agent/executor/turn_engine.rs#L105)、[流所有权](https://github.com/liquidos-ai/AutoAgents/blob/a85503b1e62b015f5bb774c1a5d90a775c0347bc/crates/autoagents-core/src/utils.rs#L48)。

PI 作为辅助对照支持单一状态负责人和收尾等待，但 JavaScript await 不能直接映射为 Rust Future 布局。以上比较不支持“一律开线程”或“一律不用任务”的结论。

## 架构结论与证据限度

需要统一三件事：数据由受控表示和非递归算法处理；运行状态与拥有请求的执行阶段分开；桌面为 Agent 原生崩溃建立独立故障域。第三项是 Kivio 的桌面可靠性选择，不是上述 SDK 全都已经提供的能力。

同进程的任务、线程和 catch_unwind 无法兜住所有 native abort。Rust 文档明确 catch_unwind 只处理 unwind，不能捕获进程终止。[Rust 官方说明](https://doc.rust-lang.org/std/panic/fn.catch_unwind.html)。进程隔离负责把 Agent 故障变成受影响运行的可报告失败；它不能替代资源限制，也不能撤销已经发生的工具副作用。

本轮已验证源码引用、历史差异与新的 schema 隔离复现。未做新的桌面回归、历史二分、完整机器栈展开或第三方 SDK 性能比较。目标架构能规定可验证的边界，不能证明任意未来 Rust、FFI 或第三方工具永不出错。
