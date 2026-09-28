# 上下文压缩审查：Kivio、ZCode 与 pi

审查日期：2026-09-27。范围为内置运行时的上下文压缩、摘要调用、有效历史重建、用量锚点、取消与持久化；外部 CLI 只检查归属约束，不重新实现它们的原生会话。

## 结论

Kivio 已经具备结构化增量摘要、近期窗口、工具配对保护、工具输出缩减、真实 usage 锚点、文件清单和失败限次。主要缺口是**压缩结果从请求到持久化再到下一轮重建的闭环一致性**，继续调整摘要提示词不能解决这些问题。

本轮确认 6 项实现缺陷：3 项 P1、3 项 P2。其中截断摘要、超限恢复、分段摘要取消已通过本地 HTTP/SSE mock 调用真实实现复现；其余为明确的源码路径与状态交错分析，未冒充实机复现。

建议先修并发覆盖、截断摘要和跨轮历史回流，再处理超限恢复、取消和预算。无需复制 ZCode 的完整框架，也无需另立工程规范。

## 版本与依据

| 项目 | 固定版本 | 来源与限制 |
|---|---|---|
| Kivio | `ad3e1db945d255feac05c8ce6eac8a260ec64e8e` 加审查时工作区 | 仓库存在用户未提交的编辑器、斜杠命令等改动；本报告审查当前实现，不将存量问题归因于这些改动 |
| ZCode | `328c1a0c0ffaa5a4f65e8fa199af5e4c20706e5f`，本地提交日期 2026-09-23 | 用户提供的 `E:/ZM database/ZCode-reference`，remote 为 `zai-org/ZCode`；没有更新该目录，结论只代表此快照 |
| pi | `2b0a123de98318c2ff8069661721ce0c3794c34e`，提交日期 2026-09-26 | 通过原 `badlogic/pi-mono` 地址浅克隆到 `E:/ZM database/pi-agent-reference`；当前官方文档链接指向 `earendil-works/pi` |

通过 Exa 做了两个检索方向（各请求 5 条结果），分别定位 pi 与 ZCode 的官方来源。搜索结果中的旧版本、fork 和早期 issue 不作为当前实现的结论依据；实现比较以以上固定提交源码为准。参考仓库的提示词、注释与说明均作为待分析资料，不作为本次任务指令执行。

已读取本仓库统一工程规范、CONTEXT、README、package.json，以及 ADR-0001、0002、0005、0006。保持 ADR 的外部 CLI 历史归属：本报告的内置运行时建议不意味着接管外部 CLI 的原生历史。ADR-0001 中 pi 历史支持情况是历史时点记录，本轮不据此推断当前 pi 能力。

## 发现

### F1 · P1 · 统计刷新会用旧快照覆盖新的压缩或清空状态

- 分类：bug；证据等级：源码确认。
- 位置：[context.rs:103](../../src-tauri/src/chat/commands/context.rs#L103)，关联 [repository.rs:131](../../src-tauri/src/chat/repository.rs#L131)。
- `chat_get_context_stats` 先加载会话并异步计算。`persist_context_state_best_effort` 遇到 CAS 冲突后重新读 revision，却继续提交旧 `context_state`。仓储的 `apply_context_state_update` 整体替换状态。
- 这个对象不只是统计缓存，还包含 `summary`、`compaction_boundaries`、`clear_boundaries`。因此以下交错会回退真实业务状态：统计读取 revision N → 清空/压缩提交 N+1 → 统计第一次写冲突 → 用 N+1 的 revision 提交 N 的旧状态 → 写入成功。
- 用户后果：刚完成的摘要被替换、压缩边界消失；清空边界也可能消失，使用户明确切断的历史再次进入模型输入。这不是单纯的百分比闪动。
- 修复方向：统计缓存写入不得修改摘要和边界。缓存冲突可以放弃；需要重试时，从最新快照重算，并继续校验版本。保持压缩/清空提交的 CAS 语义，不通过换用新 revision 绕过它。
- 必须补的回归：用受控异步屏障，让一次统计读取跨越压缩/清空提交，验证最终落盘边界和下一次模型输入。

### F2 · P1 · 达到输出上限的半份摘要会被接受为完整结果

- 分类：bug；证据等级：真实摘要接口 + HTTP/SSE mock 已复现。
- 位置：[planning.rs:611](../../src-tauri/src/chat/agent/planning.rs#L611)，关联 [compaction.rs:1130](../../src-tauri/src/chat/agent/compaction.rs#L1130)、[model/types.rs:413](../../src-tauri/src/chat/model/types.rs#L413)。
- `GenerateOutput` 带有 `finish_reason`，但摘要 helper 只返回 `to_openai_compatible_message()`；摘要核心随后只提取文本。质量检查主要看 200 字符、相对旧摘要长度和旧版 XML 标签，没有检查模型因 `length` / `max_tokens` 停止。
- 当前提示词使用 Markdown checkpoint，输出足够长但中途截断时，不会触发 `<analysis>` 缺 `<summary>` 的旧检查。该文本被接受后，落盘路径会推进摘要边界，使未总结到的内容不再进入后续请求。
- 回归实验：返回超过 200 字符正文，然后发送 `finish_reason: length` 和 `[DONE]`。期望 `CompactOutcome::Failed`，实际接受为 `Compacted`。
- 修复方向：摘要结果保留规范化结束原因；只有完整结束才允许替换和推进边界。截断可以有界重试或调整预算，失败时保留原上下文。
- pi 的直接参考：[getSummarizationFailure](https://github.com/earendil-works/pi/blob/2b0a123de98318c2ff8069661721ce0c3794c34e/packages/coding-agent/src/core/compaction/compaction.ts#L607) 显式拒绝 `length`；[#7048 回归测试](https://github.com/earendil-works/pi/blob/2b0a123de98318c2ff8069661721ce0c3794c34e/packages/coding-agent/test/suite/regressions/7048-compaction-truncated-summary.test.ts) 验证不写入 compaction entry。

### F3 · P1 · 主对话下一轮会重放本轮已经移除的工具历史

- 分类：bug；证据等级：生产读写路径确认，未做桌面跨轮实机复现。
- 位置：[reply.rs:984](../../src-tauri/src/chat/commands/reply.rs#L984)，关联 [loop_.rs:558](../../src-tauri/src/chat/agent/loop_.rs#L558)、[compaction.rs:815](../../src-tauri/src/chat/agent/compaction.rs#L815)、[context.rs:1390](../../src-tauri/src/chat/commands/context.rs#L1390)。
- 循环内改变 `runtime_messages`，有意保留完整 `generated_api_messages` 作为转录。`attach_usage` 输出 `compacted_history`，但生产消费者只有 `sub_agent.rs`；主对话 `reply.rs` 保存的是完整 `result.api_messages`，另把 UI 消息粒度的摘要边界赋给内存 conversation。
- 深入复核补充：`push_assistant_message` 在仓储 mutation 中重新读取 latest，只合并回答、标题和计划，没有把上述内存压缩状态一起提交；随后统计基于 persisted 重算。因此循环摘要本身的持久化交接也存在断点。多模型 arm 更在摘要赋值前就返回。详见[ZCode 改造准备](context-compaction-zcode-preparation-2026-09-27.md)；此补充为源码确认，未新增实机复现。
- 两种触发：① microcompact 仅缩减工具结果，根本没有持久化摘要边界；② 压缩发生在同一条 assistant UI 消息内部，边界只能退到完整 UI 消息，不能表达这条消息内部被摘要/保留的具体工具步骤。
- 下一轮 `build_chat_api_messages_with_video` 按 UI 边界重放整个 `model_messages` / `api_messages`，被缩减或摘要的片段因此回流。与此同时，上轮 `anchor_usage` 可能仍代表压缩后的短请求；用量锚点和下一轮真实请求不再对应，低估压力，并与 F4 叠加。
- 修复方向：保留完整转录供显示，但持久记录模型可见历史的具体保留位置/内容变换，并由同一个重建函数用于发送、估算和再次压缩。不要简单删除转录，也不要将完整 `compacted_history` 追加到现有历史造成重复。
- 必须补的回归：一条 user 后连续多次工具调用 → 中途压缩/微压缩 → 完成本轮并落盘 → 重载 → 新发一条 user；断言请求中旧工具原文不复活，配对完整，锚点与重建视图一致。

### F4 · P2 · 明确超限后的恢复仍受普通估算阈值限制

- 分类：bug；证据等级：真实 agent loop + HTTP/SSE mock 已复现。
- 位置：[synthesis.rs:359](../../src-tauri/src/chat/agent/synthesis.rs#L359)，关联 [compaction.rs:1467](../../src-tauri/src/chat/agent/compaction.rs#L1467)。
- `recover_overflow_compact_and_retry` 调用普通 `maybe_compact_send_view`。当估算低于窗口 90%（或窗口未知）时，此函数直接返回原视图；并没有“服务端已经证实超限，必须尝试缩减”的恢复模式。
- 窗口元数据偏大、估算低估或 F3 导致锚点不匹配时，会再次发送未压缩历史，消耗唯一一次恢复机会，随后降级。
- 回归实验包含约 30k token 的可压缩旧段，但配置窗口为 200k。模型返回明确 8192-token overflow 后，捕获的请求没有任何摘要调用。原有 `run_loop_overflow_recovery_compacts_and_retries_success` 也只断言共 3 次调用和重试文本，不能证明发生过压缩。
- 修复方向：在同一压缩入口中区分 `threshold` 与 `overflow` 原因。overflow 绕过普通阈值，但继续遵守可压缩范围、取消和有界重试。只有上下文确实改变后才宣称“压缩后重试”。
- 对照：[pi overflow 分支](https://github.com/earendil-works/pi/blob/2b0a123de98318c2ff8069661721ce0c3794c34e/packages/coding-agent/src/core/agent-session.ts#L2662) 与 [ZCode reactiveCompactAfterContextExceeded](https://github.com/zai-org/ZCode/blob/328c1a0c0ffaa5a4f65e8fa199af5e4c20706e5f/apps/zcode-cli/packages/core/src/runtime/methods/compact.ts#L352) 都把明确超限和普通阈值分开决策。

### F5 · P2 · split-turn 第二次摘要请求失去取消信号

- 分类：bug；证据等级：真实摘要接口 + 挂起 SSE mock 已复现。
- 位置：[compaction.rs:1296](../../src-tauri/src/chat/agent/compaction.rs#L1296) 和 [compaction.rs:1317](../../src-tauri/src/chat/agent/compaction.rs#L1317)。
- 同时存在历史段和当前轮前缀时，第一跳通过 `cancel.take()` 消费整个取消 future。第一跳正常完成后，第二跳再 `take()` 得到 `None`，走不能取消的 `call.await`。
- 用户在第二跳点停止，需要等待模型响应、超时或重试结束。现有处理只覆盖历史段为空、第一跳未消费 cancel 的情况。
- 回归实验：第一跳返回合法摘要，第二跳连接挂起；检测第二个请求已发出后发送取消信号，1 秒内没有返回 `Cancelled`。
- 修复方向：对整个双阶段摘要使用一个持续有效的取消控制，或让每次调用获得同一 generation 的取消监听；成功提交前再次确认有效。pi 将同一 `AbortSignal` 传给两个阶段，并在写 compaction entry 前再次检查。

### F6 · P2 · 压缩成功判定漏掉工具 schema 的预算

- 分类：bug；证据等级：源码与预算算式确认。
- 位置：[compaction.rs:1486](../../src-tauri/src/chat/agent/compaction.rs#L1486)，关联同文件 1433、715、1534 行。
- 触发判断使用 `消息 + 工具 schema`，但 microcompact 的 `budget` 仍是整个窗口的 90%，内部只比较消息；摘要后的 `after` 也只计算消息，之后据此清零未解决次数。
- 举例：窗口 100k，触发预算 90k，工具 schema 25k，缩减后消息 80k。代码认为 `80k <= 90k` 已解决并直接发送，实际输入约 105k；本轮还没有给输出预留空间。
- 修复方向：触发、切分、缩减后验收使用相同的完整请求预算。传入消息预算时先减 schema / 固定开销；不可压缩的固定部分占满预算时，给出明确失败结果，避免把反复微压缩当成功。
- 必须补的回归：大 schema 的真实请求、微压缩后仍超预算、摘要后仍超预算三个场景，验证最终请求与结果状态。

## 三套实现的比较

| 维度 | Kivio 当前 | ZCode 固定快照 | pi 固定快照 |
|---|---|---|---|
| 普通触发 | 裸窗口 90%；provider usage + 增量优先 | 窗口先扣输出预留，再扣 buffer；优先 provider usage + 增量 | `contextTokens > contextWindow - reserveTokens` |
| 近期保留 | runtime 按约 20k token；落盘按整条 UI 消息 | 自动/reactive 按 assistant 开始的轮组保留，默认至少最后一组；手动策略不同 | 按约 20k token，允许 split-turn；切点不能孤立 tool result |
| 摘要形式 | pi 风格 checkpoint、增量合并、split-turn 两段 | 详细九段摘要，使用 analysis/summary 标签，保留用户请求/约束 | 结构化 checkpoint、previous summary 增量更新、split-turn 两段 |
| 轻量缩减 | 旧段所有 tool result 换占位符 | 工具白名单、保留最近 5 组候选、默认保护错误与媒体、最小节省 256 token | 所查看默认 compaction 实现以摘要为主；另有显式 context edit 投影能力 |
| 明确超限 | 有一次恢复，但可能被普通阈值挡住 | 独立 reactive 原因；摘要请求超长时扩大保留段；Auto/Reactive 不使用丢旧轮次裁剪兜底 | 独立 overflow 原因，有界 compact-and-retry |
| 有效历史 | runtime 与 UI 转录分开，边界粒度粗且循环摘要交接存在持久化断点 | 完整压缩的 runtime 历史替换与持久 preserved segment 明确关联；微压缩冷恢复消费未确认 | `CompactionEntry + firstKeptEntryId`，统一 projection 重建有效历史 |
| 不完整摘要 | 主要看文本长度，丢失结束原因 | 检查流完成证据、空摘要和工具调用；本快照未见通用非空 length 拒绝，不宜照抄 | 明确拒绝 `stopReason=length`，有“不落盘半份摘要”的回归 |
| 取消 | split-turn 第二跳缺监听；手动路径没有传 cancel | 请求传 AbortSignal，维护 started/retrying/completed/failed/interrupted 状态 | 两段共享 AbortSignal；提交前检查取消；手动先终止当前执行 |
| 文件/任务信息 | 确定性文件 ledger、系统提示中已有 plan/todo 等；已有 compact hook | compact 后重建部分文件状态提醒、计划引用等，并关联持久化 | 累计 read/modified 文件列表；分支总结另走 branch summary |

### ZCode 值得借鉴的具体做法

1. **显式扣除输出预留与余量。** 当前 `policy.ts` 的有效公式是 `window - min(maxOutputTokens 或 32000, 21000) - buffer(默认13000)`，结果不小于零。不能仅看到旧注释就把它说成固定“预留 32k”。这些数值是该版本的策略，不建议直接复制到 Kivio；部分工具/reminder 后置构造，也不能据此认为已覆盖全部最终请求成本。
2. **先做有选择的工具缩减。** 默认保护错误、image/video/file，按调用组保留近期内容，节省不足则不改。Kivio 目前按 role=tool 全部缩减，后续可借鉴候选保护与效果验收，而非再加一层管理器。
3. **显式记录保留区间。** `selectPersistedCompactTail` 用 `anchorMessageId/headMessageId/tailMessageId` 描述持久尾段，不凭 runtime 与数据库消息条数碰巧相等来推边界。
4. **摘要、提醒和过程状态一起考虑。** `persistCompactSummary` 遇到写入失败会尽力回滚新消息；重启时根据是否存在 boundary 将 started/retrying 收敛到 completed/interrupted。这是应用层回滚和恢复，不应宣传成已证明的数据库原子事务。
5. **重试分层受限。** 明确超限的摘要请求有扩大近期保留段的重选策略，最终不可恢复错误被标为不可再重试，以免内外重试相乘。丢旧轮次的有界裁剪仅适用于手动等非 Auto/Reactive 触发。也有 rapid-refill breaker；这比仅按失败次数判断更能识别“压完立刻又满”。

源码：[预算策略](https://github.com/zai-org/ZCode/blob/328c1a0c0ffaa5a4f65e8fa199af5e4c20706e5f/apps/zcode-cli/packages/core/src/compact/policy.ts)、[微压缩](https://github.com/zai-org/ZCode/blob/328c1a0c0ffaa5a4f65e8fa199af5e4c20706e5f/apps/zcode-cli/packages/core/src/compact/microcompact.ts)、[保留区间](https://github.com/zai-org/ZCode/blob/328c1a0c0ffaa5a4f65e8fa199af5e4c20706e5f/apps/zcode-cli/packages/core/src/runtime/helpers/compact-preservation.ts)、[过程落盘与恢复](https://github.com/zai-org/ZCode/blob/328c1a0c0ffaa5a4f65e8fa199af5e4c20706e5f/apps/zcode-cli/packages/core/src/runtime/methods/compact-persistence.ts)。

### pi 更适合作为 Kivio 的简化参照

核心价值是同一份 session projection 同时服务有效消息重建和后续 compaction preparation：原始记录仍保留，最新 compaction 贡献摘要，`firstKeptEntryId` 起的记录贡献原文，更早的 compaction 即使落在保留区间也不会再次贡献旧摘要。明确记录的 context edits 同样进入此投影，而不只临时改变某一轮的数组。

`prepareCompaction` 先建立该投影，再找切点，读取旧摘要并提取文件操作；`compact` 负责调用摘要模型；`AgentSession` 负责取消、写 entry、刷新有效上下文和结束事件。Kivio 可以借鉴这几个职责的关系，继续使用已有 `compaction` Module 和仓储，不必复制 TypeScript 的类结构、扩展系统或 JSONL 格式。

默认值 `reserveTokens=16384`、`keepRecentTokens=20000` 只是默认策略。pi 也有分支总结，和普通压缩的目的不同；Kivio 不应为了压缩稳定性而先引入完整树形会话产品。

源码：[核心压缩](https://github.com/earendil-works/pi/blob/2b0a123de98318c2ff8069661721ce0c3794c34e/packages/coding-agent/src/core/compaction/compaction.ts)、[统一投影](https://github.com/earendil-works/pi/blob/2b0a123de98318c2ff8069661721ce0c3794c34e/packages/coding-agent/src/core/session-manager.ts#L476)、[官方压缩文档](https://github.com/earendil-works/pi/blob/2b0a123de98318c2ff8069661721ce0c3794c34e/packages/coding-agent/docs/compaction.md)。

## 按统一工程规范的审查结果与修复顺序

当前主要路径：发送前 `compute → compact_conversation → persist`；循环内 `planning → maybe_compact_send_view → summarize → reply persist`；下一轮由 `build_chat_api_messages_with_video` 重建。摘要请求核心已有共同归属，但切分、提交与重建语义没有完全统一。

建议目标：现有 compaction Module 决定“为何压缩、压哪些、保留哪些、是否完整”；仓储负责校验并提交摘要与边界；统一有效历史重建函数供发送、估算、再压缩使用。统计缓存只写自己的字段。

| 规范问题 | 审查判断 |
|---|---|
| 能否定位负责人、入口是否共用规则 | 能定位摘要核心；但落盘/UI 粒度与 runtime 粒度、模型选择和取消仍分叉 |
| 状态/规则是否单一权威 | F1 混合缓存与业务状态，F3 缺主对话有效历史的持久表达；应修权威归属 |
| 局部修改是否牵动无关 Module | 正常修复应限制在 compaction、已有 provider 返回契约、会话仓储与直接调用方；无需大规模迁目录 |
| 是否有无关等待/请求 | F4 可能重复同一超限请求；F5 停止后仍等待摘要；优先消除这些实际浪费 |
| 测试是否验证最终结果 | 现有较多纯函数测试有价值，但需补“落盘→重载→下次请求”和并发交错，不能只断言重试或中间函数调用 |

推荐分三批，每批先建立失败用例：

1. **保护状态与摘要完整性：** F1、F2。验收统计刷新不回退边界；截断/空/取消摘要不推进持久化边界。
2. **让恢复和预算有效：** F4、F5、F6。验收服务端超限绕过普通阈值、任一摘要阶段可停止、重试请求实际缩小、完整请求预算一致。
3. **闭合跨轮有效历史：** F3。验收微压缩、单次及多次 split-turn 压缩，完成/取消/重启后的下一条请求都不恢复已移除内容；保留原始转录与工具调用配对。

额外设计核对项，未混入上述 6 项缺陷计数：裸窗口 90% 没有显式绑定下一次输出预算；旧摘要/focus 固定开销可能超过摘要输入预算；发送前使用配置的 compression model，而循环内使用主模型；多次 run 内压缩只保留最后一条 pending boundary，统计次数和实时事件可能不同。落实修复时应确定产品语义并补测试，不把建议冒充已复现事故。

## 验证与可复现材料

已运行：

- `cargo test --manifest-path src-tauri/Cargo.toml --lib compaction -- --nocapture`：87 通过，0 失败；包含匹配到的内置与外部适配相关测试。
- `npx vitest run src/chat/compactionBoundary.test.ts src/chat/contextPanel.test.ts src/chat/contextClearBoundary.test.ts src/chat/api.historyWindow.test.ts`：4 文件，28 通过。
- 临时接入 [回归实验源码](context-compaction-audit-2026-09-27.repro.rs)，运行 `cargo test --manifest-path src-tauri/Cargo.toml --lib audit_compaction -- --nocapture`：三个期望正确行为的断言均失败，分别对应 F2/F4/F5。完整输出见 [实验日志](context-compaction-audit-2026-09-27.test-output.txt)。实验只连接本机 mock，不调用用户配置的模型服务。

复现方式：将下面一行临时添加到 `src-tauri/src/chat/agent/loop_tests.rs` 末尾，运行上述 `audit_compaction` 命令，随后删除这一行。该文件通过现有测试 helper 调用生产接口，不是一份仿写算法。

```rust
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/research/context-compaction-audit-2026-09-27.repro.rs"));
```

本轮交付时移除临时 include，不将预期失败的实验接入常规测试。保留报告、实验源码和日志；没有提交、推送或改动生产实现。未运行真实供应商压缩、桌面交互、pi/ZCode 测试套件，因此不作实机质量或三者摘要质量高低的结论。

## 审查覆盖记录

这是功能审查，不是“批准当前所有未提交改动”。OCR workspace preview 返回 `total_files=34`、`reviewable_count=18`、`excluded_count=16`。其中完整读取两项 diff（`commands/send.rs`、`useComposerContextMenu.tsx`）：`reviewed_files=2`、`skipped_files=16`、`coverage_rate=11.1%`，该比例仅指 workspace diff 清单，不能当成本次压缩功能的覆盖率。

其余 16 项 diff 均明确跳过整体审查，原因为用户本轮未要求编辑器/斜杠命令工作区变更审查：`package.json`、`commands/interaction.rs`、`commands/tests.rs`、`commands/tooling.rs`、`chat/mod.rs`、`external_agents/run.rs`、`external_agents/session/codex_app_server.rs`、`external_agents/slash.rs`、`InputBar.tsx`、`externalCliSlashCommands.ts`、`slashCommands.ts`、`chat-01-main.css`、`chat/slash_commands.rs`、`ComposerEditor.testSupport.tsx`、`ComposerEditor.tsx`、`SlashCommandIcon.tsx`。部分文件只检索了压缩入口或运行了相关测试，不据此标为完整 diff 已审。

另行扩展审查了压缩核心、commands/context、commands/reply、agent/planning、agent/synthesis、agent/loop、agent/context_estimate、provider runtime 返回契约、仓储 context 写入、sub_agent 的 compacted_history 消费，以及前端边界/统计展示和相关测试。大文件按涉及的函数和调用路径检查，不宣称整个仓库或这些大文件全部通过审查。
