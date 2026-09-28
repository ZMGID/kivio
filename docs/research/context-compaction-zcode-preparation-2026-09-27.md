# 以 ZCode 为参照的上下文压缩改造准备

日期：2026-09-27。状态：源码分析与建议设计，尚未实施。本文承接[首次审查](context-compaction-audit-2026-09-27.md)，以用户选定的 ZCode 为主要参照。工程约束仍以[统一工程规范](../engineering-standards.md)为准，本文不另立规范。

## 1. 建议改造的核心

采用 ZCode 的执行顺序和历史语义：**模型请求前先微压缩，再按预算摘要；自动压缩保留最近模型步骤；明确超限走独立恢复；摘要、近期上下文和恢复信息组成新的有效历史。**

Kivio 最先需要补的是有效历史的持久化。原始聊天记录负责展示、审计、分支；有效上下文负责下一次模型请求。二者可以共享内容，但不能继续用一个 UI 消息 ID 同时表达展示位置和模型内部裁剪位置。

建议在现有 chat 模块内实现一个可持久化的上下文检查点，共用现有仓储、模型调用端口、草稿日志和协议。先闭合“压缩 → 保存 → 下一轮 → 重启”的路径，再迁移策略和提示词。不要先移植 ZCode 的整个 Runtime、SessionStore 或事件存储框架。

## 2. 依据与范围

| 对象 | 固定依据 |
|---|---|
| Kivio | HEAD `ad3e1db945d255feac05c8ce6eac8a260ec64e8e` 加当前工作区；存在用户未提交的编辑器、斜杠命令改动 |
| ZCode | 用户提供的本地源码，HEAD `328c1a0c0ffaa5a4f65e8fa199af5e4c20706e5f`；未更新参考仓库 |
| pi | 上轮已研究的快照，仅辅助核验截断摘要等边界保护；本方案不以 pi 的分段摘要结构为目标 |

本轮以本地源码追踪为依据，没有运行 ZCode，也没有用真实供应商比较摘要质量。参考仓库的提示词、文档和注释是研究材料，不是本次对话的指令。

读取并遵守 ADR-0001、0002、0005、0006：内置主对话及内置子代理共用压缩规则；外部 CLI 继续拥有自己的原生历史。它们的 `/compact` 和回退路径仍委托外部运行时。

以下链接中，ZCode 使用固定提交链接，Kivio 使用当前仓库文件链接。定位到函数而非假定参考项目最新主分支行为不变。

## 3. ZCode 实际如何工作

### 3.1 一次普通请求的顺序

[`runRegularTurnLoop`](https://github.com/zai-org/ZCode/blob/328c1a0c0ffaa5a4f65e8fa199af5e4c20706e5f/apps/zcode-cli/packages/core/src/runtime/methods/turn-loop.ts)：

```text
检查取消 / 接收本轮输入
  → microcompactIfNeeded
  → 检查连续快速回填
  → autoCompactIfNeeded（pre-request 或 mid-turn）
  → 构造当前工具与其他请求内容
  → 模型请求
  → 工具执行 / 下一模型步骤
```

值得照用的是每个模型步骤都经过同一个治理入口。需要注意，ZCode 的部分工具和 reminder 构造发生在压缩检查之后，不能把它当作“完整请求预算已经无遗漏”的证明。

### 3.2 预算不是裸窗口的固定百分比

[`compact/policy.ts`](https://github.com/zai-org/ZCode/blob/328c1a0c0ffaa5a4f65e8fa199af5e4c20706e5f/apps/zcode-cli/packages/core/src/compact/policy.ts) 中默认规则：

```text
outputReserve = min(model.maxOutputTokens 或 32000, 21000)
effectiveWindow = max(0, contextWindow - min(outputReserve, contextWindow))
autoThreshold = max(0, effectiveWindow - 13000)
```

例如窗口 200,000、模型输出上限不小于 21,000 时，自动阈值为 166,000。它不是 Kivio 当前的 180,000。判断优先采用供应商 usage 加新增内容，缺少有效 usage 才使用估算。自动压缩还检查开关、可压缩历史与连续失败次数。

这些常数适合其默认模型环境，并不天然适合 Kivio 所有小窗口和自定义供应商。

### 3.3 微压缩是独立的低成本步骤

[`compact/microcompact.ts`](https://github.com/zai-org/ZCode/blob/328c1a0c0ffaa5a4f65e8fa199af5e4c20706e5f/apps/zcode-cli/packages/core/src/compact/microcompact.ts)：

- 处理白名单工具的旧结果，保留调用关系，用标记替换结果正文。
- 默认保留最近 5 组可处理的工具结果；不是“最近 5 条 UI 消息”。
- 默认跳过错误结果、媒体结果和已清理结果；最低节省约 256 token。
- 可因 token 压力或默认 60 分钟闲置触发；压力线低于完整摘要阈值。
- 即使还需摘要，也可以先让有效历史变小，不要求微压缩单独解决全部超限。

`runtime/methods/microcompact.ts` 更新运行内历史并追加 `MicrocompactBoundary` 事件。本轮在 packages 范围检索事件名和 cleared IDs，未找到冷恢复消费这些事件的路径；标准 hydrator 读取原消息 parts。因此这里只确认运行内生效和事件记录，**不宣称该快照的微压缩能够跨进程持续生效**。Kivio 需要自己明确这项保证。

### 3.4 保留单位是模型 assistant 步骤

[`rounds.ts`](https://github.com/zai-org/ZCode/blob/328c1a0c0ffaa5a4f65e8fa199af5e4c20706e5f/apps/zcode-cli/packages/core/src/compact/rounds.ts) 和 [`compact-selection.ts`](https://github.com/zai-org/ZCode/blob/328c1a0c0ffaa5a4f65e8fa199af5e4c20706e5f/apps/zcode-cli/packages/core/src/runtime/helpers/compact-selection.ts)：

- 去除由元数据标识的上下文前缀后，按 assistant 开始分组。
- 一个 assistant 与其后工具结果、用户内容、附件等归在同组，直到下一个 assistant；首个 assistant 之前的用户输入可以独立成组。
- 支持用 assistant ID 识别同一响应的多个投影片段。
- 自动与 reactive 默认保留最后 1 组，其余参与摘要；手动压缩默认不保留近期组。

这和 Kivio 的一条 assistant UI 消息不同：后者可能存着几十次模型调用和工具执行。不能按 UI 消息数量照搬。

### 3.5 明确超限不再检查普通自动阈值

[`reactiveCompactAfterContextExceeded`](https://github.com/zai-org/ZCode/blob/328c1a0c0ffaa5a4f65e8fa199af5e4c20706e5f/apps/zcode-cli/packages/core/src/runtime/methods/compact.ts) 接收失败请求实际使用的 active entries 和模型。它绕过普通 token 阈值。

[`turn-model-step.ts`](https://github.com/zai-org/ZCode/blob/328c1a0c0ffaa5a4f65e8fa199af5e4c20706e5f/apps/zcode-cli/packages/core/src/runtime/methods/turn-model-step.ts) 限制同一模型步骤的 reactive 恢复。完成工具批次后可重置该 guard；不是整个用户 turn 一生只能恢复一次。

快速回填保护另算：少于 3 个工具轮次便再次需要压缩，会积累计数；达到 3 时阻断。它和摘要失败次数是两个不同问题。

### 3.6 摘要调用太大时，自动路径先扩大近期保留段

`compact-selection.ts` 将摘要候选末尾的更多组移到 preserved tail，缩小摘要输入。能够解析供应商 token 缺口时，按缺口估算需要移动多少组；否则逐步扩大。

重要限定：[`compact-active.ts`](https://github.com/zai-org/ZCode/blob/328c1a0c0ffaa5a4f65e8fa199af5e4c20706e5f/apps/zcode-cli/packages/core/src/runtime/methods/compact-active.ts) 的 `canUseCompactSummaryTruncationFallback` 明确排除了 Auto、Reactive。自动和 reactive 无法继续重选时失败；丢弃较老轮次的有界裁剪兜底仅适用于手动等其他触发。首次报告已同步修正这一点。

扩大 tail 只能帮助“摘要请求发得出去”，可能让最终上下文仍然过大。源码也记录 `truePostCompactTokenCount` 和 `willRetriggerNextTurn`，不能以摘要请求成功代替下一请求预算检查。

### 3.7 恢复结构和执行状态分开

完整压缩后的运行历史大致为：

```text
上下文前缀 + summary + preserved entries + 恢复 reminders
```

[`compact-preservation.ts`](https://github.com/zai-org/ZCode/blob/328c1a0c0ffaa5a4f65e8fa199af5e4c20706e5f/apps/zcode-cli/packages/core/src/runtime/helpers/compact-preservation.ts) 用持久消息 ID 记录 tail 区间；不把运行时数组长度直接当作数据库消息数量。

[`compact-session.ts`](https://github.com/zai-org/ZCode/blob/328c1a0c0ffaa5a4f65e8fa199af5e4c20706e5f/apps/zcode-cli/packages/core/src/agent/compact-session.ts) 冷恢复时把 preserved segment 插回摘要后，并使保留 assistant 的旧 usage 失效。运行中时间线消息不自动成为已生效边界。

`compact-active.ts` 还恢复已批准计划的引用和部分已读文件内容/引用，再清理运行内 read state。Kivio 已有 plan、todo、goal 和 file ledger，应复用其真实负责人，避免再让摘要维护第二份权威任务状态。

### 3.8 参考实现仍有需要改进的边界

不能把“照 ZCode”理解为照抄所有细节：

| 观察 | Kivio 应采用的处理 |
|---|---|
| 非空 length 截断输出并未被通用完整性校验一律拒绝 | 摘要保留 finish reason；截断不提交 |
| 摘要请求可能携带不超过 100 个工具 schema，靠提示词禁止调用 | 保持 Kivio 摘要不带工具，并拒绝意外 tool call |
| summary、event、timeline 是多个持久步骤，存在补偿和恢复逻辑 | 复用 Kivio 单会话原子文件写，把业务记录合并提交；事件只通知 |
| microcompact 有事件，冷恢复消费未确认 | 将微压缩纳入可恢复检查点 |
| `postCompactTokenCount` 可以是摘要调用的供应商总 usage | 区分摘要调用成本和压缩后下一请求的输入用量 |
| 固定 21k/13k 常数，小窗口可能得到零阈值 | 使用模型真实限制及小窗口策略，不把零阈值当作无限压缩指令 |

## 4. Kivio 本轮补充确认的接入问题

### 4.1 主对话的摘要交接存在实际断点

[`reply.rs`](../../src-tauri/src/chat/commands/reply.rs) 在约 938–973 行把 `compaction_summary`、boundary 放进内存 conversation；随后调用 `push_assistant_message`。

但 [`messages.rs`](../../src-tauri/src/chat/commands/messages.rs) 的该函数通过仓储 `mutate` 重新取得磁盘 latest，仅 upsert 回答、标题和计划；不合并传入 conversation 的压缩状态。后续统计又基于 persisted 重新计算。因而**这段循环摘要本身并没有通过该调用落盘**；事后另一条自动压缩可能重新产生摘要，不能算原结果正确交接。

多模型分支在上述处理之前即构造消息并返回，连这段内存交接都不经过。此结论来自调用链源码；本轮没有把它描述为新增的已执行集成测试。

### 4.2 checkpoint 接口名字不等于主对话已保存上下文

[`ChatAgentHost::checkpoint_runtime`](../../src-tauri/src/chat/commands/agent_host.rs) 忽略 `_history`，实际只收取子代理结果。不能仅把 `compacted_history` 接到这个方法，就宣称持久化已经完成。

[`draft_journal.rs`](../../src-tauri/src/chat/draft_journal.rs) 当前每行保存一份 `ChatMessage` 草稿；恢复时，只要主文件已有同 ID 消息就跳过草稿。新增检查点若提前把同 ID assistant 写入主文件，而不同时修改此判定，会丢掉之后追加的草稿进度。

### 4.3 回放的真实单位还需要显式表达

[`ModelMessage`](../../src-tauri/src/chat/model/types.rs) 没有稳定消息 ID；一个 canonical Tool message 可展开成多个 wire tool messages。现有 `_ui_message_id` 仅能定位整条 UI 消息。

因此本次不建议将 wire 数组 index 直接作为持久游标，也不建议仓促为所有模型、转换器和存量 transcript 加全局 ID。

### 4.4 统计缓存与历史变更混写

[`ConversationContextState`](../../src-tauri/src/chat/types.rs) 同时包含统计和 summary/clear/boundaries。旧统计遇到 CAS 冲突后用新 revision 写回旧整体，会覆盖业务状态。新设计必须在仓储接口层限制字段，而不是要求每个调用方小心合并。

### 4.5 分支、设置和手动操作有现成语义

- `chat_set_group_selection` 当前只换选中回答；新的检查点必须验证自己属于哪个有效回答分支。
- `chat_fork_conversation` 当前重置 context_state；本次先维持此语义，从所选原始前缀重新建立上下文，不悄悄更改 fork 产品行为。
- `defaultModels.compression` 已在设置界面暴露。不能直接为了模仿 ZCode 删除用户配置；应统一所有触发入口的模型解析。
- 手动内置压缩当前没有和发送一样的 reservation，也没有完整取消通道。应共用会话运行互斥和取消语义。

## 5. 建议的目标设计

### 5.1 归属保持简单

| 负责人 | 职责 |
|---|---|
| `chat/agent/compaction.rs` | 微压缩、分组、阈值、摘要请求、重选、结果校验、失败限次；一个规则实现 |
| `chat/commands/context.rs` | UI 命令、有效历史重建和统计适配；不再另写自动压缩算法 |
| `AgentHost` 及现有主/子代理 host | 提交候选检查点、检查 run/generation、通知已提交结果 |
| `chat/repository.rs` + `draft_journal.rs` | 各自负责正式会话和运行草稿的原子提交/恢复；不调用模型 |
| 既有 protocol/reducer | 显示过程和已确认结果，不通过事件反向写业务状态 |

新抽象的收益仅是收拢有效历史的生命周期。无需新增 ContextManager → Controller → Service 链。

### 5.2 持久化选择：先用有效视图快照，暂不迁移全部消息 ID

三种方案比较：

| 方案 | 优点 | 代价 / 结论 |
|---|---|---|
| 继续只存 UI boundary + summary | 字段少 | 无法表达单条 assistant 内部裁剪；不采用 |
| 为每个 canonical entry 增加稳定 ID，再存保留区间 | 最接近 ZCode，减少正文副本 | 牵动存量迁移、协议转换、草稿与多模型归属；后续有测量依据再做 |
| 存有效历史 checkpoint + 覆盖范围 | 能精确表达微压缩和半条 UI 消息内部的保留结果；可复用 ModelMessage | 有一个必要的活动视图副本；建议本次采用 |

检查点是可用上下文的内容快照，**不是另一份完整聊天历史，也不是永久固定的 system prompt**。其大小必须随治理预算受控；媒体继续外置引用，不复制 base64。

建议字段语义如下，名称需在实现时对齐现有类型：

```text
ContextCheckpoint
  schema_version
  id / operation_id / generation
  scope: conversation_id + 所属 run/assistant（多模型时具体 arm）
  source_fence: 本次覆盖到的原始历史位置
  source_fingerprint: 已覆盖的有效分支内容 + clear 状态
  active_body: 规范化消息序列（含摘要、保留正文、微压缩标记）
  summary_record / 新增 boundary records
  usage_basis: 对应视图版本、模型和请求配置；可为空
```

`active_body` 的摘要需显式标记为合成历史摘要，不能依赖正文字符串识别其身份；`ModelMessage` 本身无 system role，需用小型带身份的封套表达合成摘要与普通消息，不把它硬塞成伪 system message。

`source_fence` 不用于切开一条 assistant。**保存检查点时，必须同时保存对应的完整原始草稿/回答快照**；检查点已覆盖到这份原始消息的末尾，内部被保留的半段直接在 active_body 中。该消息后续追加输出时，下一份持久草稿和检查点必须一起推进，不能复用旧 fence 后跳过新内容。

下一轮重建为：当前系统/工具/任务上下文 + 最新适用 checkpoint.active_body + fence 之后的新有效消息。后续完成但未再压缩的 run 可以直接追加原始新消息，不为每一轮复制检查点。

覆盖范围校验基于规范内容、分支选择与 clear 状态，不能哈希临时 base64、访问时间、统计字段或标题。只改标题不应令检查点失效。不引入另一个全局 revision 系统来规避现有 CAS。

### 5.3 提交和恢复必须一起设计

1. 操作开始捕获 source fingerprint、run ID、generation；手动压缩也取得同一会话 reservation。
2. 模型请求和候选计算在锁外进行。取消共享一个可重复观察的信号，不把一次性 future 用 `take()` 消耗掉。
3. 验证候选摘要、工具配对、有效视图、完整预算，再提交。
4. 空闲手动压缩在现有会话仓储锁内校验 source 后，单次原子写入 checkpoint、summary 和 boundary。
5. 运行中在现有草稿机制保存**原始草稿 + 活动检查点 + run/序号/源版本**的同一版本化 envelope；允许读旧版 ChatMessage 日志。恢复按完整 envelope 应用，不拆开恢复两种视图。
6. 为压缩提交定义明确持久性：在发布压缩 completed、使用新视图发下一请求前，完整 envelope 的提交必须成功；若要求掉电恢复，应对该稀疏提交 flush/sync，而不是给每个 token 加同步写。
7. 最终回答和最后一个检查点在同一次仓储 mutation 中提交。之后清理对应 run 的草稿；恢复端识别已提交的 run/序号，避免仅凭同 message ID 误丢新草稿。
8. 提交完成后才切换已确认视图并发 completed。通知失败不回滚已提交业务记录；重开从存储恢复最终状态。

运行中重复压缩要保存每个操作的边界记录，同一有效分支只需最新活动视图；其他仍可选回答的检查点按各自分支保留。不能继续只用一个 `pending_compaction_boundary` 覆盖前几次操作。全量 summary 历史的保留复用已有边界记录，不创建第二本事件账。

还需区分主会话原子替换和列表索引更新：现有仓储可能先写会话，再更新索引。若后一项失败，不能假设压缩没有落盘并重复摘要。以 operation ID 重读确认业务提交是否已存在，修复派生索引；提交结果应明确已提交与未提交，避免把多文件操作误称为一个事务。

源版本冲突时不得把旧结果套用到新 revision。标题等无关更新可在锁内验证 fingerprint 后合并；内容、clear、有效分支改变则丢弃候选或从新快照重新执行一次。

### 5.4 多模型和子代理

- 每个 arm 的 checkpoint 跟该 assistant 一起返回、一起保存；活动视图选择服从 `group_selections`，不让最后完成的 arm 写成全局上下文。
- 手动操作产生的会话 checkpoint 与回答携带的 checkpoint 使用同一个适用性选择函数：选当前历史线上最近、且 source fingerprint 仍匹配的记录。新的手动操作能覆盖旧回答上的 checkpoint，切换旧组会令受影响的记录失效。
- 运行中每个 arm 使用按 run 隔离的 envelope，不能沿用全会话“只有一份草稿”的删除语义。现有多模型禁止部分落盘的设置需要随此次接入明确迁移。
- 子代理继续由自己的 host 保存原始/有效历史，不能把父对话的 checkpoint 写进去。父代理收到的结果按普通后续输入纳入自己的上下文。

### 5.5 统计与业务记录拆开写

首先限制仓储 API：`update_context_stats` 只接收用量、测量时间、来源等缓存字段，不接受 summary、clear、boundary、checkpoint。业务修改有显式提交入口。

可暂时兼容现有前端 `contextState` 返回形状，在后端组装响应；无须立即重做全部 UI。缓存异步结果标明测量所依赖的视图版本，过期可丢弃；不要把旧比率搭配新会话 revision 当作最新结果返回。

所有压缩都使旧 usage anchor 失效；摘要模型的 usage 只计费用，不能充当下一普通请求的 input usage。下次成功模型请求才能建立与新视图对应的真实锚点。切换模型、工具 schema 或系统前缀后同样验证锚点是否仍适用。

## 6. 策略与摘要的建议取舍

| 项目 | 建议行为 | 与 ZCode 的关系 |
|---|---|---|
| 自动治理顺序 | request preflight → micro → auto summary → 再测实际请求预算 | 同方向，补全最终预算复核 |
| 微压缩 | 保留最近 5 组，白名单、错误/媒体保护、最低收益；先以 token 压力启用 | 首批照用核心规则；60 分钟闲置触发留作后续，避免同时改变过多触发条件 |
| 自动/超限保留 | 默认保留最近 1 个完整 assistant 组，必要时扩大 tail | 照用；替换固定 20k tail 为分组规则 |
| 手动压缩 | 汇总全部当前有效正文；原 UI 历史仍保留 | 照用手动不保留 tail；按钮行为应有回归覆盖 |
| 明确超限 | 绕过阈值，针对失败请求视图；同一步只做一次有效恢复 | 照用，不重发内容完全相同的失败请求 |
| 模型路由 | 全入口统一尊重 `defaultModels.compression`；未配置继承当前主模型 | 保留 Kivio 设置；默认与 ZCode 同模型方向一致 |
| 模型能力 | 分开计算工作模型的可发送预算和摘要模型的摘要输入/输出预算 | Kivio 多供应商必须适配 |
| 摘要输入太大 | 优先完整分组重选；本次先不采用“丢老轮次”兜底，包括手动 | 自动/reactive 与 ZCode 一致；手动更保守，失败保留原历史 |
| 摘要格式 | 采用 ZCode 九类信息组织，直接输出完成的摘要 | 不照抄要求输出分析过程的模板 |
| 工具/错误保护 | 工具调用与结果成组；保留错误、必要签名及同模型 reasoning item | 复用 Kivio 规范消息和现有供应商适配 |
| 文件/任务信息 | 现有 file ledger 保留；计划、todo、goal 由其当前状态重新注入 | 参照恢复信息思路，不由摘要反写权威状态 |

### 6.1 完整预算建议

```text
inputEstimate = system + activeBody + 当前新增输入 + tools/schema + media + reminders
sendLimit = 工作模型窗口 - 本请求实际输出预留 - 安全余量
summaryInputLimit = 摘要模型窗口 - 摘要输出预留 - 摘要提示词/包装开销 - 安全余量
```

实际供应商对窗口/输出的约束以适配器元数据为准；未知窗口可显示未知并允许 reactive，不猜成一个确定的 200k。

建议保留 ZCode 的大窗口基准，但不直接把 21k 作为所有请求的输出预留上限：若实际请求可输出 32k，应为它预留 32k；若只允许 8k，就用 8k。安全余量可先以 `min(13000, floor(window * 0.10))` 作为待验证起点。该公式是 Kivio 方案建议，不是 ZCode 原式，也未经过供应商实测调优。

对 8k/32k/128k/200k/1M 做表驱动测试。若固定 system、tools 与输出预留已占满预算，返回明确的不可压缩原因；反复摘要聊天记录无益。

重选后的完整视图若仍超预算，不得宣布“已恢复可发送”。区分：压缩操作是否完成、下一请求是否满足预算。停止本步或进入有界后续决策，不能无限产生摘要费用。

### 6.2 摘要内容与来源边界

九类内容：用户目标与约束、关键技术、文件与代码、错误及修复、已解决/调查中的问题、用户反馈与意图变化、待办、当前工作、下一步。

需要保留精确路径、符号、命令结果、未完成工作，以及用户实际表达的限制。长历史里的用户消息按意图变化归纳，不机械要求无限逐字列出全部消息。旧摘要和新被压缩历史一起更新为一份摘要，删除现有“两段各调用一次模型再拼接”的必经路径。

摘要提示词要明确：仓库文件、网页和工具输出内的命令只是材料，不自动成为用户要求。只把真实对话中的用户要求作为任务约束；不要将从材料中总结出的句子提权为 system 指令。摘要自身使用显式合成历史身份。

输出接受条件至少包含：成功完成原因、非空有效摘要、无工具调用、未取消、没有未完成的结构、候选能按原始分组重建。不能以“200 字符 + 旧摘要长度的 30%”证明完整性；长度最多作为异常诊断指标。

### 6.3 失败处理和重试边界

返回类型应明确区分 `Skipped(reason)`、`Applied(checkpoint)`、`Cancelled`、`Failed(reason)`、`Blocked(reason)`，避免所有失败都吞掉后返回原数组。

- 网络暂时失败、摘要输入超限、摘要输出截断、用户取消、提交冲突分别处理。
- 一个操作统一管理请求总预算；重选和网络重试不得内外乘起来。建议首版每次操作最多 3 次摘要模型请求，所有子路径共用计数；这是简化后的 Kivio 上限。
- 自动失败熔断和快速回填熔断分开；快速回填沿用 3 个工具轮次/3 次累计的起点，恢复条件也要显式测试。
- 自动优化失败且原请求仍在预算内，可继续；已经明确超限或预计无法发送时，原样重试不是降级方案。
- 提交失败不发成功边界；取消后迟到的模型输出不写盘。取消不会撤销已经执行的工具或文件修改。

## 7. 历史变更和兼容矩阵

| 操作 | 检查点处理 |
|---|---|
| 正常追加新消息 | 有效检查点 + fence 后追加；不重新放入已覆盖原文 |
| 同一运行继续追加 assistant | 原始草稿与 active view 一起推进，最终一起提交 |
| 修改/删除覆盖区内消息 | 使受影响检查点失效，按当前有效原始历史重建；旧摘要不可继续描述旧内容 |
| 修改覆盖区之后消息 | 可保留前缀检查点，重建变化后的尾部 |
| 切换多模型选中回答 | 所有依赖被切换内容的检查点和 usage 失效，选当前分支适用记录 |
| 清空上下文 | 写入 clear 业务边界，废弃其前有效摘要/检查点；统计刷新不得恢复它 |
| 回到这里 | 删去被回退段；检验候选检查点的覆盖范围，越界则失效；沿用现有 clear 回退语义 |
| Fork | 首版保持现有新会话默认 context_state 行为，不复制活动检查点 |
| 模型/工具/系统变化 | 保留兼容的语义历史，重新投影；usage 重算，按当前模型过滤不兼容签名/推理数据 |
| 外部 CLI | 不应用内置检查点，继续原生 compact / resume / rewind |
| 旧会话无新字段 | 走现有 summary/boundary 兼容读取；下一次成功治理时再产生新版检查点 |
| 检查点损坏/引用无效 | 不静默发送错配视图；保留 clear 和当前分支限制后重建，发送前再治理；仍不适合则显式失败 |

迁移使用 serde 默认值和显式 schema version。读取旧数据不触发昂贵模型调用，不一次性重写所有会话。未知版本不能静默当作可用检查点；不得因向后兼容而在新版本同时维护两套不同的有效历史真相。

## 8. 具体改动地图与删除项

| 文件 / 范围 | 改动目的 |
|---|---|
| `types.rs` | 检查点/草稿持久语义；原始历史、业务记录、统计缓存分工 |
| `agent/compaction.rs` | 分组选择、统一策略、一次摘要、结果验证、取消和有界重试 |
| `agent/provider_runtime.rs`、`agent/planning.rs` | 摘要结果带完整 finish/usage/error；统一摘要模型解析及能力预算 |
| `agent/loop_.rs`、`agent/synthesis.rs`、`agent/types.rs` | 每步共享 preflight；reactive 独立触发；传递当前有效视图与多个操作记录 |
| `agent/host.rs`、`commands/agent_host.rs`、`sub_agent.rs` | 提交结果需要可等待、可失败；主/子/多模型作用域正确 |
| `commands/context.rs` | 新重建入口、统计只读业务状态、手动调用同一压缩核心 |
| `repository.rs`、`draft_journal.rs`、`storage/conversations.rs` | 原子保存、版本化草稿、恢复与附件外置一致 |
| `commands/reply.rs`、`commands/messages.rs` | 原始回答 + 压缩状态同次交接，删掉只改局部 conversation 的假持久化路径 |
| `commands/mutations.rs` | 编辑、删除、选中组、回退、清空的适用性处理 |
| `protocol.rs` 及生成协议、现有 context/compaction UI reducer | operation ID、阶段与已提交边界；重连可恢复，避免迟到终态覆盖新操作 |

实现前重新核对工作区：`commands/send.rs`、`commands/interaction.rs`、`commands/tests.rs` 等已有用户改动，必须在当前内容上局部接入。

迁移完成后删除或收拢：

1. 命令层与循环层两套自动压缩决策，以及回复保存后的额外模型摘要；后者只保留统计刷新。
2. 旧 90% 和固定 20k 切分对新路径的控制，避免与新预算/分组并存。
3. split-turn 两次摘要拼接；UI boundary 只承担展示与旧数据兼容。
4. 只在 run 结束回传一个 pending summary/boundary 的路径。
5. CAS 冲突后套用新 revision 重写旧整个 context_state 的逻辑。
6. 摘要层反复吞掉 finish reason、取消和存储失败的转换。

## 9. 推荐实施顺序与每批完成条件

| 批次 | 范围 | 完成条件 |
|---|---|---|
| A：固定缺陷 | 将上轮 3 个失败实验转为正式回归；补统计并发、真实保存链路和多模型交接测试 | 先确认测试能在旧实现失败，再修缓存覆盖、finish/取消丢失；不宣称整套完成 |
| B：有效历史闭环 | checkpoint、版本化草稿、最终原子保存、重建、分支/clear 失效 | 一条 UI assistant 内压缩后继续工具执行，完成/取消/崩溃重开，下一请求内容一致 |
| C：迁移 ZCode 策略 | micro→auto、assistant 分组、独立 reactive、预算、单摘要和断路 | 新路径接管全部内置入口，同批删除旧决策；不存在双重摘要调用 |
| D：交互与验收 | 手动互斥/取消、事件终态、提示与真实桌面流程 | 用户可观察状态与持久化一致，已有上下文面板和分隔线兼容 |

B 批先做一个最小端到端原型：原始 UI assistant 含 3 个模型步骤 → 压缩前 2 步 → 同一 assistant 再追加 1 步 → 保存重载 → mock 捕获下一请求。它用于验证检查点方案，不是另建一个长期并行实现。此例、并行 arm 例和崩溃恢复例通过后再扩大迁移。

## 10. 必须覆盖的验收场景

这些是后续实施的测试要求，本轮未执行。

| # | 场景 | 检查最终结果 |
|---|---|---|
| 1 | 统计读取跨越 compact 提交 | 新摘要、checkpoint、边界及下一请求不被回退 |
| 2 | 统计读取跨越 clear 提交 | 被清空的历史不重新进入请求 |
| 3 | 非空摘要以 length/max_tokens 结束 | 不写成功边界，不替换有效历史 |
| 4 | SSE 断流、空完成、工具调用输出 | 失败分类正确，不把半份摘要当成功 |
| 5 | 首次摘要、重选重试、持久化前分别取消 | 都及时结束；迟到结果不提交 |
| 6 | 模型成功但写盘失败 | UI 无 completed，旧已确认视图仍可恢复 |
| 7 | 提交成功但通知丢失 | 重开后恢复已提交状态，操作不重复计数 |
| 8 | 微压缩后结束并重开 | 旧工具正文仍不进入模型，UI 原始结果仍在 |
| 9 | 一条 UI 消息内部发生压缩后继续输出 | 没有旧步骤回流，也没有漏掉后续步骤 |
| 10 | 同一运行多次 full compact | 边界记录完整，活动视图为最后一次并含后续内容 |
| 11 | 已提交草稿后崩溃，日志尾部半行 | 恢复最后完整 envelope，不拆配原始/有效视图 |
| 12 | 主文件已有同 ID、日志有更新序号 | 正确区分已终结 run 和更新草稿 |
| 13 | 多模型 A 压缩、B 未压缩或不同摘要 | 选 A/B 后各自模型历史正确，无最后完成者覆盖 |
| 14 | 子代理压缩与父代理同时推进 | 父子上下文、取消、usage 不串写 |
| 15 | 元数据窗口 200k、供应商实际报 8k 超限 | 确实改变失败请求的输入后才恢复，不受普通阈值挡住 |
| 16 | system/tools 本身超窗、无可压缩旧组 | 明确阻断，不循环摘要 |
| 17 | 摘要请求太大，扩大 tail 后仍不可发送 | 不把摘要成功误当作预算已恢复 |
| 18 | 5 组保护、错误/媒体/多 tool result | 微压缩不破坏配对、错误、签名和媒体引用 |
| 19 | 清理后消息低于阈值但加 schema 超出 | 继续治理或阻断；统计与真实请求口径一致 |
| 20 | 工作/摘要模型不同，摘要窗口更小 | 使用各自限制，不错用主模型窗口 |
| 21 | 快速回填与连续失败 | 分别限次；工具轮次和新操作的重置条件正确 |
| 22 | 编辑/删除/切组/回退/fork/clear | 依照上表选择或作废检查点，无跨边界复活 |
| 23 | 旧 api_messages、旧 model_messages、旧 summary | 向后兼容，首次读取不触发压缩，转换不丢工具结构 |
| 24 | 模型和工具 schema 改变 | usage anchor 失效，重新估算与正确投影 |
| 25 | 面板关闭重开、手动操作与发送竞争 | 操作互斥，阶段归属正确，不残留“压缩中” |

采用现有 HTTP/SSE mock 捕获最终 provider 输入；关键断言应比较内容、持久快照和恢复结果，不能仅验证“调用了 3 次”。桌面人工验收覆盖运行中停止、手动压缩、重开、多模型选中和外部 CLI 委托。

相关命令依据当前 package.json：Rust 定向测试、相关 Vitest；改协议时 `npm run protocol:generate` 后运行 `npm run protocol:check`，并按变更执行 `npm run typecheck`、`npm run lint`、`npm run architecture:check`。未做供应商实测前不对摘要质量和成本收益下结论。

## 11. 本轮交付边界

本轮完成源码路径复核、参考行为辨析、持久化方案选择、入口归属、兼容矩阵、实施顺序和验收清单。仅新增本准备文档并精确修正上轮审查表述；未修改生产逻辑、未拉取更新参考仓库、未运行新的实现测试。

进入实施时，A 批先固定并修正已知缺陷，随后用 B 批最小持久化原型验证数据方案，再推进策略迁移。若快照空间/写入开销实测不可接受，再把 active body 改为稳定 entry 引用；这属于有测量依据的存储优化，不是当前提前重写全部模型消息结构的理由。
