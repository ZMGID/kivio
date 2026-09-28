# 上下文压缩改造代码审查

日期：2026-09-27。范围为本次 ZCode 风格上下文压缩改造；没有审查同时存在的输入编辑器、行内技能与外部 CLI 改造。使用 open-code-review-delegate 获取范围与规则，由当前 agent 直接审查。本轮未修复业务代码。

2026-09-28 跟进：用户明确不处理 R1 小窗口策略，保持原状；R2、R3、R4 已实施修复。下文保留原始审查证据，修复与验收见文末。

## 发现

### R1 · P1 / high / bug：小窗口被固定预留量压成零预算，正常任务提前终止

- path: `src-tauri/src/chat/agent/compaction.rs`
- start_line: 31
- end_line: 33
- content: 当窗口为 8192、回答输出预算为 1024 时，额外扣除 13000 后自动压缩预算恒为 0。首次只有用户请求、还没有可压缩历史，也被记为一次未解决压缩；执行一个工具后，即使摘要成功，剩余 token 仍不可能小于零预算，第二次检查就触发原有两轮熔断。复现中实际只有 786 / 826 估算 token，结果却是 `compaction_thrash` 并提示上下文超窗，后续正常回答未执行。
- 修复方向：为小窗口适配预留量，并将“没有可压缩历史”与真实失败区分；不能只把零阈值改成一个极小正数。ZCode 同样有固定预留公式，但其消息不足判定和失败处理不同，直接接上 Kivio 原有熔断产生了本次回归。
- 验证：`review_repro_small_valid_context_must_not_thrash`，完整 agent 循环 + 本地 HTTP 模型替身，断言失败。

### R2 · P1 / high / bug：新增快照图片引用未纳入附件回收

- path: `src-tauri/src/chat/storage/conversations.rs`
- start_line: 141
- end_line: 145
- content: 此处将保留片段内的图片外置为 `msgimg-*` 并存储 `kivio-attachment://` 引用，但 `gc::referenced_attachment_names` 只扫描原始消息，不扫描 `context_state.summary.replay.messages`。用户上传图片原文件是 `att_*`，在快照里外置后的 `msgimg-*` 可以只有这一处引用。后续删除一条边界之后的助手回复并触发附件清理时，仍有效的快照图片会被当孤儿删除；恢复快照时只能得到图片缺失占位符。原始上传文件仍在，但快照读取路径不会自动回退到它。
- 修复方向：将快照的附件 URI 纳入统一引用集合，并覆盖保存、清理、重新加载后重建图片的测试。
- 验证：`review_repro_replay_image_must_not_be_gc_candidate` 调用实际引用收集和回收候选计算，确认活跃快照图片进入待删除名单。没有删除用户文件，也没有将此测试宣称为完整磁盘删除实测。

### R3 · P2 / medium / bug：快照重放绕过多回答选中项

- path: `src-tauri/src/chat/commands/context.rs`
- start_line: 1248
- end_line: 1257
- content: 新逻辑直接追加快照保留消息，再跳过覆盖终点前的原始消息；下面的 `group_answer_excluded_from_context` 因而无法筛选这些回答。`chat_set_group_selection` 仅更新选中映射，不使摘要或快照失效。保留片段里的回答 A 在压缩后被用户改选为 B 时，下一轮仍会发送 A，B 不进入请求。`adopt_compacted_context` 也未比较选中映射，生成期间改选还可能被迟到的快照继续沿用。
- 修复方向：选中映射变化应使受影响的摘要/快照失效或重建；迟到提交也要验证实际模型输入所依赖的选中映射。
- 验证：`review_repro_selected_answer_must_replace_snapshot_answer` 使用已保存快照夹具，执行与选中命令相同的映射变更，调用实际消息构建函数，得到含 `STALE_A`、不含 `SELECTED_B` 的请求。属于输入重建用例，没有进行真实模型多回答桌面测试。

### R4 · P2 / medium / bug：统计缓存写入冲突中断正常发送

- path: `src-tauri/src/chat/commands/send.rs`
- start_line: 323
- end_line: 325
- content: 用户消息在前面已经保存，随后统计计算包含异步等待。期间刷新上下文统计、改标题等操作会推进 revision；新的 `update_context(...conversation.revision...)` 严格检查旧版本，冲突直接由 `?` 传播，后续模型调用完全不执行，留下已入库的用户问题。此前的无条件合并虽应收紧，但不能将统计缓存竞争升级为发送失败。同文件夹的统计刷新入口已经采用冲突时返回最新状态的策略。
- 修复方向：保留 CAS 防止旧统计覆盖新摘要；冲突时重新读取最新会话、必要时重算统计，继续发送，不能直接退回无条件覆盖。
- 验证：调用链静态确认，依据 `repository.rs` 的 `update_context` 版本检查及发送入口 `?`；本轮未执行桌面并发复现。

## 验证与覆盖

- 本轮前端相关回归：3 个测试文件、60 项全部通过。
- 后端针对性审查复现：3 项均按预期暴露缺陷（0 通过、3 失败），不是编译失败。临时 `include!` 已从两个原测试文件移除，复现源码与输出保留在下方证据目录。
- `git diff --check` 通过，仅有现存 CRLF 转 LF 提示。
- 原有真实模型桌面测试覆盖正常压缩、取消、排队及重启，这些结果不证明本次发现的边界场景正确。
- OCR 清单：`total_files=44`，`reviewed_files=27`，`skipped_files=17`，`coverage_rate=61.36%`；本次压缩范围覆盖率 100%。另外阅读了 OCR 默认排除的 3 个前端测试、摘要提示词和 2 份实施/实测记录。混合文件仅审查压缩相关差异。

完整逐文件清单、跳过原因、规则、复现源码和测试输出位于本机 `C:/Users/11028/AppData/Local/Temp/kivio-compaction-review-20260927/`，其中 `coverage.json` 为机器可读覆盖记录。

以下 17 项跳过：`package.json`、`src/chat/InputBar.tsx`、`src/chat/useComposerContextMenu.tsx`、`src/styles/chat-01-main.css`、`src/chat/ComposerEditor.testSupport.tsx`、`src/chat/ComposerEditor.tsx` 属于独立输入编辑器改造；`src-tauri/src/chat/commands/interaction.rs`、`src-tauri/src/chat/commands/tooling.rs`、`src-tauri/src/chat/mod.rs`、`src-tauri/src/chat/slash_commands.rs`、`src/chat/slashCommands.ts`、`src/chat/SlashCommandIcon.tsx` 属于独立行内技能/斜杠命令改造；`src-tauri/src/external_agents/run.rs`、`src-tauri/src/external_agents/session/codex_app_server.rs`、`src-tauri/src/external_agents/slash.rs`、`src/chat/externalCliSlashCommands.ts` 属于独立外部 CLI 改造；`docs/research/context-compaction-audit-2026-09-27.repro.rs` 是旧实现的历史审计用例。


## 2026-09-28 修复与验收

按用户要求，R1 不改。R2、R3、R4 已修复，未提交 Git commit。

对照的 ZCode 本地代码是 `runtime/helpers/compact-preservation.ts`、`agent/compact-session.ts`、`agent/session-history-hydrator.ts` 和 `runtime/methods/compact-active.ts`：保留区通过持久消息 ID 重建，恢复图片时读取持久消息的媒体部件，压缩摘要持久化后才替换运行历史。Kivio 目前采用独立 replay 快照，因此补齐其引用与失效规则；没有声称 ZCode 存在 Kivio 的多回答选择或同样的 JSON revision 缓存机制。

- R2：附件回收的唯一引用收集器同时扫描 replay；复用现有 URI 解析。回归执行图片外置、写盘、读盘、清理孤儿、重新还原图片，确认活图保留、孤儿仍被回收。
- R3：选择回答统一调用 `Conversation::select_group_answer`，按组内最早回答判断是否影响摘要/快照覆盖区；既有消息编辑也复用同一失效判断。新模型回答自动选中、手动选中及仓储元数据写入均接入。迟到压缩提交同时验证选中映射。显式重复选择不失效；边界之后的独立组不影响旧摘要。
- R4：发送前保存统计复用现有 `persist_context_state_best_effort`。仍然 CAS 提交，冲突返回最新会话，不覆盖新摘要，不增加重试循环，也不让统计竞争中止已保存的用户问题。

验证：新增 4 个回归测试，其中最初 3 个先确认失败、修复后通过；第 4 个覆盖旧摘要、新快照、边界及重复选择。聊天模块共 998 通过、1 跳过；架构检查 12 项与边界检查通过；Windows debug 应用构建通过；`git diff --check` 通过。

桌面实测通过 WebView2 CDP 调用真实应用的生产 `chatApi`/Tauri 命令，使用专门构造的会话夹具和已配置 DeepSeek，不模拟模型回复。本轮没有宣称逐个点击所有 UI 按钮：

1. 旧程序中，发送同时执行 25 次改标题和 8 次统计刷新，复现 `conversation revision conflict: expected 4, actual 5`，发送失败。
2. 新程序中，同样的并发操作连续执行 3 轮，3 次发送均成功回复 `TEST_OK`，24 次统计刷新均成功。
3. 通过生产删除消息命令删掉快照边界之后的一条回复，落盘快照仍有效，外置的 `msgimg-3682e921b6a981cc.png` 仍存在。
4. 通过生产选中命令从 A 切为 B，摘要立即标记失效；真实模型续聊得到 `BRAVO-42`，不是快照中的 `ALPHA-11`。重新打开会话后仍保留选中 B 与失效状态。

保留测试会话 `conv_f9f2b149-527f-44c8-bbec-c6ed6ae86d4e`（统计并发）、`conv_cc2f9dae-e042-4fe4-a3b5-7c6242e1b893`（图片与选中回答）。脚本和前后结果在本机 `C:/Users/11028/AppData/Local/Temp/kivio-compaction-fix-20260928/`。未改用户供应商设置；自动化已与桌面分离。

## 2026-09-28 独立复核后的补充修复

独立对照 ZCode 发现视频快照发送策略和摘要超窗终态两项遗漏。用户随后明确视频项本轮不做，已撤回本轮新增的视频投影、预算处理与对应测试，保留既有视频功能和此前压缩修复。

本轮最终只修复摘要超窗终态：在 `recovery.rs` 统一识别 `context_exceeded`、`context_length_exceeded`、`context_window_exceeded`、`model_context_window_exceeded`、`model_context_exceeded`、`prompt_too_long`（容忍大小写和两端空白）。`summarize_history` 在采用任何摘要文本前将这些终态转入既有超窗分支；自动压缩移出完整近期分组，手动压缩缩减旧输入，重试耗尽返回失败、保留原历史。

依据为 ZCode `runtime/methods/compact-active-helpers.ts` 的 `createCompactContextExceededFinishError` 及 `runtime/helpers/model-errors.ts` 的终态标记集合。没有增加第二套重试流程。

两个新增回归先在旧实现上失败，修复后通过。通过本地 HTTP/SSE 模拟服务走实际 OpenAI 适配器，覆盖携带非空残缺摘要的各类超窗终态、自动/手动缩减请求后成功、耗尽后不采用残缺摘要。最终聊天模块回归 **1000 通过、1 跳过**；架构检查 12 项及依赖边界通过；`git diff --check` 通过。没有真实供应商故障注入验证；本轮桌面自动化连接被自动审批机制以 `blocked by policy` 拒绝，未执行桌面实测，亦未替换运行中的应用。

## 2026-09-28 快照回放修复（对照 ZCode 深审）

对照 ZCode `328c1a0` 的 `runtime/helpers/compact.ts`（`buildPostCompactRuntimeEntries`）与 `compact/prompt.ts`（`buildCompactSummaryMessage`）复核后，修复两项快照回放缺陷；审查中的其他发现（摘要请求不带工具、自动压缩失败即收尾、手动压缩媒体投影与超窗标记、队列失败卡住、手动 CAS）本轮未处理。

- **摘要落盘角色**：ZCode 的摘要始终是 user 消息。Kivio 运行中同样是 user，但落盘回放改成 system；各适配器会把 system 提升进系统提示，保留片段又从 assistant 工具调用开始，导致下一轮首条消息是 assistant 工具调用（Gemini 要求工具调用紧随 user 或工具结果）。现 `summary_message` 回放为 user；上下文统计按摘要前缀单独计入，不重复计数。
- **快照混入运行期消息**：快照取自运行中消息列表，而非普通落盘用的 `generated_api_messages`。
  - 工具轮次上限提示（system）被写入快照后，每轮都会提升进系统提示并禁止调用工具。`replacement_body` 与回放共用 `is_replayable` 排除 system 消息，回放时也覆盖此前已保存的快照。
  - 子 agent 报告既在快照中，又作为 `subagent-result-*` 消息追加在回复之后，会被发送两次。报告在运行中携带 `_subagent_result_id`，回放时跳过快照已含的结果消息。第一轮可能在回复草稿落库前收集报告，所以不采用直接从快照删除的方案，那样会丢失报告。

验证：新增和扩展的回归覆盖了以下几点：
- 快照首条为 user 摘要，并确认经 Gemini 适配器后首条仍是 user；
- 旧快照里的 system 提示在回放时被过滤；
- 运行循环中触达轮次上限后，快照不含该提示；
- 子 agent 报告只发送一次，快照外的报告照常回放；
- 运行期报告携带结果消息 id。

逐项撤回修复后，上述 5 项断言均失败；恢复修复后全部通过。另外，`file_ledger_flows_into_replayed_summary_message` 的断言按新角色从 system 改为 user。本轮在 Linux 容器执行：聊天模块 **1010 通过、2 跳过**；`git diff --check` 通过，改动区域 rustfmt 无差异。构建时为不相关的 Windows/macOS 专属代码（`offline_models.rs`、`windows.rs`）临时加过 Linux 占位，测试后已还原、未提交。未做真实 Gemini/Anthropic 实测，也未做桌面实测。

## 2026-09-28 其余发现的修复

用户要求修复审查中剩余全部发现（R1 小窗口策略仍按原决定不改）。依据 ZCode `328c1a0` 的 `compact-active.ts`、`compact.ts`（`autoCompactIfNeeded` / `reactiveCompactAfterContextExceeded`）、`compact-selection.ts`、`turn-loop-state.ts`、`compact-post-reminders.ts` 与 `compact-active-helpers.ts`。

- **摘要请求的工具定义**：自动压缩把本轮工具随摘要请求发送（超过 100 个时不发送），提示词末尾补充禁止调用工具的提醒；不带工具定义时（含手动压缩），工具调用和结果转为文本，避免供应商拒绝孤立的工具调用历史。
- **失败策略**：
  - 原先两次“未解决”就以 `compaction_thrash` 结束本轮；现在失败不结束本轮，请求照常发出，只计入熔断计数。
  - 熔断计数由 `ChatRuntimeState` 按会话保存，连续三次后暂停自动压缩，成功后清零；子 agent 与测试宿主只在单次运行内计数。
  - 历史不足以摘要时视为正常跳过，不发 started/failed、不计失败，这也使 R1 场景不再误报失败。
  - 压缩后仍超预算不再计为失败。
  - 快速回填在开始前判断、只在成功时记录，触发时才以已收集结果收尾。
- **手动压缩**：
  - 摘要请求不带原始视频，按发送视图同一预算收敛图片；压缩模型已知不支持视觉时去掉图片。
  - 截断重试后若首条为 assistant，先补截断标记。
  - 提交遇到版本冲突时，用与自动压缩相同的 `history_unchanged` 规则判断：被摘要历史未变则重新提交（最多三次），否则报错并保留原历史。
- **排队**：压缩失败时消耗该条目并继续发送后续消息，不再卡住队列；停止压缩仍按此前实测决定，不自动发送后续消息。
- **较小项**：
  - 文件提醒只取成功读取的文本文件结果，并注明可能是部分内容。
  - 空正文的 `length` 结束按超窗处理。
  - 自动压缩中的停止按钮显示为“停止生成”。
  - 删除未使用的 `window`、`transport_attempts`、`retry_attempts`、`focus`、`trigger` 参数。

测试夹具调整：若干测试原先依赖“只把一条用户消息摘要掉”的宽松行为，按 ZCode 规则这类情况本应跳过或失败，因此给这些夹具补一轮较早的历史，使其继续覆盖原本要验证的重选、取消、超窗重压与耗尽路径；断言本身未放宽。旧的两次失败即收尾测试替换为失败后继续回答、熔断跨轮暂停与清零、历史不足静默跳过、快速回填收尾四项。

验证：
- 逐项撤回修复后，工具定义、截断标记、空 `length`、文件提醒、媒体投影、静默跳过、失败后继续这 7 项后端新测试全部失败；前端队列与停止文案测试同样在旧实现上失败；恢复后全部通过。
- 聊天模块 **1021 通过、2 跳过**；前端 203 个测试文件 **1698 项通过**；TypeScript、ESLint、架构检查、协议一致性与 `git diff --check` 通过。
- 手动压缩冲突重试依赖 `AppHandle`，只以 `history_unchanged` 单元测试覆盖判定规则，未做并发实测。
- 未做真实供应商或桌面实测；构建所需的 Linux 临时占位已还原、未提交。

合并前复审补充两处：
- 不支持工具的供应商在每轮第一次规划请求被拒之前就会触发自动压缩，此时摘要请求带着工具定义会被拒绝，重试无效，还会触发熔断。现在被拒时去掉工具、以文本历史重试一次，不计失败；本轮已判定不支持工具时直接不带。
- 与 ZCode `rewind-message.ts` 一致，回退消息后清零熔断计数。

新增回归测试在未修复时失败、修复后通过；聊天模块 **1022 通过、2 跳过**。
