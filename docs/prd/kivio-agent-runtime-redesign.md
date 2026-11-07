# Kivio Agent 栈修复实施计划

2026-11-08，按本轮确认的 P0/P1/P2 方案实施。目标是降低固定宿主栈开销，把工具执行切到独立任务，保留权限快照、取消、进度与历史事实。本文取代先前的独立 worker 进程方案。外部 CLI 不重构。

## 完成标准

- SDK 工具执行使用 owned invocation 和独立 Tokio 任务，不依赖调用方借用的执行 Future。
- 每次运行冻结一份有效工具设置、provider、plan、权限和定义，任务使用模型当时看到的版本。
- loop 同时处理有界进度通道、完成、停止与超时；正常终态之前已经 join 对应任务。
- completion-required 工具在普通停止后 drain；其他工具在停止或超时后 abort 并 join。未知远端结果保持未知且不重试。
- 历史结果按原调用顺序写入，串并行规则及并发上限不变；OMP 保留已有独立任务。
- opt0、2 MiB 入口、真实 IPC 与取消/销毁回归通过后，删除被替代的 stacker 包装与依赖。

## P0：保留止血并补全主线程栈配置

保留工作线程 8 MiB、schema 深度/总步数限制，以及 Conversation 读取进入阻塞线程池的修改。

在现有 build.rs 的 Windows MSVC 分支设置 `/STACK:8388608`，保证从仓库根使用 --manifest-path 构建时也生效。验证 EXE 的实际 reserve。该设置覆盖初始栈，不代替 Tokio worker 配置。

## P1：缩小固定宿主状态

按准备、执行和提交责任拆解 complete_assistant_reply_inner，阶段返回拥有的数据，Future 独立堆分配。大型 Conversation、Settings、运行配置移动或共享，避免嵌套 async 状态重复持有完整值。

保留 CLI、图片、多答、Goal 和探针分支。generation 登记后立即建立 guard，再做可失败的 Goal claim。最终提交与组提交仍由既有负责人完成。

frames.ps1 仅定位候选，它按机器码模式和部分符号估计，并跳过析构，不能证明全部帧或峰值。32 KiB 排查线、64 KiB 优化目标不是唯一验收。最终检查同产物实际栈证据及完整入口的构造、poll、失败和释放。

## P2：owned 工具任务

AgentRunHandle 拥有身份与 Arc control。ToolExecutionContext 使用拥有的共享字符串。ToolInvocation 拥有 context、定义、参数及进度发送端。AppHandle 只在应用层，AppState 在任务内部取得。

应用准备先应用有效会话权限，再冻结 Arc<RunToolSnapshot>。host 和 dispatcher 为拥有的共享服务，不克隆整个 AppState。

ToolExecutor.call 接收 Arc<Self> 与 ToolInvocation，返回 static Future。实际 call 构造在 spawned async 块内执行。before/after 保留时序，其中 schema、审批与工作流 Hook 一并验收，不能因包装函数短就认定安全。

进度使用有界 channel。owner 等待 hook/call/join 时持续消费，完成后处理已入队进度再发布终态。禁止阻塞发送和无限队列。

正常取消和超时逐任务处理，不对包含 drain 工具的集合直接 abort_all。panic 通过 join 错误映射 typed failure。父 Future 异常 drop 使用 abort-on-drop 兜底，不承诺同步 Drop 能异步 drain。正常终态与 Dropped 契约区分。保留现有 round 输入顺序聚合，不再叠加顺序容器。

## 验证与交付顺序

1. 固定工作区基线、真实 IPC 工具用例及构建身份，记录已有通过和失败的诊断。
2. 完成 P0，验证实际 PE reserve；完成 P1，验证宿主与提交行为。
3. 修改 SDK 类型及任务边界，同批迁移应用、registry、AgentSession、示例和独立 consumer，删除旧接口。
4. 验证 hook、并行顺序、取消/timeout、drain、panic、父 future drop、通道背压与晚到进度；覆盖模型、恢复、摘要及销毁。
5. 2 MiB 测试核对实际执行线程，不能把工作调到 8 MiB worker 后误报通过。send_reserved 层及真实 WebView IPC 都覆盖。
6. 无 stacker 回归通过后删除相关模块、依赖和锁文件项，更新契约与 probe 文档，执行格式、类型、架构及真实工具检查。

本计划不以任务边界替代 schema 图和深 Value 的独立治理，也不声称 join 能捕获原生栈溢出。实施事实记录于 [决策日志](../research/kivio-agent-stack-fix-decisions.tsv)，未运行部分明确标注。
