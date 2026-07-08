# 技能市场 — 执行计划

## 顺序（后端 → 绑定 → 前端 → 测试）

1. **后端 marketplace.rs**
   - [ ] 抽 `install_skill_zip_bytes(bytes, skills_dir) -> Result<SkillMeta>`；`import_skill_zip` 改为读文件后调它（不改现有行为）。
   - [ ] 结构体 `MarketIndex/MarketSkill/MarketMarker/MarketInstalledInfo` + serde。
   - [ ] `chat_skills_market_fetch`（GET+解析+5min 缓存+扫描已装 marker）。
   - [ ] `chat_skills_market_install`（下载→临时目录解压→原子 rename→写 `.market.json`；失败清理）。
   - [ ] `lib.rs` 注册两个命令。
   - 验证：`cargo build --manifest-path src-tauri/Cargo.toml`。

2. **设置字段**
   - [ ] settings.rs：`ChatToolsConfig` 加 `skill_market: Option<SkillMarketConfig{ index_url: Option<String> }>`（serde default）。
   - [ ] tauri.ts：对应类型 + 默认。

3. **前端绑定**
   - [ ] tauri.ts：`MarketSkill/MarketFetchResult/MarketInstalledInfo` 类型 + `chatSkillsMarketFetch/chatSkillsMarketInstall`。

4. **前端 UI**
   - [ ] `computeSkillState(indexSkill, installed)` 纯函数 + 单测。
   - [ ] `SkillMarket.tsx`：索引地址内联输入、搜索、分类下拉、卡片网格、三态按钮、确认弹窗。
   - [ ] SkillCenter.tsx：加「已安装/市场」切换，市场态挂 `<SkillMarket>`；安装成功回调刷新本地列表。

5. **校验**
   - [ ] `npm run lint` / `npm run typecheck` / `npm test`。
   - [ ] Rust 单测（zip bytes 落盘 / 坏 zip）。
   - [ ] 手动：填一个测试索引 URL，走完 拉取→确认→安装→已装→更新 全流程。

## 验证命令
- 前端：`npm run lint && npm run typecheck && npm test`
- Rust：`cargo build --manifest-path src-tauri/Cargo.toml`；单测走 `scripts/win-cargo-test.ps1`（Windows 直跑 cargo test 二进制会 0xC0000139）。

## 回滚点
- 后端命令未注册前，前端不受影响。
- 每步独立可编译；前端 UI 是新增组件 + SkillCenter 一处切换，回退只删组件 + 还原切换。

## 风险
- 索引 `id` 与 zip 内 SKILL.md id 不一致 → 更新检测失效。缓解：以安装返回的真实 meta.id 写 marker，前端三态以 marker.id 匹配。
- 货源为空（用户没配 URL）：市场页引导填写，不报错。
