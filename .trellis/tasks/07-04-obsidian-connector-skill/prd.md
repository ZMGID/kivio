# Obsidian 连接器内置 skill

## Goal

用户配置 Obsidian 连接器（在设置里选定本机 vault 路径）后，让 chat agent 能"更好地操作 Obsidian"——理解 Obsidian 的文件格式与语法（wikilink / embed / callout / properties / tags / Bases / Canvas 等）。为此把 kepano/obsidian-skills 仓库的技能改编为 Kivio 内置 skill 打包进 app，并像 himalaya 邮件连接器门控 himalaya skill 那样：**当且仅当 Obsidian 连接器已配置（`obsidian_vault_path` 非空）时**，这些 Obsidian skill 才对模型可用。

**范围决定**：vendor 4 个 skill —— `obsidian-markdown`、`obsidian-bases`、`json-canvas`、`obsidian-cli`。**不打包 `defuddle`**（它是 WebFetch 替代、依赖外部 CLI，与 Kivio 已有 `web_fetch` 原生工具重叠，且与 vault 无关）。

## Background / 现状（已核实）

- Obsidian 连接器当前是 `vault` 类型：`src-tauri/src/connectors/obsidian.rs` 列出本机 vault，路径存入 `settings.obsidian_vault_path`；`prepare.rs:386` 把该路径（本轮已补充基本操作指引）注入系统提示。除此之外没有任何 Obsidian 专门能力。
- Kivio 有完整 Skills 子系统：内置 skill 放在 `src-tauri/resources/skills/<id>/SKILL.md`，经 tauri.conf `resources/skills → skills` 打包（`src-tauri/tauri.conf.json:42`），由 `skills/discover.rs` 从 bundled + user + external 根发现。`references/` 目录下文件被 `index_skill_files` 索引为 `Reference`，可被模型用 `skill_read_file` 读取（渐进披露）。
- SKILL.md 解析（`skills/parse.rs`）：必须有非空 `name`、`description`；`id` 取 `id:` 字段或 `slugify(name)`；文件夹名与 id/name 不一致会产生一条 warning（非致命）。
- **连接器门控先例**：`settings.rs:1327` `EMAIL_CONNECTOR_SKILL_ID="himalaya"`；`skill_connector_satisfied(skill_id, email_accounts)`（1334）当 `skill_id==himalaya` 时要求 `!email_accounts.is_empty()`；`skill_globally_available`（1343）= 启用列表 && 连接器满足；`skill_global_unavailable_error`（1352）给出人类可读原因。调用点共 5 处：
  - `skills/mod.rs:48`（`chat_skills_list` 过滤）
  - `skills/mod.rs:81`（`chat_skills_read` → `skill_global_unavailable_error`）
  - `chat/agent/prepare.rs:117`（`skill_globally_available`，控制注入模型的 skill 列表）
  - `mcp/registry.rs:553`（skill 激活工具的门控）
  - `kivio_code/executor.rs:269`（headless CLI 门控）
- kepano/obsidian-skills（MIT，© 2026 Steph Ango @kepano）文件树：
  - `obsidian-markdown/SKILL.md` + `references/{CALLOUTS,EMBEDS,PROPERTIES}.md`
  - `obsidian-bases/SKILL.md` + `references/FUNCTIONS_REFERENCE.md`
  - `json-canvas/SKILL.md` + `references/EXAMPLES.md`
  - `obsidian-cli/SKILL.md`（依赖外部 `obsidian` CLI，且需 Obsidian 正在运行）
  - `defuddle/SKILL.md`（依赖外部 `defuddle` npm CLI；是 WebFetch 的替代，与 Kivio 已有 `web_fetch` 原生工具功能重叠）

## Requirements

- R1 把 kepano/obsidian-skills 的 4 个 skill（`obsidian-markdown`、`obsidian-bases`、`json-canvas`、`obsidian-cli`）改编为 Kivio 内置 skill，vendor 到 `src-tauri/resources/skills/` 下各自文件夹（含各自 `references/`），随 app 打包。不含 `defuddle`。
- R2 保留 MIT 许可要求：在 vendored 内容中保留版权声明与许可证（例如 skills 根或各 skill 文件夹放置 `LICENSE` / 归属说明）。
- R3 新增 Obsidian 连接器门控：定义 Obsidian 连接器所属 skill id 集合，扩展 `skill_connector_satisfied` / `skill_globally_available` / `skill_global_unavailable_error`，使这些 skill 仅在 `obsidian_vault_path` 非空时可用；未配置时在列表隐藏、激活/读取时给出"需要配置 Obsidian 连接器"的清晰错误。
- R4 5 处门控调用点全部传入 Obsidian vault 已配置状态；行为与 himalaya 一致。
- R5 更新单元测试：新增"无 vault 时隐藏 Obsidian skill、有 vault 时可用"的用例，并保持既有 himalaya/pdf 用例通过。
- R6 更新 release 打包核对（`docs/RELEASE_PACKAGING.md` 及相关说明），把新增 bundled Obsidian skills 纳入"安装产物内应包含"的检查项。

## Acceptance Criteria

- [x] AC1 未配置 Obsidian 连接器时：`chat_skills_list` 不返回任何 Obsidian skill；模型系统提示中不出现这些 skill；尝试激活/读取返回"需配置 Obsidian 连接器"错误。（门控逻辑单测覆盖：`skill_globally_available_hides_obsidian_without_vault`、`skill_allowed_hides_obsidian_skill_without_vault`、`skill_global_unavailable_error_...`；GUI E2E 待手动 smoke）
- [x] AC2 配置 vault 路径后：Obsidian skill 全部出现在 skill 列表、可被模型激活并读取其 `references/`。（门控单测 `..._with vault==true` 通过；GUI E2E 待手动 smoke）
- [x] AC3 himalaya（邮件）与非连接器 skill（如 pdf）的门控行为不受影响；既有测试全绿。（`--lib skill` 76 项通过，含 himalaya/pdf 用例）
- [x] AC4 `cargo test` 中新增的 Obsidian 门控用例通过（skill 76 / obsidian 4 / slash_trigger 3 / vendored 1 全绿）；无前端改动，lint/typecheck 不涉及。
- [x] AC5 vendored skills 保留 MIT 版权/许可声明（`resources/skills/NOTICE.md`）。
- [x] AC6 `docs/RELEASE_PACKAGING.md` 列入新增 Obsidian skills 的产物核对项。

## Open Questions

（无剩余阻塞问题；defuddle 处理已定为"不打包"。）

## Out of Scope

- 不打包 `defuddle` skill（web 抓取、依赖外部 CLI、与 Kivio 自带 `web_fetch` 重叠）。
- 不为 Obsidian 实现专用 native 工具（继续用现有 read_file/glob/search/list_dir/write/edit）。
- 不把 vault 接入知识库 RAG（那是另一条线）。
- 不打包 `obsidian` 外部 CLI 二进制；`obsidian-cli` skill 在缺少该 CLI 时仅作为能力文档存在。
