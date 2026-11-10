# 2026-11-10 长任务运行故障调查

本记录调查 Kivio 内置 agent 的执行过程。测试任务是自主完成一个 Android 应用，允许子代理参与。应用业务实现不属于本次审查范围。本记录保存故障事实和验证结果，不新增工程规范。

状态：已完成本轮调查，未修改生产代码。源码基线为 `069580b3b78c040ef844a05feeb5a79ac2e27df6`。

## 证据范围

- 本地对话 `conv_bb58cafe-9a62-46bf-8df1-132d459d69e6`，重点为消息 `msg_1e43c5d6-0eab-49ee-a3ec-9248a6ecb309`。
- 同对话的子代理日志、2026 年 11 月用量日志，以及调查时保存的请求调试快照。
- 本仓库当前源码。调查开始时工作区干净。本记录及两个脚本是本轮新增文件。
- 适用决定为 ADR-0007 的 OMP 子代理契约、ADR-0008 的工具按需加载契约。没有修改这些决定。

原始对话、请求、凭据及业务文件不复制进仓库。可用以下只读脚本重新提取不含正文和工具参数的运行摘要：

```powershell
node docs/research/agent-runtime-test-2026-11-10.audit.cjs --self-test
node docs/research/agent-runtime-test-2026-11-10.audit.cjs "$env:APPDATA/com.zmair.kivio/conversations/conv_bb58cafe-9a62-46bf-8df1-132d459d69e6.json" "$env:APPDATA/com.zmair.kivio/usage/usage-2026-11.jsonl" msg_1e43c5d6-0eab-49ee-a3ec-9248a6ecb309
```

离线复现运行以下命令。需要 Node、Rust 和本机已有的 Cargo 依赖缓存，不请求模型服务，不读取业务文件，不执行模型返回的工具。

```powershell
node docs/research/agent-runtime-test-2026-11-10.repro.cjs
```

复现脚本包含两类验证。SDK 流程通过公开 `run_agent_loop` 和真实 SSE 解码器运行。宿主私有函数从当前源码提取，由最小周边数据类型驱动。后者不等于完整 Tauri 应用测试。脚本断言的是当前缺陷表现，修复对应问题后应更新为修复后的回归断言。临时编译文件在系统临时目录，SDK 编译复用自己的 target 目录。

## 发现索引

| 编号 | 优先级 | 结论 | 证据等级 |
| --- | --- | --- | --- |
| F1 | P1 | 空回复重试分支被流式校验绕过 | 真实 SDK 复现 |
| F2 | P1 | 恢复续跑后保留旧终答段和 degraded 标记 | SDK 复现、落盘记录、源码 |
| F3 | P1 | 子代理空回复被转换成丢失原因的通用失败 | 落盘记录、源码 |
| F4 | P1 | Goal 提示过时，读取结果又不向模型提供完整状态 | 实际模型历史、源码 |
| F5 | P1 | Goal 将证据引用有效当作验证通过，新鲜度判断遗漏 shell 写入 | 原函数复现、实际证据记录 |
| F6 | P1 | `vitest` 被错认成 `vite` 常驻服务 | 原函数复现、实际调用记录 |
| F7 | P2 | 思考增量中的标识符被自动插入换行 | 真实 SSE 回放、解码器复现 |
| F8 | P2 | `cd` 的重定向被当作目录的一部分 | 原函数复现、实际错误记录 |
| F9 | P2 | 空回复用量漏入运行累计，完成 Goal 后又跳过收尾记账 | SDK 复现、对账、源码 |
| F10 | P2 | 调试快照覆盖原始异常，成功口径只到适配器层 | 源码、现存记录边界 |
| F11 | P2 | Todo 收尾提示与未完成 Goal 的自主执行要求冲突 | 实际模型历史、源码 |
| F12 | 执行问题 | 参数、编辑、命令和全局进程清理错误 | 实际工具记录，按原因拆分 |
| F13 | 观察与改进项 | 长时间没有正文进度，缺少单次长运行内的收敛反馈 | 时间线、源码，未做 GUI 验证 |

## F1：空回复的一次重试被提前校验绕过

优先级 P1。已用真实 SDK 离线复现。

`planning_step` 预期通过 `planning_empty_retried` 对第一次空正文且无工具调用的响应重试一次。实际请求先经过 `stream_model_call`，后者在 `validate_stream_output` 中将相同条件转成 `ModelErrorKind::EmptyResponse`。错误返回后进入恢复策略，无法到达后面的空回复重试分支。

涉及 [planning.rs](../../crates/kivio-agent-sdk/src/agent/planning.rs)、[stream.rs](../../crates/kivio-agent-sdk/src/agent/stream.rs)、[recovery_policy.rs](../../src-tauri/src/chat/recovery_policy.rs)。此外，`planning_empty_retried` 保存在整个 RunState 中，成功请求后没有重置。修复执行顺序时还必须明确重试次数按哪一个请求计算。

离线探针在第一次返回以下内容，第二次预备返回正常回答。三种情况均只调用模型一次，第二次回答未使用：

- 正文、思考、工具调用全部为空。
- 只有思考内容。
- 正文只有不可执行的 DSML 工具标记。

最小离线探针已经归档到本记录旁的 `.repro.cjs`。前一轮真实服务回放的原始材料保留在本机 `%TEMP%/kivio-empty-response-probe/`，不作为仓库内复现的依赖。

上游首次异常尚未确诊。历史用量记录标记请求成功且含输出 token，只能证明适配器成功结束，不能证明存在有效正文或工具调用。原始异常响应已经被请求调试的 50 条环形记录覆盖。

已做真实服务验证，未执行返回的工具：短文本、短工具调用均正常。使用两个失败子代理保存的上下文及当前工具定义回放，也分别返回有效的 `read`、`write` 调用。再将四份原始 SSE 回放到真实 SDK 解码器，均正确解析。这些验证不等于原始请求的逐字复现，不能排除偶发上游输出异常或历史解析边界。

后续处理需要让空回复真正经过有界重试，并保存异常响应的终止原因、正文长度、思考长度、工具片段及用量。不能仅增加重试次数而丢失首次异常证据。

提高网络重试设置不能修复这条路径，错误发生在适配器成功返回之后，属于 agent 对响应有效性的处理。

## F2：恢复续跑后，阶段失败仍留在最终回复

SDK 的 `answer_boundary` 允许 SessionPolicy、后续消息或待交付的子代理结果使一次候选终答继续执行。`restore_completed_result` 把恢复回答的 segments 放回 RunState，`Answer::absorb` 又把该回答加入历史。后续成功回答没有撤销旧候选终答段。

宿主的 `RecoveryPolicy::degrade` 另行保存 `degraded`。正常成功没有清除它，`application_result` 在整场运行结束时无条件取出这个旧值。因此 SDK 最终状态和宿主降级状态分属两处，发生了漂移。

本次落盘消息同时满足 `stream_outcome=completed`、`degraded.kind=empty_response`，正文包含第 321 轮空响应提示和第 628 轮完成回答。

用户随后补充的桌面截图确认了实际显示。正常完成回答及附件之后，消息底部仍有黄色“空响应”卡，detail 为 `Chat tools planning returned an empty assistant response`。[MessageBubble.tsx](../../src/chat/MessageBubble.tsx) 只要 message.degraded 存在就渲染卡片，没有区分它是否属于已恢复的阶段失败。这与上述落盘状态一致。

截图还暴露了同卡片的计数错误。RecoveryPolicy 的 MAX_BLOCKS 把 toolSummaries 限为最多 8 条，[DegradedAnswerCard.tsx](../../src/chat/DegradedAnswerCard.tsx) 却把 summaries.length 显示成“本轮已完成 8 个工具调用”。8 是摘要长度，不能代表整轮调用数。当前消息实际记录 646 次调用，其中含失败调用。修复时应明确展示的是完整调用统计还是摘要条数。

离线脚本先执行一个真实 echo 工具，再收到空回复，恢复策略提供阶段提示并要求继续，最后返回正常回答。实际结果为 Completed，segments 同时保留阶段失败提示和正常终答。宿主 degraded 标记的残留由源码和落盘记录确认，未在完整 Tauri 应用内重放。

负责人为 [SDK answer boundary](../../crates/kivio-agent-sdk/src/agent/loop_.rs)、[RecoveryPolicy](../../src-tauri/src/chat/recovery_policy.rs) 和 [application_result](../../src-tauri/src/chat/session_policy.rs)。修复需要区分阶段诊断与最终失败，保留诊断历史，同时确保恢复成功后的最终状态和正文一致。

不能仅凭存在正文来隐藏错误。有部分正文但最终失败时仍需保留失败提示，只有最终恢复成功时才清理过时的终止错误。

## F3：子代理交付覆盖了失败原因和部分成果

`ExcelModule` 完成 17 次工具调用后失败，`AndroidApp-2` 完成 40 次后失败。两个会话都保留了文件写入记录。父代理收到的却是重复的 `SYSTEM WARNING: Subagent ended without a final message.`。

[executor.rs](../../src-tauri/src/chat/sub_agent/executor.rs) 有三个连续处理：

- 检查 `result.degraded.kind == "empty_response"` 时返回 `EMPTY_FINAL`，丢弃具体原因。
- 最终交付在 `error == EMPTY_FINAL` 时把 output 覆盖成同一个警告，已有 tail 不能补充阶段信息。
- `terminal_preview` 将 output 与相同的 error 再次拼接，造成警告重复。

子代理报告失败本身符合实际未完成状态。问题在于父代理没有收到具体失败类别、最后有效步骤和已产生的部分成果摘要。这影响接管、复活和重新派工决策。不能把已经写入磁盘的副作用描述成什么都没做。

该问题与 F1 连成一条故障链，但不应把所有子代理错误都转换成成功。修复应保留失败状态及原始原因，并从已有 records/history 提供部分工作摘要，不推断文件已经验收。

## F4：Goal 的固定旧提示和不可读状态共同导致重复初始化

本次第 2 轮已经成功初始化 7 个标准。第 239、240、510、511 轮仍在尝试初始化。第 512、514 轮又分别猜了 `coverage`、`1` 作为标准编号。

原因有两处：

1. [reply.rs](../../src-tauri/src/chat/commands/reply.rs) 在运行开始时调用 `goal::format_prompt`，把未初始化提示放入固定 runtime context。初始化后，该文字没有在本次长运行中更新。实际模型历史仍保留 `No acceptance criteria have been initialized`。
2. [goal.rs](../../src-tauri/src/chat/goal.rs) 的 `goal_tool_result` 只在正文返回状态和版本，完整 `goalState` 放在 structured_content。[tool_output.rs](../../src-tauri/src/chat/tool_output.rs) 对 native 工具故意只向模型发送 content。三个实际 `get_goal` 调用的结构化结果都有 7 个标准，模型可读结果却均只有 `Goal updated: get_goal. Status: Active. Version: 1`。

这解释了重复初始化和标准编号猜测。最小修复位置应为 Goal 自己的工具结果和状态提示生命周期，不宜把所有 native structured_content 都追加给模型，以免重复大对象并增加上下文。

## F5：证据引用有效不能证明验收通过

[goal/evidence.rs](../../src-tauri/src/chat/goal/evidence.rs) 的 `refresh` 使用 `validate` 是否报错直接设置 `criterion.verified`。artifact 分支只查附件 id、名称或路径是否存在，source 分支只查 URL 前缀，model_self_check 分支只查对应消息是否存在。

本次 c3 覆盖率标准最终引用 Excel 导出件，c5 幂等、撤回、锁定标准引用补单截图，仍被标记为 verified。离线脚本直接执行当前 `validate`，确认存在的 `export.xlsx` 可以支持任意非空覆盖率声明。该检查没有读附件，也没有核对声明。

新鲜度检查还有不对称。tool_result 后出现任何 native write/edit/patch 都使证据过期，即使改的是无关文档。后续 `bash` 却不会过期，因为 `is_mutating_tool` 仅按工具名判断。直接执行当前函数可复现这两种结果。shell 的真实写入内容没有进入该判断，不能声称新鲜度已被完整校验。

第 569、570 轮的“引用未找到”主要是模型把文件相对路径当工具调用 id、提交尚未注册的文件路径，并非本次发现的引用索引故障。第 572 轮使用已返回的附件 id 后被接受。

后续应区分“证据已记录”“引用可解析”“模型自检”和“机器检查通过”，并为可自动验证的标准绑定对应检查结果。不要把文件存在包装成语义验收，也不要要求一个通用函数自动理解所有研究文档和图片。

## F6：有限测试被自动放进后台

[shell.rs](../../src-tauri/src/native_tools/shell.rs) 的 `is_long_running_dev_command` 使用 `starts_with("vite")`、`starts_with("npx vite")` 等字符串前缀。`vitest` 同样命中。`run_command` 在 background 未显式提供时使用这一结果。

直接执行当前函数可复现：`npx vitest run`、`npx vitest ls`、`vitest run` 均返回 true，`node node_modules/vitest/vitest.mjs run` 返回 false。

主代理第 27、52、69、100、190、198 等轮的有限测试未要求后台运行，却收到后台 job。子代理 ExcelModule 的测试也发生相同情况。代理随后改用 sleep、日志文件探测、重复启动和绕开 npx 的命令。这些绕路有宿主误分类的直接诱因。

本记录不能仅凭工具的启动时间证明每次测试都互相重叠。确切的缺陷是分类错误，后台任务完成与工具返回完成因此产生了不同语义。修复需按实际命令词和参数识别开发服务，同时保留显式 background 的选择。

## F7：思考增量被按大小写修改

[model/openai.rs](../../crates/kivio-agent-sdk/src/model/openai.rs) 的 `glued_summary_separator` 为兼容没有 summary_index 的 GPT 摘要标题，按前一段末尾小写、后一段开头大写等条件插入两个换行。`append_chat_stream_reasoning` 把这个规则也用于普通 Chat reasoning delta。

最小 SSE 先返回 `Create App`，再返回 `Container safely.`，现有解码器得到 `Create App\n\nContainer safely.`。这是对普通增量的改写。

前一轮保存的真实 Android 上下文回放响应也可确认该问题。去除两端空白后，线上思考原文为 2233 个字符，解码结果为 2269 个字符，首处差异把 `bodyLarge` 改成 `body\n\nLarge`。本仓库离线脚本不依赖这份私有响应，使用最小 SSE 即可重现。

这会影响存储、展示和再发送的思考内容。没有证据证明它造成最初的空回复。修复需要保留普通 delta 的原始拼接，将标题展示处理限制到能识别为摘要的内容，不能用大小写猜测所有模型的协议边界。

## F8：重定向被误当作 cd 目录

第 118 轮命令以 `cd /d/ZMdata/Yuy/android 2>/dev/null && ...` 开始。实际目录没有空格，宿主却提示路径有空格，建议 cwd 为 `/d/ZMdata/Yuy/android 2>/dev/null`。

[shell.rs](../../src-tauri/src/native_tools/shell.rs) 的 `find_cd_and_separator` 只找字符串 `&&`，`parse_leading_cd_prefix` 将前面的全部文本视为路径，`normalize_run_command` 再因重定向前的空格拒绝它。直接运行原函数可以重现。

第 323 轮同时传 cwd 和 `cd ... &&` 则是模型真实误用，当前拒绝正确。两种错误必须分开。对不能可靠解析的 shell 前缀应保留原命令，或明确要求 cwd，不能生成一个包含重定向的假目录建议。

## F9：两处记账缺口

第一处与 F1 相同。适配器已经记录请求成功及 provider usage，SDK 在流式校验处返回错误，`planning_step` 只在成功分支调用 `merge_usage_with_billing`，导致失败响应未进入运行累计。

主回复落盘输入为 51,159,706，输出为 405,798。628 条 planning 用量记录合计输入 51,210,889，输出 405,840。差额正好为失败响应的 51,183 输入和 42 输出。8 次压缩请求的 822,785 输入、35,510 输出另在 context_compaction 操作中记录，不能与这一次遗漏混为一谈。

真实 SDK 离线探针也证明了遗漏。三个响应分别使用输入 10、20、30，输出 1、2、3。中间空回复后续恢复成功，结果累计只剩输入 40、输出 4，漏了中间请求。

第二处在 [reply.rs](../../src-tauri/src/chat/commands/reply.rs)。回复结束后先用 `goal::is_running` 过滤 Goal，再决定是否调用 `record_run`。模型已经在本轮执行 `goal_complete` 时，Goal 为 Completed，被过滤掉，最终清理与累计不发生。本次保存的 Goal 已完成，但仍持有 active_run_id，input_tokens、output_tokens、total_tokens 均为 null。`goal::record_run` 本身会清理这些字段，问题在调用条件。

还有一个必须纠正的口径：628 条 planning 记录中有 626 条未返回缓存统计。累计 cached_input_tokens=0 不能证明全部请求都未命中缓存。[context_usage.rs](../../crates/kivio-agent-sdk/src/model/context_usage.rs) 本身会跳过未知缓存字段。本记录不将客户端 token 合计换算为实际账单。

## F10：原始异常证据在长任务中被覆盖

[request_debug.rs](../../src-tauri/src/chat/request_debug.rs) 的 REQUEST_DEBUG_CAPACITY 为 50，磁盘镜像每次重写为相同的最近 50 条，并非长期归档。response 保存的是解析后的摘要，正文最多 4000 字符，没有原始 SSE。

[model/openai.rs](../../src-tauri/src/chat/model/openai.rs) 在解码完成后记录 status=success，之后 SDK 才可能因没有有效正文或工具调用而拒绝该响应。因此成功用量记录与最终 empty_response 并不矛盾，分别描述两个阶段。

本次主运行有 628 次 planning 请求，原始故障请求已不在调试快照。两份子代理最末失败请求也只能找到 usage，无法恢复原始 finish_reason、正文和工具片段。不能据此断言服务端真的没有发送内容，或断言某一种未支持字段就是源头。

应对异常请求单独保存受限的诊断材料，并区分 HTTP 成功、协议解码成功和 agent 输出有效。无需把全部长对话长期归档，更不能把凭据写进故障日志。

## F11：Todo 收尾提示与 Goal 执行要求冲突

恢复后第 322 轮的推理明确讨论了两个相反要求。Goal 要求继续完成任务，Todo 提示却说 `Then end with one short closing sentence`。

[todo.rs](../../src-tauri/src/chat/todo.rs) 的 `final_check_reminder` 在 BeforeFinish 对开放条目发出这条提示。该入口也能接收到恢复生成的候选终答，没有根据 Goal 是否仍活动来调整收尾措辞。于是 F2 的阶段失败又触发了一个要求结束的提醒。

模型这次选择继续，没有观察到因此提前终止。宿主提示冲突已由模型历史和源码证实。建议清单提醒只表达条目的真实完成状态，是否继续或结束由同一运行流程决定，避免另一条提示独立要求停机。

## F12：工具使用错误与进程操作问题

主运行 646 次工具调用中有 35 次 error，分布为 initialize_goal_criteria 5 次、task 2 次、bash 18 次、edit 8 次。错误并非同一种原因。

- Goal 的 5 次初始化错误及 2 次未知标准编号与 F4 直接相关，不能全部归因于模型遗忘。
- task 的顶层 solutionSpace 和把原生 read 放进 eval tools 列表，均违反当前公开 schema。宿主正确拒绝。第一次预启动的 AndroidApp 保存为 aborted，requests=0、toolCount=0，没有证据表明非法参数已执行了子代理工作。
- edit 包括一次不完整 JSON、一次缺少 old_string，以及 6 次旧文本未匹配。现有错误明确要求重读原文。这些记录不能证明编辑器丢字。思考改写 F7 是独立已证明问题，不据此推断每次编辑失败的原因。
- bash 包括 F8 的错误拒绝、重复 cwd 的正确拒绝、缺少 Python、错误文件路径、Node 字符串转义或 shell 替换错误，以及数据库探测的 7 分钟、5 分钟、3 分钟超时。F6 能解释测试进入后台，不能单独解释所有数据库生命周期错误。这里不越界审查被开发应用的数据库代码。
- 初次环境探测将用户名 Zhime 写成 Zime，忽略已显示的 ANDROID_HOME 路径，后续却解释成权限问题。现存证据支持路径写错，未证明其所声称的权限原因。
- 计划阶段 ask_user 缺少选项 id、present_artifacts 使用未返回的附件 id，属于模型参数误用，宿主拒绝正确。
- 多个测试命令通过管道或末尾 echo/grep 输出结果，最终 shell 退出码不一定是测试进程退出码。这是命令本身的语义，不能让运行时靠猜 stdout 将其改判成功或失败。

主运行第 22、77、215 轮使用 `taskkill /F /IM postgres.exe`，作用域为整台机器。全程自主开发授权不能证明这些进程全部属于当前任务。没有发现误杀其他数据库的证据，但该操作有越出项目资源范围的风险。shell 当前允许该命令不代表出现了鉴权绕过。应使用已持有的 job、PID 或数据库目录关闭自己启动的进程。

## F13：长任务的可见进度与收敛边界

主运行约 155 分钟。正文进度从第 23 轮到第 298 轮间隔约 62 分钟，第 321 轮到第 446 轮约 48 分钟，第 446 轮到最终回答约 29 分钟。这些时间由最近工具时间近似定位，不当作精确的模型响应时延。

工具卡、思考和子代理状态仍可能提供进度，本次没有检查实际桌面显示，不能描述成界面完全无反馈。问题是缺少面向用户的阶段说明，失败后的策略变化也主要留在推理和命令中。

`session_policy.rs` 向 SDK 传入默认 RunLimits，主代理没有单次运行的模型步数或总时长上限。Goal 的三轮无进展判定发生在自动 run 结束后，不能约束同一个 run 内数百次模型调用。628 次 planning 本身不证明失控，任务确实较大，但 F6 的误分类和重复探测缺少及时收敛信号。

建议提供按阶段的进展和失败摘要，对重复的同类错误提示切换策略。是否增加运行限额需要明确产品语义，不把本次观察直接变成统一的强制步数限制。

## 已排除的误判

- 子代理日志中出现两份完整历史，不等于模型上下文被发送两遍。中间有 compaction 记录，[session.rs](../../src-tauri/src/chat/sub_agent/session.rs) 回放时替换 worker.history，并处理 appendStart 游标。
- subagent_completion_probe 只更新进度，不直接终止子代理。该测试共有 12 次探测请求。没有证据说明探测导致两次空回复。
- 子代理自动选择思考级别并非全部失效。ExcelModule 记录为 medium，AndroidApp-2 为 xhigh，后续角色为 low，与各自的用量记录一致。
- 真实服务的短请求、保存上下文回放和当前解析器均能产生有效调用。尚未复现一个必然触发最初空回复的上游请求。
- 未发现“所有参数错误都应重试”或“所有后端非零退出都是宿主 bug”的依据。只有已证明属于运行时的错误才列为修复项。

## 修复顺序与验证要求

优先处理 F1、F2、F3 的失败与恢复链，确保部分成果保留、恢复能继续、终态一致。再处理 F4 的模型可见 Goal 状态和 F6 的有限命令执行语义，同时修正 F5 的验证标记。F7、F8、F9 可分别在协议解码、shell 适配、运行收尾的原负责人内修复。F10 的异常证据保存应随 F1 一起完成，以便继续查上游首次异常。

每项修复先把本记录的缺陷复现改为行为回归用例。对涉及宿主的项补真实 native 工具调用、子代理失败后父代理接管、恢复后再成功、Goal 完成与取消的应用验证。F12 的危险进程命令不在真实机器重放。

## 本轮验证边界

已运行审计脚本自检、对真实日志的只读审计、离线原函数探针、公开 SDK 流程和最小 SSE 解码复现。前一轮还做过同一服务的短文本、短工具、两个保存上下文的请求回放，未执行返回工具。本轮没有新增真实模型调用。

没有重跑完整 Android 开发任务，没有改生产代码，没有替换用户正在运行的应用，没有做 GUI、macOS 或 Linux 实机验收。最初异常响应的源头仍待新的原始异常证据。
