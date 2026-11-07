# Kivio Agent SDK 第二版方案

状态为实施中。T1、T2、T3、T4a 已完成测试，T4b 正在实现。更新于 2026-11-07。

执行队列分为九个交付单元。T1 核心契约，T2 Session 与 hooks，T3 四协议 Provider，T4a Skill，T4b MCP，T5a 上下文，T5b 会话树，T5c 持久化迁移，T6 应用回归与交付。详细范围、依赖和验收集中在第 5 节。下一项是 T4b。

第二版面向两类使用者。外部 Rust 程序可以直接创建 Agent、注册工具、选择协议并管理多轮运行。Kivio 桌面、自动化和 OMP 子代理继续共享同一核心，维护者只在一个地方修改协议和运行规则。

"第二版"表示第二阶段能力建设，不表示现在发布 Cargo 2.0。现有版本为 `0.1.0-alpha.1`。建议交付为新的 alpha，随迁移说明一起确定版本号。

## 1. 使用本方案

工程规则以[统一工程规范](../engineering-standards.md)为唯一入口。本文记录范围、选择和验收，不增加平行工程规范。领域名称以 [CONTEXT.md](../../CONTEXT.md) 为准，子代理行为继续遵守 [ADR-0007](../adr/0007-subagents-follow-omp.md)。

按第 5 节逐批实施。一个勾选项只有在对应测试、日志或源码差异存在后才算完成。实施者记录基线 SHA、最终 SHA、命令及结果。研究结论不等于功能验收。

本计划按统一工程规范执行存量迁移。每项先核对下文的现有实现，优先直接复用，其次提取并移除应用依赖，最后补确实缺失的能力。Pi 用来校验职责和行为，不作为重写现有 Kivio 实现的理由。新增代码须对应具体缺口，不能只因命名与 Pi 不同而替换已有模块。

本计划采用 poteto-mode 的调查及多阶段规划方法，并按仓库轻量规则缩减通用程序模板。实施按每批实际影响选择验证，不安排固定十条验证通道或未经测量的性能门槛。任务状态记录在第 5 节。本次实施由用户明确授权按队列推进并逐项测试、提交，不自动发布或合并。

调查阶段没有性能测量；实施验证和反馈路径记录于第 5 节及决策记录。

## 2. 固定研究基线

- Kivio 基线为 `0d007492`，SDK 首版已经合并。现有未提交应用修改不属于本方案。
- Pi 固定为 `085c84e98e7f4c950f2be606466fff4f75d8fe7c`。它的 agent 核心、模型类型和 SDK 创建入口相比首版研究基线 `4b11a722` 没有差异。本方案使用固定源码，不依赖滚动文档推断。
- 初次研究时执行 SDK 单元测试，98 项通过。独立消费者的两组契约测试共 18 项通过。后续计划审查未重跑这些测试，没有验证桌面全量测试、真实模型或跨平台运行。

参考源码见附录 A。首版交付范围及历史验证见[首版计划](kivio-agent-sdk-plan.md)和[首版决策记录](../research/kivio-agent-sdk-decisions.tsv)。

## 3. 确定完整能力范围

下表按外部使用者的完整接入过程界定第二版范围。SDK 只接收通用能力，产品功能保留在应用层。

| 能力 | 当前 Kivio SDK | Pi 的参考做法 | 第二版决定 |
|---|---|---|---|
| 创建 Agent | 直接组合六个循环参数，自己实现信箱并重复配置身份 | `Agent` 持有历史、队列、活动运行 | 增加可直接使用的会话入口，同时保留现有高级宿主入口 |
| 模型和请求配置 | `GenerateOptions` 有温度和 provider options，循环配置未完整传入 | 模型选择和 stream options 显式传递 | 贯通已有选项，定义覆盖优先级及能力不足时的行为 |
| Provider | 只有 Chat Completions 可独立使用 | 模型核心、provider 工厂和协议实现分别导出 | Chat、Responses、Anthropic、Gemini 四协议独立可用，桌面共享实现 |
| 工具注册 | 有 `StaticTool` 和 schema，接入仍需自己写 executor | 工具声明、execute、更新及前后钩子组成一个可注册单元 | 提供基于现有类型的注册执行器，授权策略显式注入 |
| 工具数据 | 类型化输出仍经过 OpenAI 形状 JSON 再导入历史 | `ToolResultMessage` 直接进入历史 | 类型化贯通，保留错误、图片、结构化内容和展示数据的不同用途 |
| 控制和排队 | 底层单次 handle 已有代际隔离，宿主提供队列 | Agent 管当前取消控制器与两类输入队列 | 管理独立运行状态，补齐等待完成、取消后再运行和未消费输入规则 |
| 错误和结束 | 模型错误类型化，循环仍返回字符串，结果与事件终态粒度不同 | 消息保留 error/aborted，Agent 负责完成生命周期 | 保留错误来源和部分结果，统一可观察结束原因 |
| 历史与恢复 | 有规范历史、增量和旧 JSON codec，缺少便捷独立快照契约 | Agent 管内存历史，产品层管理 session store | 支持显式导入导出及版本校验，持久化仍由宿主负责 |
| 会话树与分支 | 应用已有复制新对话式 fork、截断式 rewind/regenerate；SDK 没有统一树模型 | SessionManager 用带 id/parentId 的 entry log 和当前 leaf 投影上下文 | P1 交付非破坏性分支、active leaf、切换及恢复；P0 先确定 entry 和投影契约 |
| 上下文 | 有 `SessionPolicy` 接口，实用策略位于应用 | core 接受 context transform，产品层执行压缩 | 给出可独立使用的预算及摘要策略，复用应用现有算法 |
| 技能加载 | 应用已实现目录发现、解析、目录提示、按需读取及激活；独立 SDK 未提供 | 资源加载由 coding-agent 会话组合 | 交付按需启用的技能加载模块，支持显式来源、目录摘要、按需正文、参数和资源相对路径 |
| MCP 接入 | 应用使用 rmcp 管理 stdio、Streamable HTTP、工具发现及调用；独立 SDK 未提供 | 不以 Pi core 的范围限制 Kivio 的集成能力 | 交付按需启用的 MCP 客户端及工具适配，复用现有协议库和连接生命周期 |
| 有界运行 | 有单工具超时，缺少公开统一运行步数及截止时间配置 | Agent 的取消和产品策略组合控制运行 | 加入显式可选模型步数与时间限制，保留清理和部分结果 |
| 扩展 | 已有模型、工具、宿主、policy 和事件接口 | 显式 stream 函数及前后处理钩子 | 收窄并文档化已有扩展点，验证自定义模型和工具可接入 |
| 观测与交付 | 有序事件、usage、独立消费者和三平台 CI 配置 | Agent 提供状态订阅和工具事件 | 补错误来源、运行完成及跨 Provider 契约测试，明确公开 API 和兼容范围 |

### 3.1 保留现有产品归属

Kivio 宿主继续管理 OAuth 登录与刷新、密钥轮换、计费落库、技能来源配置与插件安装启停、Todo、审批 UI、文件授权、OMP 身份和作业、会话持久化及应用重试策略。SDK 提供可复用的技能加载及 MCP 接入，接受这些能力的明确输入，输出可等待的结果。

提取现有代码不等于搬入整个应用模块。按以下归属筛选，只把无桌面依赖的通用部分放入 SDK。

| 归属 | 包含内容 | 接入方式 |
|---|---|---|
| SDK 核心 | 消息、模型请求、工具执行契约、循环、取消、事件、Session、通用 hooks、上下文算法和历史树 | 独立 Rust 消费者直接使用 |
| SDK 按需启用的集成 | 显式来源的技能加载、MCP 客户端及工具适配 | 通过核心接口组合，不让核心循环依赖目录扫描或 MCP 传输 |
| Kivio 应用 | Tauri、设置、插件安装启停、密钥仓库、计费、Todo、多答组、审批界面、OMP 调度、外部 CLI、桌面存储及附件界面 | 应用适配 SDK 接口并管理产品状态，不能反向进入 SDK |

hooks 的参数和返回类型使用通用消息、请求、工具、结果及提交信息，不出现 Todo、插件名称、多答组或 OMP 作业规则。桌面历史和资源的兼容验收属于应用迁移，不能据此把 `Conversation`、`ChatMessage`、`AppState` 或 `ChatMcpServer` 搬成 SDK 的必需类型。通用部分需要产品配置时，由宿主传入解析后的值或能力。

第二版提供可插入这些能力的接口和独立示例。外部 Rust 消费者不必加载 Kivio 设置，不自动扫描技能目录，也不自动获得文件或命令执行权限。

### 3.2 记录暂缓范围

以下能力保留为后续独立需求，不是因为原型没实现就认定永远不需要。

- 云厂商特有传输，例如 Bedrock、Vertex 和 Azure 独立认证流程。四个协议先覆盖 Kivio 当前主要调用路径。
- 面向外部消费者的完整文件、shell 和子代理工具包。技能加载和 MCP 客户端已纳入第二版；它们不自动授予文件、命令或远端工具执行权限。
- 自带数据库、分布式任务、跨进程恰好一次执行、远程 Agent 服务和通用工作流引擎。会话树与分支已纳入本版 P1，文件或数据库存储仍通过宿主接入。
- Python、JavaScript 绑定、自动发布和许可证修改。

这些范围不会阻止自定义 Provider、工具或持久化宿主通过公开接口接入。第二版完成标准包含相应的独立消费示例。

## 4. 采用一种核心和两种接入方式

建议保留一个 Rust crate，按现有 `model`、`tools`、`agent` 模块推进。先通过依赖和接口消除耦合，不为对应 Pi 的 npm 包数量而拆 crate。

现有循环继续是唯一执行核心。便捷 Session 只补默认所有权和使用入口。桌面继续用高级宿主路径组合相同核心及历史操作，不必为了使用第二版再包一层持有相同状态的 Session。

比较了三种实现选择。

| 选择 | 能解决什么 | 代价 | 判断 |
|---|---|---|---|
| 只加默认值和会话包装 | 缩短最小示例 | 内部 JSON、字符串错误及应用协议依赖仍在 | 只能作为局部改进，不足以交付第二版 |
| 收敛契约，补通用会话，迁移四协议 | 覆盖独立使用与桌面复用，逐批删除旧代码 | 需要同时迁移真实调用方并验证历史兼容 | 推荐 |
| 按 Pi 全面重建包体系和产品 SDK | 最接近 Pi 全套能力 | 引入新存储、资源发现和插件体系，重叠现有 Kivio 负责人 | 当前收益不足，保留为未来产品议题 |

### 4.1 由现有数据类型表达核心

以下是拟定契约，命名可在实施时按 Rust API 约定调整。实现前以消费者编译样例验证签名，不先搭通用框架。

- `RuntimeMessage` 继续表达运行历史中的消息。P1 的 entry log 保存完整历史事实，当前分支投影仍使用 `RuntimeMessage`，不另造一套消息类型。`ModelMessage` 和 `MessagePart` 继续表达模型内容。`RuntimeSource` 仅承载宿主来源，不进入供应商请求。
- 工具轮直接产生类型化结果与后续消息。JSON 保留在 schema、动态参数、供应商不透明内容、存储兼容和网络边界。
- `GenerateOptions` 成为每次模型调用的唯一选项结构。请求级显式值覆盖 Provider 默认值。循环的强制禁用工具规则最后应用。未知 Provider 扩展参数保留在协议边界并验证冲突。
- 一个运行身份值提供会话、运行、消息和 generation。配置与控制句柄使用同一份身份。generation 与持久历史版本分别表达。
- 运行结果保留结束原因、错误来源、规范历史、增量、工具记录和 usage。输入准入错误与已开始运行后的失败分别表达。后者必须携带已完成结果，不能只留下字符串。
- 模型失败复用 `ModelError`。宿主检查点、工具失败、运行限制有各自明确来源，不把所有错误强制伪装为模型错误。
- Provider 错误保留 HTTP status、供应商 code、retry-after、发生阶段和重试判断所需信息。调用方不需要解析面向用户的错误文案。

T1 明确以下历史语义，并用具体输入和期望投影记录契约。固定语义不等于提前发布完整树实现，正式存储类型在 T5a 至 T5c 由实际迁移用例验证。

| 契约 | 第二版语义 |
|---|---|
| Entry 身份与关系 | 每个 entry 有会话内稳定且唯一的 ID，parent_id 指向同一会话内的已有父节点或为空。branch_id 暂不作为第二份必填拓扑关系；父链足以确定路径，未来命名分支可单独引用锚点 |
| Entry 内容 | message 复用现有消息载荷，tool call/result 保留在该消息结构中。compaction 单独记录摘要及覆盖边界。metadata/source 保留来源，不再重复保存另一套工具事实 |
| 活动位置 | active_leaf 是会话选择的当前位置，独立于最后追加 entry 保存。fork 从合法锚点建立新的活动路径，原路径末端仍可访问。不是必须增设 branch-pointer entry 类型 |
| 记录与运行消息 | SessionEntry 保存可导出的历史事实；RuntimeMessage 是当前路径供运行使用的投影，也是 message entry 可复用的载荷类型。两者职责不同，无需复制消息 schema |
| 模型投影 | 先按 active_leaf 沿父链选路径并应用有效压缩记录，得到 RuntimeMessage，再复用 request_from_history 构建模型请求。UI-only 元数据不进入模型；无法识别的模型内容明确报错，不静默当作 UI 数据丢弃 |
| 工具配对 | 当前路径上的 result 必须关联到同路径的 call。完成的调用仅有一个最终结果，保留现有拒绝、取消和失败结果语义。导航和压缩不得把 call/result 拆开；加载历史不执行工具 |
| 压缩范围 | compaction 引用其覆盖的祖先范围及首个保留边界，只改变该分支的模型投影，不删除原 entry。覆盖范围必须位于当前父链，完整保留工具轮和必要原生块 |
| 来源保留 | 已有 RuntimeSource 在快照往返中保留，在请求投影中剥离。供应商重放签名留在已有类型化内容中，不混成 UI metadata。凭据和活动执行能力不进入快照 |

T1 的设计验收至少列出普通消息、工具往返、压缩后续聊、两分支选择及来源过滤五组输入和期望结果。调用方据此实现后续任务；发现既有数据反例时修订契约并记录影响，不能用占位实现宣称已支持树。

### 4.2 让会话入口承担完整生命周期

便捷入口暂称 `AgentSession`。它持有内存历史、队列、配置和当前运行。高级应用宿主仍可直接调用核心循环，继续使用已有会话仓库和控制信箱。两种方式调用同一个循环，不串成两个会话负责人。

后文的 Session 提交及历史操作表示 SDK 会话领域能力，不要求高级宿主创建便捷 AgentSession。沿用 `AgentHost::checkpoint_runtime` 的提交边界及宿主预留机制，必要时在该边界补足契约。便捷 Session 组合这些能力，桌面直接接入同一实现，不为取得压缩或分支功能再建立历史、队列或保存负责人。

会话公开入口采用 `prompt`、`continue_run`、`steer`、`follow_up`、`subscribe`、`abort` 和 `wait_inactive`。历史及其只读投影、输入队列、订阅生命周期、当前运行与控制句柄都由该 Session 管理；核心负责产生事件，Session 负责对外订阅和顺序。它不是给六参数循环加一层转发。纯文本无工具示例不要求用户编写空 executor、空 host 或控制信箱。自定义授权工具仍需用户显式提供策略。

运行状态区分空闲、运行、停止请求中和收尾。停止只发出请求。等待结束覆盖该层拥有的模型连接、工具清理、历史更新及已配置的提交操作。便捷内存会话不宣称外部数据已经落盘。

这里的 `AgentSession::wait_inactive` 表示 Session 已收尾，不等同于当前低层 `AgentRunHandle::wait_inactive` 的"核心不再接收输入"。底层接口迁移或保留时必须区分两种契约。等待调用绑定调用时的运行 generation，不被下一轮启动拖住；已空闲时立即返回。`subscribe` 返回可释放的订阅句柄，普通观察回调不获得修改历史或决定授权的权限。并发 `prompt` 返回 Busy，排队通过显式 `steer` 或 `follow_up` 进行。

每轮创建独立取消状态。已结束的单次运行句柄不能控制新轮。会话级句柄操作当前运行，语义与单次句柄区分。未消费输入在结束、错误和取消时有可查询结果，不默默跨代混入新轮。

续跑不得自动把已完成工具重新执行。空历史与完成的 assistant 尾部需要明确行为。建议空历史报准入错误，assistant 尾部没有待处理输入时也报明确错误。用户显式要求继续回答通过新输入表达。桌面已有恢复流程通过宿主策略接入。

### 4.3 保留授权和执行边界

现有 `ToolExecutor::prepare` 是授权与验证边界。基于 `StaticTool` 的默认注册执行器复用 schema 校验、执行属性、进度及结果转换。相同入口服务串行和并行工具。

工具声明不代表执行授权。注册默认执行器时必须选择策略。纯计算 echo 示例可显式允许该工具，文件和命令权限仍由实际宿主处理。

保持 `parallel_safe`、截断参数整批拒绝、结果配对和 `drain_on_cancel` 语义。取消不回滚已产生的副作用。未知执行结果不能自动重放有副作用工具。

将 T1 的注册执行器明确为 `ToolRegistry` 职责，具体命名可沿用仓库惯例。复用 StaticTool、动态 ToolDefinition、call_typed 和现有 executor，集中完成名称冲突校验、声明查询及执行分发。注册变化在 run 准入时形成快照，运行中不任意修改本轮工具集合。Skill 工具和 MCP 工具使用同一注册入口，不另造扩展插件总线。

CommandRegistry 暂不纳入 SDK。当前应用已有 slash command 解析和派发，命令在宿主转换为 Session 操作或普通输入即可，无需改 loop。theme、package 安装、prompt-template 管理和 RPC/CLI 服务也不因 Pi 提供就自动加入。后续有明确消费者需求时再单独规划。

### 4.4 用四协议检验模型抽象

从桌面现有实现提取 Responses、Anthropic 和 Gemini 的编码、流解码、usage 与错误解析。桌面保留账号、OAuth、模型设置解析、运行时重试预算与计费回调。

按协议提供配置结构和构造入口。普通 API key 的独立消费者显式传入 endpoint、认证、client 和模型。需要短期凭据的宿主在请求边界解析凭据，不把登录刷新过程塞进通用模型循环。

协议配置承载各自类型化选项，兼容端点通过配置选择协议，不为每个品牌增加一个 Provider 类。现有模型到 `ModelRuntime` 的适配集中复用，避免四种 Provider 各自复制历史投影和事件转换。HTTP client 可使用默认值，也允许宿主注入。

一个模型步骤可能包含多次 HTTP 请求。Anthropic 原生续轮、Responses 缓存降级和传输重试必须分别记账，并明确各自次数上限与负责人。默认 SDK 不额外叠加自动重试循环。

协议能力与模型能力分开。提供最小模型能力描述，表达上下文上限、输入输出类型、工具、推理及原生搜索，用于请求前校验和配置映射。它不替代协议解析，也不要求维护完整供应商模型目录。供应商特有 reasoning 签名与加密内容继续按来源模型和 provider 隔离。

应用已有 `chat/model_metadata.rs` 的模型信息、覆盖值和协议特征解析。SDK 接收其解析结果，纯协议映射确需共享时原位提取，不维护第二份模型数据库或型号判断表。能力未知与明确不支持分别表达，未知兼容端点不能因缺少元数据被默认拒绝；协议强制约束仍在请求边界验证。

### 4.5 提供可复用上下文和快照能力

请求上下文投影不改变规范历史。摘要压缩只有在成功、缩小上下文并通过宿主检查点后才能更新当前运行上下文。P1 中通过追加 compaction entry 改变当前分支投影，不覆盖树中的原始记录。失败和取消保留原历史。

这是第二版的目标契约。现有 SDK `agent/compaction.rs` 的 `ContextUpdate::SendView` 会替换 `state.runtime_messages`，并非已经隔离的临时视图。T2 先区分请求投影和显式上下文更新，T5a 再迁移预算及摘要算法。沿用现有检查点、取消和 incoming input 处理，不能通过重写压缩循环丢掉这些行为。

从应用现有上下文模块提取可复用的预算计算、完整工具轮保留和摘要执行算法。T2 先将现有策略接入 hooks，T5a 再提取通用算法并迁移至同一套 hooks 与 Session 提交接口，不另建一套压缩管线。应用保留模型选择、持久化和产品提示。

为外部消费者提供带版本的快照导入导出入口。快照保留消息、来源和必要重放数据，不序列化凭据、活动 future 或工具执行能力。未知版本报错，旧桌面会话仍经过原兼容 codec。恢复时重新绑定 Provider 和工具授权。

此处快照针对 SDK 自己的规范历史。外部 CLI 的显示快照及原生会话回退继续遵守 ADR-0002、ADR-0006，不由 SDK 会话接管其模型历史。

新快照导入逐条校验，返回明确失败位置，不沿用旧导入器静默忽略无法解析条目的行为。快照定义为 session entry snapshot，包含历史记录、必要配置和 active leaf；如导出单分支，必须明确包含的祖先闭包及引用。正常导出在 run 收尾后进行，导入后开始新 run。若旧数据包含不完整工具轮，报告不完整及限制续跑，不能自动重放未知副作用。

第二版不提供 mid-run 可恢复状态机，不序列化程序计数器、活动 future、工具执行进度或自动重试游标，也不承诺崩溃后从某次工具调用继续。已有 AgentHost::checkpoint_runtime 仍是宿主保存历史和接收输入的边界，不因此升级为 durable run 引擎。已有部分历史保存可继续存在，但不作为可自动续接的运行快照。

### 4.6 明确观测与完成的边界

保留同步有序事件与 `AttemptDiscarded`。慢订阅者造成背压的行为写入契约，不默认增加无界队列。异步存储和审批继续走可等待的宿主操作。

Pi 的 Agent 在结束阶段等待 `agent_end` 异步监听完成，Kivio 当前通知接口不同。第二版借鉴明确完成边界，暂不把所有通知改为异步生命周期屏障。

事件和结果采用一致的结束原因。观察者断开不批准工具，也不取消运行。诊断默认提供请求身份、步骤、重试原因、耗时和 usage，不输出密钥或完整上下文。详细调试由宿主显式选择。

扩展点区分只读通知与可改变执行的操作。先明确现有请求及上下文处理、工具授权及结果、检查点及收尾接口的调用顺序、失败、取消和修改范围；确有缺口才新增接口。需要异步采集时由可选 owned 事件适配完成，不强制引入遥测后端。

第二版明确提供以下五个 Pi 风格执行 hook。名称表达职责，不要求 Rust 签名照搬 TypeScript。

Pi 的 convertToLlm 是 AgentMessage 到模型 Message 的转换函数，不能仅按名称计数认定缺少第六个行为 hook。Kivio 已有 RuntimeMessage、ModelMessage 和 request_from_history。T2 复用并公开说明该转换边界，覆盖 source 过滤及系统消息顺序；自定义产品记录由宿主投影成通用消息，不新增一套平行消息转换框架。

| Hook | 执行点及允许作用 | 必须保持的边界 |
|---|---|---|
| `prepare_request` | 每次对话模型请求前，输入已接收后，选择模型及请求选项 | 不私自取队列；修改只作用当前请求，更换模型后重新校验能力和原生签名 |
| `transform_context` | 请求准备后、协议编码前，生成此次模型发送视图 | 不原地改规范历史；不能把请求裁剪当成已持久化压缩 |
| `before_tool_call` | 工具参数校验后、真实执行前，允许阻止调用并给出原因 | 宿主授权仍是不可绕过的最终约束；任何后续参数改动都需重新校验和授权 |
| `after_tool_call` | 工具执行结束、最终结果事件及历史提交前，转换模型可见输出或补充数据 | 保存真实执行结果及副作用状态，失败不触发自动重跑；修改正文时处理结构化内容一致性 |
| `finish_turn` | 一次 assistant 响应及其工具结果已完成后，决定正常调度、再继续或结束 | 这是 turn 决策，不是整个 run 的资源清理；错误和取消不能被 hook 复活，继续受运行预算限制 |

所有执行 hook 都有类型化输入输出、错误及取消契约。默认没有 hook 时保留核心行为。钩子顺序由单一组合入口声明，不增加动态插件总线；有序修改串行生效，同一工具的 before/execute/after 有序，不同工具能否并行继续由现有执行属性决定。hook 失败保留已发生事实和部分历史，不静默放行授权或伪造成功。

hook 配置属于 Session，run 开始时固定本轮使用的配置，变更只影响后续 run。执行上下文和计数属于单次 run，不能跨轮共享取消状态。高级宿主在调用循环时提供等价的固定配置，不需要新建 Session。

顺序为接收输入、prepare_request、transform_context、现有模型消息转换、Provider 请求，然后逐工具 before/执行/after，最后 finish_turn。prepare_request 的模型与选项修改只作用当前请求；需要持续选择由 Session 配置在后续准入时体现。transform_context 返回完整替代视图；after_tool_call 只替换明确返回的字段，不深合并；before_tool_call 的拒绝生成配对的拒绝结果；finish_turn 返回正常调度、继续或结束，不绕过取消及预算。

异步执行 hook 接收本轮取消能力。取消后不采纳迟到修改，也不开始新的外部操作。hook 返回错误时将本轮标为带来源的失败并走统一收尾；工具已执行而 after hook 失败时仍保留原始工具结果，不能重跑工具。要求执行清理的宿主操作仍按现有 drain 契约等待，不能用丢弃 future 代替清理。注册配置、次序和错误行为均由公开消费者用例验证。

现有 `chat/workflow_hooks.rs` 已执行可等待的 SessionStart、UserPromptSubmit、SubagentStart、PreToolUse 和 PostToolUse。T2 通过适配接入这些处理器，保留命令执行、超时、取消、上下文记忆及参数修改后的校验授权，不新建另一套插件 hook 运行器。同一事件只调用一次。`chat/hooks.rs` 的观察通知与 SDK 执行 hooks 分别表达，不因名称相近而合并生命周期。

将应用 `SessionPolicy` 中 prompt/skill 注入、Todo 收尾、上下文策略和恢复策略逐项映射到适当入口。上下文投影走 `transform_context`，请求准备走 `prepare_request`，Todo 是否继续的判断走 `finish_turn`；压缩提交仍走 Session 检查点，模型错误恢复仍有明确策略负责人。不能为了删除 `SessionPolicy` 把所有职责塞进新的巨型 hooks trait。迁移一个行为并验证后删除旧路径；核心保留调度、配对、授权边界、取消和提交不变量。

其中 Todo、产品提示和插件 workflow hook 处理器继续位于应用层，只改为使用 SDK 的通用执行入口。SDK 不内置 Todo 策略、插件命令运行器或 OMP 调度器。保留仍有明确产品职责的 SessionPolicy 组合代码，不把删除文件本身作为目标。

### 4.7 交付按需启用的技能加载和 MCP 接入

技能和 MCP 不能仅以"可以自己实现 ToolExecutor"作为支持证明。独立消费者应能直接组合现成模块，不依赖 Tauri 或应用设置。核心循环仍只消费规范消息和工具；集成模块通过现有公开接口接入。优先在同一 crate 使用可选 feature 隔离依赖，不先拆一套包体系。

按需启用指消费者可以不配置这些模块，不表示实施时可以省略。Skill 加载和基础 MCP 工具接入都属于第二版必交付范围。

技能模块从 `src-tauri/src/skills/` 提取解析、目录构建、正文读取及激活逻辑。宿主显式提供扫描根目录、项目路径或资源来源，并选择启用技能。模块定义同名冲突优先级，先提供目录摘要，再按需读取正文；保留参数替换、技能目录及相对资源定位，文件变化、压缩或历史回退后允许重新加载。应用专有默认目录、插件启停及安装界面由应用适配。加载技能只引入指令，不自动执行脚本或扩大工具授权。

复用 `chat/resources.rs` 中 `Catalog` 与 `ResourceSessions` 的对话资源快照和失效语义。目录在初始化或显式 reload 时发现，正文使用时读取，不在每轮、重试或压缩时重新扫描目录，也不遍历注入全部资源文件。正文重新加载与目录重新发现是两个操作。同名优先级、插件启停失效、reload 失败保留原快照均按 CONTEXT 保留。若将通用快照生命周期提取进 SDK，桌面改为使用该实现，不保留两套缓存。

MCP 模块复用当前 rmcp，提供 stdio 和 Streamable HTTP 连接、工具分页发现及变更刷新、工具定义映射、调用和结果转换。连接负责人管理复用、超时、取消及关闭，明确共享连接与单轮 Agent 的不同生命周期。保留图片、结构化内容和工具错误；工具名映射必须可逆且无冲突。断线后未知结果的调用不自动重放，服务端 annotations 不作为本地授权依据。宿主传入启动配置、凭据及授权策略，应用保留 OAuth 登录界面、密钥存储和状态展示。

本次源码确认的是 MCP 工具链路，不能据此宣称已支持全部 MCP 能力。实施前单列 resources、resource templates、prompts、sampling、elicitation 的现状和第二版支持矩阵；未验收的能力标记为不支持，不能把 rmcp 库具备的 API 直接算作 SDK 已交付。基础 tools 接入是本版明确承诺。

### 4.8 P1：会话树、分支与历史投影

当前桌面的 `chat_fork_conversation` 复制选定前缀到新对话，`chat_rewind_to_message` 截断用户提问及后续消息，regenerate 截断后再生成。缺口是统一、可恢复、保留旧路径的树状历史，不能描述为 Kivio 从未实现过这三个动作。

Pi 的准确参照是 `SessionEntry` 的 `id/parentId` 关系及 `SessionManager` 的当前 leaf。工具结果属于 message entry，branch pointer 不是必须新增的一种 entry；compaction、模型设置变化等才是其他有语义的记录。Kivio 不逐字复制类型清单。

T1 按第 4.1 节确定身份、历史投影、配对、压缩覆盖范围和提交归属的行为契约。此时不发布无人消费的完整树 API。T5a 用旧历史和压缩实例验证最小结构，T5b 扩展树与导航，T5c 接入持久化迁移。`SessionEntry` 的目标是保存 message、compaction 和影响重放的配置变更，message 继续复用 `RuntimeMessage`。会话树是模型历史事实来源，当前路径与兼容显示视图由明确映射得到，不能成为第二份可独立修改的模型历史。压缩新增摘要及保留边界记录，不删除其他分支依赖的原始 entry。

操作语义如下。

- `fork`：从明确锚点创建新分支，保留原路径。SDK 分支与桌面的"复制为新对话"分别提供明确操作，不能悄悄改变现有命令含义。
- `rewind`：把活动位置移动到约定边界，后续生成创建新路径。桌面"回到这里"仍返回该用户输入供编辑，不自动发送。
- `regenerate`：从目标回答前的有效边界生成新 sibling 路径，保留原回答。新运行重新经过授权，不自动执行历史工具。
- `switch_branch`：选择合法目标并重建上下文、压缩边界及适用模型配置。已完成工具结果作为历史读取，不执行工具；不复制活动作业、权限授予或外部 CLI 进程。
- `active_leaf`：允许根前空位置，需独立于"最后追加 entry"保存。只切分支而未发消息，关闭重开也必须停在同一位置。

只有当前运行已收尾且获得历史修改预留才可切换历史。便捷 Session 管理自身准入，高级桌面宿主复用现有 `ChatSendReservation`，不另建相互独立的锁。运行中返回 Busy，调用方可显式 abort 并等待。首版仅允许完整消息/工具轮边界，拒绝导致 tool call/result 断配的锚点。新运行绑定 branch、revision 和 generation，旧轮晚到结果不得写入另一分支。

保存 entry 与活动位置更新必须有一致提交语义。宿主可使用现有原子快照或事务，不要求自建事件数据库。新快照保存树及活动位置；旧线性历史迁为单链，保留原消息身份和附件引用，不能恢复迁移前已删除的消息。迁移应可验证且保留源数据，损坏父链、重复 ID、环和不存在的 active leaf 必须明确失败。

桌面复用 `chat/repository.rs` 的 revision 检查、`mutate_expected` 和现有原子写入，不增设独立 SDK 文件存储来竞争写入。SDK 的 entry 身份与桌面 `ChatMessage.id` 建立稳定关联；ChatMessage 的工具卡片、附件、多答组及展示字段继续由应用维护。当前模型路径由树投影得到，但不能直接把整份 Conversation.messages 替换成 RuntimeMessage。T5c 明确多答组选中结果、清空上下文边界及压缩记录的映射，并验证显示和模型输入各自保持正确。

多答组指已有的同一问题保留多个模型回答，通过 `group_id` 关联、`group_selections` 选择后续上下文使用的回答。这里只要求迁移保持现有行为，不新增多答组能力，也不要求它与会话树分支一一对应。

提交 revision、快照格式版本和运行 generation 分别表达。桌面提交使用 repository 的 Conversation.revision，只有 repository 递增；SDK 接收和校验宿主提供的提交版本，不再维护一个竞争的桌面 revision。便捷内存 Session 自行管理其提交版本。导入快照不覆盖宿主当前 CAS 版本，保存失败或冲突不能推进活动位置。

附件回收要计算所有保留分支的引用；分支回退不会撤销文件、命令或远端操作的副作用。Todo 等产品状态由宿主按所选路径恢复，OMP 作业身份和投递状态按 ADR-0007 保持独立。外部 CLI 的原生历史所有权及回退依旧遵守 ADR-0001/0002/0006，不把 SDK 的 leaf 当成 CLI 已成功回退的证据。

## 5. 按独立交付单元依次推进

计划采用九个交付单元，按 T1、T2、T3、T4a、T4b、T5a、T5b、T5c、T6 顺序推进。Skill 和 MCP 分别验收，上下文、会话树和持久化迁移分别验收。T1 至 T4b 为 P0，T5a 至 T5c 为 P1，T6 为整体验收。P1 仍属于第二版范围。T1、T2、T3 已完成，其余任务按表中顺序推进，下一项为 T4a。

以下任务表是唯一执行编号。任务内部仍可分小提交，完成验收后再推进。直接调用方迁移及重复实现删除随对应任务完成，最终验收不承担积压实现。

| 顺序 | 任务 | 执行前置 | 可独立验收的结果 | 状态 |
|---|---|---|---|---|
| T1 | 核心契约与 ToolRegistry | 无 | 类型化工具往返、完整选项、错误和明确历史投影语义 | 已完成 |
| T2 | AgentSession 与 hooks | T1 | 可控制、可扩展的多轮 Agent | 已完成 |
| T3 | 四协议 Provider | T2 | 四协议通过同一套公共契约用例 | 已完成 |
| T4a | Skill 加载 | T3 | 目录发现、按需激活及显式重新加载 | 进行中 |
| T4b | MCP 接入 | T4a | 两种传输发现、授权、工具执行和清理 | 未开始 |
| T5a | 上下文与压缩 | T4b | 当前路径投影、预算、摘要及取消失败处理 | 未开始 |
| T5b | 会话树与分支 | T5a | 内存中的分叉、回退、再生成和分支切换 | 未开始 |
| T5c | 持久化与旧历史迁移 | T5b | 完整 entry snapshot 保存、导入、重开和应用接入 | 未开始 |
| T6 | 应用回归与 SDK 交付 | T5c | 文档、消费示例、实机、三平台及打包证据 | 未开始 |

执行前置表示本次串行队列，不表示每项都有新的代码依赖。例如 MCP 不依赖 Skill 实现，二者通过 T1 的同一工具注册入口接入。T5b 不等待新的存储格式，先验证树和投影算法；T5c 再一次性接入持久化与桌面迁移，避免中途替换用户历史。

每项开始时记录当前 SHA、工作区改动和相关基线结果。此前测试数量不能代替当前验收。每项结束记录实现提交、命令、结果及剩余问题。状态只从未开始变为进行中、已完成或受阻，未测项目不能标记通过。真实应用调用发生变化的任务当时就做回归，T6 只做组合复核。

### 按现有实现确定每项改动

以下为复用来源及真实缺口，不是应用文件整体搬迁清单。沿用现有行为测试，为新增行为补测试。协议迁移用同一输入对照原实现与新入口，迁移后删除重复实现，保留必要产品适配和旧数据读取。

| 任务 | 复用或提取来源 | 新增范围 |
|---|---|---|
| T1 | SDK history、model/types、model/error、StaticTool、call_typed、schema；桌面 tool_output、model_metadata | 工具注册接入、选项透传、运行错误契约及明确历史语义 |
| T2 | SDK loop、control、events、execute、compaction；应用 SessionPolicy、workflow_hooks 和信箱 | 默认 Session 所有权与五个执行 hook，复用现有消息转换 |
| T3 | SDK Chat、SSE、usage；应用 Responses、Anthropic、Gemini 和 model/agent_runtime | 协议脱离应用依赖，增加独立构造入口 |
| T4a | skills/discover、parse、catalog、runtime；chat/resources | 无 Tauri 的加载入口及 Skill 工具注册 |
| T4b | MCP conn、manager、state、result、registry 和 rmcp | 显式配置的客户端及统一工具适配 |
| T5a | SDK compaction 和检查点；context_policy、context_estimate、context_measure | 通用预算、摘要及 entry 投影集成，不重写估算或压缩算法 |
| T5b | T5a 投影及 entry；已有 fork、rewind、regenerate 语义 | 内存父链和非破坏性分支导航 |
| T5c | repository、storage、历史转换、现有附件处理及历史界面 | 版本化导入导出、全树保存恢复及桌面迁移 |
| T6 | consumer fixture、examples、check-agent-sdk、CI、README、contracts | 组合验证、迁移说明与交付清单 |

每项说明复用了哪里、提取了什么、新增对应哪个缺口、移除了哪些重复路径。不能复制旧代码后长期维护两份。外部消费者及存量数据的兼容按既定策略处理，不为删除 API 破坏旧数据可读性。SDK 不引入 AppState、Conversation 或产品规则。

实施基线已记录于 [决策记录](../research/kivio-agent-sdk-v2-decisions.tsv)。先跑 SDK 与独立消费端的定向测试，再执行桌面编译和回归；桌面构建是当前较长的反馈步骤。性能改动按第 6 节测基线，不以代码量或代理数量充当效率证据。

### T1 统一核心契约与工具接入

目标是让后续会话、Provider 和资源模块共享同一套消息、配置和错误定义。先设计领域类型，再迁移实现。在第 4.1 节明确 entry、配对、压缩范围、来源过滤及活动位置的语义，用具体输入输出验收设计，不提前实现完整树。

主要入口为 `crates/kivio-agent-sdk/src/agent/`、`src/model/`、`src/tools/`、独立消费者，以及桌面的 `chat/session_policy.rs`、`chat/tool_output.rs` 和相关配置装配。`src/model/` 与 `src/tools/` 均指 SDK crate 内目录。

- [x] 记录聊天、自动化、OMP 三类宿主的调用和保存顺序。固定无工具、工具往返、失败保留历史和取消后新轮的黑盒基线，列出需迁移的公开接口与调用方。
- [x] 贯通 `RuntimeMessage` 和类型化工具结果，删除 execute、rounds 内部 OpenAI JSON 往返。接入复用 `StaticTool` 的 ToolRegistry 注册执行器，复用 schema 校验和显式授权。
- [x] 贯通 `GenerateOptions`，确定协议选项和最小模型能力描述。验证 planning、synthesis、recovery 的参数覆盖顺序，以及最终工具禁用规则。
- [x] 统一结构化错误与结束原因。保留 ModelError 来源、部分历史、usage 和工具事实，区分未开始运行的准入错误及开始后的失败。迁移返回值与事件消费方。
- [x] 固定当前 Session 与 hook 所需的数据契约，以公开消费样例验证接口。明确第 4.1 节历史语义及五组期望投影，后续实现以此为验收依据。完整结构在 T5a、T5b 通过实际场景验证，不发布未使用类型或占位成功实现。
- [x] 迁移同仓调用方和示例，删除已替代类型与转换。执行 SDK、独立消费者、依赖边界及受影响桌面测试。

验收时从公开 API 注册一个工具并完成真实 loopback HTTP 工具往返。检查模型实际收到的选项与结果，覆盖非法参数、拒绝授权、串并行顺序、图片、截断及输出错误。失败事件、返回值和最终历史必须一致。消费者不负责转换历史或发布工具终态。

实施验收（2026-11-07）：SDK 106 项、独立消费者 37 项、桌面 Rust 全库 3198 项通过，56 项按现有配置忽略。SDK 示例编译、rustdoc、依赖边界、架构检查及计划链接通过。桌面测试进程使用已安装 Python，避免 Windows Store 别名；未执行真实供应商或桌面 UI 实机验收，留在对应交付阶段记录。

### T2 交付 AgentSession 与执行 hooks

依赖 T1。先完成 Session 所有权，再加入 hooks，最后迁移产品策略。主要入口为 SDK 的 agent、control、events、host 和 session 模块，以及桌面 `chat/session_policy.rs` 与三个宿主入口。

- [x] 实现 `AgentSession` 的 `prompt`、`continue_run`、`steer`、`follow_up`、`subscribe`、`abort` 和 `wait_inactive`。Session 管理历史、队列、订阅、身份与当前运行句柄，高级宿主复用同一循环。
- [x] 定义空闲、运行、停止请求中和收尾状态。区分 Session 完成与底层核心 inactive，等待绑定当前 generation。落实 Busy、未消费输入查询、订阅释放和取消后新轮语义。
- [x] 加入明确的最大模型步数与截止时间配置，保持工具清理和部分结果。默认值不得悄悄截断现有桌面任务。
- [x] 实现 `transform_context`、`prepare_request`、`finish_turn`、`before_tool_call`、`after_tool_call`。按第 4.6 节固定 Session 配置、本轮状态、返回值、顺序、错误及取消契约。验证现有 request_from_history 承担 convertToLlm 对应转换，不重复新增转换框架。
- [x] 适配现有 workflow hooks，验证前后工具处理器各执行一次、参数修改后重新校验及授权。区分现有 SendView 更新与新的请求投影，验证上下文清空、压缩检查点和新到输入不丢失。
- [x] 逐项迁移 `SessionPolicy` 中 prompt、技能注入、Todo 和上下文处理行为。保留明确的恢复及提交负责人，删除已迁移旧入口，不复制成另一个巨型策略接口。
- [x] 改写最小示例，使无工具会话不需要空 host、executor 或手写信箱。验证现有高级宿主及 OMP 的身份、容量、作业和投递规则。

验收覆盖完整多轮、两类输入顺序、重复发送、错误后再发送、取消并等待清理、晚到结果隔离、future 丢弃和保存失败。hooks 需证明请求投影不改历史、阻止工具不执行、处理结果不重跑工具、继续决策不重复请求，且预算耗尽和取消不可被 hook 绕过。

实施验收（2026-11-07）：SDK 106 项、独立消费者 70 项、桌面 Rust 全库 3200 项通过，56 项按现有配置忽略；四个离线示例、示例编译、rustdoc、依赖边界、架构与文档链接检查通过。公开 Session 消费用例覆盖 Busy、取消 drain、generation 等待、future 丢弃、未消费输入、订阅和保存失败。工具与请求 hooks 验证错误、panic、预算、投影隔离及原生模型签名过滤。

保留 `before_step` 作为正式历史消息的准入操作：临时请求视图不能替代 Todo 的持久提醒，启动或工具 turn 钩子也不能覆盖连续 follow-up 的每个请求边界。应用复用原 Todo 计数逻辑，新增跨运行及连续 follow-up 回归，`before_finish` 则已迁入 `finish_turn`。桌面、自动化和 OMP 继续使用原准入及保存负责人。真实供应商和桌面 UI 实机未在本项执行。

### T3 完成四协议 Provider

依赖 T2。主要入口为 SDK `src/model/` 和应用 `src-tauri/src/chat/model/`。先固定 Chat 行为，再按 Responses、Anthropic、Gemini 顺序提取。每种协议单独完成迁移和验证后再处理下一种。

- [x] 用同一套参数化 conformance fixture 固定四协议公共请求、增量与收集结果、工具回放、usage/cache/reasoning、错误及终态契约。复用已有 SSE、usage、消息类型和 ModelRuntime 适配，协议差异放在 fixture 参数中。
- [x] 提取 Responses，验证 encrypted reasoning、store:false、SSE-only、缓存降级和原生搜索兼容。
- [x] 提取 Anthropic，验证 signed、redacted、native blocks、工具名映射、pause_turn 有界续轮，以及最终用量和累计计费。
- [x] 提取 Gemini，验证 thoughtSignature、2.5/3.x 思考选项、图片、函数调用、grounding 和既有 Antigravity 适配。
- [x] 每个协议同步迁移聊天、标题、提示优化、压缩、视觉、自动化和子代理调用。应用保留账号、OAuth、密钥轮换、计费、请求日志和业务重试，删除重复编解码实现。

验收采用第 6 节公共与专属矩阵。每种协议必须由独立消费者完成工具循环和后续对话，比较迁移前后夹具的请求与规范结果。真实供应商 smoke 单独记录，缺凭据的项目不得标记通过。

Responses 子步骤验收（2026-11-07）：SDK 146 项、独立消费者 77 项、桌面 Rust 3168 项通过，56 项忽略；33 项原协议测试随实现移入 SDK，桌面测试数量相应减少。Chat 与 Responses 共用参数化消费者矩阵；补齐普通 JSON reasoning summary 与 SSE 的一致性。桌面类型检查、示例、rustdoc 和依赖边界通过。下一协议为 Anthropic；T3 尚未整体验收。

Anthropic 子步骤验收（2026-11-07）：SDK 164 项、独立消费者 79 项、桌面 Rust 3155 项通过，56 项忽略；13 项原协议测试随实现移入 SDK，并新增 5 项协议边界测试。三协议共用消费者矩阵，新增 JSON/SSE 原生续轮、8 次续轮上限和最终用量与累计计费的区分。桌面类型检查、示例、rustdoc 和依赖边界通过。Claude Code 工具名、模型配置和请求计费保留原应用负责人；下一协议为 Gemini。真实供应商 smoke 未执行。

Gemini 及 T3 整体验收（2026-11-07）：SDK 188 项、独立消费者 80 项、桌面 Rust 3136 项通过，56 项忽略；18 项 Gemini 协议测试与 1 项视频大小测试移入 SDK。四协议共用 10 项参数化消费者测试，覆盖公开 Provider、工具授权、续聊、取消、错误与流终态。Gemini 新签名保留来源，旧字符串签名可读；Chat wire 不再携带该元数据。视频大小校验由 SDK 和应用共享。类型检查、示例、rustdoc、依赖边界和架构检查通过。真实供应商与桌面 UI 未在 T3 验证，T6 单列记录。

### T4a 交付 Skill 加载

执行前置为 T3，技术上复用 T1、T2。主要入口为 `src-tauri/src/skills/`、`chat/resources.rs` 和 SDK 资源集成模块。

- [x] 提取不依赖 Tauri 的目录发现、解析、摘要、按需正文、参数替换及相对资源定位。宿主提供来源和启用策略，安装、设置及插件管理留在应用。
- [x] 将 Skill 激活工具注册到 T1 的同一 ToolRegistry，不建立第二套工具分发。
- [x] 保留同名优先级、冻结目录、正文按需读取和 reload 失败保留旧快照。验证无效 frontmatter、缺失或变更文件、禁用和白名单。
- [x] 迁移桌面与子代理调用并删除重复加载逻辑。验证普通发送和重试不重复扫描，加载技能不执行脚本或扩大权限。

验收由独立消费者完成目录发现、列出技能、按需激活和重新加载，不要求其编写加载器。压缩后正文重新加载在 T5a 作组合验证。

T4a 验收（2026-11-07）：启用 skills 的 SDK 218 项、独立消费者 87 项、桌面 Rust 3112 项通过，56 项忽略；24 项原 Skill 测试移入 SDK。新增消费者测试通过实际模型工具循环验证按需加载、正文变更、授权和失败 reload。默认 feature 示例、rustdoc、依赖边界及架构检查通过，独立源码审查没有确认缺陷。Windows 无创建符号链接权限，该分支未验证；Unix 符号链接测试未在本机执行。桌面 UI 未在本项验证。

### T4b 交付 MCP 接入

执行前置为 T4a，技术上复用 T1、T2。主要入口为现有 MCP conn、manager、state、result 和 registry，继续使用 rmcp。

- [ ] 提取显式配置的 stdio 与 Streamable HTTP 客户端、分页发现和 list_changed 刷新。宿主提供凭据，OAuth 登录、密钥存储和状态界面留在应用。
- [ ] 将 MCP 声明及动态 schema 接入同一 ToolRegistry。保留命名映射、图片、结构化内容、工具错误及授权边界。
- [ ] 保留连接复用、并发、超时、取消、重连和关闭语义。验证子进程回收和共享连接隔离，未知调用结果不自动重放，annotations 不当作授权。
- [ ] 迁移桌面与子代理调用，删除重复 wire 及工具桥接。逐项记录 tools、resources、resource templates、prompts、sampling、elicitation 支持情况，基础 tools 为明确承诺。

验收由独立消费者分别通过两种传输完成发现、授权、执行和关闭。复用现有安全测试，检查输入修改后重新校验、凭据不进入日志及取消不遗留受管进程。不能以底层 rmcp 的能力当作 SDK 已支持证明。

### T5a 交付上下文与压缩

依赖 T4b。本项完成当前路径的投影及通用上下文算法，不实现分支 UI 或新存储后端。主要入口为 SDK compaction 与应用 context_policy、context_estimate、context_measure。

- [ ] 按 T1 语义用现有历史实例验证最小 entry 载荷和单路径投影，明确 RuntimeMessage 与模型请求转换边界。
- [ ] 提取预算、token anchor、完整工具轮保留和摘要算法，接入 T2 hooks 与原有检查点。复用已有取消和 incoming input 处理。
- [ ] 压缩生成摘要及覆盖边界记录，只改变模型投影，不删除原历史事实。区分手动清空上下文、临时发送裁剪和持久压缩。
- [ ] 验证超窗恢复有界、失败和取消保留有效历史、工具配对、usage anchor、跨模型签名隔离及 Skill 正文重新加载。
- [ ] 同步迁移应用算法调用，通过现有宿主接口保持当前持久化行为，T5c 再引入新 entry snapshot 格式。

验收由独立消费者完成长对话、多次压缩和后续请求，检查实际请求与预期上下文一致。无需完整树或新快照格式即可完成本项。

### T5b 交付会话树与分支

依赖 T5a。本项只完成 SDK 内存树和导航语义。现有桌面存储和历史操作保持原路径，T5c 再完成安全切换。

- [ ] 实现父链索引、active leaf 及路径投影，复用现有消息和 T5a 压缩算法。
- [ ] 提供 fork、rewind、regenerate、分支枚举与切换。沿用现有命令的锚点约定，明确复制为新对话与同会话分支的差别。
- [ ] 验证无效父链、重复 ID、环、非法活动位置及工具轮边界。运行中拒绝切换，晚到结果不能写入其他分支。
- [ ] 验证分支双方继续互不修改、旧回答仍可访问、跨压缩边界选择及模型配置和签名隔离。历史结果只读取，不重跑工具。
- [ ] 从所有保留路径枚举附件引用，为 T5c 的回收逻辑提供明确输入，不在 SDK 执行桌面附件删除。

验收使用公开 API 在内存中创建、切换和继续两条路径，检查每次模型输入及原路径不变。此时不宣称关闭重开、磁盘迁移或桌面新分支功能已交付。

### T5c 交付持久化与旧历史迁移

依赖 T5b。本项复用现有 repository 和原子保存，一次性接入完整 entry snapshot。主要入口为 SDK 导入导出、chat/repository、storage、commands/mutations 和现有历史界面。

- [ ] 实现版本化 session entry snapshot 导入导出，包含完整树、必要配置和 active leaf。明确未知版本、损坏父链、缺失附件及不完整工具轮的处理。
- [ ] 将旧 conversation 映射为可验证的初始路径，保留稳定身份、来源、附件、压缩边界、上下文清空边界和多答组选中结果。保存迁移来源，不伪造已丢失分支。
- [ ] 复用 repository 的 CAS 和原子写入，保存失败不推进活动位置。只切分支而不发消息也需持久化；导入快照不覆盖宿主当前提交版本。
- [ ] 迁移桌面内置 Agent 的 fork、rewind、regenerate，接入最小分支选择入口。保留 ChatMessage 展示数据和复制为新对话行为，附件回收检查全树引用。
- [ ] 验证保存重开、跨压缩分支恢复、签名隔离、Skill 重新加载及 MCP 结果不重跑。回归 Todo、外部 CLI 原生历史及 OMP 作业投递语义。
- [ ] 明确快照恢复只恢复会话历史并开启新 run，不恢复活动工具、future、执行游标或自动重试状态。沿用部分历史检查点，但不新增 durable run 引擎。

验收由独立消费者和桌面完成保存、导入、重开、切换与续聊。旧路径及附件仍可访问，迁移前后已选路径的显示和模型输入正确，失败不损坏源数据。

### T6 完成应用回归与 SDK 交付

依赖 T5c。主要入口为 SDK 导出、README、契约、示例、独立消费者、检查脚本及现有 CI。此项验证前八个交付单元已完成的应用迁移，不集中补做前八个交付单元遗留功能。

- [ ] 复核聊天、自动化和 OMP 共享核心、协议及集成实现。检查已废弃调用方和重复逻辑已删除，未引入桌面与 Session 双写。
- [ ] 整理公开 API。提供最小 Agent、自定义模型、自定义工具、hooks、Skill、MCP、宿主历史提交及树恢复的独立消费示例。
- [ ] 更新 README、契约和迁移指南，明确支持矩阵、错误、授权、完成边界、MSRV、feature、依赖范围、semver 和 API 兼容策略。记录现有 GPL-3.0-or-later，外部分发前单列许可适配决策，不在此任务自动改许可证。
- [ ] 执行第 6 节相关 Rust、前端、协议和架构检查，验证打包后消费以及 Windows、macOS、Linux 的实际 CI 结果。配置文件存在不算通过，记录各平台实际 run 和结论。
- [ ] 完成桌面主对话、自动化、OMP、审批、停止续轮、历史重开和分支选择的实机验收。记录真实模型兼容证据，列明凭据或平台造成的未验证项目。

- [ ] 汇总各阶段 MCP、工具授权、凭据处理及资源清理的安全审查和行为证据，修复新增问题。安全检查随对应实现完成，不全部拖到最终任务。

验收后给出完整交付清单与未验证范围，不用本地 HTTP 夹具代替真实供应商兼容结论。版本号在交付时确定，公开发布或合并不由本计划自动触发。

## 6. 验证完整行为

| 范围 | 必须验证的结果 |
|---|---|
| 四协议公共契约 | 增量与收集结果（收集允许底层 SSE）、工具往返、禁止工具最终覆盖扩展参数及原生工具、错误体、空响应、流式缺失 terminal 拒绝成功、截断参数、取消断流、usage、历史继续、自定义 header 和 endpoint |
| Chat | reasoning 扩展、工具增量分片、兼容参数优先级、DSML 输入规范化保持原归属 |
| Responses | 文本与工具完成事件、encrypted reasoning 顺序及跨模型隔离、缓存参数降级、store:false、SSE-only、原生搜索和 xAI 现有兼容路径 |
| Anthropic | thinking 与 redacted 签名、来源隔离、工具 ID 配对、工具大小写映射、原生搜索块、pause_turn 有界续轮、最终 usage 与累计计费区分 |
| Gemini | 同 part 及相邻 part 的 thoughtSignature、inlineData、图片输出、函数调用配对、2.5/3.x 思考选项、grounding、工具模式映射和 usage |
| 应用鉴权与兼容配置 | Codex、Claude Code、Antigravity 等既有请求行为；刷新并发及取消不丢失已轮换凭据，刷新失败不暗中切换身份，重试预算不叠加 |
| 会话与工具 | 控制准入、两类输入消费顺序、终态唯一、取消清理、并发宽度、恢复后不重复副作用、授权失败不执行 |
| 上下文与保存 | 投影不改规范历史、压缩失败保留原数据、保存失败可恢复、外置附件恢复、跨模型原生内容隔离 |
| 技能与 MCP | 技能发现及按需加载、资源路径与禁用策略；两种 MCP 传输完整工具往返、动态工具刷新、取消与连接清理、未知副作用不重放 |
| Hooks 与会话树 | 请求视图与持久历史隔离、钩子顺序及失败、正常 turn 调度；分叉/回退/再生成/切换、无新增消息时活动位置持久化、跨压缩分支恢复、旧历史迁移和全树附件引用 |
| 桌面实机 | 主对话和自动化正常与失败，OMP 批量、IRC、父空闲结果送达、停止后续轮，审批和历史重开 |

沿用以下命令。实施后根据实际新增测试路径补充过滤命令。

```powershell
cargo test --locked --manifest-path crates/kivio-agent-sdk/Cargo.toml
cargo test --locked --manifest-path tests/fixtures/agent-sdk-consumer/Cargo.toml
cargo check --locked --manifest-path crates/kivio-agent-sdk/Cargo.toml --examples
cargo doc --locked --manifest-path crates/kivio-agent-sdk/Cargo.toml --no-deps
py -3 scripts/check-agent-sdk.test.py
py -3 scripts/check-agent-sdk.py
powershell -File scripts/win-cargo-test.ps1 --lib chat::model
powershell -File scripts/win-cargo-test.ps1 --lib chat::agent
powershell -File scripts/win-cargo-test.ps1 --lib chat::commands
powershell -File scripts/win-cargo-test.ps1 --lib chat::sub_agent
npm run protocol:check
npm run architecture:check
```

完整应用整合还执行现有 Rust 全库、前端测试、类型与 lint。实机使用 Dev 身份，并核对实际可执行文件、进程和测试会话归属，避免占用正式数据或其他线程的 Dev 实例。

性能验证先记录同场景基线，再交错运行基线和新实现。确定性模型用于分离 SDK 开销与网络等待。记录请求次数、工具执行次数、历史复制量或分配量、事件开销及取消到清理完成耗时。取消后的受管工具清理成本单列。当前未做性能测量，不承诺提速或虚构阈值；性能改动必须在实施前约定可测预算。

## 7. 管理兼容与风险

SDK 仍处于 alpha，可调整公开接口，但需要迁移指南和独立消费编译验证。同仓调用方随所属批次迁移，删除迁移完成的旧入口。桌面存量历史格式保持可读，不能把源码 API 兼容和用户数据兼容混为一件事。

主要风险是 Provider 提取时丢失特殊协议行为、会话入口与桌面状态双写、错误统一时丢失部分历史、重试叠加造成重复副作用，以及树迁移或附件回收破坏未激活分支。第 5、6 节分别给出直接验证入口。

暂不根据文件大小拆模块，也不直接统一 planning 与纯文本请求机制。若同一生命周期规则仍分散，再通过已有失败回归证明收益后收敛。

## 附录 A. 源码证据

Kivio 的主要入口如下。

- [循环与边界](../../crates/kivio-agent-sdk/src/agent/loop_.rs)、[控制句柄](../../crates/kivio-agent-sdk/src/agent/control.rs)、[事件](../../crates/kivio-agent-sdk/src/agent/events.rs)。
- [运行配置](../../crates/kivio-agent-sdk/src/agent/types.rs)、[模型类型和选项](../../crates/kivio-agent-sdk/src/model/types.rs)、[工具执行](../../crates/kivio-agent-sdk/src/agent/execute.rs)、[工具轮](../../crates/kivio-agent-sdk/src/agent/rounds.rs)。
- [会话策略](../../crates/kivio-agent-sdk/src/agent/session.rs)、[桌面会话组合](../../src-tauri/src/chat/session_policy.rs)、[桌面模型配置与传输](../../src-tauri/src/chat/model/mod.rs)、[桌面模型契约](../../src-tauri/src/chat/model/README.md)。
- [独立消费者](../../tests/fixtures/agent-sdk-consumer)、[SDK 契约](../../crates/kivio-agent-sdk/docs/contracts.md)、[边界检查](../../scripts/check-agent-sdk.py)。
- [现有技能发现](../../src-tauri/src/skills/discover.rs)、[技能激活](../../src-tauri/src/skills/runtime.rs)、[MCP 传输](../../src-tauri/src/mcp/conn.rs)、[连接管理](../../src-tauri/src/mcp/manager.rs)、[工具适配](../../src-tauri/src/mcp/registry.rs)。
- [现有 fork、rewind 与 regenerate](../../src-tauri/src/chat/commands/mutations.rs)、[核心 SessionPolicy](../../crates/kivio-agent-sdk/src/agent/session.rs)。
- [已有 workflow hooks](../../src-tauri/src/chat/workflow_hooks.rs)、[对话资源快照](../../src-tauri/src/chat/resources.rs)、[模型能力解析](../../src-tauri/src/chat/model_metadata.rs)。
- [现有压缩控制](../../crates/kivio-agent-sdk/src/agent/compaction.rs)、[应用上下文策略](../../src-tauri/src/chat/context_policy.rs)、[历史存储](../../src-tauri/src/chat/repository.rs)、[桌面消息与上下文类型](../../src-tauri/src/chat/types.rs)。

Pi 的引用固定到研究提交。

- [Agent 状态、队列与运行收尾](https://github.com/earendil-works/pi/blob/085c84e98e7f4c950f2be606466fff4f75d8fe7c/packages/agent/src/agent.ts)。
- [工具结果与主循环](https://github.com/earendil-works/pi/blob/085c84e98e7f4c950f2be606466fff4f75d8fe7c/packages/agent/src/agent-loop.ts)。
- [工具和扩展契约](https://github.com/earendil-works/pi/blob/085c84e98e7f4c950f2be606466fff4f75d8fe7c/packages/agent/src/types.ts)。
- [模型与多模态类型](https://github.com/earendil-works/pi/blob/085c84e98e7f4c950f2be606466fff4f75d8fe7c/packages/ai/src/types.ts)、[核心导出与协议边界](https://github.com/earendil-works/pi/blob/085c84e98e7f4c950f2be606466fff4f75d8fe7c/packages/ai/src/index.ts)。
- [产品会话工厂](https://github.com/earendil-works/pi/blob/085c84e98e7f4c950f2be606466fff4f75d8fe7c/packages/coding-agent/src/core/sdk.ts)、[最小接入](https://github.com/earendil-works/pi/blob/085c84e98e7f4c950f2be606466fff4f75d8fe7c/packages/coding-agent/examples/sdk/01-minimal.ts)。
- [树状 SessionEntry、leaf 与历史投影](https://github.com/earendil-works/pi/blob/085c84e98e7f4c950f2be606466fff4f75d8fe7c/packages/coding-agent/src/core/session-manager.ts)。

Pi 的可等待订阅、工具消息和状态所有权是参考证据。Kivio 的 Rust 接口、审批和 OMP 生命周期需要按本项目已有约束实现，不以函数名称相同作为对齐完成的证据。

## 附录 B. 原则如何改变方案

- Foundational Thinking 使消息、选项和终态契约先于 Provider 搬迁，避免每个协议各补一套字段。
- Model the Domain 使会话空闲、运行、停止请求和收尾分别表达，避免复用一个 stopped 标记承担全部生命周期。
- Laziness Protocol 使方案保留一个 crate，并从现有实现提取协议和上下文算法，减少重复实现。
- Sequence Work into Verifiable Units 使三个 Provider 分批迁移，每批同时迁移桌面调用并验证后再推进。
- Prove It Works 使完成标准落在独立消费者、实际请求、规范历史和桌面结果上，编译成功不替代运行证据。
