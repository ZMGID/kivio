# PRD — Skill 市场（ClawHub 目录 + URL 安装）

## 背景
Kivio 现有 Skill 只能从本地目录/zip 手动导入（设置→技能页的 `chatSkillsImport`）。
参照 LiveAgent 的 Skills Hub，补一个"技能市场"：既能浏览远程 ClawHub 目录一键安装，
也能粘贴 GitHub 仓库 / zip URL 安装。承接上一轮已完成的 MCP 市场（`0c0ce05`）的模式。

## 目标（本轮范围）
1. **ClawHub 目录**：浏览 / 排序 / 搜索 clawhub.ai 的 skill，一键下载安装。
2. **URL 安装**：粘贴 GitHub 仓库 URL 或直链 zip URL 安装。
3. 安装落盘复用现有 `install_skill_zip_bytes`，装完刷新技能列表，与手动导入的 skill 无差别。

## 非目标（本轮不做）
- 不做 skill 的发布/上传、评分、评论。
- 不做异步进度任务 + 取消（LiveAgent 有 install_start/status/cancel）——skill 包小、单用户本地操作，
  一次阻塞下载即可。若将来包体变大再加。（ponytail）
- 不做 skill 名兼容归一 / `_meta.json` 溯源（LiveAgent 有）——Kivio 解析以 SKILL.md 的 id 为准，暂不需要。
- 不做外部 CLI（claude-code/cursor）skill 目录扫描导入。
- 不动 MCP 市场。

## 已确认的技术事实（研究结论）
- ClawHub 列表 `/api/v1/skills?sort=&limit=&cursor=&nonSuspiciousOnly=true`、
  搜索 `/api/v1/search?q=` 均返回 `Access-Control-Allow-Origin: *` → **前端可直接 fetch 浏览**。
- 下载 `/api/v1/download?slug=&tag=latest&ownerHandle=` 返回 `application/zip`（CORS `*`）。
- **重名 slug 不带 ownerHandle 会 409 "Ambiguous"**；列表卡片常缺 ownerHandle，需 search 消歧补齐（同 LiveAgent）。
- 下载走 Rust（复用 reqwest + `install_skill_zip_bytes`），避免二进制/重定向边界问题；浏览走前端。
- `install_skill_zip_bytes(bytes, skills_dir)` 已存在：找首个 SKILL.md、按其目录前缀解压到 `{skills_dir}/{id}`、
  覆盖旧目录、失败清理。GitHub 仓库 zip 的顶层 `repo-ref/` 前缀会被自动 strip。

## 验收标准
- [ ] 设置→技能页出现"技能市场"入口，打开后有 ClawHub 与 URL 两种方式。
- [ ] ClawHub：能按 downloads/stars/installs/updated/newest 排序浏览、能搜索、能翻页。
- [ ] 点安装：重名 slug 能自动消歧 owner 并成功下载安装；装完列表出现该 skill 且可启用。
- [ ] URL：粘贴一个公开 GitHub 仓库 URL（含 SKILL.md）能装成功；粘贴直链 .zip 能装成功。
- [ ] 消歧失败 / 下载失败 / zip 无 SKILL.md 时有明确错误提示，不留半个技能目录。
- [ ] `npm run lint` / `npm run typecheck` / 新增单测通过；`cargo` 编译通过。
- [ ] 手动冒烟：dev 下三种安装路径各跑一次。
