# Implement — 重做内置专家套件

## 执行顺序

### 步骤 A — 定义重写（`chat/storage.rs::builtin_assistant_definitions`）
1. 重写 writer/coder/researcher/data 的 system_prompt（专业化，统一五段结构）；coder 的 skill_ids 改为 `[diagram]`，data 加 `diagram`。
2. 新增 3 条：`asst_builtin_frontend`(frontend-design,diagram)、`asst_builtin_translator`(docx,pdf)、`asst_builtin_docsmith`(doc-coauthoring,docx,xlsx,pdf,diagram)。
- 验证：`cargo build`；更新 `builtin_assistant_tests`（`storage.rs:1522`）断言数量=7、id/skill 合法。

### 步骤 B — 非破坏 v2 迁移
3. `settings.rs`：加 `builtin_assistants_seeded_v2: bool`（字段 + default_false）。
4. `chat/storage.rs`：加 `merge_builtin_assistants_v2(app, now)`：load → 按 id upsert 内置定义 → 保留其它 → save。加单测：预置「1 内置旧版 + 1 用户自建」→ merge 后「内置被更新、用户自建保留、新增内置出现」。
5. `lib.rs`（v1 块之后）：加 v2 守卫块，成功置 flag+persist、失败回滚。
- 验证：`cargo test --manifest-path src-tauri/Cargo.toml --lib chat::storage`（对照 CLAUDE.md 基线）。

### 步骤 C — 前端图标
6. `src/chat/assistantIcons.tsx`（`builtinAssistantGlyph`）补 frontend/translator/docsmith → 🎨/🌐/📄。
- 验证：`npm run typecheck && npm run lint`。

### 步骤 D — 联调
7. 手测：全新 profile（或临时清 flag）→ 7 个专家显示正常；模拟老用户（置 v1=true,v2=false + 造一个用户自建）→ 重启后新内置到位、自建保留。

## 验证命令
- `cargo test --manifest-path src-tauri/Cargo.toml --lib chat::storage`
- `npm run typecheck && npm run lint`
- 手测新装 / 老用户升级两条路径（AC1/AC2/AC4）

## 风险 / 回滚点
- `storage.rs` 大文件：改动限定 `builtin_assistant_definitions` + 新 merge fn + 测试。
- 迁移出错不置 flag、下次重试；merge 保留用户项，故失败不丢数据。
- A/B/C 三步相对独立可分别 revert。
</content>
