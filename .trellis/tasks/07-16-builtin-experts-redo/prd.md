# PRD — 重做内置专家套件

## Goal / 用户价值
把现有 4 个"随便做、占位"的内置聊天专家（写作/编程/研究/数据）重做为一批**专业、可直接上手用**的专家套件（升级 4 个 + 扩充到 6–8 个），并确保**已有用户也能拿到**（现有 seed 只跑一次且会清空用户自建）。

## Background / 已确认事实（代码勘查）
- "专家套件" = 内置聊天助手人设 `builtin_assistant_definitions`（`src-tauri/src/chat/storage.rs:312`），当前 4 个：写作(`asst_builtin_writer`)、编程(`asst_builtin_coder`)、研究(`asst_builtin_researcher`)、数据(`asst_builtin_data`)。
- `ChatAssistant`（`chat/types.rs:553`，TS 镜像 `src/chat/types.ts:255`）行为字段：`system_prompt`（核心）、`skill_ids`（技能白名单）、`mcp_server_ids`、`provider_id`/`model`；其余（name/description/icon/color/source/built_in…）仅展示/分类。行为字段在建会话时冻结进 `ChatAssistantSnapshot`（`types.rs:589`）。
- 内置专家的可视 emoji 实际来自 `builtinAssistantGlyph(id)`（`AssistantCenter.tsx:402`），`icon` 字段对内置基本不显示 → 新增专家需在 `builtinAssistantGlyph` 补 id→glyph 映射。
- **skill_ids 语义**：白名单 + 进"可激活技能目录"给模型看（`prepare.rs:44/507`、`loop_.rs:156` 硬 gate），**非自动激活**；空 = 不给任何技能。
- 可挂内置技能（id）：`diagram`、`doc-coauthoring`、`docx`、`xlsx`、`pdf`、`frontend-design`、`mcp-builder`、`skill-creator`；连接器门控（不宜默认挂）：`himalaya`、`obsidian-*`。
- `provider_id`/`model` 留空 = 跟随用户默认模型（`commands/catalog.rs:224`）→ **保持留空**。
- 语言：定义为中文硬编码，UI 不本地化。本任务**保持中文**；prompt 内声明"默认跟随用户语言"，英文用户可正常用。
- **⚠️ 交付/迁移**：`seed_builtin_assistants_v1`（`storage.rs:394`）在 `lib.rs:243` 启动时按 `settings.builtin_assistants_seeded_v1` 只跑一次，且 `save_assistant_index` **整表覆盖**（会清空用户自建）。已 seed v1 的用户改定义**收不到**。必须新增**非破坏性 v2 迁移**（新 flag + 按 id upsert，保留用户自建）。

## Requirements
- **R1 专家名册**：升级现有 4 个 + 扩充到共 6–8 个，领域互不重叠、覆盖高频场景。每个含：专业化 system_prompt（角色、工作方式、输出规范、边界/诚实性）、正确的 `skill_ids` 编排、`builtinAssistantGlyph` 图标、`color`、精炼 `description`。名册见 design.md。
- **R2 技能编排**：只挂真正相关且非连接器门控的内置技能（如数据分析挂 pdf/docx/xlsx/diagram，前端挂 frontend-design/diagram）。
- **R3 模型/语言**：`provider_id`/`model` 留空（跟随用户默认）；中文定义，prompt 声明跟随用户语言。
- **R4 非破坏性 v2 迁移**：新增 `builtin_assistants_seeded_v2` flag + merge 迁移（按 `asst_builtin_*` id upsert：替换同 id 内置项、缺失则新增，**保留全部用户自建/非内置项**），在 `lib.rs` 启动处按 v2 flag 守卫，成功置 flag、失败回滚重试。新装用户 v1 仍先跑（或 v2 直接覆盖 v1 语义——见 design 决策）。
- **R5 前端展示**：新增专家 id 在 `builtinAssistantGlyph` 有图标；AssistantCenter/Picker 正常显示、可用、可"用它开新会话"。
- **R6 去 AI 味（硬约束，最高优先级）**：所有专家产出必须像"人写的"，不带生成式 AI 的通病。每个 system_prompt 植入一段具体的反 AI 腔规范（见 design"共享文风块"），写作/翻译/文档三个专家尤其严格。禁止：套话空转过渡（"综上所述""在当今…时代""值得注意的是"）、谄媚开场（"当然！""很高兴为你…"）、复述用户问题、无脑加粗/滥用 emoji/动辄分点、堆形容词与正确的废话、过度免责 hedging、生硬翻译腔。要求：直给、具体、有信息量、说人话、该短就短。

## Acceptance Criteria
- [x] AC1（测试+逻辑）：`builtin_assistant_definitions` 返回 7 个专家，id/skill/长度约束经 `builtin_assistant_tests` 断言通过。运行时展示需手测。
- [x] AC2（单测）：`merge_v2_updates_builtins_and_preserves_user_assistants` 断言旧内置被更新、新增内置补齐、用户自建保留。
- [x] AC3（逻辑）：v2 由 `builtin_assistants_seeded_v2` flag 守卫，成功置位、持久化失败回滚重试（镜像 v1，`lib.rs`）。
- [x] AC4（逻辑）：skill_ids 已正确编排；provider/model 留空跟随用户默认（运行时路径未改）。选专家开会话需手测。
- [x] AC5：`cargo test chat::storage`（5 项通过）+ `npm run typecheck` + `lint` + `cargo build --lib` 通过。
- [x] AC6（测试+文风）：每个 prompt 均含 `NO_AI_FLAVOR_STYLE`（测试断言）；prompt 本身按去 AI 腔重写。实际产出质量需人工试用评审。

## 追加需求（试用反馈）
- **R7 常用（收藏夹）模型**：广场/已安装重复 → tab 改为 **常用**(首位·对话栏只列这些) / 套件广场(浏览全部内置) / 我的(自建)。内置默认不在常用（`installed:false`），从广场「添加到常用」后才在对话栏可选/调用；AssistantPicker 只列 `installed!==false`。`installed` flag 复用为「常用」语义。不加全局默认助手（沿用集级）。
- **R8 徽章修复**：列表卡片对内置同时显示绿标「内置」+ 灰字「内置」→ 去掉灰字，仅自定义显示「自定义」。
- **迁移注意**：内置默认 `installed` 由 true 改为 false 走 v2 merge 下发；已在本机跑过 v2（flag=true）的开发机需重置 `builtin_assistants_seeded_v2` 才会重新应用（未发布，一次性）。
## Out of Scope
- 中英双语字段（保持中文）。
- 连接器门控技能（himalaya/obsidian）默认挂载。
- 每专家固定 provider/model（保持跟随用户默认）。
- 全局默认助手（沿用集级默认）。

## Open Questions
- 无阻塞项（名册已按 design.md 提案实现：7 个）。
