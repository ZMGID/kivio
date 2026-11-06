# Kivio Agent SDK 分批实现计划

更新于 2026-11-07。当前分支为 `codex/kivio-agent-optimize`。

目标是逐步形成可独立引用的 Kivio Agent SDK。Kivio 桌面、自动化和 OMP 子代理使用同一套核心实现，其他 Rust 程序也能接入模型、工具和自己的宿主。

本文件整理实现顺序和验收条件。SDK-01、SDK-02 已实现；SDK-02 的完整 Rust 库测试和所有构建目标检查已通过。SDK-03 至 SDK-07 尚未开始。工程规则继续以[统一工程规范](../engineering-standards.md)为唯一入口。本计划不增加另一套全仓规范，也不改变已有产品 ADR。

## 1. 确定首版范围

首版按现有 Rust 技术栈实施。SDK 的名称为 Kivio Agent SDK，拟用 Cargo 包名 `kivio-agent-sdk`，拟放在 `crates/kivio-agent-sdk/`。这些路径尚未创建。

首版完成须同时满足以下条件。

- 一个仓库外的 Rust 消费程序只依赖 SDK 的公开接口，就能注册工具、调用真实模型、接收流事件、停止运行并继续一段历史。
- SDK 可以独立构建和测试，依赖图不包含 Tauri、Kivio 应用 crate、桌面 UI 协议或应用全局状态。
- 桌面、自动化和子代理使用同一个 SDK 循环，旧循环和已迁移的重复实现已经删除。
- Kivio 的审批、插话、压缩、工具产物、历史保存及 OMP 生命周期保持原行为。
- 公开 API、可运行示例、支持范围、错误和取消语义有文档，并由消费方测试覆盖。

先完成供本仓和外部示例使用的预发布接口。公开发布包、跨语言绑定、远程服务、动态插件平台、会话树存储和通用图执行器不属于这七项实现。后续有独立需求时另行安排。SDK 化不要求将现有异步 IO 循环重写为 sans-I/O 状态机。

## 2. 对齐参考实现与已有约束

源码调查基线如下。

- Kivio 为 `7a7e10b81b37b35758dff840a32d2e07138bdee6`。
- Pi 为 `4b11a7228d20c37637dd8cb01dabf1fd3c154297`。已拉取到本机 `C:/Users/Zhime/.codex/references/pi-mono`，原 `badlogic/pi-mono` 地址转向 `earendil-works/pi`。
- OMP 子代理行为由 [ADR-0007](../adr/0007-subagents-follow-omp.md)和[已有实现计划](omp-subagents-plan.md)约束。SDK 提取不重定这些语义。

| 参考 | 借鉴到哪一项 | 采用范围 |
|---|---|---|
| [Pi 模型调用边界](https://github.com/earendil-works/pi/blob/4b11a7228d20c37637dd8cb01dabf1fd3c154297/packages/agent/src/agent-loop.ts#L397) | SDK-02 | 区分运行历史、请求投影和供应商协议，保留类型化结果 |
| [Pi 工具与事件类型](https://github.com/earendil-works/pi/blob/4b11a7228d20c37637dd8cb01dabf1fd3c154297/packages/agent/src/types.ts#L424) | SDK-03、SDK-04 | 分开模型结果与宿主展示数据，明确事件生命周期 |
| [Pi 会话压缩](https://github.com/earendil-works/pi/blob/4b11a7228d20c37637dd8cb01dabf1fd3c154297/packages/coding-agent/src/core/agent-session.ts#L3081) | SDK-05 | 会话拥有触发、取消和提交，算法拥有摘要计算 |
| [Rig 的 Tool](https://docs.rs/rig/latest/rig/tool/trait.Tool.html)，查阅时页面标记为 0.46.0 | SDK-03 | Rust 关联类型表达工具参数、输出和错误，动态工具在分发边界适配 |
| [LangGraph 持久化职责](https://docs.langchain.com/oss/python/langgraph/persistence) | SDK-05 | 明确会话检查点与跨会话数据的不同所有权，不引入其图执行器或存储格式 |

Rig 与 LangGraph 的依据是 2026-11-06 查阅的官方文档，不等同于完成它们全仓的源码评审。实现前若依赖某个具体行为，应核对当时采用的版本。参考项目不自动成为本仓的新依赖。

以下调查基线决定迁移顺序；各项完成情况见第 4 节。

- `planning.rs` 的 `FinalAnswer` 不携带答案，通过 `RunState` 回填终答状态。类型化流结果又经过 JSON 组装和解析。
- `AgentRunConfig` 和 `RunState` 混合核心运行数据、应用配置和产品状态。
- `AgentHost` 同时承担通知、审批、持久化和 OMP 能力，并向核心暴露 `tauri::AppHandle`。
- `compaction.rs` 反向访问 `commands::context`，涉及手动压缩、图片估算和重放来源。
- 原生工具已有 `NativeToolEntry` 注册表和逐工具的 `parallel_safe`，应继续复用。
- 模型层已有 `ModelMessage`、`MessagePart`、`GenerateRequest` 和 `LanguageModelProvider`，但实际 provider 仍依赖 `AppState` 和应用设置。
- `scripts/check-architecture.mjs` 当前检查 TypeScript 依赖，不能证明 Rust 核心已隔离。

## 3. 划定目标职责

| 所有者 | 持有的内容 | 对调用方提供什么 |
|---|---|---|
| SDK 核心 | 单次运行、模型步骤、工具批次、待处理输入、取消与终态 | 类型化请求、结果、事件和运行控制 |
| 模型适配 | 供应商协议转换、传输和流解码 | 通用模型请求与响应；应用提供已解析的凭据和配置 |
| 工具适配 | 参数解析、工具实现、结果转换及执行属性 | 工具声明和可调用实现 |
| 会话与功能模块 | 会话历史提交、压缩策略、Todo、技能、OMP 协作规则 | 有具体输入和返回值的步骤准备、续跑决定、检查点等接口 |
| Kivio 宿主 | Tauri、UI segments、应用配置、用户审批界面、应用存储 | 将 SDK 事件映射到现有产品协议，并实现需要等待的宿主操作 |

默认内置的能力可以位于 SDK 核心之外，桌面仍按原配置启用。先按已有模块组织功能，不为每项能力新建 crate 或通用插件注册系统。

通知事件与控制调用分别设计。审批回复、持久化成功确认、下一步输入和取消清理必须有明确返回值。不能把它们改成发出后不等待的通知。

## 4. 按七项实现依次推进

依赖顺序为 `SDK-01 → SDK-02 → SDK-03 → SDK-04 → SDK-05 → SDK-06 → SDK-07`。每项可以包含多个小提交，但每个提交须保持可构建。完成本项的行为验证后再推进下一项。

### SDK-01 让模型步骤返回完整结果

**交付物。** 一次模型步骤的明确结果契约，以及可重复执行的现有行为基线。

**主要位置。** `agent/planning.rs`、`agent/loop_.rs`、`agent/finalize.rs`、`agent/steering.rs`、`agent/loop_tests.rs`。路径均相对于 `src-tauri/src/chat/`。

- [x] 先运行相关现有测试，记录实际通过与失败，不把历史报告当作当前基线。
- [x] 让 `PlanningStepOutcome::FinalAnswer` 自携响应，继续和收尾消费同一个值。
- [x] 删除 `RunState` 的终答回填字段和确认无必要的流式标记。收窄辅助函数可修改的状态。
- [x] 用明确结果表达本步骤的继续、工具调用、完成、取消和恢复，不把所有产品状态塞进一个新枚举。
- [x] 保留现有终答检查顺序与 hook 配对。正常 planning 已有终答时直接 finalize，保留无工具和供应商不支持工具时的 fallback。

**验收。** 纯文本、工具往返、空响应重试、终答时插话、follow-up、取消和已完成工具的保存均保持原行为。终答边界的新消息只触发一次继续，不重复保存或执行工具。测试通过真实 `run_agent_loop` 入口观察结果。

**基线发现。** 2026-11-06 的实测发现：普通 planning 空响应在 `validate_stream_output` 中被拒绝，只发出一次请求，没有进入 `RetryEmptyResponse`。原注释描述的一次重试不适用于这条路径。SDK-01 固定其当前报错行为，保留现有重试结果分支；是否统一空响应的重试入口，留给 SDK-02 的错误模型工作，不在结构重构中改变策略。新增终答检查点测试验证普通输入只续跑一次、已保存的子代理输入不重复提交、工具只执行一次，以及循环尾检查点的消息顺序。

重构前执行 `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/win-cargo-test.ps1 --lib chat::agent`：274 通过、0 失败、2 忽略（真实供应商测试）。首次新增的空响应测试误以为会重试，实际为 273 通过、1 失败；核对流校验代码后改为记录当前行为，再得到上述基线。新工作树通过现有 `build:swift` 和 `build:word` 脚本补齐 Tauri 构建资源。

**实现结果。** `FinalAnswer(Value)` 拥有终答，主循环局部变量接收收尾值；继续路径直接将答案写入两份历史。删除 `ChatPlanningStep`、`planning_final_message`、`planning_final_streamed` 和不可达的补发分支。Todo 最终检查改为只读函数，返回提醒，由循环按原顺序追加。保留两次 finishing 检查点、Hook 释放时机、取消和 synthesis fallback；其他结果分支沿用已有枚举。

**验证结果。** 重构后执行同一命令：274 通过、0 失败、2 忽略，与基线逐项测试结果一致。`npm run architecture:check` 的 12 项检查器测试及现有边界检查通过（该检查器覆盖 TypeScript，不证明 Rust SDK 隔离）。`git diff --check` 通过；四个生产文件的 rustfmt 检查只有 `loop_.rs` 原有的审批配置链格式差异，本批改动未引入新的格式差异。独立源码审查未发现行为回归；注释审查确认删除的 8 行说明无遗留引用，没有新增注释或抑制规则。

未进行桌面实机与真实供应商验证；两项 live 测试仍按原配置忽略。SDK-01 不包含独立 crate 提取或 SDK 发布。

### SDK-02 贯通类型化消息与模型接口

**交付物。** SDK 可使用的消息、模型请求、响应、结束原因和错误模型。

**主要位置。** `chat/model/types.rs`、`chat/model/mod.rs`、`chat/agent/provider_runtime.rs`、`planning.rs`、`synthesis.rs`、`stream.rs`、`stop.rs`，以及历史构建和结果保存的直接调用方。

- [x] 复用并收敛已有 `ModelMessage`、`MessagePart`、`GenerateRequest` 和 `LanguageModelProvider`，明确其与 `ProviderRuntime` 的分工，合并无收益的转发。
- [x] 拆除通用消息对 `mcp::ChatToolArtifact`、通用错误对应用 `api` 分类函数的反向依赖，以通用内容和错误数据承载所需语义。
- [x] 明确系统提示、运行消息和消息来源元数据的形状，保留请求历史与持久化增量的不同语义。
- [x] 删除模型步骤内部的 `ChatStreamOutput → JSON → 字段解析` 往返。DSML、旧工具配对修复及供应商兼容逻辑集中在对应输入边界。
- [x] 将已知结束原因和核心错误类型化。外部未知原因保留原值与诊断，不靠匹配错误文案控制核心流程。
- [x] 保留工具参数、JSON Schema、MCP 负载和供应商不透明块需要的动态 JSON。
- [x] 将桌面设置解析成请求所需配置，开始移除通用接口中的整个 `Settings` 和 `AppState`。

**验收。** 现有 OpenAI Chat、Responses、Anthropic、Gemini 请求和回放测试通过。图片、工具调用 ID、DSML、Gemini 签名、Anthropic 原始块、Responses 推理项和来源标记不丢失。旧会话读入后继续执行正确，不要求本批改写用户存储格式。

**实现结果。** 运行循环使用 `RuntimeMessage`，以 `RuntimeBody::System | Model` 区分系统提示和模型消息，以 `RuntimeSource` 保存来源。工作历史与生成增量仍独立维护。`ProviderRuntime` 负责应用请求策略和宿主配合，`LanguageModelProvider` 负责协议调用；普通模型步骤直接传递 `GenerateOutput`，删除 `ChatStreamOutput`、三组旧 completion 包装函数、重复的 `provider_messages` 和响应 JSON 字段解析。旧格式转换留在入口、存储、OMP 快照和实际 L2 压缩边界。

`FinishReason` 保留类型、原始值和详情；`ModelError` 保存分类、状态码、供应商错误码及流重试资格。模型类型使用通用 `ModelArtifact` 和解析后的 `ModelTool`，不再依赖 MCP 产物或应用错误分类器。桌面分发器逐次解析 OAuth 和供应商配置；四个适配器借用所选 HTTP client、共享重试状态与用量/调试回调。部分模型元数据和协议头仍使用应用供应商配置，独立 crate 隔离留给 SDK-06。

**行为与兼容。** Responses 加密推理项现在能随纯文本终答、压缩后的终答保存并重放。恢复结果区分实际模型重放和本地降级文本，保留空正文降级时的原推理。DSML 在空响应及截断检查前归一化，保留自然语言前缀和调用 ID；`length` 截断的 DSML 批次拒绝执行。Anthropic 起始块中的完整工具参数与增量参数按协议分别处理，规划终答沿用规范消息的文本块换行。旧图片别名在入口转换后保留字节、detail 和回放能力。普通 planning 原始空响应仍只请求一次后报错，不在本项改变重试策略。

**验证记录。** 2026-11-07 执行 `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/win-cargo-test.ps1 --lib`，3154 通过、0 失败、55 按原配置忽略。全量测试首次发现 9 个 Python 占位程序导致的环境失败；仅将已安装 Python 的目录加入测试进程 PATH 后重跑，未修改系统配置。另一个图片计量失败由生产路径测试暴露，补齐兼容入口后通过。SDK-02 的模型、循环、保存/重载、压缩、图片计量、OMP 和 HTTP 重试测试均包含在此次执行中。`cargo check --manifest-path src-tauri/Cargo.toml --all-targets` 和 `git diff --check` 通过；构建仍有存量警告。`npm run architecture:check` 的 12 项检查器测试及边界检查通过；该工具只检查 TypeScript，不能证明 Rust SDK 隔离。

独立源码审查发现并修复两处 Anthropic 回归。注释审查删除 6 条新增冗余说明，将流重试字段改为表达实际语义的名字，并让原图片用量测试调用生产类型化路径。审查未要求新增依赖或另建工程规范。未运行真实供应商、OAuth 登录或桌面实机流程；本项不提取或发布 SDK crate。

### SDK-03 统一类型化工具契约

**交付物。** 可注册、可执行、可测试的 SDK 工具接口，兼容静态 Rust 工具与动态 MCP 工具。

**主要位置。** `chat/agent/execute.rs`、`rounds.rs`、`mcp/native_registry.rs`、`mcp/registry.rs`、`mcp/types.rs`，以及实际工具适配器。

- [ ] 为静态工具提供参数、输出和错误的类型约束。复用已安装的 serde 与 schemars，避免额外复制 schema。
- [ ] 动态工具在解析和校验边界后进入统一执行流程，保留校验失败和参数截断时拒绝执行的行为。
- [ ] SDK 只执行调用方显式注册的工具，不自动扫描或启用桌面的文件、命令和用户配置。
- [ ] 区分模型可见结果、结构化结果和宿主展示数据。将 `McpToolCallResult` 适配到通用工具结果，避免 SDK 核心依赖 MCP 或桌面产物类型。
- [ ] 复用现有注册表，把工具特有的超时与调度属性归回工具定义，减少执行器按名称分支。
- [ ] 保留现有逐工具并行属性、审批和会话授权检查。写操作不能仅因公开了并行选项就自动并发。
- [ ] 迁移调用方后删除被替代的规则和结果转换，不留下两套工具执行流程。

**验收。** 新增一个普通工具只需工具实现和注册，不修改核心循环。覆盖未知工具、非法参数、拒绝审批、超时、取消、结果准备失败、串并行混合及动态工具调用。工具卡与模型接收的结果一致，失败不会显示为成功。

### SDK-04 分开事件、运行控制和宿主操作

**交付物。** 可供不同宿主消费的候选 `AgentEvent`、运行句柄和必要的宿主接口。类型名在实现时与已有命名统一。

**主要位置。** `chat/agent/host.rs`、`stream.rs`、`hooks.rs`、`commands/agent_host.rs`、`sub_agent/executor.rs`、`automation/application.rs`，必要时涉及现有协议映射和测试。

- [ ] 明确运行、步骤、消息和工具事件的配对、顺序、终态及迟到事件规则。
- [ ] 让 SDK 事件表达运行事实，Kivio 适配器负责生成 UI segments 和现有 IPC 消息。复用现有协议生成入口，不建立第二套桌面传输协议。
- [ ] 定义停止、steer、follow-up 的受理、消费及清空语义。同一会话的并发提交须有明确串行或忙碌结果，不暗中重入。
- [ ] 明确会话、单次运行和历史版本的身份及所有权，调用方提交意图或快照，不直接修改内部 map、锁和状态字段。
- [ ] 保留审批与检查点的可等待调用。明确取消何时停止接收结果、何时等待工具清理、何时允许下一轮启动。
- [ ] 明确事件消费者断开或变慢时的行为和资源释放，避免无界事件积压。
- [ ] 从核心接口移除 Tauri `AppHandle`，将 speculative task 所需应用操作留给 OMP 宿主适配器。

**验收。** 一个内存事件接收器和 Kivio 适配器观察到相同的运行事实。取消后无迟到结果污染下一轮；事件结束与资源释放语义一致；审批不会因事件监听器缺失而被默认允许。三类宿主的结果、事件和持久化行为均通过集成测试。

### SDK-05 隔离会话与产品功能策略

**交付物。** 核心循环可在不启用 Todo、技能和 OMP 的情况下运行；桌面通过已有模块组合保留这些能力。

**主要位置。** `chat/agent/loop_.rs`、`compaction.rs`、`context_estimate.rs`、`commands/context.rs`、`commands/reply.rs`、`chat/todo.rs`、`skills/`、`chat/sub_agent/` 及 workflow hooks 所属模块。

- [ ] 先迁移手动压缩流程和来源 ID 累计，消除 `agent` 对 `commands::context` 的反向依赖。图片估算归现有估算模块，共用摘要算法继续复用。
- [ ] 再把 Todo 提醒、技能激活和功能特有的续跑规则收回对应模块，给循环返回明确输入或决定。
- [ ] 最后迁移 OMP 检查点、投递和等待协作。保留先后顺序，不能让多个 hook 隐式争抢同一份历史。
- [ ] 压缩策略负责是否触发和采用结果，会话负责人负责持久化提交，核心只应用已确认的上下文更新。
- [ ] 保留 CAS、checkpoint-before-replace、取消和失败时原历史不变，以及已落盘子代理消息不重复提交的约束。
- [ ] 对恢复文档明确区分恢复对话、继续运行和重放外部副作用。没有完成记录的写文件或命令不自动重放。

**验收。** 按上述三个小批次分别验证。SDK 基础循环关闭所有产品功能仍能完成模型和工具往返；桌面默认能力完整。覆盖压缩失败与并发编辑、插话与子结果竞态、Todo 最终检查、技能 reload，以及 OMP idle、parked、复活、kill 和用户停止后的自动开轮抑制。

### SDK-06 抽出独立 crate 并接回生产入口

**交付物。** `crates/kivio-agent-sdk/` 可独立编译，三个生产入口依赖同一个 SDK。此路径为拟新增。

**主要位置。** 新 SDK crate、`src-tauri/Cargo.toml`、锁文件、`chat/model/`、`chat/agent/`、三个宿主及必要的构建脚本。

- [ ] 迁移已经收敛的核心、公开类型和必要接口，应用通过 path dependency 引用。先保持单个 SDK crate，以模块区分职责。
- [ ] 核心和公开接口不能依赖 Kivio 应用 crate、Tauri、`AppState`、`Settings` 或 UI DTO。用独立消费测试和 Cargo 依赖图验证。
- [ ] 提取至少一条实际模型协议实现供非桌面宿主使用，建议从现有 OpenAI-compatible Chat 路径开始。复用协议编码和流解码，不另写演示专用 provider。
- [ ] 凭据和应用配置在宿主解析，provider 接收窄配置和传输依赖。桌面 OAuth、密钥轮换、请求日志及计费仍由原负责人接入，重试职责和次数不能叠乘。
- [ ] 将桌面、自动化和 OMP 子代理接回 SDK，同时删除旧循环实现。其余已有供应商可先通过应用适配器接入同一 SDK，文档明确独立 SDK 首版提供哪些协议。
- [ ] 核对 `chat/model` 被 Lens、翻译等非 Agent 路径复用的调用方，保留它们的能力并避免复制 provider 实现。
- [ ] 核对新增 crate 对锁文件、CI 缓存和 Windows 测试脚本目标目录的影响，不为引入 SDK 强制重排整个 Cargo workspace。

**验收。** SDK 独立测试不启动桌面或读取用户配置。外部消费程序通过公开接口完成真实模型、普通工具、流事件和取消。Kivio 三类入口使用同一核心；已有供应商能力无回退。仅 fake provider 的测试通过不足以完成本项。

### SDK-07 固定首版消费方式与持续验收

**交付物。** 有明确支持范围的 SDK 预发布版本、文档、示例和独立质量检查。

**主要位置。** SDK README、公开 API 文档、examples、外部消费 fixture、`.github/workflows/ci.yml` 和必要的现有检查脚本。

- [ ] 提供最小接入、工具调用、事件处理、取消、steer 与 follow-up、历史继续的可运行示例。示例只调用公开 API。
- [ ] 加入独立 consumer fixture，验证调用方无需应用内部类型、私有模块或桌面运行时。
- [ ] 写清公开与内部接口、错误分类、事件顺序、取消完成、会话持久化责任和预发布兼容策略。
- [ ] 写清 SDK 的工具注册能力与桌面审批 UI 的区别，宿主必须明确选择授权策略，不能通过遗漏适配获得权限。
- [ ] 配置 SDK 独立的 Windows、macOS、Linux 编译和离线契约测试。桌面继续按现有支持平台验收。
- [ ] 用本地 HTTP 服务驱动生产协议实现，验证第一次返回工具调用、第二次收到对应工具结果、最终流式答案和续聊。断流与错误注入也通过该边界验证。
- [ ] 实际有变化的运行路径做基线对照，记录模型请求次数、工具执行次数、取消清理和持久化结果。性能测量分离网络等待与 SDK 自身开销，不把架构重构直接称为提速。
- [ ] 完成一次受支持 provider 的真实模型 smoke test 和 Windows 桌面回归。无模型凭据或无对应平台时记录未验证，不宣称该项完成。
- [ ] 删除迁移期兼容入口和过时文档，检查包内容和许可证声明。首版交付不自动推送、公开发布或修改现有许可证。

**验收。** 全新消费方按 README 接入成功，CI 能阻止 SDK 重新依赖桌面，所有迁移入口共享一个核心。每个支持平台和 provider 的已测范围有证据，未测范围明确列出。

## 5. 执行时复用现有验证入口

以下命令来自现有项目。每批先运行相关测试，再按实际改动执行完整要求，不为纯文档变更启动桌面构建。

```powershell
powershell -File scripts/win-cargo-test.ps1 --lib chat::agent
powershell -File scripts/win-cargo-test.ps1 --lib chat::model
powershell -File scripts/win-cargo-test.ps1 --lib chat::commands
powershell -File scripts/win-cargo-test.ps1 --lib chat::sub_agent
```

模块迁移后应改用新的测试过滤路径。全量 Rust 测试使用 `scripts/win-cargo-test.ps1`。涉及前端与共享协议时，执行 `npm run typecheck`、`npm run lint`、`npm run test` 和 `npm run architecture:check`；协议变化通过现有 `protocol:generate` 与 `protocol:check` 管理。

SDK crate 创建后，再增加以下检查。这些命令目前尚不可执行。

```powershell
cargo test --manifest-path crates/kivio-agent-sdk/Cargo.toml
cargo check --manifest-path crates/kivio-agent-sdk/Cargo.toml --examples
cargo doc --manifest-path crates/kivio-agent-sdk/Cargo.toml --no-deps
```

SDK 的确定性测试先用脚本化模型与受控工具固定事件、请求和写入结果，真实网络调用另做 smoke test。涉及桌面行为的批次使用 Dev 身份实机验证，防止触碰正式版用户数据。性能阈值在取得同场景基线后确定，本计划不预填未经测量的数字。

## 6. 记录决定与当前状态

本计划采用 Foundational Thinking，先收敛类型与行为边界，再提取 crate。采用 Sequence Work into Verifiable Units，每批给出可观察验收结果并删除迁移后的旧路径。采用 Minimize Reader Load，以调用方需要了解的状态和执行顺序减少作为收益。

SDK-01 已完成模型步骤结果的内部重构及 Agent 自动化测试。当前尚未提取独立 SDK crate，也未进行 SDK 消费原型、基准测试或实机验证。后续公开 Rust 签名、事件背压方案和 provider 配置拆分需要在对应任务中用小型消费样例验证，再固定为公开契约。它们不作为等待用户回答的架构偏好问题。

进度检查点为七项串行实现，重叠文件不并行修改。调查和独立验证可以并行。每项记录变更提交、检查结果和未验证范围；当前不创建 PR、不启动定时任务，也不估算未经验证的工期。

决策记录见 [SDK 规划决策记录](../research/kivio-agent-sdk-decisions.tsv)。下一项为 **SDK-03 统一类型化工具契约**。
