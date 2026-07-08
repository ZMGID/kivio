# 技能市场 — 技术设计

## 数据契约

### 索引 JSON（远程，货源提供）
```jsonc
{
  "version": 1,                       // 索引 schema 版本
  "skills": [
    {
      "id": "pdf-tools",              // 必须与 SKILL.md 解析出的 skill id 一致（安装匹配靠它）
      "name": "PDF 工具",
      "description": "…",
      "author": "…",                  // 可选
      "version": "1.2.0",             // 语义/字符串版本，用于更新检测
      "category": "文档",              // 可选，用于分类筛选
      "tags": ["pdf", "office"],      // 可选
      "downloadUrl": "https://…/pdf-tools.zip",  // 必须，zip 内含 SKILL.md
      "iconUrl": "https://…/pdf.png", // 可选，卡片图标
      "previewUrl": "https://…/x.png",// 可选，卡片截图预览
      "homepage": "https://…"         // 可选
    }
  ]
}
```
> `id` 必须与 zip 内 `SKILL.md` frontmatter 的 id 对齐——否则装完后本地目录名与索引对不上，"已装/更新"检测会失效。校验放在安装成功后（用返回的 `SkillMeta.id` 回写 marker）。

### 本地安装标记 `.market.json`（写入 `{user_skills_dir}/{id}/.market.json`）
```json
{ "id": "pdf-tools", "version": "1.2.0", "indexUrl": "https://…/index.json", "installedAt": "2026-07-08T…Z" }
```

## 后端（src-tauri/src/skills/marketplace.rs，新文件）

结构体（serde，camelCase 对齐前端）：
- `MarketIndex { version: u32, skills: Vec<MarketSkill> }`
- `MarketSkill { id, name, description, author?, version, category?, tags, download_url, icon_url?, preview_url?, homepage? }`
- `MarketMarker { id, version, index_url, installed_at }`
- `MarketInstalledInfo { id, version }`（供前端算三态；也可前端读 marker，但集中在后端更稳）

命令（注册进 `lib.rs` invoke_handler）：
1. `chat_skills_market_fetch(index_url: String) -> Result<MarketFetchResult,String>`
   - 复用 `api` 的 reqwest client（或 `web_fetch` 的 client）GET；`serde_json` 解析。
   - 5 分钟内存缓存：`Mutex<Option<(url, Instant, MarketIndex)>>` on AppState 或模块 static（用 `once_cell`）。
   - 同时扫描已装 marker，返回 `installed: Vec<MarketInstalledInfo>`，前端一次拿全算三态。
2. `chat_skills_market_install(app, skill: MarketSkill, index_url: String) -> SkillImportResult`
   - reqwest GET `download_url` → bytes。
   - **复用** zip 落盘：把 `import_skill_zip` 的解压核心抽成 `install_skill_zip_bytes(bytes, skills_dir) -> Result<SkillMeta>`（现 `import_skill_zip` 改为读文件后调它）。
   - 装到临时目录再原子 rename 进 `user_skills_dir/{id}`——失败不留半个目录（满足 A6）。若已存在同 id 先删（更新场景）。
   - 成功后写 `.market.json`（用返回 meta.id + skill.version + index_url）。
   - 任何步骤失败 → `SkillImportResult{ success:false, error }`。

复用点：`import_skill_zip`（mod.rs:215）解压逻辑、`user_skills_dir`、`SkillImportResult` 类型。

## 设置

`ChatToolsConfig` 增 `skillMarket?: { indexUrl?: string }`（settings.rs + tauri.ts 类型）。默认 `indexUrl` 空。设置面板技能相关处加一个输入框（或在 SkillCenter 市场页顶部内联可编辑）。为省事：**索引地址输入内联在市场页顶部**，改动即存 settings，不单独做设置分区。

## 前端

### tauri.ts
- 类型：`MarketSkill` / `MarketFetchResult` / `MarketInstalledInfo`。
- 绑定：`chatSkillsMarketFetch(indexUrl)`、`chatSkillsMarketInstall(skill, indexUrl)`。

### SkillCenter.tsx
- 顶部加「已安装 / 市场」两态切换（segmented）。市场态渲染 `<SkillMarket>`。
- 新组件 `SkillMarket.tsx`：
  - 顶部：索引地址内联输入（空则引导填写）、搜索框、分类下拉。
  - 卡片网格：iconUrl/previewUrl（无则占位）、name、author、description、category/tags chip、右下角按钮（安装/已安装/更新）。
  - 三态由 fetch 返回的 installed 列表算：未装=「安装」实心按钮；已装且版本同=「已安装」禁用；已装但版本不同=「更新」。
  - 点安装/更新 → 确认弹窗（复用现有轻量确认；无则内联 confirm 区）→ 调 install → 成功后重拉 fetch + `onSkillsChanged`。
- 确认弹窗：技能名 + 来源域名 + "该技能含可运行脚本，激活时可能在本机执行代码，仅从可信来源安装" 文案 + 取消/确认。

## 安全 / 边界

- 安装是唯一写盘点，前置确认弹窗（R6）是信任闸。
- zip 解压已有 `..` 路径穿越防护（mod.rs:252），bytes 版沿用。
- 下载设超时（复用 api client 的超时）；大文件不做流式（技能包小），但设合理上限（如 50MB）拒绝超大响应。
- 断网/坏 JSON/坏 zip → 可读错误，临时目录清理。

## 测试

- Rust：`install_skill_zip_bytes` 用内存造一个含 SKILL.md 的 zip，断言落盘 + marker 写入；坏 zip 断言 Err 且不留目录。
- 前端：`SkillMarket` 三态计算纯函数（`computeSkillState(indexSkill, installed)`）单测。
