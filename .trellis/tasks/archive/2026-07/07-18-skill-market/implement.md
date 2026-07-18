# Implement — Skill 市场

## 顺序清单

### 1. 后端命令
- [ ] `skills/mod.rs`：新增 `pub fn chat_skills_install_from_url(app, url) -> SkillImportResult`
  - URL 归一：github 仓库 → codeload zip；其它原样。
  - reqwest GET（复用 `crate::api` 客户端 / system_proxy blocking client）+ 大小上限 50MB + 60s 超时。
  - `install_skill_zip_bytes(bytes, &user_skills_dir(&app))` → 组 `SkillImportResult`。
  - 单元测试：URL 归一函数（github/tree/裸仓库/直链 zip）留一个 `#[test]`。
- [ ] `lib.rs`：`invoke_handler` 注册 `chat_skills_install_from_url`。
- [ ] `api/tauri.ts`：加 `chatSkillsInstallFromUrl(url)` binding（返回同 `chatSkillsImport` 结构）。

### 2. 前端数据层
- [ ] `src/settings/skillMarket.ts`：移植 clawHub.ts（list/search/normalize/resolveOwner/downloadUrl）。
- [ ] `src/settings/skillMarket.test.ts`：list 归一、owner 消歧（多候选按 version/downloads 收窄）、downloadUrl 构造 —— 3~4 个 vitest，用 fakeFetch。

### 3. 前端 UI
- [ ] `src/settings/SkillMarketModal.tsx`：ClawHub tab（排序+搜索+卡片+翻页+安装）/ URL tab（输入+安装）。
- [ ] `SettingsShell.tsx` 技能页加"技能市场"按钮 + 渲染 modal，`onInstalled`→`refreshChatSkills()`。

## 验证命令
```
npx vitest run src/settings/skillMarket.test.ts
npx tsc --noEmit            # 期望 0 error
npx eslint src/settings/skillMarket.ts src/settings/SkillMarketModal.tsx src/settings/skillMarket.test.ts --max-warnings 0
cargo build --manifest-path src-tauri/Cargo.toml --bin kivio 2>&1 | tail    # 编译过
cargo test --manifest-path src-tauri/Cargo.toml url_normaliz 2>&1 | tail    # URL 归一测试
```

## Review gates
- 2 后端命令写完 → 先 `cargo build` 过再接前端。
- 前端三件写完 → tsc/lint/vitest 全绿。
- 全部完成 → `npm run dev` 手动冒烟：ClawHub 装 1 个（重名 slug 消歧）、GitHub URL 装 1 个、.zip URL 装 1 个。

## Rollback
- 纯新增：删 2 前端文件 + 回退 `skills/mod.rs`/`lib.rs`/`tauri.ts`/`SettingsShell.tsx` 的新增行即可，无数据迁移。
