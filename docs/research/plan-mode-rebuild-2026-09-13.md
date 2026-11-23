# Plan 模式重建调查

调查日期：2026-09-13。先调查官方产品行为，再对照 Kivio；本次只写研究文档，不修改生产代码。外部页面均已实际打开，以下把来源事实与本产品建议分开。

## 建议结论

建议重建为：**先调查，生成并保存一份 Markdown 计划；用户在文档上修改，主代理按选定文档继续执行。** 聊天承载讨论和简短说明，文件承载当前计划。Plan 不实施业务改动，但可以保存自己的计划文档。

本项目需要一起调整产物生成、文件保存、文档入口和执行交接；只缩短提示词或给旧回复增加“导出”按钮，还不能解决当前内容与执行内容不一致的问题。第一版不增加任务评分、自动验收、强制 Todo 或多层批准。

## 外部产品的实际做法

### Cursor：计划可编辑，保存位置不必是项目

**官方事实。** Plan 先研究代码，再生成可编辑计划；用户可以通过聊天或 Markdown 修改，点击 Build 开始实现。计划默认保存在用户主目录，也可以 Save to workspace。帮助页把展示形态称为可读写的虚拟文件。[Plan Mode](https://cursor.com/docs/agent/plan-mode)、[Plan mode help](https://cursor.com/help/ai-features/plan-mode)。发布文章明确计划是含文件位置与代码引用的 Markdown，可直接编辑。[Introducing Plan Mode](https://cursor.com/blog/plan-mode)。

**证据边界。** 当前打开的正式文档没有承诺主目录下的精确子路径、跨会话自动恢复细节、Build 对未保存编辑的处理或原子快照机制。因此不把常见的 `~/.cursor/plans` 说成此次已核实的当前规范，也不据此声称无工作区窗口全部功能都可用。

### Claude Code：不改源代码，仍能写和编辑计划

**官方事实。** Plan 指导代理研究并写计划，不编辑源代码；计划展示后可以继续修改，或批准后转到执行权限模式。`Ctrl+G` 可在外部编辑器直接编辑计划；也允许退出 Plan 而不批准计划。其通用会话文档提供 `--continue`、`--resume` 恢复历史对话。[Plan mode 与转执行](https://code.claude.com/docs/en/permission-modes#analyze-before-you-edit-with-plan-mode)、[恢复会话](https://code.claude.com/docs/en/common-workflows#resume-previous-conversations)。

**证据边界。** 恢复对话不等于承诺自动挑选最新计划文件。本轮设置参考页打开遇到页面体积限制，因此不采用搜索摘要里的 `plansDirectory` 默认路径作为已核实依据。批准页面也没有解释执行阶段是否再次读取磁盘、是否锁定某个文件版本。这里不能声称 Claude 的具体路径与版本语义已全部查明。

### Gemini CLI：明确的计划文件写入例外

**官方事实。** Plan 允许在计划目录写入和修改 `.md`，其他调查工具按只读范围使用。默认目录为 `~/.gemini/tmp/<project>/<session-id>/plans/`，自定义目录须在项目范围内。用户用 `Ctrl+X` 编辑、保存后，CLI 检测修改并调整计划；批准后转执行。默认会话及计划保留 30 天，自定义计划目录不自动删除。[工具范围](https://geminicli.com/docs/cli/plan-mode/#tool-restrictions)、[协作编辑](https://geminicli.com/docs/cli/plan-mode/#collaborative-plan-editing)、[目录](https://geminicli.com/docs/cli/plan-mode/#custom-plan-directory-and-policies)、[清理](https://geminicli.com/docs/cli/plan-mode/#cleanup)。

**官方事实补充。** `exit_plan_mode` 接收计划 Markdown 路径，展示计划并处理批准或修改意见。[Planning tools](https://geminicli.com/docs/tools/planning/)。工具页目录示例少了会话层，主指南更具体；这也是不把文档示例当永久存储协议的原因。

**证据边界。** 文档的外部编辑检测不能推出任意并发编辑都无冲突，也没有解释批准后又改文件时执行读取哪个版本。目录中的 `<project>` 不能直接证明无项目聊天的存储行为。Gemini 的策略讨论加正式批准、hooks 等是其产品选择，本项目不必复制。

### Codex：确认规划入口，不推断自动文件保存

**官方事实。** CLI 的 `/plan` 进入规划模式，可同时提供规划请求；`/copy` 可以复制最近完成的回复或计划。桌面命令文档同样提供 `/plan`，并将先规划、再设置持续目标作为一种工作流。[CLI 命令](https://learn.chatgpt.com/docs/developer-commands?surface=cli)、[桌面命令](https://learn.chatgpt.com/docs/reference/slash-commands)。

**证据边界。** 本次打开的这两页没有承诺计划自动保存为 Markdown，也没有定义计划文件目录。因此这里只确认规划与执行的区分，不把“自动落盘”归为 Codex 已核实能力。

## 能借鉴什么，不能推成统一规范

**研究推断。** 这三个例子说明“Plan 不实施产品改动”与“允许保存自身计划文档”可以同时成立。计划也可以是独立可编辑材料，而不只是聊天气泡里的某段正文。但保存目录、清理期限、恢复方式和转执行交互各不相同，不能合并成行业统一做法。

**对 Kivio 的建议。** 用一份明确关联当前任务的 Markdown 承载当前计划，聊天用于讨论；修改计划时同步这份文档，开始执行时读取用户当前看到的内容。这里“读取当前内容”是拟议的产品保证，不是三家文档都证明的行为。不要只从旧消息里重新猜哪一段像计划。

没有项目时仍能保存计划，可考虑应用管理的任务目录；有项目时提供保存到项目的选择。具体路径应由 Kivio 自身定义并显示，而不是仿造外部产品目录。文件保存成功、读取实际正文等属于文档功能本身，不需要扩展成 Todo、评分、验收或多轮批准流程。

## 本轮未核实的事项

- 三家当前安装版本的真实交互及重启恢复，没有执行桌面复现。
- 计划未保存编辑、外部并发修改与点击执行同时发生时的快照语义。
- 完全无工作目录的会话支持情况，以及工作区搬迁后的文件关联。
- Claude 精确计划目录设置：搜索可见相关条目，但本轮原始参考页未成功打开，因此不作为结论。

以上空白应由 Kivio 明确定义，不能用推测的上游行为补齐。下面是本项目源码对照和拟议设计。


## 本项目目前怎么运行

源码检查基于 `e3b90f20` 及当前工作区；本节描述现状，不代表已经实施重构。仅讨论内置 Kivio Agent，外部 Claude/Codex CLI 的 Plan 交互不合并改写。

| 环节 | 当前实现与问题 | 源码依据 |
| --- | --- | --- |
| 生成 | 长提示要求逐项调查、特定情形必须联网，并固定先 Findings 再 Plan。它规定回答形状，没有定义计划文档。 | [plan.rs](../../src-tauri/src/chat/plan.rs)，`format_prompt` |
| 识别 | 从整段最终回复判断是否含至少两个步骤，或计划关键词加一个步骤；前后端各实现一遍。普通调查列表可能被识别为计划，叙述型计划又可能被漏掉。后两项是据规则作出的推断。 | [plan.rs](../../src-tauri/src/chat/plan.rs)，`is_executable_plan_text`；[agentPlan.ts](../../src/chat/agentPlan.ts) |
| 保存 | 生成结束且仍处于 Plan 时，复制整段回复到 `AgentPlanState.plan`。结构只有模式、状态、正文、更新时间，没有文档身份和路径；消息与会话均保存计划状态。 | [types.rs](../../src-tauri/src/chat/types.rs)，`AgentPlanState`；[messages.rs](../../src-tauri/src/chat/commands/messages.rs)，`capture_agent_plan_draft_if_needed` |
| 工具 | 允许只读工具、提问、Todo 与技能入口；未提供专用计划保存工具，通用文件写入不在只读集合中。Plan 也没有加载子代理工具。 | [tooling.rs](../../src-tauri/src/chat/commands/tooling.rs)，`agent_plan_allows_tool`；[reply.rs](../../src-tauri/src/chat/commands/reply.rs)，工具组装 |
| 界面 | 正文下只有计划草案及执行按钮，没有关联文件的打开、编辑入口。已有文件卡和侧栏 Markdown 预览可复用，但后者只是预览，不是现成文档编辑器。 | [MessageBubble.tsx](../../src/chat/MessageBubble.tsx)，`AgentPlanAction`；[GeneratedFileArtifacts.tsx](../../src/chat/GeneratedFileArtifacts.tsx)；[FileTreePanel.tsx](../../src/chat/dock/FileTreePanel.tsx) |
| 执行 | 后端先选取消息计划并标为 Approved、切到 Act；前端随后另外发送“按这条计划开始执行”。界面把 Approved 显示为“已按这条计划执行”，早于真正开始执行。 | [interaction.rs](../../src-tauri/src/chat/commands/interaction.rs)，`chat_execute_agent_plan`、`approve_agent_plan_for_execution`；[Chat.tsx](../../src/chat/Chat.tsx)，`handleExecuteAgentPlan` |
| 上下文 | 保存的计划正文注入模式提示；如果引入文件，却继续使用旧字符串，用户编辑文件后模型仍可能收到旧计划。实际请求与上下文统计还有两处工具组装入口。 | [plan.rs](../../src-tauri/src/chat/plan.rs)，`format_prompt`；[prepare.rs](https://github.com/ZMGID/kivio/blob/d7a305629814cf9f258ec9453f177590bfe7db2c/src-tauri/src/chat/agent/prepare.rs)；[context.rs](../../src-tauri/src/chat/commands/context.rs) |

已有的会话 JSON 持久化能保留旧计划，但不等于生成了一份用户可直接管理的计划文档。仓库中虽有 `docs/superpowers/plans/` 的历史文件，也不能据此认定内置 Plan 已实现自动文档保存。

## 建议的使用流程

1. 用户进入 Plan，说明要做什么。主代理读取相关代码；需要外部事实时联网，需要用户决定时提问，不机械要求每次全面调查。
2. 主代理形成方案，调用计划保存工具，直接生成 Markdown 文件。回复只说主要决定，并给出文件卡，例如 `登录流程重构.md · 打开 · 执行当前版本`。
3. 用户可以在聊天里说“把第二步改成……”，主代理更新同一份文档；也可以打开文件自行修改。第一版复用侧栏预览，编辑可先交给系统编辑器，不必先造富文本编辑器。
4. 用户点击“执行当前版本”或明确说开始执行，读取该文件当时的内容，一次性交给执行任务。默认转 Act；用户明确要求 Orchestrate 时使用现有协作模式，不再重复确认一次。
5. 重启后仍可打开这份计划，继续修改或执行。单纯切换到 Act 不自动执行旧计划；切回 Plan 也不自动覆盖文件。

Plan 负责把事情想清楚并留下可用材料，Act/Orchestrate 负责执行。主代理拿到计划后仍然判断现场情况，计划文档不充当强制调度表。

## 文件怎么保存：建议默认值

以下是 Kivio 的拟议产品约定，不是外部产品的统一路径。

| 场景 | 建议位置与处理 |
| --- | --- |
| 有绑定目录的项目 | 默认 `<项目>/docs/plans/<日期>-<主题>-<短ID>.md`，便于用户发现、编辑和自行提交 Git。用户明确指定位置时遵循其选择。 |
| 普通会话 | 使用已有独立会话工作目录下的 `plans/`，不把不同会话的计划混在一起。 |
| 项目尚未绑定目录 | 暂存到应用管理的 `<app_data>/plans/<会话ID>/`，文件卡明确显示位置；无需为了写计划先配置项目。以后保存到项目时更新关联。 |

同一计划继续修改同一个文件，新主题使用新 ID，避免所有会话争用根目录 `PLAN.md`。计划不因为超时或某次执行结束被清除；用户保存到项目或导出的文档也不应随会话删除。应用管理的暂存文件可以跟随会话删除规则，界面必须能分清两种位置。

现有 [storage.rs](../../src-tauri/src/chat/storage.rs) 的 `resolve_conversation_working_directory` 已区分项目目录与普通会话工作目录；未绑定目录的项目会报错，因此第三种情况需要明确补充，不能假设现有解析器会自动回退。

## 最小实现边界

### 明确保存计划，不再猜回答格式

建议提供一个小工具 `save_plan(title, content, plan_id?)`：没有 ID 时新建，有 ID 时更新。路径由应用按上述规则分配，返回真实文件路径和计划 ID。它只负责当前计划文档，不打开对任意业务文件的写权限。用户明确指定其他保存位置时，由应用记录为该计划目标，而不是让模型每次任意选择路径。

保存动作本身表明“这是计划”，不再用标题、步骤数量或关键词识别新计划。普通聊天、澄清问题和调查进展仍可正常输出，不必强行生成文档。写入成功才显示已保存；磁盘问题直接说明并保留已有内容，不伪造文件链接，也不把它升级为对回答质量的判决。

### 文件是当前正文，消息保留当时记录

当前计划只需要稳定 ID、标题、路径、更新时间等关联信息；沿用消息正文保存当时讨论即可。文件更新后，不再把会话 JSON 中旧的 `plan` 字符串作为另一个可写副本。预览重新读文件，模型修改前读当前文件，避免覆盖用户刚改的内容。

开始执行时读取一次实际文档，将这份正文与计划 ID、路径一起留在该次执行请求里。之后用户再编辑文件，不偷偷改变已经发出的执行要求；要让正在执行的代理采用调整，通过消息明确通知它。这是执行内容可追溯的最小做法，不需要另建版本审批系统。文件已丢失时直接提示重新定位或恢复，不能静默改用旧正文。

### 保存、展示、执行用同一个文档关联

现有 `AgentPlanAction` 改为文档入口；`Approved` 不再被显示为“已执行”。可以直接显示“计划已保存”，执行是否开始来自实际任务事件，不新增计划成败状态。

计划选择与执行请求接入现有发送通道，避免目前“先标记已执行，再由前端另发一句话”的分离操作。生成正文可按任务需要写目标、方案、关键修改位置和必要验证办法，不要求固定章节、Todo 勾选或机器打分。

### 旧数据与现有模式

旧会话保持可读。第一次继续修改或执行某份旧计划时，把用户选中的旧正文转为文档并绑定，之后走新路径；不启动时批量导出全部历史，也不覆盖现有文档。保留持久化 `plan` 模式值，逐步淘汰新流程中的格式识别与 Approved 语义。

第一版沿用 Plan 现有的只读调查能力。若以后开放子代理调查，必须让子代理继承 Plan 的只读范围；不能仅移除 `!plan_mode` 条件就放出全能力子代理。这属于后续能力扩展，不是保存计划文档的前置条件。

## 建议替换的精简模式提示

> 先了解相关代码和约束，需要时查资料或询问关键选择。形成可执行方案后保存为计划文档，回复简述主要决定并给出文档入口。用户调整方案时更新同一文档。Plan 阶段只调查和修改计划文档；用户要求开始执行后，按选定计划转入执行。

工具参数、保存位置、读取方式由工具说明提供，不再全部塞进模式提示。文档里的步骤和测试说明帮助理解方案，不承担回答放行功能。

## 实施顺序与观察项

先做文档保存、关联和打开，再接上当前版本的执行交接，最后迁移旧入口并移除重复识别。后端范围主要是 `plan.rs`、`types.rs`、`messages.rs`、`interaction.rs`、`tooling.rs` 与请求/上下文组装；前端范围主要是 `agentPlan.ts`、`MessageBubble.tsx`、`Chat.tsx` 和现有文档预览入口。相关协议字段与 TypeScript 类型同步调整。

实施后需要观察：中文文件名和空格路径；修改后执行拿到新正文；重启还能打开；两份计划不互相覆盖；旧计划可继续用；文件保存问题不显示假成功；点击执行不会重复发送；Plan 保存文档时仍不能修改业务代码。它们是文档和执行功能应有的行为检查，不是模型输出验收流程。
