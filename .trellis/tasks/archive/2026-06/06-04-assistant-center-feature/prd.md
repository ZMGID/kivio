# Kivio 助手套件中心 PRD

## Goal

把当前 Chat 侧边栏里的“助手中心”升级为 Kivio 的“助手套件中心”。它不只是管理单个助手，而是管理可复用的专家能力包：一个套件可以包含基础助手设定、快捷命令、数据连接和知识技能。用户可以从套件广场安装/启用内置套件，也可以创建自己的套件，并在聊天中快速启用对应能力。

第一版目标是做成本地、轻量、可复用的桌面能力中心，不做云端市场或团队平台。它要复用 Kivio 已有的 Chat、Skill Runtime、MCP/内置工具、默认模型、上下文管理和助手快照能力，让用户能把“提示词 + 模型 + 工具连接 + 技能 + 快捷入口”保存下来，避免每次对话手动组合。

## Template Analysis

用户提供的参考模板体现了三层结构：

* 套件广场层：展示可安装的专家套件，支持搜索、刷新、创建、筛选、已安装数量和套件卡片。每张卡片展示名称、作者/来源、简介、技能数量、数据连接数量、版本和添加入口。
* 套件详情层：进入某个套件后，顶部展示返回、套件名称、版本、来源、编辑、分享/复制、聊天中启用开关。主体分为快捷命令、数据连接、知识技能。
* 助手编辑层：第一张图里的“助手”更像套件中的可启动助手设定，包含名称、描述、系统提示词、开场白、开场问题、保存、应用到当前对话、开始聊天等操作。

模板中的三类核心能力：

* 快捷命令：如 `/做凭证`、`/内部审计`。本质是预设任务入口，通常包含命令名、描述、示例输入、触发词和追加 prompt。点击后可以启动聊天或填入输入框。
* 数据连接：如 Notion。对应外部数据源、MCP server、内置工具、文件/网页/记忆等可调用能力。它不是助手本身，而是套件允许运行时使用的连接能力。
* 知识技能：如“做凭证”“内部审计”。对应专业知识模块和可复用流程，和 Kivio 现有 Skill Runtime 高度接近，但在套件里需要有启用状态、说明、触发词和依赖连接。

## Product Principles

* 套件是能力包，助手是套件内的默认对话人格/运行配置。
* Skill 是套件可绑定的专业能力，不等同于助手。
* 数据连接控制工具可见性和运行权限，不能只停留在 UI 标签。
* 快捷命令要能真正影响运行时 prompt 或输入内容，不能只是展示文案。
* 历史对话必须使用创建时的套件/助手快照，避免后续编辑导致语义漂移。
* 套件中心只使用 Chat 外层侧边栏作为全局导航，主内容区不再做独立二级侧边栏。
* 第一版优先本地可用、架构清晰、迁移安全，暂不追求云端广场和复杂工作流。

## Current Repo Context

Kivio 已经具备这些基础：

* `src/chat/AssistantCenter.tsx`：已有助手中心雏形，支持助手列表、创建、编辑、复制、删除、启动聊天。
* `src/chat/Chat.tsx`：Chat 主视图已有 `conversation` / `settings` / `assistants` 三种视图。
* `src/chat/types.ts`：已有 `ChatAssistant`、`ChatAssistantSnapshot`、`Conversation.assistant_id`、`Conversation.assistant_snapshot`、`active_skill_id`。
* `src-tauri/src/chat/types.rs`：后端已有对应的 `ChatAssistant`、`ChatAssistantSnapshot`、`ChatAssistantIndex`。
* `src-tauri/src/chat/commands.rs`：已有 `chat_get_assistants`、`chat_create_assistant`、`chat_update_assistant`、`chat_duplicate_assistant`、`chat_delete_assistant`，创建对话时已经支持助手快照和模型/Skill 覆盖。
* `src-tauri/src/chat/agent/prepare.rs`：已有 assistant prompt segment、Skill 目录、工具 preset 过滤和运行时 system 拼装逻辑。
* `src-tauri/src/skills/*`：已有 Skill 扫描、读取、激活、脚本运行能力。
* `src-tauri/src/mcp/*` 和 `settings.chat_tools`：已有 MCP、内置工具、Skill runtime、工具确认和工具轮次设置。

## Research References

* [`research/assistant-center-patterns.md`](research/assistant-center-patterns.md) - comparable products treat custom assistants as reusable profiles combining identity, instructions, starters, model/runtime choices, tools/capabilities, optional knowledge/context, and lifecycle actions.

## Research Notes

* OpenAI GPTs expose name, description, conversation starters, instructions, knowledge, recommended model, capabilities/actions, preview/testing, version history, duplicate/delete/share.
* Claude Projects separate project chat history, knowledge base, and project instructions; Claude Skills are focused repeatable workflows with metadata, instructions, resources, scripts, and tests.
* Gemini Gems / Managed Agents validate the "save prompt + tools + files/skills as an invokable assistant ID" model.
* Dify Agents emphasize persona, output format, constraints, workflow steps, tool-use guidance, knowledge base, preview/debug, and publishing.
* Poe prompt bots include bot identity, base model, prompt, optional knowledge base, greeting, markdown/temperature, and create/start flow.

## Core Concepts

### Assistant Suite

`AssistantSuite` 是新的产品层概念。第一版可以在代码里复用/扩展 `ChatAssistant` 存储，也可以引入新类型；产品上统一称为“助手套件”。

字段建议：

* `id`
* `name`
* `description`
* `icon`
* `color`
* `source`: `builtin | user | imported`
* `author`
* `version`
* `category`
* `tags`
* `system_prompt`
* `greeting`
* `conversation_starters`
* `provider_id`
* `model`
* `tool_preset`
* `enabled`
* `installed`
* `archived`
* `built_in`
* `quick_commands`
* `data_connectors`
* `knowledge_skills`
* `created_at`
* `updated_at`

### Quick Command

快捷命令是套件内的任务入口。

字段建议：

* `id`
* `name`
* `slash`
* `description`
* `placeholder`
* `prompt`
* `starter_text`
* `requires_suite_enabled`
* `enabled`

运行时行为：

* 从套件详情点击快捷命令：若没有当前对话，则用该套件创建新对话；若已有当前对话，可填入输入框或直接发送。
* 用户在输入框输入 `/命令`：若当前对话启用了该套件，则匹配命令并把命令 prompt 作为运行时附加指令。
* 快捷命令必须进入 `assistant_snapshot` 或对话运行上下文，保证历史可复现。

### Data Connector

数据连接是套件声明需要的工具/数据源。

字段建议：

* `id`
* `name`
* `kind`: `builtin_tool | mcp | skill_tool | memory | file | web | future`
* `description`
* `tool_ids`
* `server_id`
* `required`
* `enabled`
* `configured`

运行时行为：

* 套件启用后，只允许其数据连接声明的工具进入该套件上下文，除非用户选择“继承全部聊天工具”。
* 未配置的连接在详情页显示为未连接，运行时不注入对应工具。
* 第一版只做本地声明和已有工具/MCP/Skill runtime 映射，不做第三方 OAuth。

### Knowledge Skill

知识技能是套件内的专业能力模块，可以绑定 Kivio Skill，也可以是轻量内置说明。

字段建议：

* `id`
* `name`
* `description`
* `trigger_phrases`
* `skill_id`
* `prompt`
* `recommended_tools`
* `requires_connectors`
* `enabled`

运行时行为：

* 如果绑定 `skill_id`，运行时可优先激活对应 Kivio Skill。
* 如果没有绑定 `skill_id`，则作为套件内的专业说明注入 system prompt。
* 用户提问命中触发词时，自动加强对应技能说明；用户也可以在详情页手动选择技能启动聊天。

## MVP Scope

第一版做“本地助手套件中心”，包含：

* 侧边栏入口从“助手中心”进入套件中心。
* 套件中心主内容区直接排布顶部操作、说明横幅、tabs、筛选和套件卡片，不再使用左侧助手列表/右侧编辑器的二级侧栏结构。
* 套件中心包含“套件广场”和“已安装/我的套件”视图。
* 套件广场只展示本地内置套件模板，不接远程 marketplace。
* 用户可以安装/启用内置套件、创建自定义套件、编辑、复制、归档/删除。
* 套件详情展示快捷命令、数据连接、知识技能三类能力。
* 用户可以从套件启动新聊天，或应用套件到当前对话。
* 创建对话时保存套件/助手快照。
* 发送消息时按套件配置注入 system prompt、快捷命令 prompt、Skill、工具策略。
* 保留现有 `ChatAssistant` 能力并迁移为默认套件，避免破坏已有用户数据。

## Out Of Scope

* 远程公开市场、云端套件下载、发布审核和排行榜。
* 团队协作、组织权限、共享安装。
* 套件版本历史、回滚和多人编辑。
* 第三方 OAuth 连接器。
* 独立知识库索引/RAG。
* 多 Agent 图形工作流编排。
* 复杂行业套件的大规模内容生产。第一版只做少量高质量内置模板。

## User Stories

* 作为普通用户，我可以打开助手套件中心，看到 Kivio 内置的通用、翻译、截图分析、编程/数据、写作套件。
* 作为普通用户，我可以启用一个套件，然后在聊天中使用它的开场问题或快捷命令。
* 作为进阶用户，我可以创建自己的套件，填写系统提示词，选择模型、工具策略、Skill 和快捷命令。
* 作为经常做重复任务的用户，我可以把一个常用任务保存为 `/快捷命令`，下次直接调用。
* 作为历史对话用户，我希望编辑套件后，旧对话仍然按当时的配置工作，而不是被静默改变。
* 作为谨慎用户，我希望套件里的数据连接明确显示是否启用、是否已配置、会开放哪些工具。

## UX Requirements

### Host Navigation And Page Layout

* Chat 外层侧边栏仍然作为全局导航入口，`扩展` 下的当前项进入“专家套件/助手套件”。
* 套件中心内容区不再出现独立的左侧列表栏，也不做“左列表 + 右编辑器”的 split view。
* 进入套件中心后，主区域直接按照模板铺开：页面标题、说明文案、顶部操作区、横幅、tabs、筛选、卡片网格。
* 技能和连接器不作为套件中心内的二级侧边栏项；它们在套件详情页作为内容分区出现。
* 列表、详情、编辑都是同一主内容区里的页面状态：从卡片进入详情，从详情进入编辑，通过顶部返回回到列表。

### Suite Plaza

参考模板第二张图：

* 顶部：标题“专家套件”或“助手套件”、刷新、搜索框、创建按钮。
* 顶部操作直接排在主内容区右上角，不放进独立侧栏。
* 标题区可以包含一行说明文案，例如“专家套件是面向角色/行业的工具套件，在对话框中输入 @ 或 / 即可使用。”
* 可选展示一张浅色说明横幅，用来表达“本地内置套件/让 Kivio 帮我创建”这类入口；横幅不能挤占卡片网格的主要空间。
* 信息条：用于反馈“想要更多套件？”或本地模板说明。
* Tabs：`套件广场`、`已安装`、可选 `我的`。
* 筛选：全部、内置、用户创建、已启用、未配置。
* 卡片：名称、图标、来源、简介、快捷命令数、知识技能数、数据连接数、版本、添加/启用按钮。
* 卡片布局保持紧凑，避免营销落地页风格。

### Suite Detail

参考模板第三、四张图：

* 顶部：返回列表、套件图标、名称、版本、来源、编辑、复制/分享占位、聊天中启用开关。
* 简介：显示套件用途、适用场景、触发词摘要。
* 快捷命令区：列表行展示 `/命令`、示例输入、箭头/启动按钮。未启用套件时禁用并提示“请先启用此套件以使用快捷命令”。
* 数据连接区：展示连接名称、状态、启用开关。第一版只支持已有工具映射和配置状态提示。
* 知识技能区：展示技能名称、描述、触发词、绑定 Skill 状态。

### Suite Editor

编辑页保留现有助手编辑器的直接性，但必须是主内容区全宽页面，不再使用左侧助手列表 + 右侧表单：

* 基础信息：名称、描述、图标、颜色、分类、标签。
* 助手设定：系统提示词、开场白、开场问题。
* 运行设置：模型供应商、模型、工具策略、默认 Skill。
* 快捷命令：新增/编辑/删除命令，设置 slash、描述、示例、prompt。
* 数据连接：选择已有内置工具、MCP server、Skill runtime、记忆、网页/文件能力。
* 知识技能：绑定已有 Kivio Skill 或创建轻量知识技能说明。
* 底部操作：保存、应用到当前对话、开始聊天。

## Functional Requirements

### Data And Persistence

* 系统必须能读取旧版 `ChatAssistant` 数据，并迁移/适配为套件。
* 新建套件必须持久化基础信息、系统提示词、模型覆盖、工具策略、快捷命令、数据连接、知识技能。
* 删除套件应优先归档，不应破坏历史对话。
* 内置套件应可恢复；用户编辑内置套件时，推荐通过“复制为我的套件”完成，避免直接改模板。
* 创建对话时保存 `assistant_id` / `suite_id` 和完整快照。
* 快照至少包含：基础助手设定、模型覆盖、工具策略、默认 Skill、快捷命令、数据连接、知识技能。

### Runtime

* 创建聊天时，如果套件指定 provider/model，则覆盖默认聊天模型；否则继承全局默认模型。
* 发送消息时，system prompt 拼装顺序为：
  1. 全局 Chat system prompt
  2. 套件基础 system prompt
  3. 当前快捷命令 prompt
  4. 命中的知识技能说明或绑定 Skill
  5. 数据连接/工具权限说明
  6. 现有 Skill/MCP/内置工具目录
  7. 记忆和上下文模块
* 套件工具策略必须影响实际可调用工具列表。
* 如果套件绑定 Skill，`active_skill_id` 应随对话创建/应用同步。
* 如果快捷命令命中绑定 Skill 或知识技能，运行时应优先使用相关 Skill/说明。
* 未启用或未配置的数据连接不得出现在当前套件可用工具中，除非工具策略显式继承全部工具。

### Commands

* 支持从详情页点击快捷命令启动聊天或填入输入框。
* 支持在当前套件对话中输入 `/命令` 触发命令。
* 命令触发后，用户可看到输入内容仍然自然，不应暴露内部 prompt 拼接细节。
* 命令名称冲突时，以当前套件内命令优先；跨套件命令不自动全局注册。

### Built-in Suites

第一版内置 5 个套件：

* 通用助手：日常问答、梳理想法、轻任务处理。
* 翻译润色助手：翻译、改写、语气调整、双语表达。
* 截图分析助手：截图、界面、报错、视觉信息分析。
* 编程/数据助手：代码解释、调试、脚本、数据分析。
* 写作助手：文章、文案、提纲、总结、表达优化。

每个内置套件至少包含：

* 1 个基础 system prompt。
* 1 个开场白。
* 3 个开场问题。
* 2 到 4 个快捷命令。
* 0 到 3 个推荐知识技能或绑定 Skill。
* 合理的工具策略。

## Non-Functional Requirements

* UI 必须保持桌面工具气质：紧凑、可扫描、轻量，不做大型营销页。
* 页面必须支持浅色和深色主题。
* 小窗口下文本不能溢出按钮或卡片。
* 套件中心不能明显拖慢 Chat 首屏加载，重数据应懒加载。
* 数据迁移必须向后兼容旧对话和旧助手。
* 所有新增前后端类型必须保持 snake_case 存储、前端兼容 camelCase 的现有风格。
* 修改跨前后端类型时，需要同步更新 `src/api/tauri.ts`、`src/chat/types.ts`、Rust 类型和命令返回结构。

## Implementation Plan

### Phase 1 - Data Model And Compatibility

* 扩展 `ChatAssistant` 或新增 `AssistantSuite` 类型。
* 增加 `quick_commands`、`data_connectors`、`knowledge_skills`、`source`、`version`、`installed/enabled` 等字段。
* 扩展 `ChatAssistantSnapshot`，冻结套件运行所需字段。
* 更新 storage normalize/migration，确保旧助手自动拥有空数组字段和默认版本/来源。
* 更新 TypeScript API 类型。

### Phase 2 - Runtime Integration

* 扩展创建对话逻辑，支持套件快照和默认 Skill/模型覆盖。
* 扩展 `agent_prepare`，按套件层级拼装 prompt。
* 扩展工具过滤逻辑，让数据连接和工具策略影响实际工具列表。
* 增加快捷命令解析和运行时附加 prompt。
* 增加后端单元测试覆盖快照、迁移、工具过滤和 prompt 拼装。

### Phase 3 - Suite Center UI

* 重构 `AssistantCenter.tsx` 为套件中心布局。
* 移除套件中心内部独立侧栏；主内容区直接展示广场、详情、编辑三种页面状态。
* 实现套件广场/已安装/我的套件 tabs。
* 实现卡片列表、搜索、筛选、刷新、创建。
* 实现套件详情页三段结构：快捷命令、数据连接、知识技能。
* 实现编辑页分区和基础 CRUD。
* 接入从套件开始聊天、应用到当前对话。

### Phase 4 - Built-in Suites And Polish

* 编写 5 个内置套件模板。
* 增加空状态、未配置连接状态、无模型/provider 状态。
* 做浅色/深色主题适配。
* 做窗口尺寸和文本溢出检查。
* 跑 lint、typecheck、Rust tests。

## Acceptance Criteria

* [ ] 侧边栏入口显示“助手套件”或“助手中心”，并能打开套件中心。
* [ ] 套件中心内容区没有独立二级侧边栏；列表、详情、编辑都在主内容区直接排布。
* [ ] 套件中心有套件广场和已安装/我的套件视图。
* [ ] 套件卡片展示名称、描述、来源、版本、快捷命令数、知识技能数、数据连接数。
* [ ] 用户能安装/启用内置套件。
* [ ] 用户能创建、编辑、复制、归档/删除自定义套件。
* [ ] 套件详情页展示快捷命令、数据连接、知识技能三类内容。
* [ ] 用户能从套件启动新聊天。
* [ ] 用户能把套件应用到当前对话。
* [ ] 快捷命令能启动聊天或影响发送时的运行 prompt。
* [ ] 套件绑定的 Skill 能在对话创建/运行时生效。
* [ ] 套件数据连接/工具策略能影响实际工具列表。
* [ ] 使用套件创建的对话保存完整快照。
* [ ] 编辑套件后，旧对话不会静默改变行为。
* [ ] 旧版助手数据能正常读取并显示为套件。
* [ ] 无 provider/model、未配置连接、无套件等状态有清晰提示。
* [ ] `npm run lint` 通过。
* [ ] `npm run typecheck` 通过。
* [ ] `cargo test --manifest-path src-tauri/Cargo.toml` 通过。

## Open Questions

* 第一版名称最终用“助手中心”还是“助手套件”？推荐 UI 主标题用“助手套件”，侧边栏可用“助手中心”降低理解成本。
* 快捷命令点击后默认是“填入输入框等待用户确认”，还是“直接发送”？推荐默认填入输入框，详情页可提供直接开始按钮。
* 自定义知识技能第一版是否允许用户手写，还是只能绑定现有 Kivio Skill？推荐两者都支持：轻量手写说明 + 可选绑定 Skill。
* 数据连接第一版是否允许用户在套件内直接配置 MCP server？推荐先只选择已有连接，新增/配置仍跳转到设置页。
* 内置套件模板是否允许用户直接编辑？推荐不直接编辑，使用“复制为我的套件”。

## Definition Of Done

* PRD 与实现保持一致。
* 代码完成后通过 lint、typecheck、相关 Rust tests。
* UI 在常见窗口尺寸下无明显溢出、重叠或不可点击状态。
* 历史旧助手、旧对话可继续读取。
* 变更涉及的新类型、运行时规则和迁移策略被记录到相关 Trellis spec 或任务 notes 中。
