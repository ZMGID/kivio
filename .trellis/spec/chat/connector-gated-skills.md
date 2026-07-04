# 连接器门控 Skill 契约

> 某些 bundled skill 只在其“连接器”配置就绪时才对模型可见/可激活。新增这类 skill 时必须一致地穿过全部门控点，否则会“永远可见”或“永远不可用”。

## 门控核心（`src-tauri/src/settings.rs`）

- 每类连接器一个 id 集合 + 一个“已配置”判定：
  - Email：`EMAIL_CONNECTOR_SKILL_ID = "himalaya"`；`email_connector_configured(accounts) = !accounts.is_empty()`
  - Obsidian：`OBSIDIAN_CONNECTOR_SKILL_IDS = ["obsidian-markdown","obsidian-bases","json-canvas","obsidian-cli"]`；`obsidian_connector_configured(vault_path) = !vault_path.trim().is_empty()`
- 三个判定函数按连接器就绪状态收敛：
  - `skill_connector_satisfied(skill_id, email_accounts, obsidian_vault_configured) -> bool`
  - `skill_globally_available(chat_tools, skill_id, email_accounts, obsidian_vault_configured)` = `is_skill_enabled && skill_connector_satisfied`
  - `skill_global_unavailable_error(chat_tools, skill_id, email_accounts, obsidian_vault_configured, skill_name)` — 返回人类可读原因（disabled / 具体连接器未配置）

**红线**：每新增一个连接器，就给这三个函数各加一个“该连接器已配置”的入参并加分支；不要用别的机制旁路。

## 必须穿透的门控调用点（编译器兜底：漏改即编译失败）

每处都能从 `settings` 就地求值 `obsidian_connector_configured(&settings.obsidian_vault_path)`：

| 位置 | 作用 |
|---|---|
| `chat/agent/prepare.rs` `skill_allowed_for_conversation` | 会话级：控制注入模型的 skill 目录 |
| `chat/agent/prepare.rs`（catalog 闭包） | 由 `obsidian_vault_path: Option<&str>` 折成 bool |
| `chat/commands.rs` `try_apply_skill_slash_trigger` / `resolve_forced_skill_id`（各自调用点透传） | 斜杠触发 / 强制 pin |
| `skills/mod.rs`（`chat_skills_list` 过滤 + `chat_skills_read`） | GUI skill 列表 / 读取 |
| `mcp/registry.rs` `call_skill_tool` | **模型激活 skill 的硬门**（`skill_activate` 实际走这里） |
| `kivio_code/executor.rs` | headless CLI 门控 |

## Vendored skill 落地约定

- 放 `src-tauri/resources/skills/<id>/`，经 `tauri.conf.json` `"resources/skills": "skills"` 递归打包；`references/` 会被索引为 `Reference`，可 `skill_read_file`。
- SKILL.md 显式写 `id: <folder>`，与文件夹名一致（否则 `parse_skill_record` 报 folder-mismatch warning；且 id 必须落在对应连接器 id 集合内）。
- 第三方来源保留许可（如 `resources/skills/NOTICE.md` 存 MIT + 版权）。

## Dev 环境陷阱（实测踩坑）

`tauri dev` 下 `resource_dir()` = `target/debug`，`resources/skills` 是**构建时快照**复制到 `target/debug/skills`。**增量 `cargo watch` 重编译不会重跑资源复制**，新增 bundled skill 在 dev 里会“Skill not found”，直到完整重建或手动同步 `target/debug/skills`。打包构建（`tauri build`）无此问题。验证 dev 行为前先确认 `target/debug/skills` 已含新 skill。

来源：07-04-obsidian-connector-skill
