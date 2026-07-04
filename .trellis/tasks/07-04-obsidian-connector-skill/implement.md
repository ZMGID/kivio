# Implement — Obsidian 连接器内置 skill

上下文加载顺序：本文件 + `prd.md` + `design.md`。Windows 下 Rust 测试用 `scripts/win-cargo-test.ps1`（见记忆：plain `cargo test` 二进制启动失败 0xC0000139）。

## 执行清单（有序）

### A. Vendor 4 个 skill 内容

- [ ] A1 用 curl 抓取 4 个 skill 的原始文件到临时目录：
  - `obsidian-markdown/SKILL.md` + `references/{CALLOUTS,EMBEDS,PROPERTIES}.md`
  - `obsidian-bases/SKILL.md` + `references/FUNCTIONS_REFERENCE.md`
  - `json-canvas/SKILL.md` + `references/EXAMPLES.md`
  - `obsidian-cli/SKILL.md`
  - 抓取模板：`curl -s https://raw.githubusercontent.com/kepano/obsidian-skills/main/skills/<id>/<file>`
- [ ] A2 在 `src-tauri/resources/skills/<id>/` 下落地各文件（用 Write 写入抓取到的原文）。
- [ ] A3 每个 `SKILL.md` frontmatter 显式加 `id: <folder>`；核对 `name`/`description` 非空。
- [ ] A4 写 `src-tauri/resources/skills/NOTICE.md`：MIT 全文 + `© 2026 Steph Ango (@kepano)` + 来源 URL。
- [ ] A5 obsidian-cli / bases / canvas / markdown 正文中若出现与 Kivio 环境冲突的操作措辞，最小化对齐（提到读写 vault 用 read_file/write_file/edit_file/glob_files/search_files）；保留 obsidian-cli 的外部 CLI 依赖说明原样。

### B. 门控核心（`src-tauri/src/settings.rs`）

- [ ] B1 新增 `OBSIDIAN_CONNECTOR_SKILL_IDS` 常量与 `obsidian_connector_configured(&str)->bool`。
- [ ] B2 `skill_connector_satisfied` 加 `obsidian_vault_configured: bool` 参数 + Obsidian 分支。
- [ ] B3 `skill_globally_available` 加参数、透传。
- [ ] B4 `skill_global_unavailable_error` 加参数 + "requires a configured Obsidian connector" 分支。

### C. 门控调用点透传

- [ ] C1 `chat/agent/prepare.rs:111` `skill_allowed_for_conversation` 加 `obsidian_vault_configured` 参数、透传给 `skill_globally_available`。
- [ ] C2 `chat/agent/prepare.rs:553` 调用点传入 vault 状态（该处能拿到 settings / obsidian_vault_path）。
- [ ] C3 `chat/commands.rs:3425` 与 `:3462` 两处 `skill_allowed_for_conversation` 调用传入 vault 状态。
- [ ] C4 `skills/mod.rs:48`（list 过滤）、`:81`（read 门控）传入 `obsidian_connector_configured(&settings.obsidian_vault_path)`。
- [ ] C5 `mcp/registry.rs:553` 传入 vault 状态（确认该上下文的 settings 来源）。
- [ ] C6 `kivio_code/executor.rs:269` 传入 headless settings 的 vault 状态。

### D. 系统提示衔接（`chat/agent/prepare.rs` vault 注入块）

- [ ] D1 在已加的基本操作指引尾部补一句：可激活 obsidian-markdown / obsidian-bases / json-canvas / obsidian-cli skill 获取 Obsidian 语法/操作细节（zh + en 两版）。核对既有测试 `chat_prompt_includes_obsidian_vault_path` 仍含断言子串。

### E. 测试

- [ ] E1 `settings.rs` 新增：`skill_globally_available_hides_obsidian_without_vault`（无 vault → 4 个 id 全 false；有 vault → 全 true；pdf 不受影响）。
- [ ] E2 `settings.rs` 新增：`skill_global_unavailable_error` 对 Obsidian id 在无 vault 时返回连接器错误、有 vault 时返回 None。
- [ ] E3 更新既有测试签名：所有 `skill_globally_available` / `skill_connector_satisfied` / `skill_global_unavailable_error` / `skill_allowed_for_conversation` 的调用（含 prepare.rs 测试 1183/1190/1197/1203/1222、settings.rs 3657/3670）补齐新参数。

### F. Release 文档

- [ ] F1 `docs/RELEASE_PACKAGING.md` 第 72-74 行的产物核对列表追加 `skills/obsidian-markdown|obsidian-bases|json-canvas|obsidian-cli/SKILL.md`；注明这些 skill 不依赖 Pyodide。

## 验证命令

- [ ] `cd src-tauri && cargo check --no-default-features`（快速编译校验）
- [ ] Rust 测试：`powershell -File scripts/win-cargo-test.ps1`（对齐记忆中的 Windows 跑法；与 HEAD 基线比较，忽略既有 ~14 个 --lib 环境类失败）
- [ ] 若动到前端：`npm run lint && npm run typecheck`
- [ ] dev app 手动 smoke：未配置 vault → chat skill 列表无 obsidian-*；配置后 → 4 个出现且可激活。

## 风险与回滚点

- 风险：签名扩散漏改调用点 → 编译失败即暴露（安全）。
- 风险：obsidian-cli 无外部 CLI 时不可运行 → 属能力文档，PRD Out of Scope 已声明。
- 回滚：删 `resources/skills/{obsidian-markdown,obsidian-bases,json-canvas,obsidian-cli,NOTICE.md}` + `git checkout` 门控相关文件；无 schema/持久化变更。
