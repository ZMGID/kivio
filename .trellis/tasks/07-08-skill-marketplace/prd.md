# 技能市场（Skill Marketplace）

## 背景

技能系统目前纯本地：技能是 `user_skills_dir` 下的目录（`SKILL.md` + 附带文件），只能从本地文件夹/zip 导入（`chat_skills_import`）。用户希望在技能页面接入一个"市场"，可浏览远程技能目录并一键安装。

已确认的产品决策：
- **货源**：可配置的 JSON 索引 URL（设置项，默认可空/占位；之后可指向自建或 Anthropic 仓库），代码不写死来源。
- **信任**：安装前弹确认框，说明来源 + 含可运行脚本的风险，确认后才下载落盘。
- **v1 范围（全都要）**：市场浏览 + 安装 + 已装标记；分类/标签筛选；更新检测；卡片截图/预览。

## Requirements

- R1 设置新增「技能市场索引地址」(`chatTools.skillMarket.indexUrl`)；空则市场页提示去配置。
- R2 后端 `chat_skills_market_fetch(indexUrl)`：GET 索引 JSON、解析为结构化列表；网络/解析错误返回可读 error（不 panic）。短期内存缓存（如 5 分钟）避免重复拉取。
- R3 后端 `chat_skills_market_install(skill)`：下载 `downloadUrl` 的 zip → 复用 zip 落盘逻辑装入 `user_skills_dir` → 在技能目录写 `.market.json` 标记（id/version/indexUrl/installedAt）。返回 `SkillImportResult`。
- R4 已装 + 更新检测：扫描已装技能的 `.market.json`，与索引项按 `id` 匹配；`version` 不同 = 有更新。市场卡片显示三态：未安装 / 已安装 / 可更新。
- R5 SkillCenter 增加「市场」视图：卡片列表（图标或截图、名字、描述、作者、分类/标签 chip）、搜索、分类筛选、安装/更新按钮。
- R6 安装前确认弹窗：显示技能名、来源 URL、"含可运行脚本、激活时可能执行代码"的提示；确认后才安装。
- R7 安装/更新成功后刷新本地技能列表（复用现有 `onSkillsChanged` / 列表刷新）。

## Acceptance Criteria

- [x] A1 配置有效索引 URL 后，市场页能拉到并渲染技能卡片；URL 空或失败有明确提示，不白屏。（空态/错误态已实现，待真实索引目检）
- [x] A2 点安装先弹确认框；确认后技能下载并出现在本地列表，卡片转「已安装」。（代码完成，待端到端目检）
- [x] A3 索引里 version 高于本地 `.market.json` 记录时，卡片显示「可更新」，点更新后版本刷新。（computeSkillState + marker 已实现）
- [x] A4 分类筛选 + 搜索可用；无结果有空态。
- [x] A5 `npm run lint` / `npm run typecheck` / `npm test`（148）通过；Rust `install_skill_zip_bytes` 两个单测通过。
- [x] A6 断网/坏 URL/坏 zip 均降级为错误提示，不崩溃、不留半个技能目录。（坏 zip 单测证明无残留）

## 范围外

- 技能评分/评论、付费、账号体系。
- 服务端索引的托管与策展（属产品运营，非本任务；本任务只消费索引）。
- 自动更新（仅做"检测 + 手动点更新"）。
