# 正文误折叠：消息语义与展示方案调研

调研日期：2026-09-12。范围：基于已复现的 Kivio 会话，比较消息分类与折叠方案；本次仅研究，不修改生产代码。外部依据为官方 API 文档，本地依据为当前工作区源码。文中的设计规则是建议，不代表供应商对客户端界面的要求。

## 结论

建议分两步：**短期让语义未知的普通正文保持可见，只折叠思考、工具记录及来源明确的过程通知；中期将“执行阶段”和“消息意图”拆成两个字段，按实际供应商能力适配。** 不再把“后面还有工具”作为折叠正文的条件。复制和导出应使用相同的正文选择规则，否则会出现看得到四个问题、复制却只有最后一段的另一种缺失。

本次案例不是证明模型违反了协议。DeepSeek 允许普通内容和工具调用同轮出现；客户端无法只靠工具调用的存在推断内容不重要。模型在最后重复问题可以缓解阅读断裂，但不能作为界面完整性的保证。

## 已确认的本地事实（修复前）

本节记录调研时的代码，基线为 `4dfd65007cc8fe333fd9036dd327ba0a416299aa`；实施后的行为见文末补记，原源码行号可能已移动。

前次真实会话复现顺序为：约 1,208 字的 Q1–Q4 正文（`text/tool_loop`）→ 环境检查工具 → 约 167 字的末尾正文（`plain`），最后要求用户回答“上面四个问题”。展开后能看到问题，默认折叠时看不到。

前次诊断使用 `npx vitest run src/chat/fold-diagnostic.test.ts`，结果为 `visibleContainsQuestions=false`、`foldedContainsQuestions=true`、`visibleRefersToHiddenQuestions=true`，正文可见性断言失败。该临时测试已清理，这不是仓库当前可运行的测试命令。本轮研究没有重新执行该测试。后续可用以下三段脱敏输入建立永久回归，不需要保存真实会话全文或 ID：

```text
text/tool_loop: “Q1 目标用户是谁？Q2 有何差异？Q3 首个交付是什么？Q4 有哪些约束？”
tool/tool_loop: 环境检查及结果
text/plain: “环境检查完成。请回答上面四个问题。”
```

| 环节 | 当前行为及影响 | 源码 |
| --- | --- | --- |
| 数据类型 | `Auxiliary / Plain / ToolLoop / Synthesis` 是 Kivio 本地阶段枚举，并非供应商的 commentary/final 协议 | [types.rs](../../src-tauri/src/chat/types.rs#L351) |
| 内置规划循环 | 正文先以 `ToolLoop` 创建，无工具调用的分支再改为 `Plain`；带工具的正文可能保留原阶段 | [planning.rs](../../src-tauri/src/chat/agent/planning.rs#L152)、[无工具分支](../../src-tauri/src/chat/agent/planning.rs#L378) |
| 前端分组 | `tool_loop/auxiliary` 的 text 被当作过程；另有 `index < lastProcessIndex`，连 `plain/synthesis` 也可能因后面出现过程而收进组 | [分类](../../src/chat/segments.ts#L405)、[分组](../../src/chat/segments.ts#L449) |
| 存储正文 | `content_from_segments` 只拼接 `Text + Plain/Synthesis` | [messages.rs](../../src-tauri/src/chat/commands/messages.rs#L660) |
| 历史归一 | 正文存在、但上述聚合为空时补 synthesis；直接批量改 phase 或简单全量拼接会影响补段及去重 | [messages.rs](../../src-tauri/src/chat/commands/messages.rs#L496) |
| 外部 CLI | 已专门避免将 text 标为 ToolLoop，以防漏聚合和补段重复；但仍受前端位置规则影响 | [run.rs](../../src-tauri/src/external_agents/run.rs#L2724) |
| 复制、导出 | 复制直接用 `message.content`，导出同样读取它；仅修视觉分组不足以恢复这两条路径的完整性 | [复制](../../src/chat/MessageBubble.tsx#L1130)、[导出](../../src-tauri/src/chat/export.rs#L74) |
| 模型回放 | 存储另有 `model_messages`，优先作为回放来源；为空才保留 `api_messages` 兜底 | [messages.rs](../../src-tauri/src/chat/commands/messages.rs#L145) |

由此推断，根因是同一个 phase 同时承担执行时序、正文聚合和可见性判断，且前端又叠加了位置推断。不能将 `ToolLoop` 全部改名为 `commentary` 就视为解决。

## 官方协议能提供什么证据

### OpenAI：有明确 phase，但它不是折叠指令

官方 Responses 指南说明 assistant item 可携带 `phase: commentary`（中间的用户可见更新）或 `final_answer`（完成答案）。手动回放时必须保留原 phase；使用 `previous_response_id` 时由 API 保留先前状态。该文档没有要求客户端必须把 commentary 折叠，也不能据此推断 Codex 桌面内部实现。[OpenAI Model guidance：Phase parameter](https://developers.openai.com/api/docs/guides/latest-model?model=gpt-5.5#phase-parameter)

对 Kivio 的建议：只有实际接入端点、模型及事件确实提供该字段时才保留和映射，缺失时使用 unknown。不要从 OpenAI 兼容 URL、模型名称或工具事件顺序推导 phase。API 中间更新可作为独立样式或折叠候选；是否默认折叠仍是产品策略，尤其应保证等待用户回答的交互入口可见。

### DeepSeek：content 与工具调用可以共存

Chat Completions 响应区分 `content`、`reasoning_content` 和 `tool_calls`；`finish_reason=tool_calls` 表示调用工具，不表示正文属于思考或可隐藏内容。当前查阅的该响应 schema 没有与 commentary/final_answer 等价的 message intent 字段。[DeepSeek Chat Completions API](https://api-docs.deepseek.com/api/create-chat-completion/)

官方 thinking 示例直接展示同轮非空 `content` 加 `tool_calls`；其工具循环将完整 assistant message 回传，保留内容、思考和调用信息。文档还单独规定携带 tools 时 reasoning 的回放要求。因此 UI 隐藏、正文复制与模型上下文不能共用一个“删掉过程”操作。[DeepSeek Thinking Mode](https://api-docs.deepseek.com/guides/thinking_mode/)

本建议适用于已查阅的 Chat Completions 协议。截图里的 `deepseek-v4-flash` 也可能是别名或代理模型；未验证该会话实际端点与原始事件，不将官方当前其他模型名或其他协议的能力套用到它。

### Anthropic：内容块类别不等于正文意图

Messages API 在同一 assistant `content` 数组中允许 `text` 和 `tool_use`；官方示例正是先 text 再 tool_use。工具结果需按其消息及关联 ID 规则回传。该结构能区分文本和工具，不能单独区分“关键问题”与“过程旁白”。[Anthropic Handle tool calls](https://platform.claude.com/docs/en/agents-and-tools/tool-use/handle-tool-calls)

适配建议：保留原内容块身份与顺序；普通 text 默认可见，不能因 stop reason 为 tool_use 将整个 assistant 消息归为思考或过程。显示层的分组不应改变协议回放结构。

## 可行方案比较

以下成本为相对工程判断，尚未进行实现工时估算或真实供应商联调。

| 方案 | 对当前问题的效果 | 代价与边界 | 结论 |
| --- | --- | --- | --- |
| A. 保守保留普通正文 | 直接保住工具前、工具间问题；支持缺失语义的历史和供应商 | 进度短句会增多；需同步修复制/导出投影 | **短期首选** |
| B. 独立 intent + 来源 + 能力适配 | 有依据地区分正文和更新，不混用执行阶段 | 跨事件、持久化、适配器和回放；无原生语义的供应商仍需 A 兜底 | **中期主线** |
| C. 仅提示词：最终答案自包含 | 可减少“见上文”的断裂 | 不保证遵从；中断/失败可能没有最终答案；不修历史，还可能重复长正文 | 辅助，不作为修复 |
| D. 长度、问号、标题等启发式 | 可能保住本次长问题 | 短问题、代码、表格、不同语言都可误判；流式增长还会跨阈值跳动 | 不用于自动隐藏 |
| E. 整个 Working 默认展开 | 改动少，现有内容容易找回 | 根本分类仍错；工具/思考噪声及渲染成本上升；复制仍缺内容 | 可作临时回退或用户偏好 |

也不建议为本问题新增一轮 LLM 分类：它增加等待和成本，分类仍会出错，而且历史或流式内容将随二次分类改变位置。若以后评估分类器，最多用于建议收起，不应作为内容完整性的唯一依据。

## 短期实现边界

1. 移除正文基于 `lastProcessIndex` 的折叠推断。`kind=reasoning/tool` 可维持过程展示；`kind=text` 默认留在正文时间线上，即便旧 phase 是 `tool_loop`。不要把 unknown 当作 final，也不要把它当作 commentary。
2. 审计 `auxiliary` 的各个生产者。只有确认来自应用的纯状态通知才继续折叠；不能因枚举名字听起来像辅助内容就隐藏其中的模型文本。问题卡、审批和产物卡维持独立可见的交互规则。
3. 增加明确的“用户正文投影”规则，为显示、复制及导出提供一致的正文选集。可以先以独立 selector 实现，避免立即重定义既有 `message.content`。工具原始输出和 reasoning 不应因为全量拼接而混入复制正文。
4. 保留原 phase、segment ID/order 和回放数据，不进行不可逆的历史批量改写。不因 UI 变化修改 `model_messages/api_messages`；编辑消息等依赖 `message.content` 的路径另外审计。
5. 保护历史补段：优先用稳定 ID、来源和已知补段关系去除同源副本，不用“两个字符串一样”全局去重。内容相同的两次模型发言可能本来就有意义。旧记录无法确定重叠时，保留信息优先并记录未决兼容分支；不能悄悄丢弃正文。

## 中期数据设计

建议新增独立可选字段（命名仅为候选，尚未定稿）：

```text
kind: text | reasoning | tool
execution_phase: 现有 phase 语义
message_intent: unknown | commentary | final_answer
intent_source: provider | application | absent
provider_metadata: 保留实际原始 phase 等语义
```

这里 `message_intent` 是应用归一后的意图，`execution_phase` 是执行阶段。`kind` 判定内容类别；意图缺失时默认保留正文；原生 provider phase 原样保存并按协议回放；只有应用自身产生且确定为通知的文本可标 application commentary。工具数量、tool-loop 轮数和“最后一段”都不能赋予可信意图。

供应商能力应落在实际 adapter/协议路径，不能仅按品牌开关。即使某供应商支持 Responses，也需核实该端点真正返回 phase、流式开始和完成事件如何携带字段、兼容代理是否透传。格式支持不等于模型正确使用语义；仍需允许用户展开和查看全部更新。

不能只给 `ChatMessageSegment` 加字段便声称已支持原生意图。当前本地 Responses 路径需审核 [output text/item 事件处理](../../src-tauri/src/chat/model/responses.rs#L1084)、[StreamPart](../../src-tauri/src/chat/model/types.rs#L448)及[ModelMessage](../../src-tauri/src/chat/model/types.rs#L104)。完整传递链应为：原始 output item 的 ID/phase → 流式事件（delta 关联正确 item）→ 模型消息和聊天 segment → 持久化/恢复 → 展示投影；回放另外将原始语义传回供应商。一个 response 内多个 assistant item 不能先合并为一个字符串再赋单一 intent，否则 commentary/final 边界已丢失。

对于缺少原生意图的模型，可以补充提示：工具前只写简短进度；需要用户回答的问题放在可交互出口；最后回复包含继续任务必需的信息。但提示生成的自由文本标签不能直接升级成可信 provider intent。若未来需要结构化 intent，必须作为独立协议设计验证工具调用兼容、增量解析和回放，不能只加 `<commentary>` 标签并隐藏匹配文本。

## 流式、历史和交互稳定性

- **流式稳定性**：新 text 先可见，后续 tool/reasoning 不把已展示的正文搬进折叠壳。provider intent 迟到时，建议本轮已可见文本不自动收起；下次加载再按明确规则呈现。不要等待完整一轮才显示正文。
- **稳定身份**：用 segment ID 或确定的过程组身份维持展开状态、焦点和滚动锚点，避免数组索引变化造成重新挂载；多个过程区可以被正文分开，保持时间顺序。
- **异常结束**：取消、超时、工具错误、达到输出上限时没有 final_answer，也必须留下已产生正文和可行动错误；“有最终答案才展示”不成立。
- **历史兼容**：无 intent 的记录走 unknown；保留旧 JSON 可读性与 fallback。覆盖只有 message.content、只有 segments、二者重叠、normalize 补 synthesis 四种输入。未知 enum 值需有兼容降级策略，不能导致整条记录解析失败。
- **复制/导出**：默认输出完整用户正文，按原顺序包含工具前问题和最终补充；明确区分“复制正文”和可选的“导出完整执行记录”。这属于产品建议，不是供应商协议要求。
- **可访问性**：折叠按钮应支持 Enter/Space 与正确 `aria-expanded`，可通过 `aria-controls` 关联内容；这些规范说明操作方式，没有规定本次哪些正文应隐藏。[WAI-ARIA Disclosure Pattern](https://www.w3.org/WAI/ARIA/apg/patterns/disclosure/)
- **性能**：保留折叠工具详情按需挂载；修正文选择不等于展开所有工具结果。沿用已有[滚动锚定测试](../../src/chat/MessageList.disclosure.test.tsx)和[气泡折叠测试](../../src/chat/MessageBubble.test.tsx)。

## 实施时的验证矩阵

下表是后续修复的验收要求，本次调研未新增或运行实现测试。

| 场景 | 应验证的结果 |
| --- | --- |
| 本次 Q1–Q4 → 工具 → “回答上面问题” | Q1–Q4 默认可见；复制、导出包含四题与最后补充，各一次 |
| plain/synthesis → 后续工具或 reasoning | 后续事件不将前面正文折叠 |
| 短问题、长解释、代码、表格、多语言正文 | 与字数、问号、标题无关，未知意图始终保留 |
| text 流式增长 → tool delta → final | 文本不突然消失，顺序稳定，滚动和焦点不跳 |
| 显式 commentary/final_answer 和缺失 phase | 明确元数据可映射；缺失按 unknown；模型回放保留原 phase |
| DeepSeek content + reasoning_content + tool_calls | 三种内容分开，正文保留，协议历史不因显示规则改变 |
| Anthropic text + tool_use | text 可见；工具 ID 与 tool_result 顺序不受影响 |
| 外部 CLI 多轮 text/tool 交错 | 不受位置折叠影响，不触发 synthesis 重复 |
| 取消、错误、截断、无最终正文 | 已有普通正文仍可读可复制 |
| 旧 ToolLoop/auxiliary、缺字段、补 synthesis、相同内容多次出现 | 不丢正文，不无依据地全局去重；历史读取稳定 |
| 问题卡、审批卡、子代理卡、产物卡 | 独立交互继续可见，键盘可操作 |
| 长工具链 | 工具详情继续懒挂载，折叠展开状态及滚动锚点稳定 |

建议优先扩展现有 [segments.test.ts](../../src/chat/segments.test.ts)，随后覆盖气泡复制与后端导出/归一测试。针对模型回放，应比较修改前后请求结构，确保 UI 修复没有意外重写上下文。

## 尚需确认

1. 本次 DeepSeek 会话实际使用的 adapter、代理端点及原始响应是否包含未被保存的意图元数据；品牌和截图不足以确认。
2. `auxiliary` 全部生产者中哪些是确定的应用通知，哪些可能包含用户需阅读的信息。
3. 历史 synthesis 补段是否已有可可靠区分的来源标识；若没有，兼容去重需要专项样本，不能先实施全量 phase 迁移。
4. 用户希望明确的 commentary 完成后默认收起，还是留为淡色进度；这一偏好不影响短期保住 unknown 正文。
5. `message.content` 除复制/导出外的搜索、编辑、摘要、会话列表用途，需要在实现阶段完整枚举，防止投影调整造成次生变化。

可先推进方案 A 的具体实现设计，不需要先切换模型或改用另一套 API。方案 B 应在 A 的保守 fallback 上逐个接入；提示词仅作为体验改善。官方文档支持的是协议语义区分，本文并未核实或声称复刻 Codex 桌面的内部折叠算法。

## 实施补记（2026-09-12）

已落实方案 A：正文不再按 phase 或工具位置折叠；思考、工具详情仍按需展开。`messageBody.ts` 提供显示与复制投影，Rust 导出遵循相同的共享 fixture。阶段、存储正文与模型回放均未修改。供应商原生 intent 的端到端适配属于方案 B，未在本次引入。

兼容处理覆盖：旧系统补出的 synthesis 副本、只有 content 的旧消息、缺失正文段的时间线、流式 raw delta 拼接（含单独空白段）及持久化 Plain/Synthesis 聚合镜像。只去除有系统补段 ID 且确认为同源镜像的副本，不对独立模型发言做全局文本去重。

验证记录：

- 正文可见性、复制与 Rust 导出均先由回归测试复现失败，再通过修复。
- 全量前端测试：`npm test -- --maxWorkers=2 --minWorkers=1`，1,473 项通过。最初默认并发运行时有旧标题目录预期失败及一项性能用例超时；更新目录预期后低并发全量通过。
- 全量之后补充纯空白流式段的边界回归；正文、气泡、标题目录定向重跑共 53 项通过。
- 最终 Rust 导出验证：`.\scripts\win-cargo-test.ps1 --lib chat::export::tests`，4 项通过；共享 fixture 覆盖前后端一致性。
- `npm run typecheck`（含协议一致性检查）通过；修改涉及的 TypeScript/TSX 文件 ESLint 通过。
- 浏览器使用真实 `MessageBubble` 组件及脱敏样例验收：四个问题可见，`Worked for 23s` 默认折叠；展开工具记录不移动或隐藏正文。临时预览页面已清理。

### Standards 审查

无剩余规范问题。补审发现的纯空白流式段镜像重复已修复，并由共享 fixture 覆盖。

### Spec 审查

方案 A 的正文可见、复制、导出及历史兼容要求已落实。审查发现的流式 raw delta 镜像重复已修复，并通过真实 `applyStreamDeltaToSnapshot` 回归。未引入方案 B 或修改模型上下文。

两轴审查剩余问题：Standards 0，Spec 0。
