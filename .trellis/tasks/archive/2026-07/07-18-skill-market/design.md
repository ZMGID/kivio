# Design — Skill 市场

## 架构：浏览走前端，下载装载走 Rust
沿用 MCP 市场的分层，但因 skill 是真实文件、下载是 zip 二进制，落盘必须走 Rust。

```
[SkillMarketModal.tsx]  ClawHub 浏览/搜索/翻页/消歧 owner (前端 fetch, CORS *)
        │  produce downloadUrl (ClawHub 带 owner) 或 用户粘贴的 URL
        ▼
api.chatSkillsInstallFromUrl(url)  ── Tauri command ──▶ [skills/mod.rs]
        │                                                   reqwest 下载 bytes
        │                                                   (github 仓库 URL → codeload zip 重写)
        ▼                                                   install_skill_zip_bytes(bytes, user_skills_dir)
   刷新技能列表 (refreshChatSkills)                          返回 SkillImportResult (复用)
```

## 契约

### 后端新增 1 个命令
`chat_skills_install_from_url(app, url: String) -> SkillImportResult`（`skills/mod.rs`）
- 归一 URL：
  - `github.com/{owner}/{repo}`（可带 `/tree/{ref}`）→ `https://codeload.github.com/{owner}/{repo}/zip/{ref|HEAD}`。
  - 其它（含 clawhub 下载链、直链 .zip）→ 原样。
- 用 `crate::api` 现有的 reqwest 客户端 GET（follow redirects、带 UA、超时 60s）；
  校验 `content-length`/大小上限（复用或新增常量，≤ 50MB）。
- `install_skill_zip_bytes(bytes, &user_skills_dir(&app))`。
- 错误串行返回到 `SkillImportResult.error`（已有结构）。
- 注册：`lib.rs` 的 `invoke_handler![]` 加该命令；`api/tauri.ts` 加 binding。

### 前端数据层 `src/settings/skillMarket.ts`（移植 LiveAgent clawHub.ts，精简）
- 类型 `ClawHubSkillCard`（slug/displayName/summary/downloads/stars/installs/updatedAt/ownerHandle/downloadUrl）。
- `listClawHubSkills({sort,cursor,limit})` → `/api/v1/skills`。
- `searchClawHubSkills({query,limit})` → `/api/v1/search`。
- `resolveClawHubSkillOwner(card)`：card 已带 owner 直接返回；否则 `searchClawHubSkills(slug)` +
  `selectClawHubOwnerCandidate`（按 updatedAt/version/downloads/summary 逐步收窄），拿不到唯一 owner 抛错。
- `buildClawHubDownloadUrl(slug, owner)` → `/api/v1/download?slug=&tag=latest&ownerHandle=`。
- 归一函数照搬（字段兜底 stats.*）。

### 前端 UI `src/settings/SkillMarketModal.tsx`
- 复用 `.kv-modal-backdrop/.kv-modal`、Button/IconButton/Input（同 McpMarketModal）。
- 两个 tab：
  - **ClawHub**：排序下拉（5 选项）+ 搜索框（防抖）+ 卡片列表（名称/摘要/下载·星标数）+ 翻页；
    每卡"安装"→ `resolveClawHubSkillOwner` → `buildClawHubDownloadUrl` → `chatSkillsInstallFromUrl` → 成功打勾。
  - **URL**：一个输入框 + 安装按钮，直接 `chatSkillsInstallFromUrl(url)`。
- props：`{ lang, onInstalled: () => void, onClose }`；安装成功回调父级刷新列表。

### 接线
`SettingsShell.tsx` 技能页现有"导入 Skill / 导入 zip"按钮旁加"技能市场"按钮 → 打开
`SkillMarketModal`，`onInstalled` 调 `refreshChatSkills()` + `onSettingsChange()`（与现有导入一致）。

## 取舍 / 风险（ponytail）
- **owner 消歧失败**：多个发布者且元数据无法区分时 `resolveClawHubSkillOwner` 抛错，UI 提示"该 skill 有多个发布者，无法自动确定"。可接受，同 LiveAgent。
- **一个仓库多个 skill**：`install_skill_zip_bytes` 只装首个 SKILL.md。v1 接受，UI 文案注明"安装仓库内第一个技能"。升级路径：Rust 扫描所有 SKILL.md 批量装。
- **非原子安装**：`install_skill_zip_bytes` 已是"清旧目录→解压→失败清理"，非 temp+rename。单用户本地、包小，可接受（该函数注释已标注升级路径）。
- **同步阻塞下载**：不做进度条/取消。50MB 上限 + 60s 超时兜底。

## 兼容性
- 不改任何现有类型/命令/settings 结构；纯新增 1 命令 + 2 前端文件 + 1 按钮。
- 装出来的 skill 与本地导入完全同构（都过 `install_skill_zip_bytes` / `import_skill_dir`）。
