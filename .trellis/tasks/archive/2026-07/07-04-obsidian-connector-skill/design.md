# Design — Obsidian 连接器内置 skill

## 架构总览

两块独立又协作的改动：

1. **内容侧（vendored skills）**：把 kepano/obsidian-skills 的 4 个 skill 文件夹放进 `src-tauri/resources/skills/`，随 app 打包，被现有 `skills/discover.rs` 自动发现。零 Rust 逻辑改动即可被列出/读取——除非被门控隐藏。
2. **门控侧（connector gate）**：复用 himalaya 的连接器门控机制，新增"Obsidian 连接器所属 skill 集合"，用 `obsidian_vault_path` 非空作为满足条件，穿过全部门控函数与调用点。

## 内容侧

### 目录布局（新增）

```
src-tauri/resources/skills/
  obsidian-markdown/
    SKILL.md
    references/{CALLOUTS.md, EMBEDS.md, PROPERTIES.md}
  obsidian-bases/
    SKILL.md
    references/FUNCTIONS_REFERENCE.md
  json-canvas/
    SKILL.md
    references/EXAMPLES.md
  obsidian-cli/
    SKILL.md
  NOTICE.md          # MIT 归属：© 2026 Steph Ango (@kepano) + 许可证全文
```

### 获取与改编

- 用 `curl` 从 `raw.githubusercontent.com/kepano/obsidian-skills/main/skills/<id>/...` 抓取原始文件（WebFetch 会截断，必须用 curl 落地原文）。
- 每个 `SKILL.md` frontmatter 显式加 `id: <folder>`（`obsidian-markdown` / `obsidian-bases` / `json-canvas` / `obsidian-cli`），确保 id 稳定且与文件夹名一致，避免 `parse_skill_record` 的 folder-mismatch warning。
- `name` / `description` 保留上游英文（模型面向，触发匹配依赖描述；与既有 bundled skill 如 frontend-design/mcp-builder 一致均为英文）。
- 正文与 references 基本原样保留（技术语法参考），仅在必要处让措辞与 Kivio 环境自洽（如提到"用文件工具读写 vault"时对齐 read_file/write_file/edit_file/glob_files/search_files）。**不改变** obsidian-cli 对外部 `obsidian` CLI 的依赖描述——保留 `obsidian help` 等原文，作为能力文档。
- 许可证：`skills/NOTICE.md` 保留 MIT 版权与许可全文（满足"substantial portions 需含版权声明"）。

### 打包

`tauri.conf.json:42` 已有 `"resources/skills": "skills"`，递归复制含子目录，无需改配置。references/ 会被 `index_skill_files` 索引为 `Reference`，模型可 `skill_read_file` 渐进读取。

## 门控侧

### 常量与判定（`settings.rs`）

```rust
/// Obsidian 连接器所属 bundled skill——vault 未配置时隐藏。
pub const OBSIDIAN_CONNECTOR_SKILL_IDS: &[&str] =
    &["obsidian-markdown", "obsidian-bases", "json-canvas", "obsidian-cli"];

pub fn obsidian_connector_configured(vault_path: &str) -> bool {
    !vault_path.trim().is_empty()
}
```

### 函数签名扩展（`settings.rs`）

统一在既有 `email_accounts` 之后追加一个 `obsidian_vault_configured: bool`：

```rust
pub fn skill_connector_satisfied(
    skill_id: &str,
    email_accounts: &[EmailAccountConfig],
    obsidian_vault_configured: bool,
) -> bool {
    if skill_id == EMAIL_CONNECTOR_SKILL_ID {
        return email_connector_configured(email_accounts);
    }
    if OBSIDIAN_CONNECTOR_SKILL_IDS.contains(&skill_id) {
        return obsidian_vault_configured;
    }
    true
}

pub fn skill_globally_available(chat_tools, skill_id, email_accounts, obsidian_vault_configured) -> bool { ... }

pub fn skill_global_unavailable_error(chat_tools, skill_id, email_accounts, obsidian_vault_configured, skill_name) -> Option<String> {
    // 新增分支：连接器未满足且属于 Obsidian 集合 → "Skill requires a configured Obsidian connector: {name}"
}
```

设计取舍：沿用"逐个基础类型参数"而非引入 `ConnectorContext` 结构体，与现有 `email_accounts: &[...]` 传参风格一致，改动可预测；调用点都能就地从 `settings.obsidian_vault_path` 求值。

### 包装函数（`chat/agent/prepare.rs`）

`skill_allowed_for_conversation(chat_tools, assistant, skill_id, email_accounts)` 追加 `obsidian_vault_configured: bool`，透传给 `skill_globally_available`。

### 调用点改造（各处从 settings 求值 `obsidian_connector_configured(&settings.obsidian_vault_path)`）

| 文件:行 | 函数 | 改动 |
|---|---|---|
| `settings.rs` 核心 3 函数 | — | 加参数 + Obsidian 分支 |
| `chat/agent/prepare.rs:111` | `skill_allowed_for_conversation` | 加参数、透传 |
| `chat/agent/prepare.rs:553` | 调用点 | 传入 vault 状态 |
| `chat/commands.rs:3425, 3462` | 调用 `skill_allowed_for_conversation` | 传入 vault 状态 |
| `skills/mod.rs:48` | `chat_skills_list` 过滤 | 传入 `obsidian_connector_configured(&settings.obsidian_vault_path)` |
| `skills/mod.rs:81` | `chat_skills_read` | 同上 |
| `mcp/registry.rs:553` | skill 激活门控 | 同上 |
| `kivio_code/executor.rs:269` | headless 门控 | headless settings 亦有 `obsidian_vault_path` |

### 系统提示衔接（`prepare.rs` vault 注入块，本任务前已加基本操作指引）

在 vault 路径注入文本尾部补一句：提示模型可激活 `obsidian-markdown` / `obsidian-bases` / `json-canvas` / `obsidian-cli` skill 获取 Obsidian 语法与操作细节（对齐 himalaya 提示"activate the himalaya skill"的做法）。仅当 vault 已配置时该块才出现，天然与门控一致。

## 数据流

配置 vault → `settings.obsidian_vault_path` 非空 → 各门控点 `obsidian_connector_configured()==true` → 4 个 skill 通过 `skill_connector_satisfied` → 出现在 `chat_skills_list` / 注入模型 skill 列表 / 可激活可读 references。未配置 → 全部隐藏 + 激活/读取报"需配置 Obsidian 连接器"。

## 兼容性 / 回滚

- 纯增量：新增 skill 文件夹 + 门控分支；不动既有 himalaya/pdf 等路径（新分支只对 Obsidian id 生效，默认 `true`）。
- 函数签名变更是编译期强约束——漏改调用点会编译失败，不会静默错行为。
- 回滚：删除 4 个 skill 文件夹 + 还原门控函数签名即可；无持久化 schema 变更。

## 风险

- **签名扩散**：核心 3 函数 + 包装函数共 ~8 个调用点，需全部更新（编译器兜底）。
- **obsidian-cli 外部依赖**：无 `obsidian` CLI 时模型调用会失败——由 skill 正文说明其前置条件缓解，属能力文档，非本任务保证可运行。
- **打包核对**：release 需确认 4 个新 skill 进入安装产物（RELEASE_PACKAGING.md 增核对项）。
